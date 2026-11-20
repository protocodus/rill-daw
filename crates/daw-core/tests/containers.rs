use daw_core::{
    AutomationParameter, AutomationTarget, Effect, EffectLane, EffectSlot, MAX_CONTAINER_DEPTH,
    PluginInputRoute, Project, commands::SongEdit,
};

fn chain(inserts: Vec<EffectSlot>) -> EffectSlot {
    EffectSlot::new(
        "R1 Chain",
        Effect::Container {
            device: "r1-chain".into(),
            lanes: vec![EffectLane { inserts }],
        },
    )
}

fn device(name: &str) -> EffectSlot {
    EffectSlot::new(
        name,
        Effect::Builtin {
            device: "r1-filter".into(),
        },
    )
}

fn project_with(slot: EffectSlot) -> (Project, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project.track_mut(track).unwrap().inserts.push(slot);
    (project, track)
}

#[test]
fn nested_effects_are_found_saved_and_reloaded() {
    let inner = device("Inner");
    let inner_id = inner.id;
    let (project, track) = project_with(chain(vec![chain(vec![inner])]));
    project.validate().unwrap();
    assert_eq!(
        project.track(track).unwrap().effect(inner_id).unwrap().name,
        "Inner"
    );
    let text = serde_json::to_string(&project).unwrap();
    let reloaded: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(reloaded, project);
}

#[test]
fn nesting_depth_and_duplicate_ids_are_bounded() {
    let mut slot = device("Leaf");
    for _ in 0..MAX_CONTAINER_DEPTH {
        slot = chain(vec![slot]);
    }
    let (project, _) = project_with(slot.clone());
    project.validate().unwrap();
    let (project, _) = project_with(chain(vec![slot]));
    assert!(project.validate().is_err());

    let leaf = device("Leaf");
    let (project, _) = project_with(chain(vec![leaf.clone(), leaf]));
    assert!(project.validate().is_err());
}

#[test]
fn nested_effects_take_no_extra_inputs() {
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let mut leaf = device("Leaf");
    leaf.inputs.push(PluginInputRoute {
        source_track: source,
        bus: 1,
        first_channel: 0,
        gain: 1.0,
        pre_fader: false,
    });
    let track = project.add_audio_track("Audio");
    project
        .track_mut(track)
        .unwrap()
        .inserts
        .push(chain(vec![leaf]));
    assert!(project.validate().is_err());
}

#[test]
fn edits_and_automation_reach_nested_effects() {
    let leaf = device("Leaf");
    let leaf_id = leaf.id;
    let other = device("Other");
    let other_id = other.id;
    let (mut project, track) = project_with(chain(vec![leaf, other]));
    let target = AutomationTarget {
        track,
        parameter: AutomationParameter::VstParameter {
            effect: Some(leaf_id),
            id: 1,
        },
    };
    target.validate(&project).unwrap();
    daw_core::commands::EditBatch {
        label: "Nested".into(),
        edits: vec![
            SongEdit::SetInsertEnabled {
                track: track.into(),
                effect: leaf_id,
                enabled: false,
            },
            SongEdit::ReorderInsert {
                track: track.into(),
                effect: other_id,
                index: 0,
            },
        ],
    }
    .apply(&mut project)
    .unwrap();
    let Effect::Container { lanes, .. } = &project.track(track).unwrap().inserts[0].effect else {
        unreachable!()
    };
    assert_eq!(lanes[0].inserts[0].id, other_id);
    assert!(!lanes[0].inserts[1].enabled);
    project.remove_insert(track, leaf_id).unwrap();
    assert!(project.track(track).unwrap().effect(leaf_id).is_none());
    assert!(target.validate(&project).is_err());
}
