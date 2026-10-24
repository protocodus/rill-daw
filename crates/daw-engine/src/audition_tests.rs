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
        })
        .unwrap();
    let route = &track.routes[0];
    assert_ne!(
        route.output[32],
        route.output[BLOCK - 1],
        "Playback automation must still advance"
    );
}
