use daw_core::{
    AutomationLane, AutomationParameter as Parameter, AutomationPoint, AutomationTarget, Clip,
    Effect, EffectSlot, History, Instrument, Interpolation, MAX_BEAT, Project, TrackKind, new_id,
};

fn curve(interpolation: Interpolation, points: &[(f64, f32)]) -> AutomationLane {
    AutomationLane {
        target: AutomationTarget {
            track: 1,
            parameter: Parameter::TrackGain,
        },
        points: points
            .iter()
            .map(|&(beat, value)| AutomationPoint::new(beat, value))
            .collect(),
        interpolation,
    }
}

fn fixture() -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_midi_track("Synth");
    let lane = project
        .add_automation_track(
            "Volume",
            AutomationTarget {
                track,
                parameter: Parameter::TrackGain,
            },
        )
        .unwrap();
    (project, track, lane)
}

#[test]
fn step_linear_and_smooth_hold_endpoints_and_handle_empty_lanes() {
    for interpolation in [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::Smooth,
    ] {
        let mut lane = curve(interpolation, &[(1.0, 0.2), (3.0, 0.8)]);
        assert_eq!(lane.value_at(-100.0), Some(0.2));
        assert_eq!(lane.value_at(f64::NEG_INFINITY), Some(0.2));
        assert_eq!(lane.value_at(f64::INFINITY), Some(0.8));
        assert_eq!(lane.value_at(f64::NAN), None);
        assert_eq!(lane.value_at(1.0), Some(0.2));
        assert_eq!(lane.value_at(3.0), Some(0.8));
        let expected = if interpolation == Interpolation::Step {
            0.2
        } else {
            0.5
        };
        assert!((lane.value_at(2.0).unwrap() - expected).abs() < 1e-6);
        lane.points.pop();
        assert_eq!(lane.value_at(100.0), Some(0.2));
        lane.points.clear();
        assert_eq!(lane.value_at(2.0), None);
    }
}

#[test]
fn smooth_has_a_continuous_bounded_peak_and_differs_from_linear() {
    let lane = curve(Interpolation::Smooth, &[(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)]);
    assert!((lane.value_at(0.5).unwrap() - 0.75).abs() < 1e-6);
    assert!((lane.value_at(1.5).unwrap() - 0.75).abs() < 1e-6);
    let epsilon = 0.0001;
    let left = (1.0 - lane.value_at(1.0 - epsilon).unwrap()) / epsilon as f32;
    let right = (lane.value_at(1.0 + epsilon).unwrap() - 1.0) / epsilon as f32;
    assert!(
        (left - right).abs() < 0.002,
        "Smooth curve has a derivative discontinuity"
    );
    for step in 0..=2000 {
        assert!((0.0..=1.0).contains(&lane.value_at(step as f64 / 1000.0).unwrap()));
    }
}

#[test]
fn smooth_preserves_monotonicity_and_flat_segments_at_uneven_spacing() {
    let lane = curve(
        Interpolation::Smooth,
        &[(0.0, 0.0), (0.01, 0.8), (100.0, 0.9), (101.0, 1.0)],
    );
    for segment in lane.points.windows(2) {
        let mut previous = segment[0].value;
        for step in 0..=1000 {
            let beat = segment[0].beat + (segment[1].beat - segment[0].beat) * step as f64 / 1000.0;
            let value = lane.value_at(beat).unwrap();
            assert!(value >= previous - 1e-6);
            assert!(value >= segment[0].value && value <= segment[1].value);
            previous = value;
        }
    }
    let flat = curve(
        Interpolation::Smooth,
        &[(0.0, 0.2), (1.0, 0.2), (2.0, 0.8), (3.0, 0.8)],
    );
    assert_eq!(flat.value_at(0.5), Some(0.2));
    assert_eq!(flat.value_at(2.5), Some(0.8));
    let tiny = curve(
        Interpolation::Smooth,
        &[(0.0, 0.0), (1e-320, 0.5), (1.0, 1.0)],
    );
    tiny.validate().unwrap();
    assert!(tiny.value_at(5e-321).unwrap().is_finite());
    assert!((0.0..=0.5).contains(&tiny.value_at(5e-321).unwrap()));
}

#[test]
fn parameter_conversion_covers_full_native_ranges() {
    for parameter in [
        Parameter::TrackGain,
        Parameter::TrackPan,
        Parameter::SendGain { bus: 1 },
        Parameter::InsertGain { effect: 1 },
        Parameter::DelayTime { effect: 1 },
        Parameter::DelayFeedback { effect: 1 },
        Parameter::DelayMix { effect: 1 },
        Parameter::VstParameter {
            effect: None,
            id: 42,
        },
    ] {
        let range = parameter.range();
        assert_eq!(parameter.denormalize(0.0), *range.start());
        assert!((parameter.denormalize(1.0) - range.end()).abs() < 1e-6);
        for normalized in [0.0, 0.125, 0.5, 0.75, 1.0] {
            assert!(
                (parameter.normalize(parameter.denormalize(normalized)) - normalized).abs() < 1e-6
            );
        }
        assert_eq!(parameter.denormalize(-1.0), *range.start());
    }
    assert_eq!(Parameter::TrackPan.denormalize(0.5), 0.0);
    assert_eq!(Parameter::DelayFeedback { effect: 1 }.denormalize(0.5), 0.0);
    assert_eq!(Parameter::SendGain { bus: 1 }.denormalize(1.0), 16.0);
}

#[test]
fn point_edits_preserve_ids_sort_and_roll_back_invalid_positions() {
    let (mut project, _, track) = fixture();
    let first = project.automation_lane(track).unwrap().points[0].id;
    assert_eq!(
        project.upsert_automation_point(track, -0.0, 0.25).unwrap(),
        first
    );
    let middle = project.upsert_automation_point(track, 2.0, 0.75).unwrap();
    project
        .move_automation_point(track, middle, 8.0, 1.0)
        .unwrap();
    let lane = project.automation_lane(track).unwrap();
    assert_eq!(
        lane.points
            .iter()
            .map(|point| point.beat)
            .collect::<Vec<_>>(),
        vec![0.0, 4.0, 8.0]
    );
    assert_eq!(lane.points[2].id, middle);
    let before = project.clone();
    for (beat, value) in [
        (4.0, 0.5),
        (f64::NAN, 0.5),
        (-1.0, 0.5),
        (MAX_BEAT + 1.0, 0.5),
        (2.0, f32::NAN),
        (2.0, 1.1),
    ] {
        assert!(
            project
                .move_automation_point(track, middle, beat, value)
                .is_err()
        );
        assert_eq!(project, before);
    }
    assert_eq!(
        project.remove_automation_point(track, middle).unwrap().id,
        middle
    );
    let remaining: Vec<_> = project
        .automation_lane(track)
        .unwrap()
        .points
        .iter()
        .map(|point| point.id)
        .collect();
    for point in remaining {
        project.remove_automation_point(track, point).unwrap();
    }
    project.validate().unwrap();
    assert_eq!(project.automation_lane(track).unwrap().value_at(10.0), None);
}

#[test]
fn target_uniqueness_type_and_global_point_ids_are_validated() {
    let (mut project, source, lane) = fixture();
    assert!(
        project
            .add_automation_track(
                "Duplicate",
                AutomationTarget {
                    track: source,
                    parameter: Parameter::TrackGain
                }
            )
            .is_err()
    );
    let folder = project.add_folder("Folder");
    for target in [
        AutomationTarget {
            track: folder,
            parameter: Parameter::TrackPan,
        },
        AutomationTarget {
            track: lane,
            parameter: Parameter::TrackPan,
        },
        AutomationTarget {
            track: u64::MAX,
            parameter: Parameter::TrackPan,
        },
        AutomationTarget {
            track: source,
            parameter: Parameter::SendGain { bus: folder },
        },
        AutomationTarget {
            track: source,
            parameter: Parameter::VstParameter {
                effect: None,
                id: 0,
            },
        },
    ] {
        assert!(target.validate(&project).is_err());
    }
    let valid = project.clone();
    project.automation_lane_mut(lane).unwrap().points[0].id = source;
    assert!(project.validate().is_err());
    project = valid.clone();
    project.automation_lane_mut(lane).unwrap().points[1].beat = 0.0;
    assert!(project.validate().is_err());
    project = valid.clone();
    project.automation_lane_mut(lane).unwrap().points[0].value = f32::INFINITY;
    assert!(project.validate().is_err());
    project = valid;
    let mut duplicate = project.track(lane).unwrap().clone();
    duplicate.id = new_id();
    for point in &mut duplicate.automation.as_mut().unwrap().points {
        point.id = new_id();
    }
    project.tracks.push(duplicate);
    assert!(project.validate().is_err());
}

#[test]
fn lanes_are_control_only_and_do_not_extend_audio_exports() {
    let (mut project, source, lane) = fixture();
    project
        .track_mut(source)
        .unwrap()
        .clips
        .push(Clip::midi("Phrase", 0.0, 2.0, vec![]));
    project.upsert_automation_point(lane, 100.0, 1.0).unwrap();
    assert_eq!(project.end_beat(), 2.0);
    assert_eq!(project.routing_order().unwrap(), vec![source]);
    let bus = project.add_bus("Bus");
    let before = project.clone();
    assert!(project.set_output(lane, Some(bus)).is_err());
    assert!(project.add_send(lane, bus, 1.0, false).is_err());
    assert_eq!(project, before);
    project
        .track_mut(lane)
        .unwrap()
        .inserts
        .push(EffectSlot::new("Invalid", Effect::Gain { gain: 1.0 }));
    assert!(project.validate().is_err());
}

#[test]
fn deleting_sources_sends_and_inserts_removes_only_dependent_lanes() {
    let (mut project, source, gain_lane) = fixture();
    let bus = project.add_bus("Bus");
    project.add_send(source, bus, 0.5, false).unwrap();
    let send_lane = project
        .add_automation_track(
            "Send",
            AutomationTarget {
                track: source,
                parameter: Parameter::SendGain { bus },
            },
        )
        .unwrap();
    let delay = EffectSlot::new(
        "Echo",
        Effect::Delay {
            time_ms: 250.0,
            feedback: 0.4,
            mix: 0.25,
        },
    );
    let effect = delay.id;
    project.track_mut(source).unwrap().inserts.push(delay);
    let delay_lane = project
        .add_automation_track(
            "Wet",
            AutomationTarget {
                track: source,
                parameter: Parameter::DelayMix { effect },
            },
        )
        .unwrap();
    assert!(
        AutomationTarget {
            track: source,
            parameter: Parameter::InsertGain { effect }
        }
        .validate(&project)
        .is_err()
    );
    let mut history = History::new();
    history.commit(&project);
    project.remove_insert(source, effect).unwrap();
    assert!(project.track(delay_lane).is_none());
    assert!(project.track(send_lane).is_some());
    assert!(history.undo(&mut project));
    assert!(project.track(delay_lane).is_some());
    project.remove_send(source, bus).unwrap();
    assert!(project.track(send_lane).is_none());
    assert!(project.track(gain_lane).is_some());
    project.delete_track(source).unwrap();
    assert!(
        project
            .tracks
            .iter()
            .all(|track| track.kind != TrackKind::Automation)
    );
    project.validate().unwrap();
}

#[test]
fn deleting_bus_removes_its_send_automation_but_keeps_source_gain() {
    let (mut project, source, gain_lane) = fixture();
    let bus = project.add_bus("Return");
    project.add_send(source, bus, 0.5, true).unwrap();
    let send_lane = project
        .add_automation_track(
            "Return level",
            AutomationTarget {
                track: source,
                parameter: Parameter::SendGain { bus },
            },
        )
        .unwrap();
    let bus_lane = project
        .add_automation_track(
            "Bus pan",
            AutomationTarget {
                track: bus,
                parameter: Parameter::TrackPan,
            },
        )
        .unwrap();
    project.delete_track(bus).unwrap();
    assert!(project.track(send_lane).is_none());
    assert!(project.track(bus_lane).is_none());
    assert!(project.track(gain_lane).is_some());
    project.validate().unwrap();
}

#[test]
fn replacing_vst_instrument_discards_old_parameter_ids_and_is_undoable() {
    let (mut project, source, gain_lane) = fixture();
    project
        .set_instrument(
            source,
            Instrument::Vst3 {
                path: "/plugins/first.vst3".into(),
            },
        )
        .unwrap();
    let target = AutomationTarget {
        track: source,
        parameter: Parameter::VstParameter {
            effect: None,
            id: 42,
        },
    };
    assert!(
        project
            .add_automation_track("Unknown initial value", target)
            .is_err()
    );
    let vst_lane = project
        .add_automation_track_with_value("Cutoff", target, 0.6)
        .unwrap();
    let initial = project.clone();
    project
        .set_instrument(
            source,
            Instrument::Vst3 {
                path: "/plugins/first.vst3".into(),
            },
        )
        .unwrap();
    assert_eq!(project, initial);
    let mut history = History::new();
    history.commit(&project);
    project
        .set_instrument(
            source,
            Instrument::Vst3 {
                path: "/plugins/second.vst3".into(),
            },
        )
        .unwrap();
    assert!(project.track(vst_lane).is_none());
    assert!(project.track(gain_lane).is_some());
    assert!(history.undo(&mut project));
    assert_eq!(project, initial);
    assert!(history.redo(&mut project));
    assert!(project.track(vst_lane).is_none());
}

#[test]
fn automation_roundtrips_reserves_ids_and_legacy_tracks_need_no_new_field() {
    let (mut project, _, lane) = fixture();
    project.automation_lane_mut(lane).unwrap().interpolation = Interpolation::Smooth;
    let point = project.upsert_automation_point(lane, 2.0, 0.9).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("automated.daw.json");
    project.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded, project);
    assert!(new_id() > point);
    let mut history = History::new();
    history.commit(&project);
    project
        .move_automation_point(lane, point, 3.0, 0.1)
        .unwrap();
    assert!(history.undo(&mut project));
    assert_eq!(project, loaded);
    let mut legacy = serde_json::to_value(Project::demo()).unwrap();
    for track in legacy["tracks"].as_array_mut().unwrap() {
        track.as_object_mut().unwrap().remove("automation");
    }
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    legacy.validate().unwrap();
    assert!(legacy.tracks.iter().all(|track| track.automation.is_none()));
}
