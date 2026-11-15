//! Shared format admission and bounded, header-only sample inspection.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};
use symphonia::core::{
    common::Limit,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
};

/// Extensions understood by the built-in decoder. Codec variants within a
/// container still need to be supported (AAC-LC, for example, not HE-AAC).
pub const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "aac", "m4a", "mp4", "caf",
];

pub fn is_supported_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            SUPPORTED_AUDIO_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    pub format: String,
    pub codec: String,
    /// Original PCM precision, when the codec/container declares it. Lossy
    /// streams usually do not have a meaningful bit depth.
    pub bits_per_sample: Option<u32>,
    pub file_bytes: u64,
}

// Seeking past audio payloads is allowed, but probing may read at most 8 MiB.
// This bounds pathological tags/indexes without decoding or scanning a source.
const PROBE_READ_BUDGET: usize = 8 * 1024 * 1024;
struct ProbeSource {
    file: File,
    bytes: u64,
    remaining: usize,
}
impl Read for ProbeSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(io::Error::other(
                "Audio metadata exceeds the 8 MiB probe budget",
            ));
        }
        let count = buffer.len().min(self.remaining);
        let count = self.file.read(&mut buffer[..count])?;
        self.remaining -= count;
        Ok(count)
    }
}
impl Seek for ProbeSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
impl MediaSource for ProbeSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.bytes)
    }
}

/// Inspect container/codec headers without instantiating an audio decoder or
/// reading packets. Missing duration/precision remains unknown. Call off-thread;
/// the desktop also isolates codec/container parsers in a disposable process.
pub fn probe_audio_metadata(path: &Path) -> Result<AudioMetadata> {
    let file = File::open(path).context("Open sample metadata")?;
    let stat = file.metadata().context("Read sample metadata")?;
    ensure!(
        stat.is_file() && stat.len() >= 4,
        "Choose a nonempty audio file"
    );
    ensure!(
        stat.len() <= crate::MAX_IMPORTED_AUDIO_BYTES as u64,
        "Encoded audio exceeds the 4 GiB import limit"
    );
    let source = ProbeSource {
        file,
        bytes: stat.len(),
        remaining: PROBE_READ_BUDGET,
    };
    let reader = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            MediaSourceStream::new(Box::new(source), Default::default()),
            FormatOptions::default().prebuild_seek_index(false),
            MetadataOptions::default()
                .limit_tag_bytes(Limit::Maximum(64 * 1024))
                .limit_visual_bytes(Limit::Maximum(0)),
        )
        .context("Read sample container metadata")?;
    let track = reader
        .default_track(TrackType::Audio)
        .context("Sample has no audio track")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .context("Sample audio codec is unknown")?;
    let decoder = symphonia::default::get_codecs()
        .get_audio_decoder(params.codec)
        .context("Sample audio codec is not supported")?;
    let duration_seconds = track
        .time_base
        .zip(track.duration)
        .and_then(|(base, duration)| base.calc_duration(duration))
        .map(|time| time.as_secs_f64())
        .or_else(|| {
            track
                .num_frames
                .zip(params.sample_rate)
                .filter(|(_, rate)| *rate > 0)
                .map(|(frames, rate)| frames as f64 / f64::from(rate))
        })
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    let bits_per_sample = {
        use symphonia::core::codecs::audio::well_known::*;
        match params.codec {
            CODEC_ID_AAC | CODEC_ID_MP3 | CODEC_ID_VORBIS => None,
            // ISO sample entries may describe a generic 16-bit decoded layout;
            // the ALAC configuration is the authoritative source precision.
            CODEC_ID_ALAC => params
                .extra_data
                .as_deref()
                .and_then(|data| symphonia_common::apple::audio::alac::MagicCookie::read(data).ok())
                .map(|cookie| u32::from(cookie.bit_depth))
                .filter(|bits| *bits > 0),
            CODEC_ID_PCM_F32LE
            | CODEC_ID_PCM_F32LE_PLANAR
            | CODEC_ID_PCM_F32BE
            | CODEC_ID_PCM_F32BE_PLANAR => Some(32),
            CODEC_ID_PCM_F64LE
            | CODEC_ID_PCM_F64LE_PLANAR
            | CODEC_ID_PCM_F64BE
            | CODEC_ID_PCM_F64BE_PLANAR => Some(64),
            _ => params.bits_per_sample.filter(|bits| *bits > 0),
        }
    };
    Ok(AudioMetadata {
        duration_seconds,
        sample_rate: params.sample_rate,
        channels: params.channels.as_ref().map(|channels| channels.count()),
        format: match reader.format_info().short_name {
            "wave" => "WAV".into(),
            "isomp4" => "MP4".into(),
            name => name.to_ascii_uppercase(),
        },
        codec: decoder.codec.info.short_name.to_ascii_uppercase(),
        bits_per_sample,
        file_bytes: stat.len(),
    })
}
