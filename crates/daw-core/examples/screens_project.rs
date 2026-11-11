//! Write the song that `scripts/capture-screens.sh` photographs: the demo's
//! MIDI parts on R1 Mini, its drum loop, a filtered bus with a send, and a
//! volume curve, so every screen and dialog has something real to show.
//!
//! cargo run -p daw-core --example screens_project -- out.rill

use anyhow::{Context, Result};
use daw_core::{
    AutomationParameter, AutomationPoint, AutomationTarget, Effect, EffectSlot, Instrument,
    Project, TrackKind,
};

fn main() -> Result<()> {
    let output = std::env::args()
        .nth(1)
        .context("Usage: screens_project output.rill")?;
    let audio = tempfile::tempdir()?;
    let drums = Project::demo_with_audio(audio.path().join("drums.wav"))?;
    let mut project = Project::demo();
    project.tracks.retain(|track| track.kind == TrackKind::Midi);
    for track in &mut project.tracks {
        track.instrument = Instrument::new_track_default();
    }
    project.tracks.extend(drums.tracks);

    let chords = project.tracks[0].id;
    project.track_mut(chords).unwrap().inserts.push(filter());
    let space = project.add_bus("Space");
    project.track_mut(space).unwrap().inserts.push(filter());
    project.add_send(chords, space, 0.5, false)?;

    let volume = project.add_automation_track(
        "Chords volume",
        AutomationTarget {
            track: chords,
            parameter: AutomationParameter::TrackGain,
        },
    )?;
    let lane = project.automation_lane_mut(volume).unwrap();
    lane.points = [
        (0.0, 0.2),
        (4.0, 0.55),
        (8.0, 0.4),
        (12.0, 0.7),
        (16.0, 0.3),
    ]
    .into_iter()
    .map(|(beat, value)| AutomationPoint::new(beat, value))
    .collect();

    project.save(&output)?;
    println!("Wrote {output}");
    Ok(())
}

fn filter() -> EffectSlot {
    EffectSlot::new(
        "R1 Filter",
        Effect::Builtin {
            device: "r1-filter".into(),
        },
    )
}
