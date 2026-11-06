//! Bounded, strict source admission. The desktop invokes this in a disposable
//! decoder process; imported projects continue to use canonical float WAVs.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom},
    path::Path,
};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    common::Limit,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::{MediaSourceStream, Monitor},
    meta::MetadataOptions,
};

pub const MAX_IMPORTED_AUDIO_BYTES: usize = 512 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedAudioInfo {
    pub sample_rate: u32,
    pub channels: usize,
    pub frames: u64,
    pub decoded_bytes: u64,
    pub source_format: String,
}

fn validate_spec(sample_rate: u32, channels: usize) -> Result<()> {
    ensure!(
        (8_000..=384_000).contains(&sample_rate),
        "Audio sample rate must be between 8000 and 384000 Hz"
    );
    ensure!(
        (1..=2).contains(&channels),
        "Only mono and stereo audio imports are supported"
    );
    Ok(())
}

/// Convert a source into a NEW managed WAV without modifying the original.
/// A limit, malformed packet, partial stream, nonfinite sample or verification
/// failure discards the staged output. This function is intended for a one-shot
/// child process so codec aborts and allocation failures cannot unwind into UI.
pub fn decode_audio_to_wav(
    input: &Path,
    output: &Path,
    max_decoded_bytes: usize,
) -> Result<DecodedAudioInfo> {
    ensure!(
        (1..=MAX_IMPORTED_AUDIO_BYTES).contains(&max_decoded_bytes),
        "Invalid decoded audio budget"
    );
    ensure!(
        !output.exists(),
        "Decoded output must be a new managed file"
    );
    let metadata = input.metadata().context("Read audio source metadata")?;
    ensure!(metadata.is_file(), "Choose an audio file");
    ensure!(
        metadata.len() <= MAX_IMPORTED_AUDIO_BYTES as u64,
        "Encoded audio exceeds the 512 MiB import limit"
    );
    ensure!(metadata.len() >= 4, "Audio file is empty or truncated");
    let mut file = File::open(input).context("Open audio source")?;
    let mut header = [0u8; 12];
    let count = file.read(&mut header)?;
    file.seek(SeekFrom::Start(0))?;
    let format = if count >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        let declared = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
        ensure!(declared <= metadata.len(), "WAV file is truncated");
        return decode_wav(file, output, max_decoded_bytes);
    } else if &header[..4] == b"fLaC" {
        "FLAC"
    } else if &header[..4] == b"OggS" {
        validate_ogg(&mut file)?;
        "OGG"
    } else if &header[..3] == b"ID3" || (header[0] == 0xff && header[1] & 0xe0 == 0xe0) {
        validate_mp3(&mut file)?;
        "MP3"
    } else {
        bail!("Unsupported or malformed audio; choose WAV, MP3, FLAC or OGG Vorbis");
    };
    decode_compressed(file, output, max_decoded_bytes, format)
}

/// MPEG readers can treat a partial last frame as ordinary EOF. Validate the
/// framing first so a truncated source is never imported as a shorter success.
/// Only tag headers and four-byte frame headers are retained in memory.
fn validate_mp3(file: &mut File) -> Result<()> {
    let length = file.metadata()?.len();
    let mut audio_end = length;
    if audio_end >= 128 {
        file.seek(SeekFrom::Start(audio_end - 128))?;
        let mut marker = [0; 3];
        file.read_exact(&mut marker)?;
        if &marker == b"TAG" {
            audio_end -= 128;
        }
    }
    if audio_end >= 32 {
        file.seek(SeekFrom::Start(audio_end - 32))?;
        let mut footer = [0; 32];
        file.read_exact(&mut footer)?;
        if &footer[..8] == b"APETAGEX" {
            let size = u64::from(u32::from_le_bytes(footer[12..16].try_into().unwrap()));
            ensure!(
                (32..=8 * 1024 * 1024).contains(&size) && size <= audio_end,
                "MP3 metadata is corrupt or too large"
            );
            audio_end -= size;
            // An optional APE header is outside the tag's declared footer size.
            if audio_end >= 32 {
                file.seek(SeekFrom::Start(audio_end - 32))?;
                let mut marker = [0; 8];
                file.read_exact(&mut marker)?;
                if &marker == b"APETAGEX" {
                    audio_end -= 32;
                }
            }
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut head = [0; 10];
    file.read_exact(&mut head)
        .context("MP3 header is truncated")?;
    let mut position = 0;
    if &head[..3] == b"ID3" {
        ensure!(
            (2..=4).contains(&head[3]) && head[6..10].iter().all(|byte| byte & 0x80 == 0),
            "MP3 metadata header is corrupt"
        );
        let size = head[6..10]
            .iter()
            .fold(0u64, |size, byte| (size << 7) | u64::from(*byte));
        ensure!(
            size <= 8 * 1024 * 1024,
            "MP3 metadata exceeds the 8 MiB import limit"
        );
        position = 10
            + size
            + if head[3] == 4 && head[5] & 0x10 != 0 {
                10
            } else {
                0
            };
        ensure!(position <= audio_end, "MP3 metadata is truncated");
    }
    let mut frames = 0;
    while position < audio_end {
        ensure!(audio_end - position >= 4, "MP3 frame header is truncated");
        file.seek(SeekFrom::Start(position))?;
        let mut header = [0; 4];
        file.read_exact(&mut header)?;
        let bits = u32::from_be_bytes(header);
        let version = (bits >> 19) & 3;
        let layer = (bits >> 17) & 3;
        let bitrate_index = ((bits >> 12) & 15) as usize;
        let rate_index = ((bits >> 10) & 3) as usize;
        ensure!(
            bits >> 21 == 0x7ff
                && version != 1
                && layer == 1
                && bitrate_index != 15
                && rate_index != 3,
            "MP3 frame header is corrupt or unsupported"
        );
        ensure!(bitrate_index != 0, "Free-format MP3 is not supported");
        let bitrate = if version == 3 {
            [
                0u32, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ][bitrate_index]
        } else {
            [
                0u32, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160,
            ][bitrate_index]
        };
        let rate = [44_100u32, 48_000, 32_000][rate_index]
            / if version == 3 {
                1
            } else if version == 2 {
                2
            } else {
                4
            };
        let frame_length = u64::from(
            (if version == 3 { 144_000 } else { 72_000 }) * bitrate / rate + ((bits >> 9) & 1),
        );
        ensure!(
            frame_length >= 4 && position + frame_length <= audio_end,
            "MP3 frame is truncated"
        );
        position += frame_length;
        frames += 1;
    }
    ensure!(frames > 0, "MP3 contains no complete audio frames");
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

struct CanonicalWriter {
    file: tempfile::NamedTempFile,
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    info: DecodedAudioInfo,
    limit: usize,
}

impl CanonicalWriter {
    fn new(output: &Path, rate: u32, channels: usize, format: &str, limit: usize) -> Result<Self> {
        validate_spec(rate, channels)?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let file =
            tempfile::NamedTempFile::new_in(parent).context("Create staged decoded audio")?;
        let writer = hound::WavWriter::new(
            BufWriter::new(file.reopen()?),
            hound::WavSpec {
                channels: channels as u16,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        Ok(Self {
            file,
            writer: Some(writer),
            info: DecodedAudioInfo {
                sample_rate: rate,
                channels,
                frames: 0,
                decoded_bytes: 0,
                source_format: format.into(),
            },
            limit,
        })
    }

    fn push(&mut self, sample: f32) -> Result<()> {
        ensure!(sample.is_finite(), "Audio contains non-finite samples");
        ensure!(
            self.info.decoded_bytes + 4 <= self.limit as u64,
            "Audio exceeds the decoded import budget"
        );
        self.writer
            .as_mut()
            .context("Decoded writer already finalized")?
            .write_sample(sample)?;
        self.info.decoded_bytes += 4;
        Ok(())
    }

    fn finish(mut self, output: &Path) -> Result<DecodedAudioInfo> {
        let frame_bytes = self.info.channels as u64 * 4;
        ensure!(
            self.info.decoded_bytes > 0 && self.info.decoded_bytes.is_multiple_of(frame_bytes),
            "Audio has no complete frames"
        );
        self.info.frames = self.info.decoded_bytes / frame_bytes;
        self.writer
            .take()
            .context("Decoded writer already finalized")?
            .finalize()?;
        self.file.as_file().sync_all()?;
        self.file
            .persist_noclobber(output)
            .map_err(|error| anyhow::anyhow!("Commit decoded audio: {}", error.error))?;
        Ok(self.info)
    }
}

fn decode_wav(file: File, output: &Path, limit: usize) -> Result<DecodedAudioInfo> {
    let mut reader = hound::WavReader::new(BufReader::new(file)).context("Read WAV header")?;
    let spec = reader.spec();
    validate_spec(spec.sample_rate, usize::from(spec.channels))?;
    ensure!(
        u64::from(reader.len()) * 4 <= limit as u64,
        "WAV exceeds the decoded import budget"
    );
    let declared = u64::from(reader.len());
    let mut writer = CanonicalWriter::new(
        output,
        spec.sample_rate,
        usize::from(spec.channels),
        "WAV",
        limit,
    )?;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            ensure!(
                spec.bits_per_sample == 32,
                "Only 32-bit float WAV is supported"
            );
            for sample in reader.samples::<f32>() {
                writer.push(sample.context("Decode WAV sample")?)?;
            }
        }
        hound::SampleFormat::Int => {
            ensure!(
                (1..=32).contains(&spec.bits_per_sample),
                "Unsupported WAV bit depth"
            );
            let scale = 2.0f64.powi(i32::from(spec.bits_per_sample) - 1);
            for sample in reader.samples::<i32>() {
                writer.push((f64::from(sample.context("Decode WAV sample")?) / scale) as f32)?;
            }
        }
    }
    ensure!(
        writer.info.decoded_bytes == declared * 4,
        "WAV file is truncated"
    );
    writer.finish(output)
}

fn decode_compressed(
    file: File,
    output: &Path,
    limit: usize,
    format_name: &str,
) -> Result<DecodedAudioInfo> {
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            source,
            FormatOptions::default(),
            MetadataOptions::default()
                .limit_tag_bytes(Limit::Maximum(64 * 1024))
                .limit_visual_bytes(Limit::Maximum(0)),
        )
        .context("Read compressed audio container")?;
    let track = format
        .default_track(TrackType::Audio)
        .context("Audio file has no decodable audio track")?;
    let track_id = track.id;
    let expected = track.num_frames;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .context("Audio codec is not supported")?;
    if let (Some(rate), Some(channels)) = (params.sample_rate, params.channels.as_ref()) {
        validate_spec(rate, channels.count())?;
        if let Some(frames) = expected {
            ensure!(
                frames
                    .checked_mul(channels.count() as u64 * 4)
                    .is_some_and(|bytes| bytes <= limit as u64),
                "Audio exceeds the decoded import budget"
            );
        }
    }
    ensure!(
        params
            .max_frames_per_packet
            .is_none_or(|frames| frames <= 262_144),
        "Audio packet declares too many frames"
    );
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(
            params,
            &AudioDecoderOptions::default().gapless(true).verify(true),
        )
        .context("Audio codec is not supported")?;
    let mut writer: Option<CanonicalWriter> = None;
    let mut samples = Vec::<f32>::new();
    while let Some(packet) = format
        .next_packet()
        .context("Read audio packet; the source may be corrupt or truncated")?
    {
        ensure!(
            packet.track_id == track_id,
            "Multiple or chained audio streams are not supported"
        );
        ensure!(
            packet.data.len() <= MAX_PACKET_BYTES,
            "Encoded audio packet exceeds the import limit"
        );
        let decoded = decoder
            .decode(&packet)
            .context("Decode audio packet; the source may be corrupt or truncated")?;
        let spec = decoded.spec();
        let rate = spec.rate();
        let channels = spec.channels().count();
        validate_spec(rate, channels)?;
        ensure!(
            decoded.frames() <= 262_144,
            "Decoded audio packet exceeds the import limit"
        );
        let count = decoded
            .frames()
            .checked_mul(channels)
            .context("Decoded audio size overflow")?;
        let current = writer
            .as_ref()
            .map_or(0, |writer| writer.info.decoded_bytes);
        ensure!(
            (count as u64)
                .checked_mul(4)
                .and_then(|bytes| bytes.checked_add(current))
                .is_some_and(|bytes| bytes <= limit as u64),
            "Audio exceeds the decoded import budget"
        );
        if writer.is_none() {
            writer = Some(CanonicalWriter::new(
                output,
                rate,
                channels,
                format_name,
                limit,
            )?);
        }
        let writer = writer.as_mut().unwrap();
        ensure!(
            writer.info.sample_rate == rate && writer.info.channels == channels,
            "Audio format changes inside the source"
        );
        samples.clear();
        samples
            .try_reserve_exact(count)
            .context("Allocate bounded audio packet")?;
        decoded.copy_to_vec_interleaved(&mut samples);
        ensure!(
            samples.len() == count,
            "Audio decoder returned an incomplete packet"
        );
        for sample in &samples {
            writer.push(*sample)?;
        }
    }
    ensure!(
        decoder.finalize().verify_ok != Some(false),
        "Decoded audio checksum verification failed"
    );
    let writer = writer.context("Audio contains no decoded frames")?;
    let frames = writer.info.decoded_bytes / (writer.info.channels as u64 * 4);
    if let Some(expected) = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)
        .and_then(|track| track.num_frames)
        .or(expected)
    {
        ensure!(
            frames == expected,
            "Audio is truncated or its declared duration is inconsistent"
        );
    }
    writer.finish(output)
}

/// Symphonia can recover/resynchronize damaged OGG pages. Imports instead
/// reject missing pages, bad CRCs and absent EOS explicitly, never silently
/// accepting only the undamaged portions of a source.
fn validate_ogg(file: &mut File) -> Result<()> {
    let mut reader = BufReader::new(&mut *file);
    let mut serial = None;
    let mut sequence = 0u32;
    let mut ended = false;
    let mut packet_bytes = 0usize;
    loop {
        let mut header = [0u8; 27];
        let n = reader.read(&mut header[..1])?;
        if n == 0 {
            break;
        }
        ensure!(!ended, "Chained OGG streams are not supported");
        reader
            .read_exact(&mut header[1..])
            .context("OGG page is truncated")?;
        ensure!(
            &header[..4] == b"OggS" && header[4] == 0,
            "OGG page header is corrupt"
        );
        let page_serial = u32::from_le_bytes(header[14..18].try_into().unwrap());
        if let Some(serial) = serial {
            ensure!(
                serial == page_serial,
                "Multiple OGG streams are not supported"
            );
        } else {
            ensure!(
                header[5] & 2 != 0,
                "OGG beginning-of-stream page is missing"
            );
            serial = Some(page_serial);
        }
        ensure!(
            u32::from_le_bytes(header[18..22].try_into().unwrap()) == sequence,
            "OGG pages are missing or out of order"
        );
        sequence = sequence.wrapping_add(1);
        let checksum = u32::from_le_bytes(header[22..26].try_into().unwrap());
        header[22..26].fill(0);
        let mut crc = symphonia::core::checksum::Crc32::new(0);
        crc.process_buf_bytes(&header);
        let mut segments = [0u8; 255];
        let count = usize::from(header[26]);
        reader
            .read_exact(&mut segments[..count])
            .context("OGG segment table is truncated")?;
        crc.process_buf_bytes(&segments[..count]);
        let length: usize = segments[..count]
            .iter()
            .map(|byte| usize::from(*byte))
            .sum();
        for segment in &segments[..count] {
            packet_bytes = packet_bytes
                .checked_add(usize::from(*segment))
                .context("OGG packet size overflow")?;
            ensure!(
                packet_bytes <= MAX_PACKET_BYTES,
                "OGG packet exceeds the import limit"
            );
            if *segment < 255 {
                packet_bytes = 0;
            }
        }
        let mut body = vec![0u8; length];
        reader
            .read_exact(&mut body)
            .context("OGG page body is truncated")?;
        crc.process_buf_bytes(&body);
        ensure!(crc.crc() == checksum, "OGG page checksum is corrupt");
        ended = header[5] & 4 != 0;
    }
    ensure!(ended, "OGG end-of-stream page is missing or truncated");
    drop(reader);
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}
