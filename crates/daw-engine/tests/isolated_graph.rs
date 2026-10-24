//! Opt-in integration against installed native plugins; normal tests need no VST binaries.
//!
//! Build the helper with `cargo build -p daw-plugins --bin daw-plugin-host`, then run:
//! `DAW_TEST_VST3=/path/to/instrument.vst3 DAW_TEST_VST3_EFFECT=/path/to/effect.vst3
//! cargo test -p daw-engine --test isolated_graph -- --ignored --nocapture`.

use anyhow::{Context, Result};
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Effect, EffectSlot,
    Instrument, Interpolation, Note, Project,
};

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
    source_track.instrument = Instrument::Vst3 {
        path: instrument,
        class_id: None,
    };
    source_track.clips.push(Clip::midi(
        "Offset note",
        0.0,
        1.0,
        vec![Note::new(60, 0.25, 0.5, 105)],
    ));
    source_track.inserts = vec![EffectSlot::new(
        "Native effect",
        Effect::Vst3 {
            path: effect,
            class_id: None,
        },
    )];
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
            .as_chunks::<2>()
            .0
            .iter()
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

/// CHOWTapeModel exposes an Output Gain parameter. This test drives that parameter
/// through the engine's automation lane, including its muted/manual fallback.
#[test]
#[ignore = "requires CHOWTapeModel in DAW_TEST_VST3_EFFECT"]
fn isolated_vst_effect_parameter_automation_changes_rendered_audio() -> Result<()> {
    let effect = std::env::var("DAW_TEST_VST3_EFFECT").context("Set DAW_TEST_VST3_EFFECT")?;
    let probe = daw_plugins::VstEffect::load(std::path::Path::new(&effect), 48_000.0, 256)?;
    assert!(probe.isolation_pid().is_some());
    let parameter = probe
        .parameters()?
        .into_iter()
        .find(|parameter| {
            parameter.name.eq_ignore_ascii_case("Output Gain") && !parameter.read_only
        })
        .context("This native automation test requires an Output Gain parameter (CHOWTapeModel)")?;
    drop(probe);
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = false;
    let fixture = AudioFixture(std::env::temp_dir().join(format!(
        "rill-native-{}-{}.wav",
        std::process::id(),
        daw_core::new_id()
    )));
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..72_000)
            .map(|frame| {
                (std::f32::consts::TAU * 440.0 * frame as f32 / 48_000.0).sin() * 0.18 * 100.0
                    / 127.0
            })
            .collect(),
    }
    .write_wav(&fixture.0)?;
    let source = project.add_audio_track("Steady audio source");
    let slot = EffectSlot::new(
        "Automated native gain",
        Effect::Vst3 {
            path: effect,
            class_id: None,
        },
    );
    let slot_id = slot.id;
    let track = project.track_mut(source).unwrap();
    track.gain = 0.25;
    track.clips.push(Clip::audio(
        "Steady tone",
        0.0,
        3.0,
        fixture.0.to_string_lossy().into_owned(),
    ));
    track.inserts.push(slot);
    let lane = project.add_automation_track_with_value(
        "Native output level",
        AutomationTarget {
            track: source,
            parameter: AutomationParameter::VstParameter {
                effect: Some(slot_id),
                id: parameter.id,
            },
        },
        0.1,
    )?;
    let automation = project.automation_lane_mut(lane).unwrap();
    automation.interpolation = Interpolation::Step;
    automation.points = vec![
        AutomationPoint::new(0.0, 0.1),
        AutomationPoint::new(1.0, 0.9),
        AutomationPoint::new(2.0, 0.1),
    ];
    let automated = daw_engine::render(&project, 48_000, 3.0)?;
    assert!(automated.iter().all(|sample| sample.is_finite()));
    let levels = [
        window_rms(&automated, 0.25, 0.4),
        window_rms(&automated, 0.75, 0.9),
        window_rms(&automated, 1.25, 1.4),
    ];
    assert!(
        levels[1] > levels[0] * 4.0,
        "Native gain did not rise: {levels:?}"
    );
    assert!(
        levels[1] > levels[2] * 4.0,
        "Native gain did not fall: {levels:?}"
    );
    project.track_mut(lane).unwrap().mute = true;
    let manual = daw_engine::render(&project, 48_000, 3.0)?;
    let manual_levels = [
        window_rms(&manual, 0.25, 0.4),
        window_rms(&manual, 0.75, 0.9),
        window_rms(&manual, 1.25, 1.4),
    ];
    assert!(manual_levels[0] > 0.001);
    assert!(
        (0.8..1.25).contains(&(manual_levels[1] / manual_levels[0])),
        "Muted lane changed gain: {manual_levels:?}"
    );
    assert!((0.8..1.25).contains(&(manual_levels[2] / manual_levels[0])));
    eprintln!(
        "Isolated VST output automation RMS low/high/low={levels:?}; muted={manual_levels:?}"
    );
    Ok(())
}

/// Separate coverage for the instrument parameter path (`effect: None`).
#[test]
#[ignore = "requires Surge XT in DAW_TEST_VST3"]
fn isolated_vst_instrument_automation_changes_audio_without_shifting_midi() -> Result<()> {
    let instrument = std::env::var("DAW_TEST_VST3").context("Set DAW_TEST_VST3")?;
    let probe = daw_plugins::VstInstrument::load(std::path::Path::new(&instrument), 48_000.0, 256)
        .context("Preparing isolated instrument parameter discovery")?;
    assert!(probe.isolation_pid().is_some());
    let parameter = probe
        .parameters()
        .context("Enumerating native instrument parameters")?
        .into_iter()
        .find(|parameter| {
            parameter.name.eq_ignore_ascii_case("Global Volume") && !parameter.read_only
        })
        .context("This native automation test requires a Global Volume parameter (Surge XT)")?;
    drop(probe);
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = false;
    let source = project.add_midi_track("Automated native instrument");
    let track = project.track_mut(source).unwrap();
    track.instrument = Instrument::Vst3 {
        path: instrument,
        class_id: None,
    };
    track.gain = 0.7;
    track.latency_samples = 127;
    track.clips.push(Clip::midi(
        "Offset held note",
        0.0,
        3.0,
        vec![Note::new(60, 0.25, 2.75, 105)],
    ));
    let lane = project.add_automation_track_with_value(
        "Native instrument volume",
        AutomationTarget {
            track: source,
            parameter: AutomationParameter::VstParameter {
                effect: None,
                id: parameter.id,
            },
        },
        0.2,
    )?;
    let automation = project.automation_lane_mut(lane).unwrap();
    automation.interpolation = Interpolation::Step;
    automation.points = vec![
        AutomationPoint::new(0.0, 0.2),
        AutomationPoint::new(1.0, 0.9),
        AutomationPoint::new(2.0, 0.2),
    ];
    let automated = daw_engine::render(&project, 48_000, 3.0)
        .context("Rendering isolated native instrument automation")?;
    assert_eq!(automated.len(), 144_000);
    assert!(automated.iter().all(|sample| sample.is_finite()));
    let levels = [
        window_rms(&automated, 0.25, 0.4),
        window_rms(&automated, 0.75, 0.9),
        window_rms(&automated, 1.25, 1.4),
    ];
    assert!(
        levels[0] > 1e-6,
        "Instrument must sound before the upward step: {levels:?}"
    );
    assert!(
        levels[1] > levels[0] * 4.0,
        "Instrument volume did not rise: {levels:?}"
    );
    assert!(
        levels[1] > levels[2] * 4.0,
        "Instrument volume did not fall: {levels:?}"
    );
    // MIDI starts at beat .25, frame 6000. A level-relative threshold measures
    // onset independently of the automated volume and tolerates the synth attack.
    let threshold = (levels[0] * 0.001) as f32;
    let onset = automated
        .as_chunks::<2>()
        .0
        .iter()
        .position(|frame| frame[0].abs() > threshold)
        .context("Automated instrument produced no audible onset")?;
    assert!(
        (6_000..6_200).contains(&onset),
        "MIDI onset moved away from frame 6000: {onset}"
    );
    assert!(
        automated[..6_000 * 2]
            .iter()
            .all(|sample| sample.abs() < threshold)
    );
    eprintln!("Isolated Surge instrument automation RMS low/high/low={levels:?}; onset={onset}");
    Ok(())
}

fn window_rms(samples: &[f32], start: f64, end: f64) -> f64 {
    let start = (start * 48_000.0) as usize;
    let end = (end * 48_000.0) as usize;
    let sum: f64 = (start..end)
        .map(|frame| f64::from(samples[frame * 2]).powi(2))
        .sum();
    (sum / (end - start) as f64).sqrt()
}

struct AudioFixture(std::path::PathBuf);
impl Drop for AudioFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
