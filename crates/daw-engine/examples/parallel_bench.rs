//! Compare complete offline renders, including preparation/output allocation.
//! cargo run --release -p daw-engine --example parallel_bench
//! Optional RILL_BENCH_VST3 / RILL_BENCH_VST_EFFECT select installed plug-ins.
//! RILL_BENCH_TRACKS, RILL_BENCH_BEATS, RILL_BENCH_WORKERS override defaults.
use anyhow::{Context, Result, ensure};
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let instrument = std::env::var("RILL_BENCH_VST3").ok();
    let effect = std::env::var("RILL_BENCH_VST_EFFECT").ok();
    let plugins = instrument.is_some() || effect.is_some();
    let tracks = number("RILL_BENCH_TRACKS", if plugins { 4 } else { 32 })?;
    let beats = number("RILL_BENCH_BEATS", 16)?;
    let workers = number(
        "RILL_BENCH_WORKERS",
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(2)
            .clamp(2, 8),
    )?;
    ensure!(
        (1..=64).contains(&tracks) && (1..=256).contains(&beats) && (1..=32).contains(&workers),
        "Benchmark limits: 1..64 tracks, 1..256 beats, 1..32 workers"
    );
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("benchmark.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..beats * 24_000)
            .flat_map(|frame| {
                let phase = frame as f32 * 0.03;
                [phase.sin() * 0.1, (phase * 1.03).sin() * 0.1]
            })
            .collect(),
    }
    .write_wav(&source)?;
    let mut project = Project::new();
    for index in 0..tracks {
        let track = if let Some(path) = &instrument {
            let track = project.add_midi_track(format!("Instrument {index}"));
            project.set_instrument(
                track,
                Instrument::Vst3 {
                    path: PathBuf::from(path)
                        .canonicalize()?
                        .to_string_lossy()
                        .into_owned(),
                    class_id: None,
                },
            )?;
            let notes = (0..beats)
                .step_by(2)
                .flat_map(|beat| {
                    [48, 55, 60, 64].map(|pitch| Note::new(pitch, beat as f64, 1.8, 90))
                })
                .collect();
            project.track_mut(track).unwrap().clips.push(Clip::midi(
                "Chords",
                0.0,
                beats as f64,
                notes,
            ));
            track
        } else {
            let track = project.add_audio_track(format!("Audio {index}"));
            project.track_mut(track).unwrap().clips.push(Clip::audio(
                "Audio",
                0.0,
                beats as f64,
                source.to_string_lossy(),
            ));
            track
        };
        let channel = project.track_mut(track).unwrap();
        channel.gain = 1.0 / tracks as f32;
        if let Some(path) = &effect {
            channel.inserts.push(EffectSlot::new(
                "VST effect",
                Effect::Vst3 {
                    path: PathBuf::from(path)
                        .canonicalize()?
                        .to_string_lossy()
                        .into_owned(),
                    class_id: None,
                },
            ));
        }
    }
    let started = Instant::now();
    let serial = daw_engine::render_with_workers(&project, 48_000, beats as f64, 1)?;
    let serial_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let parallel = daw_engine::render_with_workers(&project, 48_000, beats as f64, workers)?;
    let parallel_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let automatic = daw_engine::render(&project, 48_000, beats as f64)?;
    let automatic_seconds = started.elapsed().as_secs_f64();
    for audio in [&serial, &parallel, &automatic] {
        ensure!(
            audio.iter().all(|value| value.is_finite()),
            "Nonfinite render output"
        );
        ensure!(
            audio.iter().any(|value| value.abs() > 1e-6),
            "Benchmark plug-in default preset produced no audio"
        );
    }
    if !plugins {
        ensure!(
            serial == parallel && serial == automatic,
            "Scheduler changed deterministic audio output"
        );
    }
    let duration = serial.len() as f64 / 96_000.0;
    println!(
        "{tracks} {} tracks; {duration:.1}s at48kHz stereo; preparation + output allocation included",
        if plugins { "VST" } else { "audio" }
    );
    println!(
        "serial: {serial_seconds:.4}s ({:.1}x realtime)",
        duration / serial_seconds
    );
    println!(
        "{workers} workers: {parallel_seconds:.4}s ({:.1}x realtime, {:.2}x serial speed)",
        duration / parallel_seconds,
        serial_seconds / parallel_seconds
    );
    println!(
        "automatic: {automatic_seconds:.4}s ({:.1}x realtime)",
        duration / automatic_seconds
    );
    if plugins {
        println!(
            "Independent fresh plug-in instances may produce different random/modulated samples; exact sample determinism is verified by the scheduler test fixture."
        );
    }
    Ok(())
}

fn number(name: &str, default: usize) -> Result<usize> {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().with_context(|| format!("Invalid {name}")))
        .unwrap_or(Ok(default))
}
