use super::*;
use daw_core::Clip;

#[test]
fn paused_preview_does_not_render_timeline_audio_or_advance_transport() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tone.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.5; 8_000],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let id = project.add_audio_track("Timeline");
    project.track_mut(id).unwrap().clips.push(Clip::audio(
        "Tone",
        0.0,
        2.0,
        path.to_string_lossy(),
    ));
    let mut renderer = Renderer::new(&project, 8_000, false).unwrap();
    renderer.set_preview(true, 1.0).unwrap();
    let mut output = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    for _ in 0..4 {
        renderer
            .render_preview(&mut output, &mut positions)
            .unwrap();
        assert!(output.iter().all(|frame| *frame == [0.0; 2]));
        assert!(positions.iter().all(|position| *position == 1.0));
    }
    assert!(renderer.set_audition(Some((id, 60, 100))).is_err());
    renderer.set_preview(false, 1.0).unwrap();
    renderer.render(&mut output, &mut positions).unwrap();
    assert!(output.iter().any(|frame| frame[0] > 0.1));
    assert!(positions[BLOCK - 1] > 1.0);
}

#[test]
#[ignore = "requires installed VST instrument via RILL_TEST_VST_INSTRUMENT"]
fn native_held_note_audition_is_audible_and_releases_without_transport() {
    let plugin = std::env::var("RILL_TEST_VST_INSTRUMENT").unwrap();
    let mut project = Project::new();
    let track = project.add_midi_track("Audition");
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path: plugin,
                class_id: None,
            },
        )
        .unwrap();
    let mut renderer = Renderer::new(&project, 48_000, true).unwrap();
    renderer.set_preview(true, 7.0).unwrap();
    renderer.set_audition(Some((track, 60, 100))).unwrap();
    let mut output = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    let mut peak = 0.0_f32;
    for _ in 0..200 {
        renderer
            .render_preview(&mut output, &mut positions)
            .unwrap();
        for frame in &output {
            peak = peak.max(frame[0].abs()).max(frame[1].abs());
        }
        assert!(
            positions
                .iter()
                .all(|position| (*position - 7.0).abs() < 1e-9)
        );
    }
    assert!(peak > 0.001, "Assigned instrument must audition audibly");
    renderer.set_audition(None).unwrap();
    for _ in 0..2000 {
        renderer
            .render_preview(&mut output, &mut positions)
            .unwrap();
    }
    assert!(
        output.iter().flatten().all(|value| value.abs() < 0.0001),
        "Note-off must release the instrument"
    );
    renderer.set_preview(false, 7.0).unwrap();
    assert!(renderer.audition.is_none());
}

#[test]
fn paused_audition_holds_gain_pan_and_send_automation_at_the_cursor() {
    use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget};
    let mut project = Project::new();
    project.tempo = 120.0;
    let bus = project.add_bus("Audition path");
    let target = project.add_bus("Send destination");
    project.track_mut(bus).unwrap().latency_samples = 32;
    project.add_send(bus, target, 1.0, false).unwrap();
    for parameter in [
        AutomationParameter::TrackGain,
        AutomationParameter::TrackPan,
        AutomationParameter::SendGain { bus: target },
    ] {
        let id = project
            .add_automation_track(
                "Curve",
                AutomationTarget {
                    track: bus,
                    parameter,
                },
            )
            .unwrap();
        project
            .track_mut(id)
            .unwrap()
            .automation
            .as_mut()
            .unwrap()
            .points = vec![
            AutomationPoint::new(0.99, 0.0),
            AutomationPoint::new(1.01, 1.0),
        ];
    }
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let track = renderer
        .tracks
        .iter_mut()
        .flatten()
        .find(|track| track.id == bus)
        .unwrap();
    for _ in 0..2 {
        track.mix_left.fill(0.5);
        track.mix_right.fill(0.5);
        track
            .process(BlockContext {
                start: 4000,
                count: BLOCK,
                sample_rate: 8000,
                tempo: 120.0,
                timeline: false,
                live_input: true,
            })
            .unwrap();
    }
    for route in &track.routes {
        let expected = 0.5 * 0.75 * route.gain.value(1.0);
        assert!(
            route
                .output
                .iter()
                .all(|frame| (frame[0] - expected).abs() < 1e-5
                    && (frame[1] - expected).abs() < 1e-5),
            "Paused automation swept within a processing block or applied pre-cursor latency"
        );
    }
    track.mix_left.fill(0.5);
    track.mix_right.fill(0.5);
    track
        .process(BlockContext {
            start: 4000,
            count: BLOCK,
            sample_rate: 8000,
            tempo: 120.0,
            timeline: true,
            live_input: true,
        })
        .unwrap();
    let route = &track.routes[0];
    assert_ne!(
        route.output[32],
        route.output[BLOCK - 1],
        "Playback automation must still advance"
    );
}

/// Rising zero crossings per second of the left channel.
fn preview_frequency(frames: &[[f32; 2]], rate: f32) -> f32 {
    let crossings: Vec<f32> = frames
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0][0] < 0.0 && pair[1][0] >= 0.0)
        .map(|(index, pair)| index as f32 + pair[0][0] / (pair[0][0] - pair[1][0]))
        .collect();
    (crossings.len() - 1) as f32 * rate / (crossings[crossings.len() - 1] - crossings[0])
}

#[test]
fn builtin_instrument_auditions_and_follows_the_panel_wheels() {
    use daw_devices::devices::mini;
    let mut project = Project::new();
    let track = project.add_midi_track("Mini");
    project
        .set_instrument(
            track,
            Instrument::Builtin {
                device: "r1-mini".into(),
            },
        )
        .unwrap();
    // One sine, filter open: the pitch is easy to read.
    let parameters = [
        (mini::OSC1_MODEL, 1.0),
        (mini::OSC1_WAVE, 3.0),
        (mini::MIX_OSC2, 0.0),
        (mini::CUTOFF, 20_000.0),
        (mini::FILTER_ENV, 0.0),
        (mini::BEND_RANGE, 12.0),
    ];
    project.track_mut(track).unwrap().instrument_parameters = parameters
        .iter()
        .map(|&(id, plain)| daw_core::PluginParameterValue {
            id,
            value: f64::from(mini::DESCRIPTOR.parameter(id).unwrap().to_normalized(plain)),
        })
        .collect();
    let mut renderer = Renderer::new(&project, 48_000, true).unwrap();
    renderer.set_preview(true, 0.0).unwrap();
    renderer.set_audition(Some((track, 69, 127))).unwrap();
    let render = |renderer: &mut Renderer, blocks: usize| {
        let mut output = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        let mut frames = Vec::new();
        for _ in 0..blocks {
            renderer
                .render_preview(&mut output, &mut positions)
                .unwrap();
            frames.extend_from_slice(&output);
        }
        frames
    };
    let held = render(&mut renderer, 100);
    let frequency = preview_frequency(&held[held.len() / 2..], 48_000.0);
    assert!((frequency - 440.0).abs() < 1.0, "{frequency}");
    renderer.set_audition_wheels(Some((track, 1.0, 0.5)));
    let bent = render(&mut renderer, 100);
    let frequency = preview_frequency(&bent[bent.len() / 2..], 48_000.0);
    assert!((frequency - 880.0).abs() < 2.0, "{frequency}");
    // Closing the panel mid-bend detaches the wheels; the bend springs back.
    renderer.set_audition_wheels(None);
    let detached = render(&mut renderer, 100);
    let frequency = preview_frequency(&detached[detached.len() / 2..], 48_000.0);
    assert!((frequency - 440.0).abs() < 1.0, "{frequency}");
    renderer.set_audition(None).unwrap();
    render(&mut renderer, 400);
    let released = render(&mut renderer, 10);
    assert!(released.iter().flatten().all(|sample| *sample == 0.0));
}
