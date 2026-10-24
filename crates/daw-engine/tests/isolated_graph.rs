//! Opt-in integration against installed native plugins; normal tests need no VST binaries.
//!
//! Build the helper with `cargo build -p daw-plugins --bin daw-plugin-host`, then run:
//! `DAW_TEST_VST3=/path/to/instrument.vst3 DAW_TEST_VST3_EFFECT=/path/to/effect.vst3
//! cargo test -p daw-engine --test isolated_graph -- --ignored --nocapture`.

use anyhow::{Context, Result};
use daw_core::{Clip, Effect, EffectSlot, Instrument, Note, Project};

#[test]
#[ignore = "requires installed instrument/effect in DAW_TEST_VST3 and DAW_TEST_VST3_EFFECT"]
fn isolated_instrument_insert_and_parallel_buses_render_aligned_audio() -> Result<()> {
    let instrument = std::env::var("DAW_TEST_VST3").context("Set DAW_TEST_VST3")?;
    let effect = std::env::var("DAW_TEST_VST3_EFFECT").context("Set DAW_TEST_VST3_EFFECT")?;
    let mut project = Project::new();
    project.tempo = 120.0;
    let source = project.add_midi_track("Isolated instrument + insert");
    let left_bus = project.add_bus("Left delayed main");
    let right_bus = project.add_bus("Right compensated send");
    let source_track = project.track_mut(source).unwrap();
    source_track.gain = 0.7;
    source_track.instrument = Instrument::Vst3 { path: instrument };
    source_track.clips.push(Clip::midi(
        "Offset note",
        0.0,
        1.0,
        vec![Note::new(60, 0.25, 0.5, 105)],
    ));
    source_track.inserts = vec![
        EffectSlot::new("Input gain", Effect::Gain { gain: 0.8 }),
        EffectSlot::new("Native effect", Effect::Vst3 { path: effect }),
        EffectSlot::new("Output gain", Effect::Gain { gain: 0.75 }),
    ];
    let left = project.track_mut(left_bus).unwrap();
    left.pan = -1.0;
    left.latency_samples = 127;
    let right = project.track_mut(right_bus).unwrap();
    right.pan = 1.0;
    project.set_output(source, Some(left_bus))?;
    project.add_send(source, right_bus, 1.0, false)?;
    project.validate()?;

    let output = daw_engine::render(&project, 48_000, 2.0)?;
    assert_eq!(output.len(), 96_000);
    assert!(output.iter().all(|sample| sample.is_finite()));
    // The first source note is at exactly frame 6000. Compensating the inserted native
    // effect and the 127-frame branch delay must not advance either path into preroll.
    // A tiny threshold tolerates a modelled analog effect's idle noise floor.
    let first = |channel: usize| {
        output
            .chunks_exact(2)
            .position(|frame| frame[channel].abs() > 0.000_1)
            .expect("Both routed branches must produce audible output")
    };
    let left_start = first(0);
    let right_start = first(1);
    assert!(
        (5_990..6_300).contains(&left_start),
        "Left onset {left_start} is not aligned with the note at 6000"
    );
    assert!(
        (5_990..6_300).contains(&right_start),
        "Right onset {right_start} is not aligned with the note at 6000"
    );
    assert!(
        left_start.abs_diff(right_start) < 16,
        "Parallel branches drifted: left={left_start}, right={right_start}"
    );
    let peak = output
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(peak > 0.001);
    eprintln!("Isolated graph: peak={peak:.4}, left onset={left_start}, right onset={right_start}");
    Ok(())
}
