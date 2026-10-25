use daw_core::{Clip, ClipContent, History, MidiInputChannel, Note, Project};

#[test]
fn midi_routes_and_recording_timing_round_trip_and_legacy_defaults_are_quiet() {
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    assert_eq!(
        project.track(track).unwrap().midi_input,
        Some(MidiInputChannel::All)
    );
    project.track_mut(track).unwrap().midi_input = Some(MidiInputChannel::Channel { channel: 15 });
    project.precount_bars = 2;
    project.metronome_enabled = true;
    project.metronome_gain = 0.4;
    let value = serde_json::to_value(&project).unwrap();
    let decoded: Project = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded, project);
    decoded.validate().unwrap();
    let mut legacy = value;
    let object = legacy.as_object_mut().unwrap();
    for key in ["metronome_enabled", "metronome_gain", "precount_bars"] {
        object.remove(key);
    }
    legacy["tracks"][0]
        .as_object_mut()
        .unwrap()
        .remove("midi_input");
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.metronome_enabled);
    assert_eq!(legacy.metronome_gain, 0.25);
    assert_eq!(legacy.precount_bars, 0);
    assert!(legacy.track(track).unwrap().midi_input.is_none());
    assert!(
        MidiInputChannel::Channel { channel: 16 }
            .validate()
            .is_err()
    );
    project.precount_bars = 3;
    assert!(project.validate().is_err());
    project.precount_bars = 0;
    project.metronome_gain = f32::NAN;
    assert!(project.validate().is_err());
}
#[test]
fn recorded_midi_adds_independent_layer_and_invalid_publication_is_atomic() {
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    let previous = Clip::midi("Backing", 0.0, 4.0, vec![Note::new(48, 0.0, 4.0, 80)]);
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(previous.clone());
    let before = project.clone();
    let recording = Clip::midi(
        "Recorded",
        1.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 100), Note::new(64, 1.0, 1.0, 90)],
    );
    let id = project
        .insert_recorded_midi(track, recording.clone())
        .unwrap();
    assert_eq!(project.clip(track, previous.id), Some(&previous));
    assert_eq!(project.clip(track, id), Some(&recording));
    assert!(project.clip(track, id).unwrap().link_group.is_none());
    let current = project.clone();
    let invalid = Clip::midi("Invalid", 0.0, 1.0, vec![Note::new(60, 0.0, 1.0, 0)]);
    assert!(project.insert_recorded_midi(track, invalid).is_err());
    assert_eq!(project, current);
    let mut history = History::default();
    history.commit_named(&before, "Record MIDI");
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    assert!(history.redo(&mut project));
    assert_eq!(project, current);
    assert!(matches!(&recording.content,ClipContent::Midi{notes} if notes.len()==2));
}
