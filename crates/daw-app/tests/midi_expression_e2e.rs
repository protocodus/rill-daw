//! Exact MIDI 2.0 edits through the running production app, never a mock form.
mod common;
use common::Harness;
use daw_core::{Clip, Midi2ChannelVoice, Midi2NoteData, MidiNoteExpression, Note, Project};
use serde_json::{Value, json};

struct Fixture {
    track: u64,
    clip: u64,
    ghost: u64,
    note: u64,
    other: u64,
    pressure: u64,
    opaque: u64,
}

fn fixture(app: &mut Harness, precise: bool) -> Fixture {
    let mut project = Project::new();
    let track = project.add_midi_track("Expressive instrument");
    let mut note = Note::new(60, 1.5, 1.5, 100);
    let pressure = MidiNoteExpression::new(
        0.123456789,
        Midi2ChannelVoice {
            group: 3,
            channel: 5,
            status: 10,
            index: 188,
            detail: 0xa5,
            data: 0x8000_0001,
        }
        .packet()
        .unwrap(),
    );
    let opaque = MidiNoteExpression::new(
        0.75,
        Midi2ChannelVoice {
            group: 3,
            channel: 5,
            status: 15,
            index: 60,
            detail: 3,
            data: 0x1234_5678,
        }
        .packet()
        .unwrap(),
    );
    if precise {
        note.set_midi2(Midi2NoteData {
            protocol: 2,
            group: 3,
            channel: 5,
            note_index: 60,
            velocity: 32769,
            release_velocity: 40001,
            attribute_type: 9,
            attribute_data: 0xbeef,
            release_attribute_type: 7,
            release_attribute_data: 0xcafe,
            expressions: vec![pressure.clone(), opaque.clone()],
        })
        .unwrap();
    }
    let other = Note::new(64, 2.5, 0.5, 91);
    let note_id = note.id;
    let other_id = other.id;
    let mut clip = Clip::midi("Expression phrase", 2.0, 2.0, vec![note, other]);
    clip.offset = 1.0;
    clip.source_rate = 1.7;
    clip.link_group = Some(daw_core::new_id());
    let id = clip.id;
    let mut ghost = clip.clone();
    ghost.id = daw_core::new_id();
    ghost.start = 6.0;
    ghost.offset = 0.5;
    ghost.source_rate = 0.5;
    let ghost_id = ghost.id;
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([clip, ghost]);
    let path = app.directory.path().join("midi-expression.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Expression phrase");
    app.click("Window");
    app.click("Clip Editor");
    Fixture {
        track,
        clip: id,
        ghost: ghost_id,
        note: note_id,
        other: other_id,
        pressure: pressure.id,
        opaque: opaque.id,
    }
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}
fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|item| item["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    click_rect(app, rect)
}
fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn note(state: &Value, clip_id: u64, note_id: u64) -> &Value {
    clip(state, clip_id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == note_id)
        .unwrap()
}
fn expression<'a>(state: &'a Value, f: &Fixture, id: u64) -> &'a Value {
    note(state, f.clip, f.note)["midi2"]["expressions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|point| point["id"] == id)
        .unwrap()
}

fn open(app: &mut Harness, f: &Fixture) -> Value {
    let state = app.inspect();
    let rect = &state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == f.note)
        .unwrap()["rect"];
    let plot = &state["midi_editor"]["plot"];
    let hit = json!({"min":[rect["min"][0].as_f64().unwrap().max(plot["min"][0].as_f64().unwrap()),rect["min"][1].as_f64().unwrap().max(plot["min"][1].as_f64().unwrap())],"max":[rect["max"][0].as_f64().unwrap().min(plot["max"][0].as_f64().unwrap()),rect["max"][1].as_f64().unwrap().min(plot["max"][1].as_f64().unwrap())]});
    assert!(
        hit["min"][0].as_f64().unwrap() < hit["max"][0].as_f64().unwrap()
            && hit["min"][1].as_f64().unwrap() < hit["max"][1].as_f64().unwrap(),
        "Expression fixture note is offscreen: {rect}; plot {plot}"
    );
    click_rect(app, &hit);
    app.click("Notes");
    let state = app.click("Expression…");
    assert_eq!(
        state["midi_expression"]["note"], f.note,
        "Expression did not open; view={}, selected_notes={}, labels={}",
        state["view"], state["selected_notes"], state["visible_labels"]
    );
    state
}

fn visible(app: &mut Harness, field: &str) -> Value {
    for _ in 0..10 {
        let state = app.inspect();
        let diagnostic = &state["midi_expression"];
        let (_, y) = center(&diagnostic[field]);
        let body = &diagnostic["body"];
        if y >= body["min"][1].as_f64().unwrap() + 3.0 && y < body["max"][1].as_f64().unwrap() - 3.0
        {
            return state;
        }
        app.command(json!({"command":"scroll","x":body["max"][0].as_f64().unwrap()-24.0,"y":body["max"][1].as_f64().unwrap()-25.0,"delta":if y>body["max"][1].as_f64().unwrap(){-240.0}else{240.0}}));
    }
    panic!("Field {field} did not become visible");
}

fn edit(app: &mut Harness, field: &str, text: &str) -> Value {
    let state = visible(app, field);
    let (x, y) = center(&state["midi_expression"][field]);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}));
    app.key("Enter")
}

fn mouse_integer(app: &mut Harness, field: &str, value: &str) -> Value {
    let state = visible(app, field);
    let rect = &state["midi_expression"][field];
    let (_, y) = center(rect);
    let keypad = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| {
            label["text"] == "123"
                && (center(label).1 - y).abs() < 2.0
                && label["min"][0].as_f64().unwrap() >= rect["max"][0].as_f64().unwrap()
        })
        .min_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .expect("Exact editor has a mouse keypad");
    click_rect(app, keypad);
    front(app, "Clear");
    for digit in value.chars() {
        front(app, &digit.to_string());
    }
    front(app, "Apply value")
}

fn select_point(app: &mut Harness, id: u64) -> Value {
    let state = visible(app, "velocity");
    let rect = &state["midi_expression"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()["rect"];
    click_rect(app, rect)
}

fn body_click(app: &mut Harness, label: &str) -> Value {
    for attempt in 0..12 {
        let state = app.inspect();
        let body = &state["midi_expression"]["body"];
        if let Some(rect) = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| {
                rect["text"] == label
                    && center(rect).1 >= body["min"][1].as_f64().unwrap()
                    && center(rect).1 < body["max"][1].as_f64().unwrap()
            })
        {
            return click_rect(app, rect);
        }
        app.command(json!({"command":"scroll","x":body["max"][0].as_f64().unwrap()-24.0,"y":body["max"][1].as_f64().unwrap()-25.0,"delta":if attempt==0{2000.0}else{-160.0}}));
    }
    panic!("Expression body action {label} is unreachable");
}

fn history(app: &mut Harness, label: &str) -> Value {
    app.click("Edit");
    app.click(label)
}

#[test]
fn exact_attack_release_and_expression_edits_are_one_reversible_ghost_safe_transaction() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":if dark{2.0}else{1.0}}));
        let f = fixture(&mut app, true);
        let before = app.inspect();
        let opened = open(&mut app, &f);
        assert_eq!(opened["project"], before["project"]);
        assert!(
            (opened["midi_expression"]["note_song_beat"]
                .as_f64()
                .unwrap()
                - (2.0 + 0.5 / 1.7))
                .abs()
                < 1e-12
        );
        for key in ["Delete", "Backspace", "Space"] {
            let state = app.key(key);
            assert_eq!(state["project"], before["project"]);
        }
        let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert_eq!(state["project"], before["project"]);
        let draft = mouse_integer(&mut app, "velocity", "32770");
        assert_eq!(draft["midi_expression"]["velocity16"], 32770);
        assert_eq!(draft["project"], before["project"]);
        edit(&mut app, "release_velocity", "65534");
        let selected = select_point(&mut app, f.pressure);
        assert_eq!(
            selected["midi_expression"]["expression_value"],
            0x8000_0001_u64
        );
        let value = edit(&mut app, "value", "2147483651");
        assert_eq!(value["midi_expression"]["expression_value"], 2147483651_u64);
        assert_eq!(value["project"], before["project"]);
        select_point(&mut app, f.opaque);
        let removed = body_click(&mut app, "Remove point");
        assert_eq!(removed["midi_expression"]["point_count"], 1);
        assert_eq!(removed["project"], before["project"]);
        // Cancel restores every byte, including the fractional source time.
        let cancelled = app.click("Cancel");
        assert_eq!(cancelled["project"], before["project"]);
        assert_eq!(cancelled["history"], before["history"]);
        open(&mut app, &f);
        edit(&mut app, "velocity", "32770");
        edit(&mut app, "release_velocity", "65534");
        select_point(&mut app, f.pressure);
        edit(&mut app, "value", "2147483651");
        let committed = app.click("Apply");
        let changed = note(&committed, f.clip, f.note);
        assert_eq!(changed["midi2"]["velocity"], 32770);
        assert_eq!(changed["midi2"]["release_velocity"], 65534);
        assert_eq!(changed["midi2"]["attribute_type"], 9);
        assert_eq!(changed["midi2"]["attribute_data"], 0xbeef);
        assert_eq!(changed["midi2"]["release_attribute_type"], 7);
        assert_eq!(changed["midi2"]["release_attribute_data"], 0xcafe);
        assert_eq!(
            expression(&committed, &f, f.pressure)["beat"],
            expression(&before, &f, f.pressure)["beat"],
            "Editing data preserves exact source time"
        );
        assert_eq!(
            expression(&committed, &f, f.pressure)["packet"]["words"][0],
            expression(&before, &f, f.pressure)["packet"]["words"][0],
            "Reserved address/payload bits stay intact"
        );
        assert_eq!(
            expression(&committed, &f, f.pressure)["packet"]["words"][1],
            2147483651_u64
        );
        assert_eq!(
            expression(&committed, &f, f.opaque),
            expression(&before, &f, f.opaque)
        );
        assert_eq!(
            note(&committed, f.clip, f.other),
            note(&before, f.clip, f.other)
        );
        assert_eq!(
            clip(&committed, f.ghost)["content"],
            clip(&committed, f.clip)["content"]
        );
        assert_eq!(clip(&committed, f.ghost)["source_rate"], 0.5);
        assert_eq!(clip(&committed, f.clip)["source_rate"], 1.7);
        assert_eq!(
            committed["history"]["undo"].as_array().unwrap().len(),
            before["history"]["undo"].as_array().unwrap().len() + 1
        );
        assert_eq!(history(&mut app, "Undo")["project"], before["project"]);
        assert_eq!(history(&mut app, "Redo")["project"], committed["project"]);
        open(&mut app, &f);
        select_point(&mut app, f.opaque);
        body_click(&mut app, "Remove point");
        let removed = app.click("Apply");
        assert_eq!(
            note(&removed, f.clip, f.note)["midi2"]["expressions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            note(&removed, f.clip, f.other),
            note(&committed, f.clip, f.other)
        );
        assert_eq!(history(&mut app, "Undo")["project"], committed["project"]);
        let archive = app.directory.path().join("expression-edits.rill");
        app.command(json!({"command":"save","path":archive}));
        app = app.restart();
        let reopened = app.command(json!({"command":"open","path":archive}));
        assert_eq!(reopened["project"], committed["project"]);
        app.quit();
    }
}

#[test]
fn legacy_notes_gain_precise_pitch_pressure_and_controller_points_without_touching_other_notes() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1280,"height":800}));
    let f = fixture(&mut app, false);
    let before = app.inspect();
    open(&mut app, &f);
    edit(&mut app, "pitch", "60.501953125");
    edit(&mut app, "time", "0.25");
    edit(&mut app, "value", "4294967295");
    body_click(&mut app, "Add point");
    body_click(&mut app, "New point");
    body_click(&mut app, "Pressure");
    app.click("Pitch bend");
    edit(&mut app, "time", "0.5");
    body_click(&mut app, "Add point");
    body_click(&mut app, "New point");
    body_click(&mut app, "Pitch bend");
    app.click("Registered controller");
    edit(&mut app, "controller", "74");
    edit(&mut app, "time", "0.6");
    edit(&mut app, "value", "2147483649");
    body_click(&mut app, "Add point");
    body_click(&mut app, "New point");
    body_click(&mut app, "Registered controller");
    app.click("Assignable controller");
    edit(&mut app, "controller", "73");
    edit(&mut app, "time", "0.7");
    edit(&mut app, "value", "4294967294");
    let staged = body_click(&mut app, "Add point");
    assert_eq!(staged["project"], before["project"]);
    let state = app.click("Apply");
    let data = &note(&state, f.clip, f.note)["midi2"];
    assert_eq!(data["attribute_type"], 3);
    assert_eq!(data["attribute_data"], 30977);
    assert_eq!(data["note_index"], 60);
    let expressions = data["expressions"].as_array().unwrap();
    assert_eq!(expressions.len(), 4);
    for (index, (status, detail, value, time)) in [
        (10_u32, 0_u32, 0xffff_ffff_u32, 0.25),
        (6, 0, 0x8000_0000, 0.5),
        (0, 74, 0x8000_0001, 0.6),
        (1, 73, 0xffff_fffe, 0.7),
    ]
    .into_iter()
    .enumerate()
    {
        let point = &expressions[index];
        let word = point["packet"]["words"][0].as_u64().unwrap() as u32;
        assert_eq!((word >> 20) & 15, status);
        assert_eq!(word & 127, detail);
        assert_eq!(point["packet"]["words"][1], value);
        assert!((point["beat"].as_f64().unwrap() - time * 1.7).abs() < 1e-12);
    }
    assert_eq!(
        note(&state, f.clip, f.other),
        note(&before, f.clip, f.other)
    );
    assert_eq!(history(&mut app, "Undo")["project"], before["project"]);
    assert_eq!(history(&mut app, "Redo")["project"], state["project"]);
    app.quit();
}

#[test]
fn concurrent_source_edits_make_the_expression_draft_stale_without_overwriting_them() {
    let mut app = Harness::new();
    let f = fixture(&mut app, true);
    open(&mut app, &f);
    edit(&mut app, "velocity", "12345");
    let source = app.inspect();
    let changed=app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":source["song_session"]["id"],"expected_revision":source["song_session"]["revision"],"label":"Concurrent precise edit",
        "edits":[{"op":"set_note_velocity16","track":f.track,"clip":f.clip,"note":f.note,"velocity":54321}]
    }}));
    assert_eq!(note(&changed, f.clip, f.note)["midi2"]["velocity"], 54321);
    if changed["midi_expression"]["open"] == true {
        assert_eq!(changed["midi_expression"]["stale"], true);
        let state = app.click("Apply");
        assert_eq!(state["project"], changed["project"]);
        app.click("Cancel");
    }
    assert_eq!(
        note(&app.inspect(), f.clip, f.note)["midi2"]["velocity"],
        54321
    );
    app.quit();
}
