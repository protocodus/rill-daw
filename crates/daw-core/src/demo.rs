use crate::{AudioData, Clip, Project, TrackKind};
use anyhow::{Context, Result};
use std::{f64::consts::TAU, path::Path};

impl Project {
    /// Create the demo arrangement with a deterministic, original drum loop.
    /// The caller supplies the persistent/cache WAV destination; that file is
    /// replaced. No media paths point at an asset that has not been written.
    pub fn demo_with_audio(path: impl AsRef<Path>) -> Result<Self> {
        let mut project = Self::demo();
        let audio = demo_drums(project.tempo);
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).context("Create demo audio directory")?;
        }
        audio.write_wav(path).context("Write demo drum loop")?;
        let path = path.canonicalize().context("Locate generated demo audio")?;
        let track = project
            .tracks
            .iter_mut()
            .find(|track| track.kind == TrackKind::Audio)
            .expect("demo contains an audio track");
        track.name = "Circuit drums".into();
        track.gain = 0.5;
        track.clips.push(Clip::audio(
            "Circuit rhythm",
            0.0,
            16.0,
            path.to_string_lossy(),
        ));
        project.validate()?;
        Ok(project)
    }
}

fn demo_drums(tempo: f64) -> AudioData {
    const RATE: u32 = 48_000;
    let beat_seconds = 60.0 / tempo;
    let frames = (16.0 * beat_seconds * f64::from(RATE)).ceil() as usize;
    let mut samples = vec![0.0_f32; frames * 2];
    let mut noise = 0x573a_91b5_u32;
    let mut white_noise = || {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        f64::from(noise) / f64::from(u32::MAX) * 2.0 - 1.0
    };
    for half_beat in 0..32 {
        let position = half_beat as f64 * 0.5;
        let start = (position * beat_seconds * f64::from(RATE)).round() as usize;
        // Soft alternating hi-hats with a simple differentiator to emphasize highs.
        let mut previous_noise = 0.0;
        let hat_gain = if half_beat % 2 == 0 { 0.15 } else { 0.10 };
        for frame in 0..(f64::from(RATE) * 0.065) as usize {
            if start + frame >= frames {
                break;
            }
            let t = frame as f64 / f64::from(RATE);
            let noise = white_noise();
            let high = noise - previous_noise;
            previous_noise = noise;
            let hat = high * (-t / 0.013).exp() * hat_gain * (t / 0.0006).min(1.0);
            let pan = if half_beat % 2 == 0 {
                [0.75, 1.0]
            } else {
                [1.0, 0.75]
            };
            add(&mut samples, start + frame, hat, pan);
        }
        if half_beat % 4 == 0 || half_beat == 15 || half_beat == 29 {
            for frame in 0..(f64::from(RATE) * 0.38) as usize {
                if start + frame >= frames {
                    break;
                }
                let t = frame as f64 / f64::from(RATE);
                let phase = TAU * (46.0 * t + 95.0 * 0.035 * (1.0 - (-t / 0.035).exp()));
                let kick = phase.sin() * (-t / 0.075).exp() * (t / 0.001).min(1.0) * 0.85;
                add(&mut samples, start + frame, kick, [1.0, 1.0]);
            }
        }
        if half_beat % 4 == 2 {
            for frame in 0..(f64::from(RATE) * 0.22) as usize {
                if start + frame >= frames {
                    break;
                }
                let t = frame as f64 / f64::from(RATE);
                let snare = ((TAU * 184.0 * t).sin() * 0.24 + white_noise() * 0.43)
                    * (-t / 0.047).exp()
                    * (t / 0.0008).min(1.0);
                add(&mut samples, start + frame, snare, [1.0, 0.95]);
            }
        }
    }
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, value| peak.max(value.abs()));
    if peak > 0.0 {
        for value in &mut samples {
            *value *= 0.85 / peak;
        }
    }
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples,
    }
}

fn add(samples: &mut [f32], frame: usize, value: f64, pan: [f64; 2]) {
    samples[frame * 2] += (value * pan[0]) as f32;
    samples[frame * 2 + 1] += (value * pan[1]) as f32;
}
