//! Generate through the real piano-roll UI; drafts never edit the project.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project, Scale};
use serde_json::{Value, json};
use std::collections::HashSet;

fn fixture(ghost: bool, beats_per_bar: u8) -> (Harness, u64) {
    fixture_with_notes(ghost, beats_per_bar, vec![Note::new(60, 1.875, 0.375, 87)])
}

fn fixture_with_notes(ghost: bool, beats_per_bar: u8, source: Vec<Note>) -> (Harness, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    let mut project = Project::new();
    project.beats_per_bar = beats_per_bar;
    let track = project.add_midi_track("Generator test");
    let clip = Clip::midi("Generation source", 0.0, 32.0, source);
    let original = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let selected = if ghost {
        let id = project.clone_clip(track, original).unwrap();
        let clip = project.clip_mut(track, id).unwrap();
        clip.offset = 1.5;
        clip.source_rate = 0.75;
        clip.length = 16.0;
        id
    } else {
        original
    };
    let path = app.directory.path().join("generate-fixture.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let state = app.click("Fit");
    let rect = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == selected)
        .unwrap()["rect"]
        .clone();
    click_rect(&mut app, &rect);
    app.click("Window");
    let state = app.click("Clip Editor");
    assert_eq!(state["selected_clip"], selected);
    (app, selected)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn generation(state: &Value) -> &Value {
    let value = &state["midi_editor"]["generate"];
    assert!(!value.is_null(), "Generate dialog is open: {state}");
    value
}

fn draft(state: &Value) -> &[Value] {
    generation(state)["notes"].as_array().unwrap()
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn notes(state: &Value, id: u64) -> &[Value] {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn control_rect(app: &mut Harness, field: &str) -> Value {
    for _ in 0..12 {
        let state = app.inspect();
        let controls = &generation(&state)["controls"];
        let rect = &controls[field];
        assert!(
            !rect.is_null(),
            "Missing generator control {field}: {controls}"
        );
        let options = &controls["options"];
        if ["start_bar", "length_bars"].contains(&field) || options.is_null() {
            return rect.clone();
        }
        let (_, y) = center(rect);
        let top = options["min"][1].as_f64().unwrap();
        let bottom = options["max"][1].as_f64().unwrap();
        if y >= top + 2.0 && y <= bottom - 2.0 {
            return rect.clone();
        }
        let (x, anchor_y) = center(options);
        let delta = if y < top { 160.0 } else { -160.0 };
        app.command(json!({"command":"scroll", "x":x, "y":anchor_y, "delta":delta}));
    }
    panic!("Could not reveal generator control {field}");
}

fn number(app: &mut Harness, field: &str, value: &str) -> Value {
    let (x, y) = center(&control_rect(app, field));
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

fn choice(app: &mut Harness, field: &str, label: &str) -> Value {
    let rect = control_rect(app, field);
    let mut state = click_rect(app, &rect);
    // Long chord lists scroll like the real chooser. Do not require all menu
    // choices to fit in the viewport at once.
    for _ in 0..8 {
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == label)
        {
            break;
        }
        let last = state["label_rects"].as_array().unwrap().last().unwrap();
        let (x, y) = center(last);
        state = app.command(json!({"command":"scroll", "x":x, "y":y, "delta":-150.0}));
    }
    // Some choice labels also occur on the mode tabs. The open native popup
    // paints last, and its occurrence is the actionable choice we want.
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count()
        .checked_sub(1)
        .unwrap_or_else(|| panic!("Missing choice {label}: {}", state["visible_labels"]));
    app.command(json!({"command":"click", "label":label, "occurrence":occurrence}))
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

fn assert_draft_valid(state: &Value) {
    let generator = generation(state);
    let start = generator["start"].as_f64().unwrap();
    let end = start + generator["length"].as_f64().unwrap();
    assert!(
        !draft(state).is_empty(),
        "A valid range has generated notes"
    );
    for note in draft(state) {
        let at = note["start"].as_f64().unwrap();
        let length = note["length"].as_f64().unwrap();
        assert!(at >= start - 1e-8 && at + length <= end + 1e-8);
        assert!(length.is_finite() && length > 0.0);
        assert!(note["pitch"].as_u64().unwrap() <= 127);
        assert!((1..=127).contains(&note["velocity"].as_u64().unwrap()));
    }
}

#[test]
fn visible_complete_bars_preview_toggle_numeric_range_and_cancel_are_non_destructive() {
    for (dark, meter) in [(false, 4), (true, 3)] {
        let (mut app, _) = fixture(false, meter);
        app.command(json!({"command":"system_theme", "dark":dark}));
        let initial = app.inspect();
        let view = &initial["midi_editor"];
        let (x, y) = center(&view["plot"]);
        let pixels = view["pixels_per_beat"].as_f64().unwrap();
        let before = app.command(json!({"command":"pointer", "phase":"drag", "button":"middle", "x":x, "y":y, "to_x":x-1.25*pixels, "to_y":y}));
        let view = &before["midi_editor"];
        let left = view["start_beat"].as_f64().unwrap();
        let right = left
            + (view["plot"]["max"][0].as_f64().unwrap() - view["plot"]["min"][0].as_f64().unwrap())
                / view["pixels_per_beat"].as_f64().unwrap();
        let bar = f64::from(meter);
        assert!(left > 0.0 && left < bar);
        let expected_start = (left / bar).ceil() * bar;
        let expected_end = (right / bar).floor() * bar;
        let opened = app.click("Generate…");
        near(
            generation(&opened)["start"].as_f64().unwrap(),
            expected_start,
        );
        near(
            generation(&opened)["length"].as_f64().unwrap(),
            expected_end - expected_start,
        );
        assert!(!generation(&opened)["range_rect"].is_null());
        assert_eq!(generation(&opened)["preview_visible"], true);
        assert!(
            !generation(&opened)["painted_notes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(opened["project"], before["project"]);
        assert_eq!(opened["history"], before["history"]);
        assert_draft_valid(&opened);
        let expected_notes = draft(&opened).to_vec();
        let hidden = app.click("Preview");
        assert_eq!(generation(&hidden)["preview_visible"], false);
        assert!(
            generation(&hidden)["painted_notes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(draft(&hidden), expected_notes);
        let shown = app.click("Preview");
        assert_eq!(draft(&shown), expected_notes, "Viewing does not regenerate");
        number(&mut app, "start_bar", "3");
        let edited = number(&mut app, "length_bars", "2");
        near(generation(&edited)["start"].as_f64().unwrap(), 2.0 * bar);
        near(generation(&edited)["length"].as_f64().unwrap(), 2.0 * bar);
        let fitted = app.click("Fit preview");
        assert_eq!(draft(&fitted), draft(&edited));
        let plot = &fitted["midi_editor"]["plot"];
        for painted in generation(&fitted)["painted_notes"].as_array().unwrap() {
            let rect = &painted["rect"];
            for axis in 0..=1 {
                assert!(
                    rect["min"][axis].as_f64().unwrap()
                        >= plot["min"][axis].as_f64().unwrap() - 0.1
                );
                assert!(
                    rect["max"][axis].as_f64().unwrap()
                        <= plot["max"][axis].as_f64().unwrap() + 0.1
                );
            }
        }
        for command in [
            json!({"command":"key", "key":"Delete"}),
            json!({"command":"key", "key":"D", "cmd":true}),
            json!({"command":"key", "key":"ArrowRight"}),
        ] {
            let blocked = app.command(command);
            assert_eq!(blocked["project"], before["project"]);
            assert_eq!(blocked["history"], before["history"]);
        }
        let escaped = app.key("Escape");
        assert!(escaped["midi_editor"]["generate"].is_null());
        assert_eq!(escaped["view"], "Clip");
        assert_eq!(escaped["project"], before["project"]);
        app.click("Generate…");
        let cancelled = app.click("Cancel");
        assert!(cancelled["midi_editor"]["generate"].is_null());
        assert_eq!(cancelled["project"], before["project"]);
        assert_eq!(cancelled["history"], before["history"]);
        app.quit();
    }
}

#[test]
fn no_complete_bar_requires_an_explicit_range_before_adding_notes() {
    let (mut app, id) = fixture(false, 4);
    let before = app.inspect();
    let (x, y) = center(&before["midi_editor"]["plot"]);
    let zoomed =
        app.command(json!({"command":"scroll", "x":x, "y":y, "delta":1000.0, "ctrl":true}));
    let view = &zoomed["midi_editor"];
    let span = (view["plot"]["max"][0].as_f64().unwrap()
        - view["plot"]["min"][0].as_f64().unwrap())
        / view["pixels_per_beat"].as_f64().unwrap();
    assert!(span < 4.0, "Fixture must not show a complete bar");
    let opened = app.click("Generate…");
    assert!(draft(&opened).is_empty());
    assert_eq!(generation(&opened)["length_bars"], 0);
    let disabled = app.click("Add notes");
    assert_eq!(disabled["project"], before["project"]);
    assert!(!disabled["midi_editor"]["generate"].is_null());
    number(&mut app, "start_bar", "1");
    let explicit = number(&mut app, "length_bars", "1");
    assert_draft_valid(&explicit);
    let added = app.click("Add notes");
    assert!(notes(&added, id).len() > notes(&before, id).len());
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn harmony_root_and_chord_choices_control_preview_pitches_without_changing_editor_scale() {
    let (mut app, _) = fixture(false, 4);
    let before = app.inspect();
    app.click("Generate…");
    choice(&mut app, "root", "D");
    let chord = choice(&mut app, "chord", "Minor 7");
    assert_eq!(
        draft(&chord)
            .iter()
            .map(|note| note["pitch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![62, 65, 69, 72]
    );
    choice(&mut app, "harmony", "Scale");
    let scale = choice(&mut app, "scale", "Major pentatonic");
    assert_eq!(draft(&scale).len(), 5);
    assert!(
        draft(&scale)
            .iter()
            .all(|note| Scale::Pentatonic.contains(2, note["pitch"].as_u64().unwrap() as u8))
    );
    number(&mut app, "velocity", "73");
    let lower = number(&mut app, "start_octave", "3");
    assert!(draft(&lower).iter().all(|note| note["velocity"] == 73));
    assert!(
        draft(&lower)
            .iter()
            .all(|note| note["pitch"].as_u64().unwrap() < 62)
    );
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    assert_eq!(
        cancelled["midi_editor"]["chord"],
        before["midi_editor"]["chord"]
    );
    assert_eq!(
        cancelled["midi_editor"]["keys"],
        before["midi_editor"]["keys"]
    );
    app.quit();
}

#[test]
fn generated_chord_commits_exact_preview_to_stretched_ghost_once_and_recalls() {
    let (mut app, id) = fixture(true, 4);
    let before = app.inspect();
    let original_notes = notes(&before, id).to_vec();
    app.click("Generate…");
    number(&mut app, "start_bar", "2");
    number(&mut app, "length_bars", "2");
    number(&mut app, "start_octave", "4");
    number(&mut app, "octaves", "2");
    let preview = number(&mut app, "shift", "1");
    let generated = draft(&preview).to_vec();
    assert_eq!(generated.len(), 6);
    assert!(
        generated
            .iter()
            .all(|note| note["start"] == 4.0 && note["length"] == 8.0)
    );
    assert_eq!(
        generated[0]["pitch"], 64,
        "Shift raises the lowest chord degree"
    );
    assert_eq!(preview["project"], before["project"]);
    let committed = app.click("Add notes");
    assert!(committed["midi_editor"]["generate"].is_null());
    let persisted = notes(&committed, id);
    assert_eq!(&persisted[..original_notes.len()], original_notes);
    assert_eq!(persisted.len(), original_notes.len() + generated.len());
    for (actual, expected) in persisted[original_notes.len()..].iter().zip(&generated) {
        assert_eq!(actual["pitch"], expected["pitch"]);
        assert_eq!(actual["velocity"], expected["velocity"]);
        near(
            actual["start"].as_f64().unwrap(),
            1.5 + expected["start"].as_f64().unwrap() * 0.75,
        );
        near(
            actual["length"].as_f64().unwrap(),
            expected["length"].as_f64().unwrap() * 0.75,
        );
    }
    let ids: HashSet<_> = persisted
        .iter()
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), persisted.len());
    for occurrence in committed["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
    {
        assert_eq!(occurrence["content"]["Midi"]["notes"], json!(persisted));
        let old = clip(&before, occurrence["id"].as_u64().unwrap());
        for field in ["start", "offset", "source_rate", "length"] {
            assert_eq!(
                occurrence[field], old[field],
                "Generation retains ghost placement"
            );
        }
    }
    assert_eq!(
        committed["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(
        committed["history"]["undo"]
            .as_array()
            .unwrap()
            .first()
            .unwrap(),
        "Generate notes"
    );
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], committed["project"]);
    let path = app.directory.path().join("generated-ghost.rill");
    app.command(json!({"command":"save", "path":path}));
    let recalled = app.command(json!({"command":"open", "path":path}));
    assert_eq!(recalled["project"], committed["project"]);
    Project::load(path).unwrap().validate().unwrap();
    app.quit();
}

#[test]
fn arpeggio_direction_speed_octaves_and_seeded_variations_match_added_notes() {
    let (mut app, id) = fixture(false, 4);
    let before = app.inspect();
    app.click("Generate…");
    app.click("Arpeggio");
    number(&mut app, "start_bar", "1");
    number(&mut app, "length_bars", "1");
    number(&mut app, "start_octave", "4");
    number(&mut app, "octaves", "1");
    choice(&mut app, "speed_multiplier", "1×");
    let up = choice(&mut app, "pattern", "Up");
    assert_draft_valid(&up);
    assert_eq!(draft(&up)[0]["pitch"], 60);
    assert_eq!(draft(&up)[1]["pitch"], 64);
    assert_eq!(draft(&up)[2]["pitch"], 67);
    let down = choice(&mut app, "pattern", "Down");
    assert_eq!(draft(&down)[0]["pitch"], 67);
    assert_eq!(draft(&down)[1]["pitch"], 64);
    assert_eq!(draft(&down)[2]["pitch"], 60);
    let faster = choice(&mut app, "speed_multiplier", "2×");
    assert_eq!(draft(&faster).len(), draft(&down).len() * 2);
    let shorter = number(&mut app, "gate", "50");
    assert!(
        draft(&shorter)
            .iter()
            .all(|note| (note["length"].as_f64().unwrap() - 0.125).abs() < 1e-8)
    );
    let wider = number(&mut app, "octaves", "2");
    assert!(
        draft(&wider)
            .iter()
            .any(|note| note["pitch"].as_u64().unwrap() >= 72)
    );
    let random = choice(&mut app, "pattern", "Random");
    assert_eq!(
        draft(&app.inspect()),
        draft(&random),
        "Repaint retains seeded preview"
    );
    let next = app.click("New variation");
    assert_ne!(draft(&next), draft(&random));
    assert_eq!(next["project"], before["project"]);
    assert_eq!(next["history"], before["history"]);
    let expected = draft(&next).to_vec();
    let committed = app.click("Add notes");
    for (actual, expected) in notes(&committed, id)[1..].iter().zip(&expected) {
        for field in ["pitch", "start", "length", "velocity"] {
            assert_eq!(actual[field], expected[field]);
        }
    }
    assert_eq!(notes(&committed, id).len(), expected.len() + 1);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn melody_algorithms_default_to_editor_scale_and_offer_counterpoint_and_tintinnabuli() {
    let (mut app, id) = fixture(false, 4);
    app.key("U");
    app.click("Chord stack");
    app.click("Scale stack");
    app.click("Natural minor");
    app.click("Dorian");
    app.key("Q");
    let before = app.inspect();
    app.click("Generate…");
    app.click("Melody");
    number(&mut app, "length_bars", "2");
    number(&mut app, "octaves", "2");
    let mut distinct_algorithms = HashSet::new();
    for algorithm in ["Random walk", "Motif variation", "Euclidean rhythm"] {
        let generated = choice(&mut app, "algorithm", algorithm);
        assert_draft_valid(&generated);
        assert!(
            draft(&generated)
                .iter()
                .all(|note| Scale::Dorian.contains(0, note["pitch"].as_u64().unwrap() as u8))
        );
        distinct_algorithms.insert(serde_json::to_string(draft(&generated)).unwrap());
        assert_eq!(generated["project"], before["project"]);
        assert_eq!(generated["history"], before["history"]);
    }
    assert_eq!(distinct_algorithms.len(), 3);
    let sparse = number(&mut app, "density", "50");
    let dense = number(&mut app, "density", "100");
    assert_eq!(draft(&dense).len(), draft(&sparse).len() * 2);
    choice(&mut app, "accompaniment", "Counterpoint");
    let counterpoint = number(&mut app, "additional_voices", "2");
    assert_draft_valid(&counterpoint);
    for voice in 0..=2 {
        assert!(
            draft(&counterpoint)
                .iter()
                .any(|note| note["voice"] == voice)
        );
    }
    let tintinnabuli = choice(&mut app, "accompaniment", "Tintinnabuli");
    assert_draft_valid(&tintinnabuli);
    assert!(
        draft(&tintinnabuli)
            .iter()
            .filter(|note| note["voice"] != 0)
            .all(|note| [0, 3, 7].contains(&(note["pitch"].as_u64().unwrap() % 12)))
    );
    let variation = app.click("New variation");
    assert_ne!(draft(&variation), draft(&tintinnabuli));
    let expected = draft(&variation).len();
    let compact = app.command(json!({"command":"viewport", "width":1100, "height":720}));
    assert_eq!(draft(&compact), draft(&variation));
    for label in ["Preview", "Add notes", "Cancel"] {
        assert!(
            compact["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == label),
            "{label} remains reachable at the minimum native window size"
        );
    }
    let committed = app.click("Add notes");
    assert_eq!(notes(&committed, id).len(), expected + 1);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn generation_preview_never_promotes_folded_notes_into_the_editing_selection() {
    for apply in [false, true] {
        for escape in [false, true] {
            if apply && escape {
                continue;
            }
            let hidden = Note::new(61, 2.0, 0.5, 81);
            let hidden_id = hidden.id;
            let (mut app, id) = fixture_with_notes(
                false,
                4,
                vec![
                    Note::new(60, 1.0, 0.5, 91),
                    hidden,
                    Note::new(67, 3.0, 0.5, 101),
                ],
            );
            let before = app.inspect();
            let original_ids: HashSet<_> = notes(&before, id)
                .iter()
                .map(|note| note["id"].as_u64().unwrap())
                .collect();
            app.command(json!({"command":"key", "key":"A", "cmd":true}));
            app.click("View");
            let folded = app.click("Only scale keys");
            assert_eq!(folded["midi_editor"]["hidden_notes"], 1);
            app.key("Escape");
            let preview = app.click("Generate…");
            assert_eq!(preview["midi_editor"]["only_scale_keys"], true);
            assert_eq!(
                preview["midi_editor"]["visible_pitches"]
                    .as_array()
                    .unwrap()
                    .len(),
                128
            );
            let shown_hidden = preview["midi_editor"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|note| note["id"] == hidden_id)
                .expect("Preview exposes the hidden pitch without selecting it");
            assert_eq!(shown_hidden["selected"], false);
            let closed = if apply {
                app.click("Add notes")
            } else if escape {
                app.key("Escape")
            } else {
                app.click("Cancel")
            };
            assert!(closed["midi_editor"]["generate"].is_null());
            assert_eq!(closed["midi_editor"]["only_scale_keys"], true);
            assert!(
                closed["midi_editor"]["visible_pitches"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|pitch| Scale::Minor.contains(0, pitch.as_u64().unwrap() as u8))
            );
            let deleted = app.key("Delete");
            assert!(
                notes(&deleted, id)
                    .iter()
                    .any(|note| note["id"] == hidden_id),
                "Original off-scale note survives immediately after dialog dismissal"
            );
            if apply {
                assert_eq!(notes(&closed, id).len(), 6);
                assert_eq!(
                    notes(&deleted, id).len(),
                    4,
                    "Delete removes only the two visible generated chord tones"
                );
                for original in notes(&before, id) {
                    assert!(notes(&deleted, id).contains(original));
                }
                let generated: Vec<_> = notes(&deleted, id)
                    .iter()
                    .filter(|note| !original_ids.contains(&note["id"].as_u64().unwrap()))
                    .collect();
                assert_eq!(generated.len(), 1);
                assert_eq!(
                    generated[0]["pitch"], 64,
                    "Generated off-scale E remains unselected in the folded editor"
                );
            } else {
                assert_eq!(notes(&deleted, id).len(), 1);
                assert_eq!(notes(&deleted, id)[0]["id"], hidden_id);
            }
            app.quit();
        }
    }
}

#[test]
fn generate_action_and_multi_draw_options_wrap_without_expanding_the_note_canvas() {
    let (mut app, _) = fixture(false, 4);
    let before = app.inspect();
    app.key("U");
    for width in [1100, 1440] {
        let state = app.command(json!({"command":"viewport", "width":width, "height":720}));
        assert_eq!(state["midi_editor"]["tool"], "MultiDraw");
        let plot = &state["midi_editor"]["plot"];
        assert!(plot["max"][0].as_f64().unwrap() <= f64::from(width) + 0.1);
        assert!(plot["max"][1].as_f64().unwrap() <= 720.1);
        assert!(plot["min"][1].as_f64().unwrap() < plot["max"][1].as_f64().unwrap());
        for label in ["Generate…", "Chord stack", "Major triad"] {
            let rect = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == label)
                .unwrap();
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
            assert!(
                rect["max"][0].as_f64().unwrap() <= f64::from(width) + 0.1,
                "{label} stays within the toolbar at {width}px"
            );
        }
        app.click("Chord stack");
        let scale = app.click("Scale stack");
        assert!(scale["midi_editor"]["plot"]["max"][0].as_f64().unwrap() <= f64::from(width) + 0.1);
        app.click("Scale stack");
        app.click("Chord stack");
    }
    assert_eq!(app.inspect()["project"], before["project"]);
    app.quit();
}
