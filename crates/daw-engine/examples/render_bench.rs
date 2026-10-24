//! Run with: cargo run --release -p daw-engine --example render_bench
use daw_core::{AudioData, Clip, Project, Track, TrackKind};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("benchmark.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..768_000)
            .map(|i| (i as f32 * 0.03).sin() * 0.1)
            .collect(),
    }
    .write_wav(&source)?;
    let mut project = Project::new();
    for index in 0..32 {
        let mut track = Track::new(format!("Audio {index}"), TrackKind::Audio);
        track.gain = 0.03;
        track
            .clips
            .push(Clip::audio("Audio", 0.0, 32.0, source.to_string_lossy()));
        project.tracks.push(track);
    }
    let started = Instant::now();
    let audio = daw_engine::render(&project, 48_000, 32.0)?;
    let elapsed = started.elapsed().as_secs_f64();
    let seconds = audio.len() as f64 / 96_000.0;
    println!(
        "32 audio tracks: {seconds:.1}s audio in {elapsed:.3}s ({:.1}x realtime)",
        seconds / elapsed
    );
    Ok(())
}
