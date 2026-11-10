//! Delivery file encoders behind one streaming interface. The renderer hands
//! over stereo float blocks; each format converts, encodes and writes them to
//! the staged temporary file without holding the uncompressed mix.
use super::*;
use std::io::{BufWriter, Seek, SeekFrom, Write};

/// Delivery file format. WAV and FLAC use the sample `encoding` chosen beside
/// them; lossy formats take float input and ignore it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    #[default]
    Wav,
    /// Lossless, 16 or 24 bit.
    Flac,
    /// Quality 0 (smallest) to 10 (best), as in `oggenc -q`.
    Vorbis { quality: u8 },
    /// Constant bitrate in kbit/s.
    Mp3 { kbps: u16 },
}

impl ExportFormat {
    pub const VORBIS_DEFAULT: Self = Self::Vorbis { quality: 5 };
    pub const MP3_DEFAULT: Self = Self::Mp3 { kbps: 192 };
    pub const MP3_BITRATES: [u16; 4] = [128, 192, 256, 320];
    /// The highest sample rate the MP3 encoder accepts.
    pub const MP3_MAX_SAMPLE_RATE: u32 = 48_000;

    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Vorbis { .. } => "Ogg Vorbis",
            Self::Mp3 { .. } => "MP3",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Vorbis { .. } => "ogg",
            Self::Mp3 { .. } => "mp3",
        }
    }

    pub fn is_lossy(self) -> bool {
        matches!(self, Self::Vorbis { .. } | Self::Mp3 { .. })
    }

    /// Whether the written samples are integers, so dither can apply.
    pub fn is_integer(self, encoding: WavEncoding) -> bool {
        match self {
            Self::Wav => encoding.is_pcm(),
            Self::Flac => true,
            Self::Vorbis { .. } | Self::Mp3 { .. } => false,
        }
    }

    /// Whether samples beyond full scale are limited to it before encoding.
    /// Float WAV keeps its headroom; everything else cannot represent it.
    pub fn clips(self, encoding: WavEncoding) -> bool {
        self != Self::Wav || encoding.is_pcm()
    }

    pub fn validate(self, encoding: WavEncoding, sample_rate: u32) -> Result<()> {
        match self {
            Self::Wav => {}
            Self::Flac => ensure!(
                encoding.is_pcm(),
                "FLAC exports are 16-bit or 24-bit; choose an integer depth"
            ),
            Self::Vorbis { quality } => {
                ensure!(quality <= 10, "Ogg Vorbis quality must be between 0 and 10")
            }
            Self::Mp3 { kbps } => {
                ensure!(
                    Self::MP3_BITRATES.contains(&kbps),
                    "MP3 bitrate must be 128, 192, 256 or 320 kbit/s"
                );
                ensure!(
                    sample_rate <= Self::MP3_MAX_SAMPLE_RATE,
                    "MP3 supports sample rates up to 48 kHz; choose a lower export rate or another format"
                );
            }
        }
        Ok(())
    }
}

/// Everything about the output file that is not the audio range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Delivery {
    pub format: ExportFormat,
    pub encoding: WavEncoding,
    pub dither: bool,
}

pub(crate) enum Encoder {
    Wav(hound::WavWriter<BufWriter<std::fs::File>>),
    Flac(Box<Flac>),
    Vorbis(Box<vorbis_rs::VorbisEncoder<BufWriter<std::fs::File>>>),
    Mp3(Box<Mp3>),
}

impl Encoder {
    pub(crate) fn create(
        file: std::fs::File,
        delivery: Delivery,
        sample_rate: u32,
    ) -> Result<Self> {
        delivery.format.validate(delivery.encoding, sample_rate)?;
        let writer = BufWriter::new(file);
        Ok(match delivery.format {
            ExportFormat::Wav => Self::Wav(hound::WavWriter::new(
                writer,
                hound::WavSpec {
                    channels: 2,
                    sample_rate,
                    bits_per_sample: delivery.encoding.bits_per_sample(),
                    sample_format: if delivery.encoding.is_pcm() {
                        hound::SampleFormat::Int
                    } else {
                        hound::SampleFormat::Float
                    },
                },
            )?),
            ExportFormat::Flac => Self::Flac(Box::new(Flac::new(
                writer,
                sample_rate,
                delivery.encoding.bits_per_sample(),
            )?)),
            ExportFormat::Vorbis { quality } => {
                let mut builder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(
                    std::num::NonZeroU32::new(sample_rate).context("Sample rate is zero")?,
                    std::num::NonZeroU8::new(2).unwrap(),
                    writer,
                    // A fixed serial keeps repeated exports byte-identical.
                    0x5249_4c4c,
                );
                builder.bitrate_management_strategy(
                    vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
                        target_quality: f32::from(quality) / 10.0,
                    },
                );
                Self::Vorbis(Box::new(
                    builder.build().context("Starting the Ogg Vorbis encoder")?,
                ))
            }
            ExportFormat::Mp3 { kbps } => Self::Mp3(Box::new(Mp3::new(writer, sample_rate, kbps))),
        })
    }

    /// `first_frame` is the absolute song position of `frames[0]`; dither
    /// noise is keyed to it so ranges match the corresponding full export.
    pub(crate) fn write(
        &mut self,
        frames: &[[f32; 2]],
        first_frame: u64,
        delivery: Delivery,
    ) -> Result<()> {
        // An empty block would end a Vorbis stream.
        if frames.is_empty() {
            return Ok(());
        }
        let integer = |frame_index: usize, channel: usize, sample: f32| {
            let noise = if delivery.dither {
                triangular_dither((first_frame + frame_index as u64) * 2 + channel as u64)
            } else {
                0.0
            };
            encode_pcm(sample, delivery.encoding.bits_per_sample(), noise)
        };
        match self {
            Self::Wav(writer) => {
                for (index, frame) in frames.iter().enumerate() {
                    for (channel, &sample) in frame.iter().enumerate() {
                        if delivery.encoding.is_pcm() {
                            writer.write_sample(integer(index, channel, sample))?;
                        } else {
                            writer.write_sample(sample)?;
                        }
                    }
                }
            }
            Self::Flac(flac) => {
                for (index, frame) in frames.iter().enumerate() {
                    for (channel, &sample) in frame.iter().enumerate() {
                        flac.pending.push(integer(index, channel, sample));
                    }
                }
                flac.drain(false)?;
            }
            Self::Vorbis(encoder) => {
                let (left, right): (Vec<f32>, Vec<f32>) = frames
                    .iter()
                    .map(|frame| (frame[0].clamp(-1.0, 1.0), frame[1].clamp(-1.0, 1.0)))
                    .unzip();
                encoder
                    .encode_audio_block([left, right])
                    .context("Encoding Ogg Vorbis")?;
            }
            Self::Mp3(mp3) => mp3.write(frames)?,
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        match self {
            Self::Wav(writer) => writer.finalize()?,
            Self::Flac(flac) => flac.finish()?,
            Self::Vorbis(encoder) => {
                encoder.finish().context("Finishing Ogg Vorbis")?.flush()?;
            }
            Self::Mp3(mp3) => mp3.finish()?,
        }
        Ok(())
    }
}

const FLAC_BLOCK: usize = 4096;

/// Frames are compressed as blocks complete; only the compressed stream is
/// retained until the header (which needs the MD5 of everything) is final.
pub(crate) struct Flac {
    writer: BufWriter<std::fs::File>,
    config: flacenc::error::Verified<flacenc::config::Encoder>,
    stream: flacenc::component::Stream,
    context: flacenc::source::Context,
    pending: Vec<i32>,
    frame_number: usize,
}

impl Flac {
    fn new(writer: BufWriter<std::fs::File>, sample_rate: u32, bits: u16) -> Result<Self> {
        use flacenc::error::Verify;
        let config = flacenc::config::Encoder::default()
            .into_verified()
            .map_err(|(_, error)| anyhow::anyhow!("FLAC encoder configuration: {error}"))?;
        let mut stream = flacenc::component::Stream::new(sample_rate as usize, 2, bits as usize)
            .map_err(|error| anyhow::anyhow!("FLAC stream: {error}"))?;
        stream
            .stream_info_mut()
            .set_block_sizes(FLAC_BLOCK, FLAC_BLOCK)
            .map_err(|error| anyhow::anyhow!("FLAC block size: {error}"))?;
        Ok(Self {
            writer,
            config,
            stream,
            context: flacenc::source::Context::new(bits as usize, 2),
            pending: Vec::with_capacity(FLAC_BLOCK * 4),
            frame_number: 0,
        })
    }

    fn drain(&mut self, last: bool) -> Result<()> {
        use flacenc::source::Fill;
        let block = FLAC_BLOCK * 2;
        while self.pending.len() >= block || (last && !self.pending.is_empty()) {
            let count = self.pending.len().min(block);
            let mut buffer = flacenc::source::FrameBuf::with_size(2, count / 2)
                .map_err(|error| anyhow::anyhow!("FLAC buffer: {error}"))?;
            buffer
                .fill_interleaved(&self.pending[..count])
                .map_err(|error| anyhow::anyhow!("FLAC buffer: {error}"))?;
            self.context
                .fill_interleaved(&self.pending[..count])
                .map_err(|error| anyhow::anyhow!("FLAC checksum: {error}"))?;
            let frame = flacenc::encode_fixed_size_frame(
                &self.config,
                &buffer,
                self.frame_number,
                self.stream.stream_info(),
            )
            .map_err(|error| anyhow::anyhow!("Encoding FLAC: {error}"))?;
            self.stream.add_frame(frame);
            self.frame_number += 1;
            self.pending.drain(..count);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        use flacenc::component::BitRepr;
        self.drain(true)?;
        let info = self.stream.stream_info_mut();
        // The short last frame must not lower the minimum block size: decoders
        // treat equal minimum and maximum as a fixed-size stream.
        let block = if self.frame_number == 1 {
            self.context.total_samples()
        } else {
            FLAC_BLOCK
        };
        let _ = info.set_block_sizes(block, block);
        info.set_md5_digest(&self.context.md5_digest());
        info.set_total_samples(self.context.total_samples());
        let mut sink = flacenc::bitsink::ByteSink::new();
        self.stream
            .write(&mut sink)
            .map_err(|error| anyhow::anyhow!("Writing FLAC: {error}"))?;
        self.writer.write_all(sink.as_slice())?;
        self.writer.flush()?;
        Ok(())
    }
}

pub(crate) struct Mp3 {
    writer: BufWriter<std::fs::File>,
    encoder: rusty_mp3::Mp3Encoder,
    sample_rate: u32,
    interleaved: Vec<f32>,
    written: u64,
}

impl Mp3 {
    fn new(writer: BufWriter<std::fs::File>, sample_rate: u32, kbps: u16) -> Self {
        Self {
            writer,
            encoder: rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
                bitrate_kbps: u32::from(kbps),
                vbr_quality: None,
            }),
            sample_rate,
            interleaved: Vec::new(),
            written: 0,
        }
    }

    fn write(&mut self, frames: &[[f32; 2]]) -> Result<()> {
        self.interleaved.clear();
        self.interleaved.extend(
            frames
                .iter()
                .flatten()
                .map(|sample| sample.clamp(-1.0, 1.0)),
        );
        self.encoder
            .push_pcm_f32(&self.interleaved, 2, self.sample_rate)
            .map_err(|error| anyhow::anyhow!("Encoding MP3: {error}"))?;
        self.drain()
    }

    fn drain(&mut self) -> Result<()> {
        while let Ok(packet) = self.encoder.next_packet() {
            self.writer.write_all(&packet)?;
            self.written += packet.len() as u64;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        let streamed = self.written > 0;
        self.encoder.finish();
        self.drain()?;
        self.writer.flush()?;
        // The Info frame at the start carries the exact length (gapless
        // playback). When it was already written as a placeholder, patch it.
        if streamed && let Some(tag) = self.encoder.lame_tag_frame() {
            let file = self.writer.get_mut();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(tag)?;
            file.seek(SeekFrom::End(0))?;
        }
        self.writer.flush()?;
        Ok(())
    }
}
