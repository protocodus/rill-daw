use daw_core::*;
use std::collections::HashMap;

fn unchanged_tracks(before: &Project, after: &Project, parent_change: Option<(u64, Option<u64>)>) {
    assert_eq!(before.tracks.len(), after.tracks.len());
    let before_by_id: HashMap<_, _> = before
        .tracks
        .iter()
        .map(|track| (track.id, track))
        .collect();
    for track in &after.tracks {
        let mut expected = before_by_id[&track.id].clone();
        if let Some((id, parent)) = parent_change
            && track.id == id
        {
            expected.parent_folder = parent;
        }
        assert_eq!(
            *track, expected,
            "Only track order and the moved root's parent can change"
        );
    }
    let mut expected = before.clone();
    expected.tracks = after.tracks.clone();
    assert_eq!(
        *after, expected,
        "Track placement cannot change song settings"
    );
    after.validate().unwrap();
}

struct FolderFixture {
    project: Project,
    first: u64,
    folder: u64,
    bus: u64,
    nested: u64,
    midi: u64,
    audio: u64,
    automation: u64,
}

fn folder_fixture() -> FolderFixture {
    let mut project = Project::new();
    let first = project.add_audio_track("First");
    let folder = project.add_folder("Folder");
    let bus = project.add_bus("Room");
    let nested = project.add_folder("Nested");
    let midi = project.add_midi_track("Synth");
    let audio = project.add_audio_track("Layer");
    let automation = project
        .add_automation_track(
            "Level",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.set_parent_folder(nested, Some(folder)).unwrap();
    project.set_parent_folder(audio, Some(folder)).unwrap();
    project.set_parent_folder(midi, Some(nested)).unwrap();
    project.set_parent_folder(automation, Some(nested)).unwrap();
    project.track_mut(folder).unwrap().collapsed = true;
    project.track_mut(nested).unwrap().mute = true;
    project.set_output(audio, Some(bus)).unwrap();
    project.add_send(midi, bus, 0.3, true).unwrap();
    let synth = project.track_mut(midi).unwrap();
    synth.instrument = Instrument::Vst3 {
        path: "Synth.vst3".into(),
        class_id: None,
    };
    synth.instrument_state = vec![17, 45, 93];
    synth.instrument_parameters = vec![PluginParameterValue { id: 3, value: 0.43 }];
    synth.latency_samples = 384;
    synth.clips.push(Clip::midi(
        "Phrase",
        8.0,
        4.0,
        vec![Note::new(60, 1.0, 2.0, 85)],
    ));
    let mut slot = EffectSlot::new(
        "EQ",
        Effect::Vst3 {
            path: "EQ.vst3".into(),
            class_id: None,
        },
    );
    slot.state = vec![4, 2];
    slot.parameters = vec![PluginParameterValue { id: 7, value: 0.27 }];
    project.track_mut(audio).unwrap().inserts.push(slot);
    project
        .track_mut(audio)
        .unwrap()
        .clips
        .push(Clip::audio("Take", 2.0, 4.0, "source.wav"));
    project
        .set_automation_override(automation, Some(0.2))
        .unwrap();
    project.validate().unwrap();
    FolderFixture {
        project,
        first,
        folder,
        bus,
        nested,
        midi,
        audio,
        automation,
    }
}

#[test]
fn canonical_order_includes_collapsed_nested_descendants_without_mutating_the_song() {
    let f = folder_fixture();
    let before = f.project.clone();
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.first,
            f.folder,
            f.nested,
            f.midi,
            f.automation,
            f.audio,
            f.bus
        ]
    );
    assert_eq!(f.project, before);
}

#[test]
fn every_track_kind_can_move_before_and_after_while_preserving_exact_contents() {
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    let midi = project.add_midi_track("MIDI");
    let bus = project.add_bus("Bus");
    let folder = project.add_folder("Folder");
    let automation = project
        .add_automation_track(
            "Pan",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackPan,
            },
        )
        .unwrap();
    let before = project.clone();
    for source in [midi, bus, folder, automation] {
        let first = project.ordered_track_ids()[0];
        assert!(
            project
                .reorder_track(source, first, TrackPlacement::Before)
                .unwrap()
        );
    }
    assert_eq!(
        project.ordered_track_ids(),
        vec![automation, folder, bus, midi, audio]
    );
    assert!(
        project
            .reorder_track(automation, audio, TrackPlacement::After)
            .unwrap()
    );
    assert_eq!(
        project.ordered_track_ids(),
        vec![folder, bus, midi, audio, automation]
    );
    unchanged_tracks(&before, &project, None);
}

#[test]
fn collapsed_folders_move_as_complete_blocks_and_into_folder_changes_only_the_root_parent() {
    let mut f = folder_fixture();
    let before = f.project.clone();
    assert!(
        f.project
            .reorder_track(f.folder, f.bus, TrackPlacement::After)
            .unwrap()
    );
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.first,
            f.bus,
            f.folder,
            f.nested,
            f.midi,
            f.automation,
            f.audio
        ]
    );
    unchanged_tracks(&before, &f.project, None);
    let destination = f.project.add_folder("Destination");
    let before = f.project.clone();
    assert!(
        f.project
            .reorder_track(f.folder, destination, TrackPlacement::IntoFolder)
            .unwrap()
    );
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.first,
            f.bus,
            destination,
            f.folder,
            f.nested,
            f.midi,
            f.automation,
            f.audio
        ]
    );
    unchanged_tracks(&before, &f.project, Some((f.folder, Some(destination))));
}

#[test]
fn before_after_and_inside_follow_destination_hierarchy_without_reordering_other_siblings() {
    let mut f = folder_fixture();
    let before = f.project.clone();
    assert!(
        f.project
            .reorder_track(f.midi, f.folder, TrackPlacement::Before)
            .unwrap()
    );
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.first,
            f.midi,
            f.folder,
            f.nested,
            f.automation,
            f.audio,
            f.bus
        ]
    );
    unchanged_tracks(&before, &f.project, Some((f.midi, None)));
    let before = f.project.clone();
    assert!(
        f.project
            .reorder_track(f.midi, f.automation, TrackPlacement::After)
            .unwrap()
    );
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.first,
            f.folder,
            f.nested,
            f.automation,
            f.midi,
            f.audio,
            f.bus
        ]
    );
    unchanged_tracks(&before, &f.project, Some((f.midi, Some(f.nested))));
    assert!(
        f.project
            .reorder_track(f.first, f.folder, TrackPlacement::IntoFolder)
            .unwrap()
    );
    assert_eq!(
        f.project.ordered_track_ids(),
        vec![
            f.folder,
            f.nested,
            f.automation,
            f.midi,
            f.audio,
            f.first,
            f.bus
        ]
    );
}

#[test]
fn no_ops_and_invalid_targets_leave_even_noncanonical_saved_order_untouched() {
    let mut f = folder_fixture();
    let before = f.project.clone();
    for (source, target, placement) in [
        (f.first, f.first, TrackPlacement::Before),
        (f.first, f.first, TrackPlacement::After),
        (f.folder, f.first, TrackPlacement::After),
        (f.audio, f.folder, TrackPlacement::IntoFolder),
        (f.midi, f.automation, TrackPlacement::Before),
    ] {
        assert!(!f.project.reorder_track(source, target, placement).unwrap());
        assert_eq!(f.project, before);
    }
    for (source, target, placement) in [
        (f.folder, f.folder, TrackPlacement::IntoFolder),
        (f.folder, f.midi, TrackPlacement::Before),
        (f.folder, f.nested, TrackPlacement::After),
        (f.folder, f.nested, TrackPlacement::IntoFolder),
        (f.first, f.midi, TrackPlacement::IntoFolder),
        (u64::MAX, f.first, TrackPlacement::Before),
        (f.first, u64::MAX, TrackPlacement::After),
    ] {
        assert!(f.project.reorder_track(source, target, placement).is_err());
        assert_eq!(f.project, before);
    }
}

#[test]
fn ordering_is_bounded_for_incomplete_editor_hierarchies_and_reordering_rejects_them() {
    let mut project = Project::new();
    let a = project.add_folder("A");
    let b = project.add_folder("B");
    let c = project.add_audio_track("Orphan");
    project.track_mut(a).unwrap().parent_folder = Some(b);
    project.track_mut(b).unwrap().parent_folder = Some(a);
    project.track_mut(c).unwrap().parent_folder = Some(u64::MAX);
    let before = project.clone();
    assert_eq!(project.ordered_track_ids(), vec![a, b, c]);
    assert!(project.reorder_track(c, a, TrackPlacement::Before).is_err());
    assert_eq!(project, before);
}
