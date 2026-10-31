//! Real mouse journeys for the piano roll's visual aids and shared musical edits.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

const PHRASE: &str = "Piano refinement phrase";

fn fixture(source: Vec<Note>, mapped: bool) -> (Harness, u64, u64) {
    fixture_with_length(source, mapped, 8.0)
}

fn fixture_with_length(source: Vec<Note>, mapped: bool, length: f64) -> (Harness, u64, u64) {
    let mut clip = Clip::midi(PHRASE, 0.0, length, source);
    if mapped {
        clip.start = 0.125;
        clip.offset = 0.5;
        clip.source_rate = 0.5;
    }
    fixture_with_clip(clip)
}

fn fixture_with_clip(clip: Clip) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Piano refinement instrument");
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("piano-refinement.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click(PHRASE);
    app.click("Window");
    app.click("Clip Editor");
    (app, track, clip_id)
}

fn notes(state: &Value) -> Vec<Value> {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .clone()
}

fn note(state: &Value, id: u64) -> Value {
    notes(state)
        .into_iter()
        .find(|note| note["id"] == id)
        .unwrap()
}

fn painted(state: &Value, id: u64) -> &Value {
    state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == id)
        .unwrap_or_else(|| panic!("Note {id} is not painted: {}", state["midi_editor"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn select(app: &mut Harness, id: u64, add: bool) -> Value {
    let state = app.inspect();
    let (x, y) = center(&painted(&state, id)["rect"]);
    let state = app.command(json!({"command":"pointer","phase":"click","button":if add {"secondary"} else {"primary"},"x":x,"y":y}));
    if add {
        app.click("Add to selection")
    } else {
        state
    }
}

fn menu(app: &mut Harness, title: &str, item: &str) -> Value {
    app.click(title);
    let state = app.click(item);
    if title == "View" {
        // View settings deliberately stay open for several changes. Dismiss
        // using the passive editor header before opening the next menu.
        app.click(&format!("Clip Editor · {PHRASE}"))
    } else {
        state
    }
}

fn unchanged(state: &Value, before: &Value) {
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    assert_eq!(state["song_session"], before["song_session"]);
}

fn history_len(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

fn assert_visible_control(state: &Value, wanted: &str, width: f64, height: f64) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"].as_str().unwrap().starts_with(wanted))
        .unwrap_or_else(|| {
            panic!(
                "Missing visible control {wanted}: {}",
                state["visible_labels"]
            )
        });
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{wanted}: {rect}");
    assert!(rect["min"][1].as_f64().unwrap() >= 0.0, "{wanted}: {rect}");
    assert!(
        rect["max"][0].as_f64().unwrap() <= width,
        "{wanted}: {rect}"
    );
    assert!(
        rect["max"][1].as_f64().unwrap() <= height,
        "{wanted}: {rect}"
    );
}

fn assert_compact_editor(state: &Value, controls: &[&str]) {
    for control in controls {
        assert_visible_control(state, control, 900.0, 700.0);
    }
    let plot = &state["midi_editor"]["plot"];
    let nine_rows = 9.0 * state["midi_editor"]["row_height"].as_f64().unwrap();
    assert!(
        plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap() >= nine_rows,
        "The compact toolbar leaves room for nine piano rows: {plot}, controls {controls:?}"
    );
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

fn preview_as_source(state: &Value) -> Vec<Value> {
    let rate = state["project"]["tracks"][0]["clips"][0]["source_rate"]
        .as_f64()
        .unwrap();
    state["midi_editor"]["preview_notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| {
            let mut note = note.clone();
            note["start"] = json!(note["start"].as_f64().unwrap() * rate);
            note["length"] = json!(note["length"].as_f64().unwrap() * rate);
            note
        })
        .collect()
}

fn transform(
    app: &mut Harness,
    track: u64,
    clip: u64,
    ids: &[u64],
    operation: &str,
    settings: Value,
) -> Value {
    let state = app.inspect();
    let mut args = json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "track_id":track,"clip_id":clip,"note_ids":ids,"operation":operation,
    });
    args.as_object_mut()
        .unwrap()
        .extend(settings.as_object().unwrap().clone());
    let state =
        app.command(json!({"command":"song_tool","name":"transform_phrase","arguments":args}));
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    state
}

fn number(app: &mut Harness, field: &str, value: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(&state["midi_editor"]["randomize"][field]);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":value}));
    app.click("Randomize preview")
}

#[test]
fn velocity_color_scale_warning_and_quiet_rows_remain_independent_visual_aids() {
    let soft = Note::new(60, 0.0, 0.5, 8);
    let loud = Note::new(60, 1.0, 0.5, 124);
    let outside = Note::new(61, 2.0, 0.5, 92);
    let ids = [soft.id, loud.id, outside.id];
    let (mut app, _, _) = fixture(vec![soft, loud, outside], false);
    let original = app.inspect();

    for dark in [true, false] {
        let state = app.command(json!({"command":"system_theme","dark":dark}));
        for id in &ids[..2] {
            let rendered = painted(&state, *id);
            let color = &rendered["fill_color"];
            assert!(color[2].as_u64().unwrap() > color[0].as_u64().unwrap());
            assert_eq!(rendered["out_of_scale"], false);
        }
        let soft_color = &painted(&state, ids[0])["fill_color"];
        let loud_color = &painted(&state, ids[1])["fill_color"];
        let luma = |color: &Value| {
            color[0].as_f64().unwrap() * 0.2126
                + color[1].as_f64().unwrap() * 0.7152
                + color[2].as_f64().unwrap() * 0.0722
        };
        assert!((luma(loud_color) - luma(soft_color)) * if dark { 1.0 } else { -1.0 } > 35.0);
        let warning = painted(&state, ids[2]);
        assert_eq!(warning["out_of_scale"], true);
        assert!(
            warning["fill_color"][0].as_u64().unwrap() > warning["fill_color"][2].as_u64().unwrap()
        );

        // Menu interactions separate these clicks; revisiting the same note
        // after changing theme must not become a delete double-click.
        let selected_id = ids[0];
        let selected = select(&mut app, selected_id, false);
        assert_eq!(painted(&selected, selected_id)["selected"], true);
        assert_eq!(
            painted(&selected, selected_id)["fill_color"],
            painted(&state, selected_id)["fill_color"]
        );
        assert!(
            painted(&selected, selected_id)["stroke_width"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        unchanged(&selected, &original);
        unchanged(&menu(&mut app, "Edit", "Clear note selection"), &original);
    }

    let before_view = app.inspect();
    let c_row = before_view["midi_editor"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["pitch"] == 60)
        .unwrap();
    let tint = &c_row["row_color"];
    assert!(tint[1].as_u64().unwrap() > tint[0].as_u64().unwrap());
    assert!(tint[1].as_u64().unwrap() > tint[2].as_u64().unwrap());
    menu(&mut app, "View", "Highlight scale");
    let without_rows = menu(&mut app, "View", "Highlight chord");
    assert_eq!(without_rows["midi_editor"]["highlight_scale"], false);
    assert_eq!(without_rows["midi_editor"]["highlight_chord"], false);
    assert_eq!(
        painted(&without_rows, ids[2])["fill_color"],
        painted(&before_view, ids[2])["fill_color"]
    );
    let no_warning = menu(&mut app, "View", "Color out-of-scale notes");
    let color = &painted(&no_warning, ids[2])["fill_color"];
    assert!(color[2].as_u64().unwrap() > color[0].as_u64().unwrap());
    let restored = menu(&mut app, "View", "Color out-of-scale notes");
    assert_eq!(
        painted(&restored, ids[2])["fill_color"],
        painted(&before_view, ids[2])["fill_color"]
    );
    unchanged(&restored, &original);
    app.quit();
}

#[test]
fn randomize_preview_reroll_cancel_and_selected_source_timing_match_the_semantic_tool() {
    let mut source = vec![
        Note::new(60, 0.125, 0.2, 9), // Retained source material before the trim.
        Note::new(64, 1.1, 0.4, 72),
        Note::new(67, 1.6, 0.3, 90),
        Note::new(69, 2.2, 0.2, 110),
        Note::new(72, 3.0, 0.3, 100),
    ];
    // Stable fixture IDs keep the seeded variation independent of parallel test order.
    for (index, note) in source.iter_mut().enumerate() {
        note.id = 10_000 + index as u64;
    }
    let chosen: Vec<_> = source[1..4].iter().map(|note| note.id).collect();
    let (mut app, track, clip) = fixture(source, true);
    select(&mut app, chosen[0], false);
    for id in &chosen[1..] {
        select(&mut app, *id, true);
    }
    let before = app.inspect();
    let narrow = app.command(json!({"command":"viewport","width":900,"height":700}));
    assert_compact_editor(
        &narrow,
        &[
            "Generate…",
            "Duplicate",
            "Glue",
            "Erase",
            "Cursor",
            "Draw settings",
            "Notes",
        ],
    );
    unchanged(&narrow, &before);
    let first = menu(&mut app, "Notes", "Randomize...");
    assert_eq!(first["midi_editor"]["randomize"]["velocity"], 16);
    assert_eq!(first["midi_editor"]["randomize"]["timing"], 0.0);
    assert_eq!(first["midi_editor"]["randomize"]["seed"], 1);
    unchanged(&first, &before);
    let narrow = app.command(json!({"command":"viewport","width":900,"height":700}));
    assert_compact_editor(
        &narrow,
        &["Apply randomize", "Cancel randomize", "New variation"],
    );
    unchanged(&narrow, &before);
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let velocity_preview = preview_as_source(&first);
    assert!(velocity_preview != notes(&before));
    for (after, original) in velocity_preview.iter().zip(notes(&before)) {
        assert_eq!(after["start"], original["start"]);
        assert_eq!(after["length"], original["length"]);
        if chosen.contains(&original["id"].as_u64().unwrap()) {
            assert!(
                (after["velocity"].as_i64().unwrap() - original["velocity"].as_i64().unwrap())
                    .abs()
                    <= 16
            );
        } else {
            assert_eq!(*after, original);
        }
    }
    let rerolled = app.click("New variation");
    assert_ne!(
        rerolled["midi_editor"]["randomize"]["seed"],
        first["midi_editor"]["randomize"]["seed"]
    );
    assert_ne!(preview_as_source(&rerolled), velocity_preview);
    unchanged(&rerolled, &before);
    unchanged(&app.click("Cancel randomize"), &before);

    menu(&mut app, "Notes", "Randomize...");
    number(&mut app, "velocity_rect", "12");
    number(&mut app, "seed_rect", "42");
    let timed = number(&mut app, "timing_rect", "0.125");
    assert_eq!(timed["midi_editor"]["randomize"]["velocity"], 12);
    assert_eq!(timed["midi_editor"]["randomize"]["seed"], 42);
    near(
        timed["midi_editor"]["randomize"]["timing"]
            .as_f64()
            .unwrap(),
        0.125,
    );
    let settings = timed["midi_editor"]["randomize"].clone();
    let expected = preview_as_source(&timed);
    assert!(
        expected
            .iter()
            .zip(notes(&before))
            .any(|(after, original)| after["start"] != original["start"])
    );
    for (after, original) in expected.iter().zip(notes(&before)) {
        if chosen.contains(&original["id"].as_u64().unwrap()) {
            assert!(
                (after["start"].as_f64().unwrap() - original["start"].as_f64().unwrap()).abs()
                    <= 0.125 * 0.5 + 1e-10
            );
            assert_eq!(after["pitch"], original["pitch"]);
            assert_eq!(after["length"], original["length"]);
        } else {
            assert_eq!(*after, original);
        }
    }
    unchanged(&timed, &before);
    let applied = app.click("Apply randomize");
    assert_eq!(notes(&applied), expected);
    assert!(applied["midi_editor"]["preview_notes"].is_null());
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    menu(&mut app, "Edit", "Undo");
    let automated = transform(
        &mut app,
        track,
        clip,
        &chosen,
        "randomize",
        json!({"velocity":settings["velocity"],"timing":settings["timing"],"seed":settings["seed"]}),
    );
    assert_eq!(automated["project"], applied["project"]);
    assert_eq!(history_len(&automated), history_len(&before) + 1);
    app.quit();
}

#[test]
fn visible_quantize_and_legato_preserve_scope_chords_and_source_mapping() {
    let source = vec![
        Note::new(60, 0.125, 0.2, 10),
        Note::new(64, 1.1, 0.2, 90),
        Note::new(67, 1.1, 0.4, 100),
        Note::new(72, 1.8, 0.3, 110),
    ];
    let chosen = [source[1].id, source[2].id];
    let untouched = [source[0].id, source[3].id];
    let (mut app, track, clip) = fixture(source, true);
    select(&mut app, chosen[0], false);
    select(&mut app, chosen[1], true);
    let before = app.inspect();
    let narrow = app.command(json!({"command":"viewport","width":900,"height":700}));
    assert_compact_editor(
        &narrow,
        &[
            "Generate…",
            "Duplicate",
            "Glue",
            "Erase",
            "Cursor",
            "Draw settings",
            "Notes",
        ],
    );
    unchanged(&narrow, &before);
    let preview = menu(&mut app, "Notes", "Quantize...");
    unchanged(&preview, &before);
    let narrow = app.command(json!({"command":"viewport","width":900,"height":700}));
    assert_compact_editor(&narrow, &["Apply quantize", "Cancel quantize"]);
    unchanged(&narrow, &before);
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let expected = preview_as_source(&preview);
    for id in chosen {
        let edited = expected.iter().find(|note| note["id"] == id).unwrap();
        // Audible beat = clip.start + (source.start - offset) / rate.
        near(
            0.125 + (edited["start"].as_f64().unwrap() - 0.5) / 0.5,
            1.25,
        );
        assert_eq!(edited["length"], note(&before, id)["length"]);
    }
    for id in untouched {
        assert_eq!(
            expected.iter().find(|note| note["id"] == id).unwrap(),
            &note(&before, id)
        );
    }
    unchanged(&app.click("Cancel quantize"), &before);
    menu(&mut app, "Notes", "Quantize...");
    let quantized = app.click("Apply quantize");
    assert_eq!(notes(&quantized), expected);
    assert_eq!(history_len(&quantized), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        quantized["project"]
    );
    menu(&mut app, "Edit", "Undo");
    let automated = transform(
        &mut app,
        track,
        clip,
        &chosen,
        "quantize",
        json!({"grid":0.25,"strength":1.0,"swing":0.5,"ends":false}),
    );
    assert_eq!(automated["project"], quantized["project"]);
    menu(&mut app, "Edit", "Undo");

    let legato = menu(&mut app, "Notes", "Legato");
    for id in chosen {
        let edited = note(&legato, id);
        near(edited["length"].as_f64().unwrap(), 0.7);
        assert_eq!(edited["start"], note(&before, id)["start"]);
        assert_eq!(edited["pitch"], note(&before, id)["pitch"]);
    }
    for id in untouched {
        assert_eq!(note(&legato, id), note(&before, id));
    }
    assert_eq!(history_len(&legato), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], legato["project"]);
    menu(&mut app, "Edit", "Undo");
    let automated = transform(&mut app, track, clip, &chosen, "legato", json!({}));
    assert_eq!(automated["project"], legato["project"]);
    app.quit();
}

#[test]
fn top_to_bottom_legato_connects_voice_ranks_across_missing_chord_tones() {
    let source = vec![
        Note::new(72, 0.0, 0.15, 100),
        Note::new(64, 0.0, 0.2, 90),
        Note::new(48, 0.0, 0.25, 80),
        Note::new(74, 1.0, 0.3, 100),
        Note::new(65, 1.0, 0.4, 90),
        Note::new(76, 2.0, 0.5, 100),
        Note::new(77, 3.0, 0.25, 100),
        Note::new(67, 3.0, 0.35, 90),
        Note::new(50, 3.0, 0.45, 80),
    ];
    let ignored = source[4].id;
    let chosen: Vec<_> = source
        .iter()
        .filter(|note| note.id != ignored)
        .map(|note| note.id)
        .collect();
    let expected_lengths = [1.0, 1.0, 3.0, 1.0, 0.4, 1.0, 0.25, 0.35, 0.45];
    let (mut app, track, clip) = fixture(source, false);
    app.click("Fit");
    menu(&mut app, "Edit", "Select all notes");
    let state = app.inspect();
    let (x, y) = center(&painted(&state, ignored)["rect"]);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    app.click("Remove from selection");
    let before = app.inspect();
    let applied = menu(&mut app, "Notes", "Legato top to bottom");
    for ((edited, original), length) in notes(&applied)
        .iter()
        .zip(notes(&before))
        .zip(expected_lengths)
    {
        near(edited["length"].as_f64().unwrap(), length);
        for field in ["id", "pitch", "start", "velocity"] {
            assert_eq!(edited[field], original[field]);
        }
    }
    assert_eq!(note(&applied, ignored), note(&before, ignored));
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    menu(&mut app, "Edit", "Undo");
    let automated = transform(
        &mut app,
        track,
        clip,
        &chosen,
        "legato",
        json!({"mode":"top_to_bottom"}),
    );
    assert_eq!(automated["project"], applied["project"]);
    assert_eq!(history_len(&automated), history_len(&before) + 1);
    app.quit();
}

#[test]
fn final_note_transform_previews_grow_the_original_clip_only_when_applied() {
    let mut last = Note::new(60, 3.9, 0.1, 90);
    last.id = 20_000;
    let id = last.id;
    let (mut app, _, clip) = fixture_with_length(vec![last], false, 4.0);
    menu(&mut app, "Edit", "Select all notes");
    let before = app.inspect();
    let preview = menu(&mut app, "Notes", "Quantize...");
    unchanged(&preview, &before);
    near(
        preview_as_source(&preview)[0]["start"].as_f64().unwrap(),
        4.0,
    );
    unchanged(&app.click("Cancel quantize"), &before);
    menu(&mut app, "Notes", "Quantize...");
    let cleared = menu(&mut app, "Edit", "Clear note selection");
    unchanged(&cleared, &before);
    assert!(
        !cleared["midi_editor"]["preview_notes"].is_null(),
        "The preview keeps its captured scope after selection changes"
    );
    let applied = app.click("Apply quantize");
    assert_eq!(
        applied["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(applied["project"]["tracks"][0]["clips"][0]["id"], clip);
    near(
        applied["project"]["tracks"][0]["clips"][0]["length"]
            .as_f64()
            .unwrap(),
        4.1,
    );
    near(note(&applied, id)["start"].as_f64().unwrap(), 4.0);
    near(note(&applied, id)["length"].as_f64().unwrap(), 0.1);
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    menu(&mut app, "Edit", "Undo");
    app.quit();

    // This source note mathematically ends at beat four, but converting its
    // source end puts it one ULP beyond the visible clip boundary.
    let mut last = Note::new(60, 6.6423958333333335, 0.15760416666666677, 90);
    last.id = 20_000;
    let original_start = last.start;
    let original_end = last.start + last.length;
    let mut source_clip = Clip::midi(PHRASE, 0.0, 4.0, vec![last]);
    source_clip.source_rate = 1.7;
    let local_end = source_clip.source_to_local(original_end);
    assert!(local_end > 4.0);
    near(local_end, 4.0);
    let (mut app, _, clip) = fixture_with_clip(source_clip);
    menu(&mut app, "Edit", "Select all notes");
    let before = app.inspect();
    menu(&mut app, "Notes", "Randomize...");
    let mut preview = number(&mut app, "timing_rect", "0.25");
    // Audition variations until the final note moves later. This uses the
    // user's reroll action rather than duplicating the deterministic RNG.
    for _ in 0..16 {
        if preview_as_source(&preview)[0]["start"].as_f64().unwrap() > original_start {
            break;
        }
        preview = app.click("New variation");
    }
    let mut expected = preview_as_source(&preview);
    // Timing variation preserves the exact source duration. Converting the
    // serialized local preview back through 1.7 can add another rounding ULP.
    expected[0]["length"] = notes(&before)[0]["length"].clone();
    let end =
        (expected[0]["start"].as_f64().unwrap() + expected[0]["length"].as_f64().unwrap()) / 1.7;
    assert!(
        end > 4.0,
        "The preview contains a note past the old clip edge"
    );
    assert_eq!(preview["project"], before["project"]);
    let before_apply = app.inspect();
    let randomized = app.click("Apply randomize");
    assert_eq!(notes(&randomized), expected);
    near(
        randomized["project"]["tracks"][0]["clips"][0]["length"]
            .as_f64()
            .unwrap(),
        end,
    );
    assert_eq!(randomized["project"]["tracks"][0]["clips"][0]["id"], clip);
    assert_eq!(history_len(&randomized), history_len(&before_apply) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        randomized["project"]
    );
    app.quit();
}

#[test]
fn editing_folded_notes_does_not_restore_the_hidden_old_selection_when_unfolding() {
    let mut inside = Note::new(60, 0.5, 0.5, 90);
    let mut outside = Note::new(61, 1.5, 0.5, 100);
    // Use a stable nonzero default-seed variation for the successful-edit check.
    inside.id = 30_000;
    outside.id = 30_001;
    let (inside_id, outside_id) = (inside.id, outside.id);
    let (mut app, _, _) = fixture(vec![inside, outside], false);
    menu(&mut app, "Edit", "Select all notes");
    let before = app.inspect();
    let folded = menu(&mut app, "View", "Only scale keys");
    assert_eq!(folded["midi_editor"]["hidden_notes"], 1);
    assert_eq!(painted(&folded, inside_id)["selected"], true);
    unchanged(&folded, &before);
    let preview = menu(&mut app, "Notes", "Randomize...");
    unchanged(&preview, &before);
    let applied = app.click("Apply randomize");
    assert_eq!(note(&applied, outside_id), note(&before, outside_id));
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    let unfolded = menu(&mut app, "View", "Only scale keys");
    assert_eq!(unfolded["midi_editor"]["hidden_notes"], 0);
    assert_eq!(painted(&unfolded, outside_id)["selected"], false);
    assert_eq!(note(&unfolded, outside_id), note(&before, outside_id));
    assert_eq!(unfolded["history"], applied["history"]);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    app.quit();
}

#[test]
fn quantize_overlap_keeps_the_existing_note_identity_and_velocity_in_one_undo() {
    let destination = Note::new(60, 1.0, 0.25, 64);
    let edited = Note::new(60, 1.12, 0.1, 110);
    let (destination_id, edited_id) = (destination.id, edited.id);
    let (mut app, _, _) = fixture(vec![destination, edited], false);
    select(&mut app, edited_id, false);
    let before = app.inspect();
    let preview = menu(&mut app, "Notes", "Quantize...");
    unchanged(&preview, &before);
    let applied = app.click("Apply quantize");
    assert_eq!(notes(&applied), vec![note(&before, destination_id)]);
    assert_eq!(painted(&applied, destination_id)["selected"], true);
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    app.quit();
}

#[test]
fn velocity_only_randomize_preserves_an_existing_trimmed_note_tail() {
    let mut held = Note::new(60, 0.0, 4.0, 90);
    held.id = 30_000;
    let id = held.id;
    let (mut app, _, _) = fixture_with_length(vec![held], false, 1.0);
    menu(&mut app, "Edit", "Select all notes");
    let before = app.inspect();
    let preview = menu(&mut app, "Notes", "Randomize...");
    unchanged(&preview, &before);
    assert_eq!(preview["midi_editor"]["randomize"]["timing"], 0.0);
    let expected = preview_as_source(&preview);
    let applied = app.click("Apply randomize");
    assert_eq!(notes(&applied), expected);
    assert_ne!(
        note(&applied, id)["velocity"],
        note(&before, id)["velocity"]
    );
    near(note(&applied, id)["start"].as_f64().unwrap(), 0.0);
    near(note(&applied, id)["length"].as_f64().unwrap(), 4.0);
    near(
        applied["project"]["tracks"][0]["clips"][0]["length"]
            .as_f64()
            .unwrap(),
        1.0,
    );
    assert_eq!(history_len(&applied), history_len(&before) + 1);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    app.quit();
}
