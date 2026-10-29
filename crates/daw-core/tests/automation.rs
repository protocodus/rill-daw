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
        override_value: None,
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
        Interpolation::Spline,
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
fn spline_segments_move_continuously_between_logarithmic_linear_and_exponential() {
    let mut lane = curve(
        Interpolation::Spline,
        &[(2.0, 0.0), (6.0, 1.0), (10.0, 0.0)],
    );
    let first = lane.points[0].id;
    let second = lane.points[1].id;
    for (shape, midpoint) in [
        (-1.0, 0.875),
        (-0.5, 0.6875),
        (0.0, 0.5),
        (0.5, 0.3125),
        (1.0, 0.125),
    ] {
        lane.set_point_curve(first, shape).unwrap();
        assert!((lane.value_at(4.0).unwrap() - midpoint).abs() < 1e-6);
        assert!(
            (lane.value_at(8.0).unwrap() - 0.5).abs() < 1e-6,
            "Changing one segment must not change its neighbor"
        );
        assert_eq!(lane.value_at(2.0), Some(0.0));
        assert_eq!(lane.value_at(6.0), Some(1.0));
        let mut previous = 0.0;
        for sample in 0..=1024 {
            let value = lane.value_at(2.0 + 4.0 * sample as f64 / 1024.0).unwrap();
            assert!(
                (previous..=1.0).contains(&value),
                "Spline must remain monotone and bounded"
            );
            previous = value;
        }
    }
    lane.set_point_curve(second, -1.0).unwrap();
    assert!((lane.value_at(8.0).unwrap() - 0.125).abs() < 1e-6);
    let mut previous = 1.0;
    for sample in 0..=1024 {
        let value = lane.value_at(6.0 + 4.0 * sample as f64 / 1024.0).unwrap();
        assert!(
            (0.0..=previous).contains(&value),
            "Falling spline must stay monotone too"
        );
        previous = value;
    }
    // Holding is independent of the displayed song extent and segment shape.
    assert_eq!(lane.value_at(f64::NEG_INFINITY), Some(0.0));
    assert_eq!(lane.value_at(-1000.0), Some(0.0));
    assert_eq!(lane.value_at(1e100), Some(0.0));
    assert_eq!(lane.value_at(f64::INFINITY), Some(0.0));
}

#[test]
fn spline_shapes_follow_point_identity_and_survive_value_replacement() {
    let mut lane = curve(Interpolation::Linear, &[(0.0, 0.2), (2.0, 0.8), (4.0, 0.6)]);
    let id = lane.points[1].id;
    lane.set_point_curve(id, 0.7).unwrap();
    assert_eq!(lane.interpolation, Interpolation::Spline);
    lane.upsert_point(2.0, 0.9).unwrap();
    assert_eq!(lane.points[1].curve, 0.7);
    lane.move_point(id, 5.0, 0.1).unwrap();
    assert_eq!((lane.points[2].id, lane.points[2].curve), (id, 0.7));
    // A final point's saved shape has no effect on the indefinite hold.
    assert_eq!(lane.value_at(f64::INFINITY), Some(0.1));
    lane.upsert_point(7.0, 0.9).unwrap();
    assert!(lane.value_at(6.0).unwrap() < 0.5);
    let before = lane.clone();
    for invalid in [f32::NAN, f32::INFINITY, -1.01, 1.01] {
        assert!(lane.set_point_curve(id, invalid).is_err());
        assert_eq!(lane, before);
    }
    assert!(lane.set_point_curve(u64::MAX, 0.0).is_err());
    assert_eq!(lane, before);
}

#[test]
fn overrides_hold_everywhere_without_changing_curves_and_resume_exactly() {
    let mut lane = curve(Interpolation::Spline, &[(4.0, 0.2), (8.0, 0.8)]);
    lane.set_point_curve(lane.points[0].id, 0.5).unwrap();
    let original = lane.clone();
    let beats = [
        f64::NEG_INFINITY,
        -10.0,
        0.0,
        4.0,
        6.0,
        8.0,
        1e100,
        f64::INFINITY,
    ];
    for value in [0.0, 0.33, 1.0] {
        lane.set_override(Some(value)).unwrap();
        for beat in beats {
            assert_eq!(lane.effective_value_at(beat), Some(value));
            assert_eq!(lane.value_at(beat), original.value_at(beat));
        }
        assert_eq!(lane.effective_value_at(f64::NAN), None);
        assert_eq!(lane.points, original.points);
    }
    let before = lane.clone();
    for value in [f32::NAN, f32::NEG_INFINITY, -0.01, 1.01] {
        assert!(lane.set_override(Some(value)).is_err());
        assert_eq!(lane, before);
    }
    lane.set_override(None).unwrap();
    assert_eq!(lane, original);
    for beat in beats {
        assert_eq!(lane.effective_value_at(beat), original.value_at(beat));
    }
}

#[test]
fn legacy_automation_defaults_new_fields_without_changing_its_sound() {
    for interpolation in [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::Smooth,
    ] {
        let original = curve(interpolation, &[(1.0, 0.1), (3.0, 0.9), (5.0, 0.2)]);
        let mut json = serde_json::to_value(&original).unwrap();
        json.as_object_mut().unwrap().remove("override_value");
        for point in json["points"].as_array_mut().unwrap() {
            point.as_object_mut().unwrap().remove("curve");
        }
        let loaded: AutomationLane = serde_json::from_value(json).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded, original);
        for step in 0..=100 {
            let beat = step as f64 * 0.1;
            assert_eq!(loaded.effective_value_at(beat), original.value_at(beat));
        }
    }
}

#[test]
fn spline_override_semantic_edits_roundtrip_undo_and_reject_invalid_batches() {
    use daw_core::{EditBatch, SongEdit};
    let (mut project, source, lane_id) = fixture();
    let target = AutomationTarget {
        track: source,
        parameter: Parameter::TrackGain,
    };
    assert_eq!(project.automation_for_target(target).unwrap().0, lane_id);
    assert_eq!(
        project.automation_lane(lane_id).unwrap().interpolation,
        Interpolation::Spline
    );
    let point = project.automation_lane(lane_id).unwrap().points[0].id;
    let original = project.clone();
    let edits = EditBatch {
        label: "Shape and override automation".into(),
        edits: vec![
            SongEdit::SetAutomationPointCurve {
                track: lane_id.into(),
                point,
                curve: -0.8,
            },
            SongEdit::SetAutomationOverride {
                track: lane_id.into(),
                value: Some(0.3),
            },
        ],
    };
    edits.apply(&mut project).unwrap();
    let mut history = History::new();
    history.commit_named(&original, "Shape and override automation");
    let changed = project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("spline-override.rill");
    project.save(&path).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
    assert!(history.undo(&mut project));
    assert_eq!(project, original);
    assert!(history.redo(&mut project));
    assert_eq!(project, changed);
    let invalid = EditBatch {
        label: "Reject invalid curve".into(),
        edits: vec![
            SongEdit::SetAutomationOverride {
                track: lane_id.into(),
                value: None,
            },
            SongEdit::SetAutomationPointCurve {
                track: lane_id.into(),
                point,
                curve: 2.0,
            },
        ],
    };
    assert!(invalid.apply(&mut project).is_err());
    assert_eq!(project, changed);
    project.automation_lane_mut(lane_id).unwrap().points[0].curve = f32::NAN;
    assert!(project.validate().is_err());
    project = changed.clone();
    project.automation_lane_mut(lane_id).unwrap().override_value = Some(f32::INFINITY);
    assert!(project.validate().is_err());
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
        .push(EffectSlot::new(
            "Invalid",
            Effect::Vst3 {
                class_id: None,
                path: "/fixture.vst3".into(),
            },
        ));
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
        Effect::Vst3 {
            class_id: None,
            path: "/echo.vst3".into(),
        },
    );
    let effect = delay.id;
    project.track_mut(source).unwrap().inserts.push(delay);
    let delay_lane = project
        .add_automation_track_with_value(
            "Wet",
            AutomationTarget {
                track: source,
                parameter: Parameter::VstParameter {
                    effect: Some(effect),
                    id: 42,
                },
            },
            0.25,
        )
        .unwrap();
    assert!(
        AutomationTarget {
            track: source,
            parameter: Parameter::VstParameter {
                effect: Some(u64::MAX),
                id: 42
            }
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
                class_id: None,
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
                class_id: None,
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
                class_id: None,
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
