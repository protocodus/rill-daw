//! Mouse-driven instrument destinations and deliberate clip entry.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn tracks(state: &Value) -> &[Value] {
    state["project"]["tracks"].as_array().unwrap()
}

#[test]
fn initial_midi_assignment_and_replacement_keep_the_same_track_and_pattern() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let initial = menu(&mut app, "Track", "Add MIDI track");
    assert_eq!(tracks(&initial).len(), 1);
    let track = tracks(&initial)[0]["id"].clone();
    let pattern = tracks(&initial)[0]["clips"][0].clone();
    common::wait_library_device(&mut app, "Rill Fixture A");
    assert!(visible(&app.inspect(), "Assign to New instrument"));
    let assigned = app.click("Assign instrument");
    assert_eq!(tracks(&assigned).len(), 1);
    assert_eq!(tracks(&assigned)[0]["id"], track);
    assert_eq!(tracks(&assigned)[0]["clips"][0], pattern);
    assert_eq!(
        tracks(&assigned)[0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    assert_eq!(assigned["popups"]["library"], false);
    assert_eq!(assigned["popups"]["instrument"], false);
    assert_eq!(assigned["view"], "Arrangement");
    assert_eq!(assigned["native_editor"]["open"], false);

    menu(&mut app, "Window", "Instrument");
    app.click("Change instrument");
    common::wait_library_device(&mut app, "Rill Fixture B");
    assert!(visible(&app.inspect(), "Assign to New instrument"));
    app.click("Rill Fixture B");
    let replaced = app.click("Assign instrument");
    assert_eq!(tracks(&replaced).len(), 1);
    assert_eq!(tracks(&replaced)[0]["id"], track);
    assert_eq!(tracks(&replaced)[0]["clips"][0], pattern);
    assert_eq!(
        tracks(&replaced)[0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    app.key("Escape");
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(tracks(&undone).len(), 1);
    assert_eq!(
        tracks(&undone)[0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    app.quit();
}

#[test]
fn assigning_from_clip_editor_returns_to_the_same_phrase_without_opening_an_editor() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    let track = project.add_midi_track("Lead");
    let first = Clip::midi("Opening", 0.0, 4.0, vec![]);
    let second = Clip::midi("Second phrase", 4.0, 4.0, vec![]);
    let phrase = second.id;
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let file = app.directory.path().join("assign-from-clip.rill");
    project.save(&file).unwrap();
    app.command(json!({"command":"open","path":file}));
    app.click("Second phrase");
    let before = menu(&mut app, "Window", "Clip Editor");
    assert_eq!(before["view"], "Clip");
    assert_eq!(before["selected_clip"], phrase);
    let clips = tracks(&before)[0]["clips"].clone();
    app.click("Choose instrument…");
    common::wait_library_device(&mut app, "Rill Fixture A");
    let assigned = app.click("Assign instrument");
    assert_eq!(assigned["view"], "Clip");
    assert_eq!(assigned["selected_track"], track);
    assert_eq!(assigned["selected_clip"], phrase);
    assert_eq!(assigned["popups"]["library"], false);
    assert_eq!(assigned["popups"]["instrument"], false);
    assert_eq!(assigned["native_editor"]["open"], false);
    assert_eq!(tracks(&assigned)[0]["clips"], clips);
    assert_eq!(
        tracks(&assigned)[0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    let explicit = menu(&mut app, "Window", "Instrument");
    assert_eq!(explicit["popups"]["instrument"], true);
    assert!(visible(&explicit, "Open editor"));
    app.quit();
}

#[test]
fn explicit_add_instrument_keeps_an_intentionally_empty_track() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let empty = menu(&mut app, "Track", "Add MIDI track");
    let original = tracks(&empty)[0].clone();
    app.key("Escape");
    menu(&mut app, "Track", "Add instrument…");
    common::wait_library_device(&mut app, "Rill Fixture A");
    assert!(visible(&app.inspect(), "Create a new MIDI track"));
    let added = app.click("Add instrument track");
    assert_eq!(tracks(&added).len(), 2);
    assert_eq!(tracks(&added)[0], original);
    assert_eq!(tracks(&added)[1]["name"], "Rill Fixture A");
    app.key("Escape");
    assert_eq!(tracks(&app.inspect()).len(), 2);
    app.quit();
}

fn clip_center(state: &Value, id: u64) -> (f64, f64) {
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"];
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

#[test]
fn pencil_double_click_and_context_actions_open_existing_clips_without_drawing() {
    let mut app = Harness::new();
    let input = app.directory.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&input)
    .unwrap();
    let mut project = Project::new();
    let midi_track = project.add_midi_track("Lead");
    let midi = Clip::midi("Melody", 0.0, 4.0, vec![]);
    let midi_id = midi.id;
    project.track_mut(midi_track).unwrap().clips.push(midi);
    let audio_track = project.add_audio_track("Audio");
    let audio = Clip::audio("Recording", 0.0, 4.0, input.to_string_lossy());
    let audio_id = audio.id;
    project.track_mut(audio_track).unwrap().clips.push(audio);
    let file = app.directory.path().join("clip-entry.rill");
    project.save(&file).unwrap();
    let original = app.command(json!({"command":"open","path":file}))["project"].clone();
    app.click("Pencil");
    let state = app.click("Fit");
    let (x, y) = clip_center(&state, midi_id);
    // egui's click counter is time-based across widgets. Let the preceding
    // toolbar clicks expire before sending this deliberate double-click.
    for _ in 0..16 {
        app.inspect();
    }
    let opened = app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    assert_eq!(opened["view"], "Clip");
    assert_eq!(opened["selected_clip"], midi_id);
    assert_eq!(opened["project"], original);
    app.click("Back");

    for (id, label) in [(midi_id, "Edit notes"), (audio_id, "Edit audio")] {
        let state = app.inspect();
        let (x, y) = clip_center(&state, id);
        app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
        let opened = app.click(label);
        assert_eq!(opened["view"], "Clip");
        assert_eq!(opened["selected_clip"], id);
        assert_eq!(opened["project"], original);
        app.click("Back");
    }
    app.quit();
}
