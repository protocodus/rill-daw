use daw_core::*;
use serde_json::json;

fn instrument(project: &mut Project, name: &str) -> u64 {
    let id = project.add_midi_track(name);
    project
        .set_instrument(
            id,
            Instrument::Vst3 {
                path: "instrument.vst3".into(),
                class_id: None,
            },
        )
        .unwrap();
    id
}
fn effect(project: &mut Project, track: u64) -> u64 {
    let slot = EffectSlot::new(
        "Insert",
        Effect::Vst3 {
            path: "effect.vst3".into(),
            class_id: None,
        },
    );
    let id = slot.id;
    project.track_mut(track).unwrap().inserts.push(slot);
    id
}
fn input(source_track: u64, bus: u32, first_channel: u32) -> PluginInputRoute {
    PluginInputRoute {
        source_track,
        bus,
        first_channel,
        gain: 0.5,
        pre_fader: false,
    }
}
fn apply(project: &mut Project, edits: serde_json::Value) -> anyhow::Result<EditResult> {
    serde_json::from_value::<EditBatch>(json!({"label":"Production routing","edits":edits}))
        .unwrap()
        .apply(project)
}

#[test]
fn legacy_defaults_master_identity_and_full_master_state_round_trip() {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let mut legacy = serde_json::to_value(&project).unwrap();
    legacy.as_object_mut().unwrap().remove("master");
    for track in legacy["tracks"].as_array_mut().unwrap() {
        for field in ["input_gain", "instrument_inputs", "instrument_output"] {
            track.as_object_mut().unwrap().remove(field);
        }
    }
    let mut restored: Project = serde_json::from_value(legacy).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.master.id, MASTER_CHANNEL_ID);
    assert_eq!(restored.master.gain, 1.0);
    assert_eq!(restored.track(track).unwrap().input_gain, 1.0);
    assert_eq!(restored.channels().count(), restored.tracks.len() + 1);
    let master_effect = effect(&mut restored, MASTER_CHANNEL_ID);
    restored.master.input_gain = 0.5;
    restored.master.gain = 0.7;
    restored.master.pan = 0.2;
    restored.master.inserts[0].state = vec![1, 2, 3];
    restored.master.inserts[0]
        .parameters
        .push(PluginParameterValue { id: 7, value: 0.3 });
    restored
        .add_automation_track(
            "Master trim",
            AutomationTarget {
                track: MASTER_CHANNEL_ID,
                parameter: AutomationParameter::TrackInputGain,
            },
        )
        .unwrap();
    restored
        .add_automation_track_with_value(
            "Master insert",
            AutomationTarget {
                track: MASTER_CHANNEL_ID,
                parameter: AutomationParameter::VstParameter {
                    effect: Some(master_effect),
                    id: 7,
                },
            },
            0.3,
        )
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for extension in ["daw.json", "rill"] {
        let path = dir.path().join(format!("master.{extension}"));
        restored.save(&path).unwrap();
        let loaded = Project::load(&path).unwrap();
        assert_eq!(loaded.master.id, MASTER_CHANNEL_ID);
        assert_eq!(loaded.master.input_gain, 0.5);
        assert_eq!(loaded.master.inserts[0].id, master_effect);
        assert_eq!(loaded.master.inserts[0].state, [1, 2, 3]);
        assert_eq!(
            loaded.master.inserts[0].parameters,
            restored.master.inserts[0].parameters
        );
        assert!(
            loaded
                .automation_for_target(AutomationTarget {
                    track: MASTER_CHANNEL_ID,
                    parameter: AutomationParameter::TrackInputGain
                })
                .is_some()
        );
        assert!(new_id() > master_effect);
    }
    let mut history = History::new();
    history.commit_named(&project, "Master processing");
    project = restored;
    assert!(history.undo(&mut project));
    assert!(project.master.inserts.is_empty());
    assert!(history.redo(&mut project));
    assert_eq!(project.master.inserts[0].id, master_effect);
}

#[test]
fn master_cannot_become_an_arranger_object_or_feedback_source() {
    let mut project = Project::new();
    let bus = project.add_bus("Bus");
    assert!(project.delete_track(MASTER_CHANNEL_ID).is_err());
    assert!(project.set_output(MASTER_CHANNEL_ID, Some(bus)).is_err());
    assert!(
        project
            .add_send(MASTER_CHANNEL_ID, bus, 1.0, false)
            .is_err()
    );
    assert!(project.set_parent_folder(MASTER_CHANNEL_ID, None).is_err());
    assert!(
        project
            .reorder_track(MASTER_CHANNEL_ID, bus, TrackPlacement::After)
            .is_err()
    );
    let before = project.clone();
    assert!(
        apply(
            &mut project,
            json!([{"op":"set_mix","channels":[{"track":0,"input_gain":0.5,"solo":true}]}])
        )
        .is_err()
    );
    assert_eq!(project, before);
    project.master.id = bus;
    assert!(project.validate().is_err());
    project.master.id = MASTER_CHANNEL_ID;
    project.tracks[0].id = MASTER_CHANNEL_ID;
    assert!(project.validate().is_err());
}

#[test]
fn sidechains_and_native_output_dependencies_are_acyclic_even_when_silent() {
    let mut project = Project::new();
    let source = project.add_audio_track("Key");
    let other = project.add_audio_track("Other key");
    let owner = instrument(&mut project, "Synth");
    let child = project
        .add_instrument_output(owner, "Output 2", 1, 0)
        .unwrap();
    let insert = effect(&mut project, child);
    project
        .set_plugin_inputs(owner, None, vec![input(source, 0, 0)])
        .unwrap();
    project
        .set_plugin_inputs(
            child,
            Some(insert),
            vec![input(source, 1, 0), input(other, 1, 0)],
        )
        .unwrap();
    let order = project.routing_order().unwrap();
    let index = |id| order.iter().position(|candidate| *candidate == id).unwrap();
    assert!(index(source) < index(owner) && index(owner) < index(child));
    assert!(index(other) < index(child));
    let before = project.clone();
    let mut silent_feedback = input(child, 0, 0);
    silent_feedback.gain = 0.0;
    assert!(
        project
            .set_plugin_inputs(owner, None, vec![silent_feedback])
            .is_err()
    );
    assert_eq!(project, before);
    assert!(
        !project
            .eligible_plugin_input_sources(owner)
            .unwrap()
            .iter()
            .any(|track| track.id == child)
    );
    let master_insert = effect(&mut project, MASTER_CHANNEL_ID);
    project
        .set_plugin_inputs(
            MASTER_CHANNEL_ID,
            Some(master_insert),
            vec![input(child, 1, 0)],
        )
        .unwrap();
    assert!(
        project
            .set_plugin_inputs(child, Some(insert), vec![input(MASTER_CHANNEL_ID, 1, 0)])
            .is_err()
    );
    project.validate().unwrap();
}

#[test]
fn processor_input_validation_rejects_duplicates_or_malformed_pairs_without_guessing_main_buses() {
    let mut project = Project::new();
    let source = project.add_audio_track("Key");
    let target = project.add_audio_track("Target");
    let insert = effect(&mut project, target);
    for routes in [
        vec![input(source, 1, 0), input(source, 1, 0)],
        vec![input(source, 1, 1)],
        vec![input(source, MAX_PLUGIN_AUDIO_BUSES, 0)],
        vec![input(source, 1, MAX_PLUGIN_AUDIO_CHANNELS)],
        vec![PluginInputRoute {
            gain: f32::NAN,
            ..input(source, 1, 0)
        }],
    ] {
        let before = project.clone();
        assert!(
            project
                .set_plugin_inputs(target, Some(insert), routes)
                .is_err()
        );
        assert_eq!(project, before);
    }
    project
        .set_plugin_inputs(target, Some(insert), vec![input(source, 0, 0)])
        .unwrap();
    project
        .set_plugin_inputs(target, Some(insert), vec![input(source, 0, 2)])
        .unwrap();
    project.validate().unwrap();
}

#[test]
fn output_children_keep_group_order_independent_processing_and_cleanup() {
    let mut project = Project::new();
    let before = project.add_audio_track("Before");
    let owner = instrument(&mut project, "Drums");
    let after = project.add_audio_track("After");
    let left = project.add_instrument_output(owner, "Kick", 1, 0).unwrap();
    let right = project.add_instrument_output(owner, "Snare", 1, 2).unwrap();
    let bus = project.add_bus("Mix");
    project.track_mut(left).unwrap().input_gain = 2.0;
    project.track_mut(right).unwrap().pan = 0.5;
    effect(&mut project, left);
    project.set_output(left, Some(bus)).unwrap();
    project.add_send(right, bus, 0.3, false).unwrap();
    assert_eq!(
        project.ordered_track_ids(),
        [before, owner, left, right, after, bus]
    );
    project
        .reorder_track(right, left, TrackPlacement::Before)
        .unwrap();
    assert_eq!(
        project.ordered_track_ids(),
        [before, owner, right, left, after, bus]
    );
    project
        .reorder_track(owner, after, TrackPlacement::After)
        .unwrap();
    assert_eq!(
        project.ordered_track_ids(),
        [before, after, owner, right, left, bus]
    );
    assert_eq!(project.track(left).unwrap().input_gain, 2.0);
    assert_eq!(project.track(right).unwrap().pan, 0.5);
    assert!(project.set_output(owner, Some(left)).is_err());
    assert!(project.add_send(before, right, 1.0, false).is_err());
    assert!(
        !project
            .eligible_output_buses(before)
            .unwrap()
            .iter()
            .any(|track| track.instrument_output.is_some())
    );
    assert!(
        project
            .add_instrument_output(owner, "Duplicate", 1, 0)
            .is_err()
    );
    let port_zero = project
        .add_instrument_output(owner, "Port zero can be auxiliary", 0, 0)
        .unwrap();
    assert!(
        project
            .track(port_zero)
            .unwrap()
            .instrument_output
            .is_some()
    );
    let insert = effect(&mut project, before);
    project
        .set_plugin_inputs(before, Some(insert), vec![input(left, 1, 0)])
        .unwrap();
    project
        .add_automation_track(
            "Kick pan",
            AutomationTarget {
                track: left,
                parameter: AutomationParameter::TrackPan,
            },
        )
        .unwrap();
    project.delete_track(owner).unwrap();
    assert!(project.track(left).is_none() && project.track(right).is_none());
    assert!(project.track(before).unwrap().inserts[0].inputs.is_empty());
    assert!(
        !project
            .tracks
            .iter()
            .any(|track| track.automation.is_some())
    );
    project.validate().unwrap();
}

#[test]
fn replacing_instrument_cleans_output_children_and_only_instrument_routes() {
    let mut project = Project::new();
    let source = project.add_audio_track("Key");
    let owner = instrument(&mut project, "Synth");
    let child = project.add_instrument_output(owner, "Extra", 1, 0).unwrap();
    let insert = effect(&mut project, owner);
    project
        .set_plugin_inputs(owner, None, vec![input(source, 0, 0)])
        .unwrap();
    project
        .set_plugin_inputs(owner, Some(insert), vec![input(source, 1, 0)])
        .unwrap();
    project.set_instrument(owner, Instrument::None).unwrap();
    assert!(project.track(child).is_none());
    assert!(project.track(owner).unwrap().instrument_inputs.is_empty());
    assert_eq!(project.track(owner).unwrap().inserts[0].inputs.len(), 1);
    project.validate().unwrap();
}

#[test]
fn bulk_native_outputs_commit_together_or_preserve_the_complete_project() {
    let mut project = Project::new();
    let owner = instrument(&mut project, "Multi");
    let ids = project
        .add_instrument_outputs(owner, vec![("Pair 2".into(), 0, 2), ("Aux".into(), 1, 0)])
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| project.track(*id).unwrap().gain == 1.0));
    let before = project.clone();
    assert!(
        project
            .add_instrument_outputs(
                owner,
                vec![("Valid".into(), 1, 2), ("Duplicate".into(), 1, 0)]
            )
            .is_err()
    );
    assert_eq!(project, before);
    assert!(
        project
            .add_instrument_outputs(owner, vec![("Many".into(), 2, 0); 129])
            .is_err()
    );
    assert_eq!(project, before);
}

#[test]
fn instrument_output_folders_follow_owner_for_assignment_and_reordering() {
    let mut project = Project::new();
    let owner = instrument(&mut project, "Owner");
    let outputs = project
        .add_instrument_outputs(owner, vec![("One".into(), 1, 0), ("Two".into(), 1, 2)])
        .unwrap();
    let folder = project.add_folder("Muted folder");
    project.track_mut(folder).unwrap().mute = true;
    project.set_parent_folder(owner, Some(folder)).unwrap();
    assert!(
        outputs
            .iter()
            .all(|id| project.track(*id).unwrap().parent_folder == Some(folder))
    );
    let before = project.clone();
    assert!(project.set_parent_folder(outputs[0], None).is_err());
    assert_eq!(project, before);
    let outside = project.add_audio_track("Outside");
    project
        .reorder_track(owner, outside, TrackPlacement::After)
        .unwrap();
    assert_eq!(project.track(owner).unwrap().parent_folder, None);
    assert!(
        outputs
            .iter()
            .all(|id| project.track(*id).unwrap().parent_folder.is_none())
    );
    let mut invalid = project.clone();
    invalid.track_mut(outputs[0]).unwrap().parent_folder = Some(folder);
    assert!(invalid.validate().is_err());
    project.validate().unwrap();
}

#[test]
fn pre_insert_mix_and_clip_mute_are_atomic_undoable_and_occurrence_local() {
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    let clip = Clip::midi("Phrase", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, id).unwrap();
    let before = project.clone();
    apply(&mut project, json!([
        {"op":"set_mix","channels":[{"track":0,"input_gain":0.5,"gain":0.8,"pan":-0.1},{"track":track,"input_gain_db":6.0}]},
        {"op":"set_clip_muted","track":track,"clip":id,"muted":true}
    ])).unwrap();
    assert!(project.clip(track, id).unwrap().muted);
    assert!(!project.clip(track, ghost).unwrap().muted);
    assert!((project.track(track).unwrap().input_gain - 10.0_f32.powf(0.3)).abs() < 1e-5);
    project.synchronize_linked_clips(&before).unwrap();
    assert!(!project.clip(track, ghost).unwrap().muted);
    project.validate().unwrap();
    let changed = project.clone();
    assert!(
        apply(
            &mut project,
            json!([{"op":"set_mix","channels":[{"track":0,"input_gain":2.0,"input_gain_db":3.0}]}])
        )
        .is_err()
    );
    assert_eq!(project, changed);
    let mut history = History::new();
    history.commit_named(&before, "Freeze source");
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    assert!(history.redo(&mut project));
    assert_eq!(project, changed);
    let mut clips = vec![
        Clip::audio("Older", 0.0, 4.0, "a.wav"),
        Clip::audio("Muted latest", 1.0, 2.0, "b.wav"),
    ];
    clips[1].muted = true;
    let schedule = AudioSchedule::new(&clips).unwrap();
    assert!(
        schedule
            .segments
            .iter()
            .all(|segment| segment.incoming == 0)
    );
}

#[test]
fn freeze_provenance_survives_split_and_recall_without_owning_deleted_sources() {
    let mut project = Project::new();
    let source = instrument(&mut project, "Original");
    let source_clip = Clip::midi("Source", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let source_id = source_clip.id;
    project.track_mut(source).unwrap().clips.push(source_clip);
    let print = project.add_audio_track("Print");
    let mut clip = Clip::audio("Printed", 0.0, 4.0, "print.wav");
    let id = clip.id;
    let master_effect = effect(&mut project, MASTER_CHANNEL_ID);
    let mut provenance = FreezeLink {
        group: new_id(),
        source_track: source,
        source_clips: vec![source_id],
        muted_tracks: vec![FreezeTrackMute {
            track: source,
            muted: false,
        }],
        input_copies: vec![FreezeInputCopy {
            track: MASTER_CHANNEL_ID,
            effect: Some(master_effect),
            route: input(print, 1, 0),
        }],
        active: true,
    };
    clip.freeze = Some(provenance.clone());
    project.track_mut(print).unwrap().clips.push(clip);
    project.set_clip_muted(source, source_id, true).unwrap();
    let source_right = project.split_clip(source, source_id, 2.0).unwrap();
    provenance.source_clips.push(source_right);
    assert_eq!(
        project.clip(print, id).unwrap().freeze.as_ref(),
        Some(&provenance)
    );
    let right = project.split_clip(print, id, 2.0).unwrap();
    assert_eq!(
        project.clip(print, right).unwrap().freeze.as_ref(),
        Some(&provenance)
    );
    project.set_clip_muted(print, right, true).unwrap();
    assert!(!project.clip(print, id).unwrap().muted);
    project.delete_track(source).unwrap();
    project.validate().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("provenance.daw.json");
    project.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(
        loaded.clip(print, id).unwrap().freeze.as_ref(),
        Some(&provenance)
    );
    assert!(new_id() > provenance.group.max(source).max(source_id));
    let mut invalid = project.clone();
    invalid
        .clip_mut(print, right)
        .unwrap()
        .freeze
        .as_mut()
        .unwrap()
        .active = false;
    assert!(invalid.validate().is_err());
    invalid = project;
    invalid
        .clip_mut(print, id)
        .unwrap()
        .freeze
        .as_mut()
        .unwrap()
        .source_clips
        .push(source_id);
    assert!(invalid.validate().is_err());
}
