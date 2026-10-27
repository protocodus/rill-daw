//! Composer tools and the piano roll use one harmonic vocabulary through real UI input.
mod common;

use common::Harness;
use daw_core::{Chord, Clip, Note, Project, Scale};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const PHRASE: &str = "Shared catalogue phrase";
const HEADER: &str = "Clip Editor · Shared catalogue phrase";

fn fixture(source: Vec<Note>) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Shared catalogue instrument");
    let clip = Clip::midi(PHRASE, 0.0, 16.0, source);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("shared-theory.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click(PHRASE);
    app.click("Window");
    app.click("Clip Editor");
    (app, track, clip_id)
}

fn tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    state
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

fn notes(state: &Value, id: u64) -> &[Value] {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn pitches(notes: &[Value]) -> Vec<u64> {
    notes
        .iter()
        .map(|note| note["pitch"].as_u64().unwrap())
        .collect()
}

fn folded_octave(lookup: &Value) -> Vec<u64> {
    lookup["midi"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pitch| 60 + pitch.as_u64().unwrap() % 12)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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

fn draw(app: &mut Harness, beat: f64, pitch: u8) -> Value {
    let state = app.inspect();
    let midi = &state["midi_editor"];
    let row = midi["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|value| value.as_u64() == Some(u64::from(pitch)))
        .unwrap();
    let x = midi["plot"]["min"][0].as_f64().unwrap()
        + (beat - midi["start_beat"].as_f64().unwrap()) * midi["pixels_per_beat"].as_f64().unwrap();
    let y = midi["plot"]["min"][1].as_f64().unwrap()
        + (row as f64 + 0.5 - midi["row_offset"].as_f64().unwrap())
            * midi["row_height"].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn popup_bounds(state: &Value) -> [f64; 4] {
    let scroll = &state["native_menu"]["scroll_rect"];
    let bounds = if scroll.is_array() {
        scroll
    } else {
        state["native_menu"]["foreground_areas"]
            .as_array()
            .unwrap()
            .iter()
            .min_by(|a, b| {
                let area = |r: &Value| {
                    (r[2].as_f64().unwrap() - r[0].as_f64().unwrap())
                        * (r[3].as_f64().unwrap() - r[1].as_f64().unwrap())
                };
                area(a).total_cmp(&area(b))
            })
            .expect("The real choice popup is open")
    };
    std::array::from_fn(|index| bounds[index].as_f64().unwrap())
}

fn scroll_popup(app: &mut Harness, delta: f64) -> Value {
    let state = app.inspect();
    let [left, top, right, bottom] = popup_bounds(&state);
    app.command(json!({"command":"scroll","x":(left+right)*0.5,"y":(top+bottom)*0.5,"delta":delta}))
}

fn choose_open_popup(app: &mut Harness, wanted: &str) -> (Value, bool) {
    // ComboBox remembers its previous scroll offset. Begin at the top using the
    // same wheel input a person uses; do not select a similarly named control
    // painted behind the foreground popup.
    scroll_popup(app, 10_000.0);
    let mut scrolled = false;
    for _ in 0..24 {
        let state = app.inspect();
        let [left, top, right, bottom] = popup_bounds(&state);
        let target = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|label| {
                if label["text"] != wanted {
                    return false;
                }
                let (x, y) = center(label);
                x >= left && x <= right && y >= top + 1.0 && y <= bottom - 1.0
            });
        if let Some(label) = target {
            return (click_rect(app, label), scrolled);
        }
        scroll_popup(app, -160.0);
        scrolled = true;
    }
    panic!(
        "Could not reveal catalogue choice {wanted}: {}",
        app.inspect()["visible_labels"]
    );
}

fn choose(app: &mut Harness, current: &str, wanted: &str) -> Value {
    app.click(current);
    choose_open_popup(app, wanted).0
}

fn assert_keys(state: &Value, field: &str, expected: &[u64]) {
    let classes: BTreeSet<_> = expected.iter().map(|pitch| pitch % 12).collect();
    let keys = state["midi_editor"]["keys"].as_array().unwrap();
    assert!(keys.len() >= 12, "A full octave is visible");
    for key in keys {
        let pitch = key["pitch"].as_u64().unwrap();
        assert_eq!(
            key[field],
            classes.contains(&(pitch % 12)),
            "{field} at pitch {pitch}"
        );
    }
}

fn generated(state: &Value) -> &Value {
    let value = &state["midi_editor"]["generate"];
    assert!(!value.is_null(), "Generator remains open");
    value
}

fn generator_control(app: &mut Harness, field: &str) -> Value {
    for _ in 0..18 {
        let state = app.inspect();
        let controls = &generated(&state)["controls"];
        let rect = &controls[field];
        assert!(!rect.is_null(), "Missing generator field {field}");
        if ["start_bar", "length_bars"].contains(&field) {
            return rect.clone();
        }
        let options = &controls["options"];
        let (_, y) = center(rect);
        let top = options["min"][1].as_f64().unwrap();
        let bottom = options["max"][1].as_f64().unwrap();
        if y >= top + 2.0 && y <= bottom - 2.0 {
            return rect.clone();
        }
        let (x, anchor_y) = center(options);
        app.command(
            json!({"command":"scroll","x":x,"y":anchor_y,"delta":if y < top {160.0} else {-160.0}}),
        );
    }
    panic!("Could not reveal generator field {field}");
}

fn generator_choice(app: &mut Harness, field: &str, wanted: &str) -> Value {
    let rect = generator_control(app, field);
    click_rect(app, &rect);
    choose_open_popup(app, wanted).0
}

fn generator_number(app: &mut Harness, field: &str, value: &str) -> Value {
    let rect = generator_control(app, field);
    let (x, y) = center(&rect);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":value}));
    app.click("Start bar")
}

fn note_values(notes: &[Value]) -> Vec<Value> {
    notes.iter().map(|note| json!({"pitch":note["pitch"],"start":note["start"],"length":note["length"],"velocity":note["velocity"]})).collect()
}

fn tool_generation(
    app: &mut Harness,
    track: u64,
    name: &str,
    harmony: (&str, &str),
    kind: &str,
    length: f64,
) -> Value {
    let state = app.inspect();
    let mut args = json!({
        "session_id":state["song_session"]["id"],"expected_revision":state["song_session"]["revision"],
        "track_id":track,"name":name,"start":24.0,"length":length,"kind":kind,
        "root":0,"start_octave":4,"octaves":1,"shift":0,"pattern":"up",
        "speed_multiplier":1.0,"gate":0.9,"velocity":100
    });
    args[harmony.0] = json!(harmony.1);
    tool(app, "generate_part", args)
}

fn named_clip<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["name"] == name)
        .unwrap()
}

#[test]
fn extended_chord_lookup_grid_stack_and_both_generators_share_pitch_classes() {
    let (mut app, track, clip_id) = fixture(vec![]);
    let label = Chord::from_name("major 9").unwrap().name();
    let lookup = tool(
        &mut app,
        "composer_chord",
        json!({"root":"C4","chord":"major 9"}),
    );
    assert_eq!(
        lookup["tool_result"]["result"]["midi"],
        json!([60, 64, 67, 71, 74])
    );
    let expected = folded_octave(&lookup["tool_result"]["result"]);
    assert_eq!(expected, vec![60, 62, 64, 67, 71]);

    app.click("Multi-draw");
    let selected = choose(&mut app, Chord::Major.name(), label);
    assert_keys(&selected, "chord", &expected);
    let stacked = draw(&mut app, 8.0, 60);
    let at_eight: Vec<_> = notes(&stacked, clip_id)
        .iter()
        .filter(|note| note["start"] == 8.0)
        .cloned()
        .collect();
    assert_eq!(pitches(&at_eight), expected);

    // Legacy Add9 keeps its one-octave drawing contract, derived from the same compound 9th.
    choose(&mut app, label, Chord::AddNine.name());
    let legacy = draw(&mut app, 12.0, 60);
    let at_twelve: Vec<_> = notes(&legacy, clip_id)
        .iter()
        .filter(|note| note["start"] == 12.0)
        .cloned()
        .collect();
    assert_eq!(pitches(&at_twelve), vec![60, 62, 64, 67]);
    choose(&mut app, Chord::AddNine.name(), label);

    app.click("Generate…");
    generator_choice(&mut app, "chord", Chord::AddNine.name());
    generator_choice(&mut app, "chord", label);
    generator_number(&mut app, "start_bar", "1");
    generator_number(&mut app, "length_bars", "1");
    generator_number(&mut app, "start_octave", "4");
    let preview = generator_number(&mut app, "octaves", "1");
    let draft = generated(&preview)["notes"].as_array().unwrap().clone();
    assert_eq!(pitches(&draft), expected);
    app.click("Add notes");
    let committed = app.click("Done");
    let accepted: Vec<_> = notes(&committed, clip_id)
        .iter()
        .filter(|note| note["start"] == 0.0)
        .cloned()
        .collect();
    assert_eq!(note_values(&accepted), note_values(&draft));

    let ai = tool_generation(
        &mut app,
        track,
        "AI extended chord",
        ("chord", "major 9"),
        "chord",
        4.0,
    );
    let ai_notes = named_clip(&ai, "AI extended chord")["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(note_values(ai_notes), note_values(&draft));
    let legacy_ai = tool_generation(
        &mut app,
        track,
        "AI legacy Add9",
        ("chord", "add_nine"),
        "chord",
        4.0,
    );
    assert_eq!(
        pitches(
            named_clip(&legacy_ai, "AI legacy Add9")["content"]["Midi"]["notes"]
                .as_array()
                .unwrap()
        ),
        vec![60, 62, 64, 67]
    );
    app.quit();
}

#[test]
fn deep_scale_picker_highlights_filters_draws_and_generates_the_composer_scale() {
    let source = vec![
        Note::new(60, 0.0, 0.5, 90),
        Note::new(61, 1.0, 0.5, 85),
        Note::new(67, 2.0, 0.5, 95),
    ];
    let (mut app, track, clip_id) = fixture(source);
    let before = app.inspect();
    let label = Scale::from_name("messiaen mode 3").unwrap().name();
    let lookup = tool(
        &mut app,
        "composer_scale",
        json!({"root":"C4","scale":"messiaen mode 3"}),
    );
    let expected = folded_octave(&lookup["tool_result"]["result"]);
    assert_eq!(expected, vec![60, 62, 63, 64, 66, 67, 68, 70, 71]);

    // This nested grid picker must remain reachable in a short window.
    app.command(json!({"command":"viewport","width":1440,"height":720}));
    app.click(format!("C · {}", Scale::Minor.name()).as_str());
    app.click(Scale::Minor.name());
    let (selected, scrolled) = choose_open_popup(&mut app, label);
    assert!(
        scrolled,
        "A deep catalogue entry is reached using the mouse wheel"
    );
    assert_eq!(selected["project"], before["project"]);
    app.click(HEADER);
    let selected = app.command(json!({"command":"viewport","width":1440,"height":1000}));
    assert_keys(&selected, "scale", &expected);
    app.click("View");
    let filtered = app.click("Only scale keys");
    assert_eq!(filtered["midi_editor"]["hidden_notes"], 1);
    assert!(
        filtered["midi_editor"]["visible_pitches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pitch| expected
                .iter()
                .any(|value| value % 12 == pitch.as_u64().unwrap() % 12))
    );
    assert_eq!(filtered["project"], before["project"]);
    assert_eq!(filtered["history"], before["history"]);
    app.click(HEADER);

    app.click("Multi-draw");
    choose(&mut app, "Chord stack", "Scale stack");
    let stacked = draw(&mut app, 8.0, 60);
    let at_eight: Vec<_> = notes(&stacked, clip_id)
        .iter()
        .filter(|note| note["start"] == 8.0)
        .cloned()
        .collect();
    assert_eq!(pitches(&at_eight), expected);
    let before_preview = stacked;

    app.click("Generate…");
    app.click("Arpeggio");
    generator_choice(&mut app, "harmony", "Scale");
    generator_choice(&mut app, "scale", Scale::Minor.name());
    generator_choice(&mut app, "scale", label);
    generator_number(&mut app, "start_bar", "2");
    generator_number(&mut app, "length_bars", "2");
    generator_number(&mut app, "start_octave", "4");
    let preview = generator_number(&mut app, "octaves", "1");
    let draft = generated(&preview)["notes"].as_array().unwrap().clone();
    assert_eq!(
        pitches(&draft).into_iter().collect::<BTreeSet<_>>(),
        expected.iter().copied().collect()
    );
    assert_eq!(preview["project"], before_preview["project"]);
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before_preview["project"]);
    assert_eq!(cancelled["history"], before_preview["history"]);

    let ai = tool_generation(
        &mut app,
        track,
        "AI extended scale",
        ("scale", "messiaen mode 3"),
        "arpeggio",
        8.0,
    );
    let ai_notes = named_clip(&ai, "AI extended scale")["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    let local_draft: Vec<_> = draft
        .iter()
        .map(|note| {
            let mut note = note.clone();
            note["start"] = json!(note["start"].as_f64().unwrap() - 4.0);
            note
        })
        .collect();
    assert_eq!(note_values(ai_notes), note_values(&local_draft));
    let legacy = tool_generation(
        &mut app,
        track,
        "AI legacy minor",
        ("scale", "minor"),
        "arpeggio",
        8.0,
    );
    let classes: BTreeSet<_> = pitches(
        named_clip(&legacy, "AI legacy minor")["content"]["Midi"]["notes"]
            .as_array()
            .unwrap(),
    )
    .into_iter()
    .map(|pitch| pitch % 12)
    .collect();
    assert_eq!(classes, BTreeSet::from([0, 2, 3, 5, 7, 8, 10]));
    app.quit();
}
