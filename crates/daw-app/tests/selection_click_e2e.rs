//! A plain click selects one clip; holding and dragging keeps the selected group.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(app: &mut Harness) -> std::path::PathBuf {
    let source = app.directory.path().join("Audio.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.1; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        1.0,
        3.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI phrase",
        2.0,
        2.0,
        vec![Note::new(60, 0.5, 1.0, 100)],
    ));
    project.add_audio_track("Audio destination");
    project.add_midi_track("MIDI destination");
    let path = app.directory.path().join("Select one or move group.rill");
    project.save(&path).unwrap();
    path
}

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn selected(state: &Value) -> Vec<u64> {
    state["selected_clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| pair[1].as_u64().unwrap())
        .collect()
}

fn point(state: &Value, clip: u64) -> (f64, f64) {
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == clip)
        .unwrap()["visible_rect"];
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer", "phase":phase, "x":at.0, "y":at.1}))
}

fn unchanged_document(state: &Value, before: &Value) {
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    assert_eq!(state["modified"], before["modified"]);
}

#[test]
fn selected_audio_and_midi_collapse_on_click_release_but_move_together_when_dragged() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let path = fixture(&mut app);
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1440] {
            app.command(json!({"command":"viewport", "width":width, "height":800}));
            for member in [0, 1] {
                app.command(json!({"command":"open", "path":path}));
                let before = menu(&mut app, "Edit", "Select all clips");
                let ids = selected(&before);
                let at = point(&before, ids[member]);
                let held = pointer(&mut app, "down", at);
                assert_eq!(selected(&held), ids);
                unchanged_document(&held, &before);
                let released = pointer(&mut app, "up", at);
                assert_eq!(selected(&released), vec![ids[member]]);
                unchanged_document(&released, &before);

                let grouped = menu(&mut app, "Edit", "Select all clips");
                let at = point(&grouped, ids[member]);
                let step = grouped["timeline"]["pixels_per_beat"].as_f64().unwrap();
                pointer(&mut app, "down", at);
                let moved = pointer(&mut app, "move", (at.0 + step, at.1));
                assert_eq!(selected(&moved), ids);
                assert_eq!(moved["project"]["tracks"][0]["clips"][0]["start"], 2.0);
                assert_eq!(moved["project"]["tracks"][1]["clips"][0]["start"], 3.0);
                let released = pointer(&mut app, "up", (at.0 + step, at.1));
                assert_eq!(selected(&released), ids);
                assert_eq!(released["project"], moved["project"]);
                assert_eq!(released["history"]["undo"], json!(["Move selected clips"]));
                let undone = menu(&mut app, "Edit", "Undo");
                assert_eq!(undone["project"], before["project"]);
                assert_eq!(selected(&undone), ids);
                let redone = menu(&mut app, "Edit", "Redo");
                assert_eq!(redone["project"], moved["project"]);
                assert_eq!(selected(&redone), ids);
            }
        }
        app.quit();
    }
}

#[test]
fn context_modifiers_cancel_and_pan_do_not_accidentally_collapse_a_group() {
    for (width, dark) in [(800, false), (1440, true)] {
        let mut app = Harness::new();
        let path = fixture(&mut app);
        app.command(json!({"command":"viewport", "width":width, "height":800}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"open", "path":path}));
        let before = menu(&mut app, "Edit", "Select all clips");
        let ids = selected(&before);

        // Context actions retain their explicit group membership semantics.
        let at = point(&before, ids[1]);
        let context = app.command(
            json!({"command":"pointer", "phase":"click", "button":"secondary", "x":at.0, "y":at.1}),
        );
        assert_eq!(selected(&context), ids);
        assert!(
            context["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!("2 clips selected"))
        );
        let removed = app.click("Remove from selection");
        assert_eq!(selected(&removed), vec![ids[0]]);
        let at = point(&removed, ids[1]);
        app.command(
            json!({"command":"pointer", "phase":"click", "button":"secondary", "x":at.0, "y":at.1}),
        );
        let added = app.click("Add to selection");
        assert_eq!(selected(&added), ids);
        let at = point(&added, ids[1]);
        app.command(
            json!({"command":"pointer", "phase":"click", "button":"secondary", "x":at.0, "y":at.1}),
        );
        let only = app.click("Select only this clip");
        assert_eq!(selected(&only), vec![ids[1]]);
        unchanged_document(&only, &before);

        for modifier in ["ctrl", "alt", "cmd"] {
            let state = menu(&mut app, "Edit", "Select all clips");
            let at = point(&state, ids[0]);
            let mut command = json!({"command":"pointer", "phase":"click", "x":at.0, "y":at.1});
            command[modifier] = json!(true);
            let clicked = app.command(command);
            assert_eq!(selected(&clicked), ids, "{modifier}");
            unchanged_document(&clicked, &before);
        }
        // A modifier pressed after pointer-down also cancels pending collapse.
        let state = menu(&mut app, "Edit", "Select all clips");
        let at = point(&state, ids[0]);
        pointer(&mut app, "down", at);
        app.command(json!({"command":"pointer", "phase":"move", "ctrl":true, "x":at.0, "y":at.1}));
        assert_eq!(selected(&pointer(&mut app, "up", at)), ids);
        for cancel in ["escape", "focus"] {
            let state = menu(&mut app, "Edit", "Select all clips");
            let at = point(&state, ids[0]);
            pointer(&mut app, "down", at);
            if cancel == "escape" {
                app.key("Escape");
            } else {
                app.command(json!({"command":"focused", "focused":false}));
                app.command(json!({"command":"focused", "focused":true}));
            }
            let released = pointer(&mut app, "up", at);
            assert_eq!(selected(&released), ids, "{cancel}");
            unchanged_document(&released, &before);
        }
        let state = app.inspect();
        let at = point(&state, ids[0]);
        let panned = app.command(json!({"command":"pointer", "phase":"drag", "button":"middle", "x":at.0, "y":at.1, "to_x":at.0-20.0, "to_y":at.1}));
        assert_eq!(selected(&panned), ids);
        unchanged_document(&panned, &before);
        let cleared = menu(&mut app, "Edit", "Clear clip selection");
        assert!(selected(&cleared).is_empty());
        unchanged_document(&cleared, &before);
        app.quit();
    }
}

#[test]
fn mixed_group_transfer_and_double_click_keep_the_intended_target() {
    let mut app = Harness::new();
    let path = fixture(&mut app);
    app.command(json!({"command":"viewport", "width":1000, "height":800}));
    app.command(json!({"command":"open", "path":path}));
    let before = menu(&mut app, "Edit", "Select all clips");
    let ids = selected(&before);
    let at = point(&before, ids[0]);
    let destination = &before["timeline"]["tracks"][2]["header"];
    let y =
        (destination["min"][1].as_f64().unwrap() + destination["max"][1].as_f64().unwrap()) * 0.5;
    let transferred = app.command(
        json!({"command":"pointer", "phase":"drag", "x":at.0, "y":at.1, "to_x":at.0, "to_y":y}),
    );
    assert_eq!(selected(&transferred), ids);
    assert_eq!(
        transferred["project"]["tracks"][2]["clips"][0]["id"],
        ids[0]
    );
    assert_eq!(
        transferred["project"]["tracks"][3]["clips"][0]["id"],
        ids[1]
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    let grouped = menu(&mut app, "Edit", "Select all clips");
    let at = point(&grouped, ids[1]);
    let opened =
        app.command(json!({"command":"pointer", "phase":"double_click", "x":at.0, "y":at.1}));
    assert_eq!(opened["view"], "Clip");
    assert_eq!(opened["selected_clip"], ids[1]);
    assert_eq!(selected(&opened), vec![ids[1]]);
    let back = app.click("Back");
    assert_eq!(back["view"], "Arrangement");
    assert_eq!(selected(&back), vec![ids[1]]);
    app.quit();
}

#[test]
fn initially_unfocused_and_batched_clicks_select_and_open_audio_and_midi() {
    for (width, dark) in [(800, false), (1440, true)] {
        let mut app = Harness::new();
        let path = fixture(&mut app);
        app.command(json!({"command":"viewport", "width":width, "height":800}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        for focused in [false, true] {
            // The no-window-manager native host can start without focus while
            // still delivering accepted pointer clicks. This is not a loss of
            // focus in the middle of an existing gesture.
            app.command(json!({"command":"focused", "focused":focused, "initial":true}));
            for member in [0, 1] {
                app.command(json!({"command":"open", "path":path}));
                let before = menu(&mut app, "Edit", "Select all clips");
                let ids = selected(&before);
                let at = point(&before, ids[member]);
                let clicked = pointer(&mut app, "click_batched", at);
                assert_eq!(selected(&clicked), vec![ids[member]], "focused={focused}");
                assert_eq!(clicked["view"], "Arrangement");
                unchanged_document(&clicked, &before);

                let grouped = menu(&mut app, "Edit", "Select all clips");
                let at = point(&grouped, ids[member]);
                let opened = pointer(&mut app, "double_click_batched", at);
                assert_eq!(opened["view"], "Clip", "focused={focused}");
                assert_eq!(opened["selected_clip"], ids[member]);
                assert_eq!(selected(&opened), vec![ids[member]]);
                unchanged_document(&opened, &before);
                app.click("Back");

                // Unfocused delivery also keeps normal separated-frame clicks
                // and selected-group dragging functional.
                let grouped = menu(&mut app, "Edit", "Select all clips");
                let at = point(&grouped, ids[member]);
                let clicked = pointer(&mut app, "click", at);
                assert_eq!(selected(&clicked), vec![ids[member]]);
                let grouped = menu(&mut app, "Edit", "Select all clips");
                let at = point(&grouped, ids[member]);
                let step = grouped["timeline"]["pixels_per_beat"].as_f64().unwrap();
                pointer(&mut app, "down", at);
                pointer(&mut app, "move", (at.0 + step, at.1));
                let moved = pointer(&mut app, "up", (at.0 + step, at.1));
                assert_eq!(selected(&moved), ids);
                assert_eq!(moved["project"]["tracks"][0]["clips"][0]["start"], 2.0);
                assert_eq!(moved["project"]["tracks"][1]["clips"][0]["start"], 3.0);
                assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
            }
        }
        app.quit();
    }
}

#[test]
fn batched_modifier_clicks_keep_explicit_group_membership_semantics() {
    let mut app = Harness::new();
    let path = fixture(&mut app);
    app.command(json!({"command":"open", "path":path}));
    for modifier in ["ctrl", "alt", "cmd", "shift"] {
        let before = menu(&mut app, "Edit", "Select all clips");
        let ids = selected(&before);
        let at = point(&before, ids[1]);
        let mut command = json!({"command":"pointer", "phase":"click_batched", "x":at.0, "y":at.1});
        command[modifier] = json!(true);
        let clicked = app.command(command);
        assert_eq!(
            selected(&clicked),
            if modifier == "shift" {
                vec![ids[0]]
            } else {
                ids
            }
        );
        assert_eq!(clicked["view"], "Arrangement");
        unchanged_document(&clicked, &before);
    }
    app.quit();
}
