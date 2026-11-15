//! Instrument names are the editing surface, while ordinary MIDI notes remain reachable.
#[path = "../../daw-plugins/tests/common/clap_probe_fixture.rs"]
mod clap_probe_fixture;
mod common;

use clap_probe_fixture::{ClapProbeFixture, SYNTH_ID};
use common::Harness;
use daw_core::{AudioData, Clip, DeviceSample, Instrument, Note, Project};
use serde_json::{Value, json};

const PHRASE: &str = "Named notes phrase";

fn open(app: &mut Harness, instrument: Instrument, notes: Vec<Note>) -> u64 {
    app.command(json!({"command":"viewport","width":1100,"height":800}));
    let mut project = Project::new();
    let track = project.add_midi_track("Named instrument");
    project.set_instrument(track, instrument).unwrap();
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi(PHRASE, 0.0, 8.0, notes));
    let path = app.directory.path().join("named-notes.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click(PHRASE);
    app.click("Window");
    app.click("Clip Editor");
    track
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn key(state: &Value, pitch: u8) -> &Value {
    state["midi_editor"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["pitch"] == pitch)
        .unwrap_or_else(|| panic!("Pitch {pitch} is not visible: {}", state["midi_editor"]))
}

fn pointer(app: &mut Harness, phase: &str, point: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":point.0,"y":point.1}))
}

fn source_notes(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "song_session",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            state[field], before[field],
            "Changing the note view changed {field}"
        );
    }
}

fn mode(app: &mut Harness, from: &str, to: &str) -> Value {
    app.click(from);
    app.click(to)
}

fn selected(state: &Value) -> Vec<u64> {
    let mut ids: Vec<_> = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|note| note["selected"] == true)
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn drum_names_audition_and_draw_their_midi_pitch_and_other_notes_remain_reachable() {
    let mut app = Harness::new();
    let path = app.directory.path().join("loaded-hit.wav");
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![0.25; 441],
    }
    .write_wav(&path)
    .unwrap();
    let mapped = Note::new(36, 0.0, 0.5, 92);
    let other = Note::new(60, 1.0, 0.5, 92);
    let mut original_ids = vec![mapped.id, other.id];
    original_ids.sort_unstable();
    let track = open(
        &mut app,
        Instrument::Builtin {
            device: "r1-drums".into(),
            samples: vec![DeviceSample {
                slot: 0,
                path: path.to_string_lossy().into(),
                name: "Round kick".into(),
            }],
            sample_names: [(0, "Custom low kick".into())].into(),
            sample_players: Default::default(),
        },
        vec![mapped, other],
    );
    let before = app.inspect();
    assert_eq!(before["midi_editor"]["row_mode"], "Named notes");
    assert_eq!(
        before["midi_editor"]["visible_pitches"],
        json!((36..52).collect::<Vec<_>>())
    );
    assert_eq!(key(&before, 36)["label"], "Custom low kick");
    assert_eq!(key(&before, 37)["label"], "Pad 2 · Empty");
    assert_eq!(before["midi_editor"]["unmapped_notes"], 1);
    for hidden in ["Multi-draw", "Generate…", "Scale guide C · Major"] {
        assert!(
            !before["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!(hidden))
        );
    }
    for (dark, density, width) in [(false, 1.0, 800), (true, 2.0, 1100)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":width,"height":800,"pixels_per_point":density}),
        );
        let state = app.command(json!({"command":"inspect","paint":true}));
        for label in ["Custom low kick", "Pad 2 · Empty", "Other MIDI notes (1)"] {
            assert!(
                state["painted_text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|text| text["text"] == label),
                "The row name/action must actually paint: {label}"
            );
        }
        let at = center(&key(&state, 36)["rect"]);
        let held = pointer(&mut app, "down", at);
        assert_eq!(held["midi_editor"]["audition"][0], track);
        assert_eq!(held["midi_editor"]["audition"][1], 36);
        let released = pointer(&mut app, "up", at);
        assert!(released["midi_editor"]["audition"].is_null());
        unchanged(&released, &before);
    }

    let piano = app.click("Other MIDI notes (1)");
    assert_eq!(piano["midi_editor"]["row_mode"], "Piano roll");
    assert_eq!(
        piano["midi_editor"]["visible_pitches"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    unchanged(&piano, &before);
    app.click("Edit");
    let all = app.click("Select all notes");
    assert_eq!(selected(&all), original_ids);
    let named = mode(&mut app, "Piano roll", "Named notes");
    assert_eq!(selected(&named).len(), 1);
    unchanged(&named, &before);
    let restored = app.click("Other MIDI notes (1)");
    assert_eq!(
        selected(&restored),
        original_ids,
        "View switches restore the hidden selection"
    );
    unchanged(&restored, &before);

    mode(&mut app, "Piano roll", "Named notes");
    app.sixteenth_snap();
    let drawing = app.click("Pencil");
    let editor = &drawing["midi_editor"];
    let x = editor["plot"]["min"][0].as_f64().unwrap()
        + (3.0 - editor["start_beat"].as_f64().unwrap())
            * editor["pixels_per_beat"].as_f64().unwrap();
    let y = center(&key(&drawing, 40)["rect"]).1;
    let hovered = pointer(&mut app, "move", (x, y));
    assert_eq!(
        hovered["midi_editor"]["pitch_hint"]["text"],
        "Pad 5 · Empty"
    );
    let drawn = pointer(&mut app, "click", (x, y));
    let added: Vec<_> = source_notes(&drawn)
        .iter()
        .filter(|note| !original_ids.contains(&note["id"].as_u64().unwrap()))
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["pitch"], 40);
    assert!(
        source_notes(&drawn).iter().any(|note| note["pitch"] == 60),
        "Unmapped notes survive editing"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.quit();
}

#[test]
fn a_pitched_instrument_without_note_names_keeps_the_complete_piano_roll() {
    let mut app = Harness::new();
    open(
        &mut app,
        Instrument::builtin("r1-mini"),
        vec![Note::new(60, 0.0, 1.0, 90)],
    );
    let state = app.inspect();
    assert_eq!(state["midi_editor"]["row_mode"], "Piano roll");
    assert_eq!(
        state["midi_editor"]["visible_pitches"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(state["midi_editor"]["note_names"], json!([]));
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!("Scale guide C · Major"))
    );
    app.quit();
}

#[test]
fn an_empty_named_map_fits_every_pad_and_space_below_the_rows_is_not_an_extra_pad() {
    let mut app = Harness::new();
    open(&mut app, Instrument::builtin("r1-drums"), Vec::new());
    app.command(json!({"command":"viewport","width":1100,"height":1100}));
    let before = app.click("Pencil");
    let first = &key(&before, 36)["rect"];
    let last = &key(&before, 51)["rect"];
    let plot = &before["midi_editor"]["plot"];
    let y = last["max"][1].as_f64().unwrap() + 12.0;
    assert!(
        y < plot["max"][1].as_f64().unwrap(),
        "Fixture leaves blank space below all 16 rows"
    );
    let gutter = (center(first).0, y);
    let held = pointer(&mut app, "down", gutter);
    assert!(
        held["midi_editor"]["audition"].is_null(),
        "Blank gutter does not audition the last pad"
    );
    pointer(&mut app, "up", gutter);
    let blank_plot = (plot["min"][0].as_f64().unwrap() + 100.0, y);
    let hovered = pointer(&mut app, "move", blank_plot);
    assert!(hovered["midi_editor"]["pitch_hint"].is_null());
    let clicked = pointer(&mut app, "click", blank_plot);
    assert!(
        source_notes(&clicked).is_empty(),
        "Blank plot does not draw the last pad"
    );
    unchanged(&clicked, &before);

    let fitted = app.click("Fit");
    let plot = &fitted["midi_editor"]["plot"];
    assert_eq!(fitted["midi_editor"]["keys"].as_array().unwrap().len(), 16);
    assert!(
        key(&fitted, 36)["rect"]["min"][1].as_f64().unwrap() >= plot["min"][1].as_f64().unwrap()
    );
    assert!(
        key(&fitted, 51)["rect"]["max"][1].as_f64().unwrap() <= plot["max"][1].as_f64().unwrap()
    );
    unchanged(&fitted, &before);
    app.quit();
}

#[test]
fn hosted_instrument_names_support_nonconsecutive_pitches_and_refresh_after_parameter_changes() {
    let fixture = ClapProbeFixture::build();
    let mut app = Harness::with_vst_path(fixture.path.parent().unwrap());
    let track = open(
        &mut app,
        Instrument::Vst3 {
            path: fixture.path.to_string_lossy().into(),
            class_id: Some(SYNTH_ID.into()),
        },
        vec![
            Note::new(36, 0.0, 0.5, 90),
            Note::new(60, 1.0, 0.5, 90),
            Note::new(40, 2.0, 0.5, 90),
            Note::new(42, 3.0, 0.5, 90),
        ],
    );
    let named = common::wait_until(&mut app, |state| {
        state["midi_editor"]["visible_pitches"] == json!([36, 40, 42])
    });
    assert_eq!(key(&named, 40)["label"], "Snare");
    let point = center(&key(&named, 40)["rect"]);
    let held = pointer(&mut app, "down", point);
    assert_eq!(held["midi_editor"]["audition"][1], 40);
    pointer(&mut app, "up", point);

    // Keyboard row nudging follows the displayed map, skipping absent MIDI
    // pitches 37–39, and has the same one-step undo as ordinary note editing.
    let first = named["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["pitch"] == 36)
        .unwrap();
    pointer(&mut app, "click", center(&first["rect"]));
    let nudged = app.command(json!({"command":"key","key":"ArrowDown","cmd":true}));
    let changed_note = source_notes(&nudged)
        .iter()
        .find(|note| note["id"] == first["id"])
        .unwrap();
    assert_eq!(changed_note["pitch"], 40);
    let mut previous = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(previous["project"], named["project"]);
    app.click("Notes");
    app.click("Move to row");
    let moved = app.click("Next named row");
    assert_eq!(
        source_notes(&moved)
            .iter()
            .find(|note| note["id"] == first["id"])
            .unwrap()["pitch"],
        40
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    app.click("Edit");
    app.click("Select all notes");
    app.click("Notes");
    let flipped = app.click("Flip vertically");
    for original in source_notes(&named) {
        let mirrored = source_notes(&flipped)
            .iter()
            .find(|note| note["id"] == original["id"])
            .unwrap();
        let expected_pitch = match original["pitch"].as_u64().unwrap() {
            36 => 42,
            42 => 36,
            pitch => pitch,
        };
        assert_eq!(
            mirrored["pitch"], expected_pitch,
            "Named rows mirror within the map; unmapped notes stay unchanged"
        );
    }
    previous = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(previous["project"], named["project"]);
    for (value, pitches, label) in [
        (1.0 / 3.0, json!([60, 64, 67]), Some("Slice B")),
        (2.0 / 3.0, json!((0..128).rev().collect::<Vec<_>>()), None),
    ] {
        let metadata = app.command(json!({"command":"song_tool","name":"plugin_describe_parameters","arguments":{"track_id":track}}));
        assert_eq!(
            metadata["tool_result"]["ok"], true,
            "{}",
            metadata["tool_result"]
        );
        // The public song edit changes the plug-in just as its parameter panel
        // does. Row-map refresh itself is observed in the running editor.
        let changed = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
            "session_id":previous["song_session"]["id"],"expected_revision":previous["song_session"]["revision"],
            "label":"Change named-note program","edits":[{"op":"set_plugin_parameters","track":track,"effect":null,"parameters":[{"id":1,"value":value}]}]
        }}));
        assert_eq!(changed["tool_result"]["ok"], true, "{changed}");
        previous = common::wait_until(&mut app, |state| {
            state["midi_editor"]["visible_pitches"] == pitches
        });
        assert_eq!(source_notes(&previous), source_notes(&changed));
        if let Some(label) = label {
            assert_eq!(key(&previous, 64)["label"], label);
            assert_eq!(previous["midi_editor"]["row_mode"], "Named notes");
        } else {
            assert_eq!(previous["midi_editor"]["row_mode"], "Piano roll");
        }
    }
    app.quit();
}
