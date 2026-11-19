//! Multi-track R1 built-in session benchmark comparing 1-worker vs automatic multicore.
//! Run with: cargo run --release --locked -p daw-engine --example r1_session_bench
use anyhow::{Result, ensure};
use daw_core::{AudioData, Clip, DeviceSample, Effect, EffectSlot, Instrument, Note, Project};
use daw_engine::OfflineRenderer;
use std::collections::BTreeMap;
use std::time::Instant;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK: usize = 256;
const WARMUP_BLOCKS: usize = 32;
const TIMED_BLOCKS: usize = 512;

struct Stats {
    worker_count: usize,
    total_ns: u64,
    p50_ns: u64,
    p95_ns: u64,
    audio: Vec<[f32; 2]>,
}

fn builtin_instrument(device: &str, samples: Vec<DeviceSample>) -> Instrument {
    Instrument::Builtin {
        device: device.into(),
        samples,
        sample_names: BTreeMap::new(),
        sample_players: BTreeMap::new(),
    }
}

fn builtin_effect(device: &str) -> EffectSlot {
    EffectSlot::new(
        device,
        Effect::Builtin {
            device: device.into(),
        },
    )
}

fn build_r1_session() -> Result<(tempfile::TempDir, Project)> {
    let dir = tempfile::tempdir()?;
    let sampler_path = dir.path().join("sampler_tone.wav");
    AudioData {
        sample_rate: SAMPLE_RATE,
        channels: 2,
        samples: (0..SAMPLE_RATE as usize * 2)
            .flat_map(|frame| {
                let t = frame as f32 / SAMPLE_RATE as f32;
                let left = (std::f32::consts::TAU * 220.0 * t).sin() * 0.35
                    + (std::f32::consts::TAU * 440.0 * t).sin() * 0.2;
                let right = (std::f32::consts::TAU * 220.0 * t).sin() * 0.3
                    + (std::f32::consts::TAU * 660.0 * t).sin() * 0.2;
                [left, right]
            })
            .collect(),
    }
    .write_wav(&sampler_path)?;

    let drum_path = dir.path().join("drum_hit.wav");
    AudioData {
        sample_rate: SAMPLE_RATE,
        channels: 2,
        samples: (0..SAMPLE_RATE as usize / 2)
            .flat_map(|frame| {
                let t = frame as f32 / SAMPLE_RATE as f32;
                let env = (-18.0 * t).exp();
                let body = (std::f32::consts::TAU * 110.0 * t).sin() * env * 0.6;
                [body, body * 0.9]
            })
            .collect(),
    }
    .write_wav(&drum_path)?;

    let sampler_wav = sampler_path.to_string_lossy().into_owned();
    let drum_wav = drum_path.to_string_lossy().into_owned();

    let mut project = Project::new();
    project.tempo = 120.0;
    let total_beats =
        ((WARMUP_BLOCKS + TIMED_BLOCKS + 4) * BLOCK) as f64 * 2.0 / SAMPLE_RATE as f64;

    let synth_chains: [&[&str]; 4] = [
        &["r1-filter", "r1-spacer"],
        &["r1-saturator", "r1-compressor"],
        &["r1-modulator", "r1-equalizer"],
        &["r1-filter", "r1-modulator", "r1-spacer"],
    ];
    for (idx, inserts) in synth_chains.iter().enumerate() {
        let track_id = project.add_midi_track(format!("Mini {idx}"));
        let track = project.track_mut(track_id).unwrap();
        track.instrument = Instrument::builtin("r1-mini");
        track.gain = 0.08;
        track.pan = ((idx as f32) - 1.5) * 0.25;
        for &effect in *inserts {
            track.inserts.push(builtin_effect(effect));
        }
        let base_pitch = 48 + (idx as u8) * 5;
        let notes: Vec<Note> = (0..8)
            .flat_map(|step| {
                let start = step as f64 * 0.5;
                [0, 4, 7].map(move |interval| Note::new(base_pitch + interval, start, 0.45, 96))
            })
            .collect();
        track
            .clips
            .push(Clip::midi("Synth", 0.0, total_beats, notes));
    }

    let sampler_chains: [&[&str]; 4] = [
        &["r1-equalizer", "r1-compressor"],
        &["r1-saturator", "r1-spacer"],
        &["r1-filter", "r1-equalizer"],
        &["r1-modulator", "r1-compressor"],
    ];
    for (idx, inserts) in sampler_chains.iter().enumerate() {
        let track_id = project.add_instrument_track(format!("Sampler {idx}"));
        let sample = DeviceSample {
            slot: 0,
            path: sampler_wav.clone(),
            name: "Tone".into(),
        };
        let track = project.track_mut(track_id).unwrap();
        track.instrument = builtin_instrument("r1-sampler", vec![sample]);
        track.gain = 0.08;
        track.pan = ((idx as f32) - 1.5) * 0.2;
        for &effect in *inserts {
            track.inserts.push(builtin_effect(effect));
        }
        let notes: Vec<Note> = (0..6)
            .map(|step| {
                Note::new(
                    57 + (idx as u8) * 2 + (step % 3) as u8,
                    step as f64 * 0.75,
                    0.7,
                    100,
                )
            })
            .collect();
        track
            .clips
            .push(Clip::midi("Sample", 0.0, total_beats, notes));
    }

    let drum_chains: [&[&str]; 4] = [
        &["r1-compressor", "r1-saturator"],
        &["r1-equalizer", "r1-filter"],
        &["r1-saturator", "r1-spacer"],
        &["r1-compressor", "r1-equalizer"],
    ];
    for (idx, inserts) in drum_chains.iter().enumerate() {
        let track_id = project.add_instrument_track(format!("Drums {idx}"));
        let pads = [0_u32, 2, 6, 8]
            .into_iter()
            .map(|slot| DeviceSample {
                slot,
                path: drum_wav.clone(),
                name: format!("Pad {slot}"),
            })
            .collect();
        let track = project.track_mut(track_id).unwrap();
        track.instrument = builtin_instrument("r1-drums", pads);
        track.gain = 0.08;
        for &effect in *inserts {
            track.inserts.push(builtin_effect(effect));
        }
        let pitches = [36_u8, 38, 42, 44];
        let notes: Vec<Note> = (0..16)
            .map(|step| Note::new(pitches[step % pitches.len()], step as f64 * 0.25, 0.2, 110))
            .collect();
        track.clips.push(Clip::midi("Kit", 0.0, total_beats, notes));
    }

    project.validate()?;
    Ok((dir, project))
}

fn run_bench(project: &Project, workers: Option<usize>) -> Result<Stats> {
    let mut renderer = OfflineRenderer::new(project, SAMPLE_RATE, workers)?;
    let mut block = [[0.0_f32; 2]; BLOCK];
    let mut positions = [0.0_f64; BLOCK];
    let mut audio = Vec::with_capacity((WARMUP_BLOCKS + TIMED_BLOCKS) * BLOCK);
    for _ in 0..WARMUP_BLOCKS {
        renderer.render(&mut block, &mut positions)?;
        audio.extend_from_slice(&block);
    }
    let mut block_ns = Vec::with_capacity(TIMED_BLOCKS);
    let suite_start = Instant::now();
    for _ in 0..TIMED_BLOCKS {
        let start = Instant::now();
        renderer.render(&mut block, &mut positions)?;
        block_ns.push(start.elapsed().as_nanos() as u64);
        audio.extend_from_slice(&block);
    }
    let total_ns = suite_start.elapsed().as_nanos() as u64;
    let worker_count = renderer.performance_snapshot().worker_count;
    block_ns.sort_unstable();
    let p50_ns = block_ns[block_ns.len() / 2];
    let p95_ns = block_ns[(block_ns.len() * 95) / 100];
    Ok(Stats {
        worker_count,
        total_ns,
        p50_ns,
        p95_ns,
        audio,
    })
}

fn main() -> Result<()> {
    let (_dir, project) = build_r1_session()?;
    let serial = run_bench(&project, Some(1))?;
    let auto = run_bench(&project, None)?;

    ensure!(
        serial.audio == auto.audio,
        "1-worker and multicore outputs diverged!"
    );
    let peak = serial
        .audio
        .iter()
        .flatten()
        .fold(0.0_f32, |acc, s| acc.max(s.abs()));
    ensure!(
        peak > 1e-3,
        "Rendered session is unexpectedly quiet ({peak})"
    );

    let rendered_seconds = (TIMED_BLOCKS * BLOCK) as f64 / f64::from(SAMPLE_RATE);
    let serial_ms = serial.total_ns as f64 / 1_000_000.0;
    let auto_ms = auto.total_ns as f64 / 1_000_000.0;
    println!(
        "12-track R1 session ({TIMED_BLOCKS} blocks x {BLOCK} frames = {rendered_seconds:.2}s @ {SAMPLE_RATE} Hz):"
    );
    println!(
        "  serial (workers={}): wall {serial_ms:.2} ms, p50 {:.1} us, p95 {:.1} us ({:.1}x realtime)",
        serial.worker_count,
        serial.p50_ns as f64 / 1000.0,
        serial.p95_ns as f64 / 1000.0,
        rendered_seconds / (serial.total_ns as f64 / 1e9)
    );
    println!(
        "  auto   (workers={}): wall {auto_ms:.2} ms, p50 {:.1} us, p95 {:.1} us ({:.1}x realtime), {:.2}x speedup",
        auto.worker_count,
        auto.p50_ns as f64 / 1000.0,
        auto.p95_ns as f64 / 1000.0,
        rendered_seconds / (auto.total_ns as f64 / 1e9),
        serial_ms / auto_ms.max(1e-9)
    );
    Ok(())
}
