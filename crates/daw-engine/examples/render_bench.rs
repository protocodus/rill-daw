//! Run with: cargo run --release -p daw-engine --example render_bench
use daw_core::{Clip, Note, Project, Track, TrackKind};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let mut project = Project::new();
    for index in 0..32 {
        let mut track = Track::new(format!("Instrument {index}"), TrackKind::Midi);
        track.gain = 0.03;
        let notes = (0..32)
            .flat_map(|beat| {
                [0, 4, 7, 12].map(move |offset| Note::new(48 + offset, beat as f64, 0.9, 90))
            })
            .collect();
        track.clips.push(Clip::midi("Pattern", 0.0, 32.0, notes));
        project.tracks.push(track);
    }
    let started = Instant::now();
    let audio = daw_engine::render(&project, 48_000, 32.0)?;
    let elapsed = started.elapsed().as_secs_f64();
    let seconds = audio.len() as f64 / 96_000.0;
    println!(
        "32 tracks / 128 simultaneous voices: {seconds:.1}s audio in {elapsed:.3}s ({:.1}x realtime)",
        seconds / elapsed
    );
    Ok(())
}
