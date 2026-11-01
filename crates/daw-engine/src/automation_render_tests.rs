use super::*;
use daw_core::{AutomationPoint, AutomationTarget, Clip, Interpolation};

fn fixture() -> (Project, tempfile::TempDir, u64) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("continuous.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 2,
        samples: (0..16_000).flat_map(|_| [0.5, 0.25]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = false;
    let source = project.add_audio_track("Source");
    let track = project.track_mut(source).unwrap();
    track.gain = 1.0;
    track
        .clips
        .push(Clip::audio("Held source", 0.0, 4.0, path.to_string_lossy()));
    (project, directory, source)
}

fn add_curve(
    project: &mut Project,
    track: u64,
    parameter: AutomationParameter,
    from: f32,
    to: f32,
) -> u64 {
    let id = project
        .add_automation_track_with_value(
            "Curve",
            AutomationTarget { track, parameter },
            parameter.normalize(from),
        )
        .unwrap();
    let lane = project.automation_lane_mut(id).unwrap();
    lane.interpolation = Interpolation::Spline;
    lane.points = vec![
        AutomationPoint::new(1.0, parameter.normalize(from)),
        AutomationPoint::new(2.0, parameter.normalize(to)),
    ];
    lane.points[0].curve = 1.0;
    id
}

#[test]
fn native_splines_and_overrides_render_before_first_and_after_last_nodes() {
    for kind in 0..3 {
        let (mut project, _directory, source) = fixture();
        let bus = project.add_bus("Return");
        project.track_mut(bus).unwrap().gain = 1.0;
        project.add_send(source, bus, 0.25, false).unwrap();
        let (parameter, from, to, manual) = match kind {
            0 => (AutomationParameter::TrackGain, 0.25, 1.0, 0.6),
            1 => (AutomationParameter::TrackPan, -0.5, 0.75, -0.2),
            _ => (AutomationParameter::SendGain { bus }, 0.25, 0.75, 0.4),
        };
        let id = add_curve(&mut project, source, parameter, from, to);
        for overridden in [false, true] {
            project.automation_lane_mut(id).unwrap().override_value =
                overridden.then(|| parameter.normalize(manual));
            if overridden {
                // Removing all nodes does not erase an explicit manual hold.
                project.automation_lane_mut(id).unwrap().points.clear();
            }
            let audio = render(&project, 8_000, 4.0).unwrap();
            // Before the first node, across the spline, and beyond the last node.
            for beat in [0.0, 0.75, 1.0, 1.5, 2.25, 3.75] {
                let progress = ((beat - 1.0_f64).clamp(0.0, 1.0)).powi(3);
                let value = if overridden {
                    manual
                } else {
                    from + (to - from) * progress as f32
                };
                let (gain, pan, send) = match kind {
                    0 => (value, 0.0, 0.25),
                    1 => (1.0, value, 0.25),
                    _ => (1.0, 0.0, value),
                };
                let expected = [
                    0.5 * gain * (1.0 - pan.max(0.0)) * (1.0 + send),
                    0.25 * gain * (1.0 + pan.min(0.0)) * (1.0 + send),
                ];
                let offset = (beat * 4_000.0) as usize * 2;
                for channel in 0..2 {
                    assert!(
                        (audio[offset + channel] - expected[channel]).abs() < 1e-5,
                        "kind={kind}, overridden={overridden}, beat={beat}, channel={channel}"
                    );
                }
            }
        }
    }
}

#[test]
fn spline_holds_survive_seeking_loop_wrap_and_streaming_export() {
    let (mut project, directory, source) = fixture();
    let id = add_curve(
        &mut project,
        source,
        AutomationParameter::TrackGain,
        0.25,
        1.0,
    );
    project.loop_enabled = true;
    project.loop_start = 0.5;
    project.loop_end = 2.5;
    for overridden in [false, true] {
        project.automation_lane_mut(id).unwrap().override_value =
            overridden.then(|| AutomationParameter::TrackGain.normalize(0.6));
        let expected_gain = |beat: f64| {
            if overridden {
                0.6
            } else {
                0.25 + 0.75 * (beat - 1.0).clamp(0.0, 1.0).powi(3) as f32
            }
        };
        let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
        for beat in [0.0, 1.5, 2.45] {
            renderer.seek(beat).unwrap();
            let mut audio = [[0.0; 2]; 1024];
            let mut positions = [0.0; 1024];
            assert_eq!(
                crate::allocation_test_support::count_allocations(|| renderer
                    .render(&mut audio, &mut positions)
                    .unwrap()),
                0
            );
            for (frame, position) in audio.into_iter().zip(positions) {
                let gain = expected_gain(position - 1.0 / 4_000.0);
                assert!((frame[0] - 0.5 * gain).abs() < 1e-5);
                assert!((frame[1] - 0.25 * gain).abs() < 1e-5);
            }
        }
        let path = directory.path().join("automated-export.wav");
        export_project_wav(&project, &path, 8_000).unwrap();
        let exported = AudioData::load_wav(&path).unwrap();
        assert!(exported.frames() > 16_000, "Export includes release tail");
        for beat in [0.0, 0.75, 1.5, 2.25, 3.75] {
            let offset = (beat * 4_000.0) as usize * 2;
            let gain = expected_gain(beat);
            assert!((exported.samples[offset] - 0.5 * gain).abs() < 1e-5);
            assert!((exported.samples[offset + 1] - 0.25 * gain).abs() < 1e-5);
        }
    }
}

#[test]
fn live_override_resume_and_slope_edits_preserve_graph_audio_and_allocate_nothing() {
    let (mut project, _directory, source) = fixture();
    let bus = project.add_bus("Return");
    project.track_mut(bus).unwrap().gain = 1.0;
    project.add_send(source, bus, 0.25, false).unwrap();
    let gain_id = add_curve(
        &mut project,
        source,
        AutomationParameter::TrackGain,
        0.25,
        1.0,
    );
    let send_id = add_curve(
        &mut project,
        source,
        AutomationParameter::SendGain { bus },
        0.25,
        0.75,
    );
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    renderer.seek(1.25).unwrap();
    let track = renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == source)
        .unwrap();
    let content = Arc::clone(&track.audio);
    let delay = track.routes[1].delay.frames.as_ptr();
    for iteration in 0..4 {
        match iteration {
            0 => {
                project.automation_lane_mut(gain_id).unwrap().override_value =
                    Some(AutomationParameter::TrackGain.normalize(0.6));
                project.automation_lane_mut(send_id).unwrap().override_value =
                    Some(AutomationParameter::SendGain { bus }.normalize(0.4));
            }
            1 => {
                project.automation_lane_mut(gain_id).unwrap().override_value = None;
                project.automation_lane_mut(send_id).unwrap().override_value = None;
            }
            2 => {
                project.automation_lane_mut(gain_id).unwrap().points[0].curve = -1.0;
                project.automation_lane_mut(send_id).unwrap().points[0].curve = -0.5;
            }
            _ => project.track_mut(gain_id).unwrap().mute = true,
        }
        let mut next = prepare.prepare(&project).unwrap();
        let before = (renderer.source_sample, renderer.logical_sample);
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| renderer
                .apply_live_edit(&mut next)
                .unwrap()),
            0
        );
        assert_eq!((renderer.source_sample, renderer.logical_sample), before);
        let track = renderer
            .tracks
            .iter()
            .flatten()
            .find(|track| track.id == source)
            .unwrap();
        assert!(Arc::ptr_eq(&content, &track.audio));
        assert_eq!(track.routes[1].delay.frames.as_ptr(), delay);
        let mut audio = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| renderer
                .render(&mut audio, &mut positions)
                .unwrap()),
            0
        );
        for (index, frame) in audio.into_iter().enumerate() {
            let beat = (before.0 + index as u64) as f64 / 4_000.0;
            let gain = if iteration == 3 {
                1.0
            } else {
                AutomationParameter::TrackGain.denormalize(
                    project
                        .automation_lane(gain_id)
                        .unwrap()
                        .effective_value_at(beat)
                        .unwrap(),
                )
            };
            let send = AutomationParameter::SendGain { bus }.denormalize(
                project
                    .automation_lane(send_id)
                    .unwrap()
                    .effective_value_at(beat)
                    .unwrap(),
            );
            assert!((frame[0] - 0.5 * gain * (1.0 + send)).abs() < 1e-5);
            assert!((frame[1] - 0.25 * gain * (1.0 + send)).abs() < 1e-5);
        }
    }
}

#[test]
fn plugin_curves_emit_held_endpoints_and_override_resume_values_without_allocating() {
    let (mut project, _directory, source) = fixture();
    let id = add_curve(
        &mut project,
        source,
        AutomationParameter::TrackGain,
        0.3,
        1.2,
    );
    let mut lane = project.automation_lane(id).unwrap().clone();
    lane.target.parameter = AutomationParameter::VstParameter {
        effect: Some(42),
        id: 7,
    };
    let mut parameters = vec![PluginAutomation {
        id: 7,
        lane,
        previous: None,
        action_sequence: 0,
        manual: None,
    }];
    let mut emitted = [(0_u32, 0.0_f64); 4];
    let mut count = 0;
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            let mut set = |id, value| {
                emitted[count] = (id, value);
                count += 1;
                Ok(())
            };
            apply_plugin_automation(&mut parameters, -100.0, &mut set).unwrap();
            apply_plugin_automation(&mut parameters, 0.0, &mut set).unwrap();
            apply_plugin_automation(&mut parameters, f64::INFINITY, &mut set).unwrap();
            parameters[0].lane.override_value = Some(0.55);
            apply_plugin_automation(&mut parameters, 1.5, &mut set).unwrap();
            parameters[0].lane.override_value = None;
            apply_plugin_automation(&mut parameters, 1.5, &mut set).unwrap();
        }),
        0
    );
    assert_eq!(count, 4);
    for ((id, value), expected) in emitted.into_iter().zip([0.2, 0.8, 0.55, 0.275]) {
        assert_eq!(id, 7);
        assert!((value - expected).abs() < 1e-6);
    }
}

#[test]
fn queued_plugin_overrides_and_prepared_resumes_follow_ui_action_order() {
    let (mut project, _directory, _) = fixture();
    // A muted owner compiles the production automation/plan path without loading
    // an external plug-in. Capture the parameter IPC at the shared setter seam.
    let owner = project.add_midi_track("Parameter owner");
    let track = project.track_mut(owner).unwrap();
    track.mute = true;
    track.instrument = Instrument::Vst3 {
        path: "unused.vst3".into(),
        class_id: None,
    };
    add_curve(
        &mut project,
        owner,
        AutomationParameter::VstParameter {
            effect: None,
            id: 7,
        },
        0.2,
        0.8,
    );
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut preparer = renderer.live_edit_preparer();
    let mut resume = preparer.prepare(&project).unwrap();
    resume.set_automation_sequence(2);
    renderer.apply_live_edit(&mut resume).unwrap();
    let parameters = &mut renderer
        .tracks
        .iter_mut()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap()
        .instrument_automation;
    let mut sends = 0;
    // Resume (2) reached the worker before the older queued override (1).
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            apply_plugin_parameter_change(parameters, 7, 0.55, Some(1), |_, _| {
                sends += 1;
                Ok(())
            })
            .unwrap();
        }),
        0
    );
    assert_eq!(sends, 0, "A stale override must never reach the plug-in");
    assert_eq!(parameters[0].lane.override_value, None);
    assert!((parameters[0].lane.effective_value_at(1.5).unwrap() - 0.275).abs() < 1e-6);

    let mut older_plan = preparer.prepare(&project).unwrap();
    older_plan.set_automation_sequence(3);
    let parameters = &mut renderer
        .tracks
        .iter_mut()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap()
        .instrument_automation;
    // Now reverse arrival order: immediate override (4) precedes the older
    // project's preparation/refinement (3). Both must preserve the newer hold.
    apply_plugin_parameter_change(parameters, 7, 0.55, Some(4), |_, _| {
        sends += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(sends, 1);
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            renderer.apply_live_edit(&mut older_plan).unwrap();
        }),
        0
    );
    let parameters = &renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap()
        .instrument_automation;
    assert_eq!(parameters[0].lane.override_value, Some(0.55));
    assert_eq!(parameters[0].action_sequence, 4);

    let mut refinement = preparer.prepare(&project).unwrap();
    refinement.set_automation_sequence(3);
    renderer.apply_live_edit(&mut refinement).unwrap();
    let parameters = &renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap()
        .instrument_automation;
    assert_eq!(parameters[0].lane.override_value, Some(0.55));

    let mut latest_resume = preparer.prepare(&project).unwrap();
    latest_resume.set_automation_sequence(5);
    renderer.apply_live_edit(&mut latest_resume).unwrap();
    let parameters = &mut renderer
        .tracks
        .iter_mut()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap()
        .instrument_automation;
    let mut heard = 0.0;
    apply_plugin_automation(parameters, 1.5, |_, value| {
        heard = value;
        Ok(())
    })
    .unwrap();
    assert_eq!(parameters[0].lane.override_value, None);
    assert!(
        (heard - 0.275).abs() < 1e-6,
        "Resume immediately reaches the plug-in"
    );
}
