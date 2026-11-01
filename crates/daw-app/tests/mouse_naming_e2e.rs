//! Names and document filenames can be created with pointer controls alone.
mod common;
use common::Harness;
use daw_core::AudioData;
use eframe::egui;
use serde_json::{Value, json};

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

/// Physical keyboard text/key events are deliberately absent from this helper.
fn mouse_text(app: &mut Harness, text: &str) {
    for character in text.chars() {
        if character.is_ascii_uppercase() {
            app.click("Caps");
        }
        let label = if character == ' ' {
            "Space".into()
        } else {
            character.to_string()
        };
        app.click(&label);
        if character.is_ascii_uppercase() {
            app.click("Caps");
        }
    }
}

fn assert_keyboard_fits(state: &Value, width: f64, height: f64) {
    for label in [
        "Keyboard",
        "Apply name",
        "Cancel",
        "q",
        "p",
        "Caps",
        "Space",
        "Backspace",
        "Select all",
        "Clear",
    ] {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == label)
            .unwrap_or_else(|| panic!("Missing {label}"));
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{label}: {rect}");
        assert!(rect["min"][1].as_f64().unwrap() >= 0.0, "{label}: {rect}");
        assert!(rect["max"][0].as_f64().unwrap() <= width, "{label}: {rect}");
        assert!(
            rect["max"][1].as_f64().unwrap() <= height,
            "{label}: {rect}"
        );
    }
}

fn assert_rename_surface_and_gap_input(app: &mut Harness, dark: bool) {
    let state = app.command(json!({"command":"inspect", "paint":true}));
    let bounds = |rect: &Value| {
        [
            rect["min"][0].as_f64().unwrap(),
            rect["min"][1].as_f64().unwrap(),
            rect["max"][0].as_f64().unwrap(),
            rect["max"][1].as_f64().unwrap(),
        ]
    };
    let label = |text: &str| {
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == text)
            .unwrap()
    };
    let contains = |outer: [f64; 4], inner: [f64; 4]| {
        outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
    };
    let fill = if dark {
        daw_ui::theme::DARK.raised
    } else {
        daw_ui::theme::LIGHT.raised
    };
    let surface = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| {
            let rect = bounds(&shape["rect"]);
            shape["fill"] == json!(fill.to_array())
                && ["Keyboard", "Apply name", "Cancel", "q", "p", "Clear"]
                    .iter()
                    .all(|text| contains(rect, bounds(label(text))))
        })
        .expect("The complete rename row and keyboard need one opaque surface");
    let surface = bounds(&surface["rect"]);
    assert!(
        surface[0] >= 0.0 && surface[1] >= 0.0 && surface[2] <= 800.0 && surface[3] <= 600.0,
        "{surface:?}"
    );
    let button = |text: &str| {
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|shape| bounds(&shape["rect"]))
            .filter(|rect| {
                contains(*rect, bounds(label(text)))
                    && rect[3] - rect[1] >= 33.0
                    && rect[3] - rect[1] <= 55.0
            })
            .min_by(|a, b| {
                ((a[2] - a[0]) * (a[3] - a[1])).total_cmp(&((b[2] - b[0]) * (b[3] - b[1])))
            })
            .unwrap()
    };
    let keyboard = button("Keyboard");
    let apply = button("Apply name");
    assert!(apply[0] - keyboard[2] >= 5.0);
    let x = (keyboard[2] + apply[0]) * 0.5;
    let y = (keyboard[1] + keyboard[3]) * 0.5;
    app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    let after = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(after["project"], state["project"]);
    assert_eq!(after["history"], state["history"]);
    assert!(has(&after, "Apply name") && has(&after, "Clear"));
    let foreground_labels = |state: &Value| {
        state["menu_label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|label| label["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        foreground_labels(&after),
        foreground_labels(&state),
        "Clicking an editor gap must not introduce a menu from the toolbar underneath"
    );
    // Native TextEdit can move its internal text by a pixel when its caret
    // loses focus. The surrounding editor and keyboard targets must stay fixed.
    for text in [
        "Keyboard",
        "Apply name",
        "Cancel",
        "q",
        "p",
        "Caps",
        "Clear",
    ] {
        let current = after["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == text)
            .unwrap();
        assert_eq!(
            bounds(current),
            bounds(label(text)),
            "Gap click moved {text}"
        );
    }
}

#[test]
fn long_clip_name_near_the_right_edge_keeps_complete_inline_actions_at_every_density() {
    let mut app = Harness::new();
    let mut project = daw_core::Project::new();
    let track = project.add_midi_track("Naming geometry");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(daw_core::Clip::midi(
            "A long current clip name with expressive musical detail and several descriptive words",
            4.0,
            4.0,
            vec![daw_core::Note::new(60, 0.0, 1.0, 90)],
        ));
    let path = app.directory.path().join("naming-geometry.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));
    for dark in [false, true] {
        for density in [1.0, 1.25, 2.0] {
            app.command(json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":density}));
            let state = app.command(json!({"command":"system_theme", "dark":dark}));
            let clip = &state["timeline"]["clips"][0]["visible_rect"];
            app.command(json!({"command":"pointer", "phase":"click",
                "x":(clip["min"][0].as_f64().unwrap()+clip["max"][0].as_f64().unwrap())*0.5,
                "y":(clip["min"][1].as_f64().unwrap()+clip["max"][1].as_f64().unwrap())*0.5}));
            let properties = menu(&mut app, "Window", "Properties");
            // The utility retains its scroll position across display changes.
            // Reveal the name row with the mouse before opening its keyboard.
            let tab = properties["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == "Clip")
                .unwrap();
            let window = properties["window_rects"]
                .as_array()
                .unwrap()
                .last()
                .unwrap();
            let x = window["max"][0].as_f64().unwrap() - 2.0 * f64::from(daw_ui::tokens::SPACE_LG);
            let y = tab["max"][1].as_f64().unwrap()
                + f64::from(daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_MD);
            for _ in 0..3 {
                app.command(json!({"command":"scroll", "x":x, "y":y, "delta":1000.0}));
            }
            let keyboard = app.click("Keyboard…");
            assert_keyboard_fits(&keyboard, 800.0, 600.0);
            assert_rename_surface_and_gap_input(&mut app, dark);
            // The driver reports clipped glyph rectangles. Compare to the full
            // measured label as well as viewport bounds so a sliced Cancel
            // label cannot masquerade as an in-bounds action.
            let ctx = egui::Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_pixels_per_point(density);
            let _ = ctx.run(egui::RawInput::default(), |_| {});
            let font = egui::TextStyle::Button.resolve(&ctx.style());
            for text in ["Keyboard", "Apply name", "Cancel"] {
                let expected = ctx.fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                        .rect
                        .width()
                });
                let rect = keyboard["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|rect| rect["text"] == text)
                    .unwrap();
                let actual = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
                assert!(
                    actual >= f64::from(expected - 1.0 / density),
                    "Clipped {text}: {rect}, expected {expected}"
                );
            }
            let cancelled = app.click("Cancel");
            assert_eq!(cancelled["project"], original["project"]);
            assert_eq!(cancelled["history"], original["history"]);
        }
    }
    app.quit();
}

#[test]
fn caps_stays_uppercase_until_toggled_off_without_editing_the_project() {
    let mut app = Harness::new();
    let original = app.inspect();
    menu(&mut app, "File", "Rename project…");
    let keyboard = app.click("Keyboard");
    assert!(has(&keyboard, "Caps"));
    assert!(!has(&keyboard, "Shift"));
    let uppercase = app.click("Caps");
    assert!(has(&uppercase, "A"));
    assert!(!has(&uppercase, "a"));
    app.click("A");
    let still_uppercase = app.click("B");
    assert!(has(&still_uppercase, "C"));
    let lowercase = app.click("Caps");
    assert!(has(&lowercase, "c"));
    let draft = app.click("c");
    assert_eq!(draft["project"], original["project"]);
    assert_eq!(draft["history"], original["history"]);
    let named = app.click("Apply name");
    assert_eq!(named["project"]["name"], "ABc");
    assert_eq!(named["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        original["project"]
    );
    app.quit();
}

fn seed(app: &mut Harness) {
    let audio = app.directory.path().join("Phrase.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4800).map(|i| (i as f32 * 0.03).sin() * 0.2).collect(),
    }
    .write_wav(&audio)
    .unwrap();
    // The fixture is imported by the ordinary visible menu and native-picker seam.
    app.command(json!({"command":"file_dialog", "kind":"import", "path":audio}));
    menu(app, "File", "Import…");
    common::wait_until(app, |state| {
        state["project"]["tracks"].as_array().unwrap().len() == 1
    });
}

#[test]
fn project_clip_and_track_names_use_pointer_text_and_supply_personalized_save_exports() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        seed(&mut app);
        let original = app.inspect();
        assert!(has(&original, "0:00.100"));
        menu(&mut app, "File", "Rename project…");
        let keyboard = app.click("Keyboard");
        assert_keyboard_fits(&keyboard, 800.0, 600.0);
        mouse_text(&mut app, "Blue 7!");
        let named = app.click("Apply name");
        assert_eq!(named["project"]["name"], "Blue 7!");
        assert!(has(&named, "0:00.100"));
        for (kind, filename) in [
            ("save", "Blue 7!.rill"),
            ("wav", "Blue 7!.wav"),
            ("midi", "Blue 7!.mid"),
        ] {
            assert_eq!(named["file_dialog_defaults"][kind], filename);
        }
        let undone = menu(&mut app, "Edit", "Undo");
        assert_eq!(undone["project"], original["project"]);
        let redone = menu(&mut app, "Edit", "Redo");
        assert_eq!(redone["project"]["name"], "Blue 7!");

        // Item keyboard entry opens directly from Properties and does not
        // change the song until Apply. Pointer keys replace the initial selection.
        let state = app.inspect();
        let clip_rect = &state["timeline"]["clips"][0]["visible_rect"];
        app.command(json!({"command":"pointer", "phase":"click",
            "x":(clip_rect["min"][0].as_f64().unwrap()+clip_rect["max"][0].as_f64().unwrap())*0.5,
            "y":(clip_rect["min"][1].as_f64().unwrap()+clip_rect["max"][1].as_f64().unwrap())*0.5}));
        menu(&mut app, "Window", "Properties");
        let keyboard = app.click("Keyboard…");
        assert_keyboard_fits(&keyboard, 800.0, 600.0);
        mouse_text(&mut app, "lead_one");
        app.click("Backspace");
        mouse_text(&mut app, "2");
        let clip = app.click("Apply name");
        assert_eq!(clip["project"]["tracks"][0]["clips"][0]["name"], "lead_on2");
        let before_cancel = clip.clone();
        menu(&mut app, "Edit", "Rename…");
        app.click("Keyboard");
        app.click("Clear");
        mouse_text(&mut app, "discard");
        let cancelled = app.click("Cancel");
        assert_eq!(cancelled["project"], before_cancel["project"]);
        assert_eq!(cancelled["history"], before_cancel["history"]);

        // Select all also works after several mouse edits and preserves spaces
        // and punctuation. Apply an unchanged name is a history no-op.
        menu(&mut app, "Edit", "Rename…");
        app.click("Keyboard");
        mouse_text(&mut app, "scratch");
        app.click("Select all");
        mouse_text(&mut app, "lead_on2");
        let same = app.click("Apply name");
        assert_eq!(same["project"], before_cancel["project"]);
        assert_eq!(same["history"], before_cancel["history"]);

        let track_name = same["project"]["tracks"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        app.click(&track_name);
        menu(&mut app, "Window", "Properties");
        app.click("Track");
        app.click("Keyboard…");
        mouse_text(&mut app, "Keys-1");
        let track = app.click("Apply name");
        assert_eq!(track["project"]["tracks"][0]["name"], "Keys-1");
        let track_undo = menu(&mut app, "Edit", "Undo");
        assert_eq!(track_undo["project"]["tracks"][0]["name"], track_name);
        menu(&mut app, "Edit", "Redo");

        // Native pickers receive the personalized defaults; the harness accepts
        // those exact defaults without injecting any filename text.
        let save = app.directory.path().join("Blue 7!.rill");
        app.command(json!({"command":"file_dialog", "kind":"save", "path":save}));
        menu(&mut app, "File", "Save");
        app.command(json!({"command":"wait_project"}));
        assert_eq!(daw_core::Project::load(&save).unwrap().name, "Blue 7!");
        let wav = app.directory.path().join("Blue 7!.wav");
        app.command(json!({"command":"file_dialog", "kind":"export", "path":wav}));
        menu(&mut app, "File", "Export WAV…");
        app.click("Export…");
        app.command(json!({"command":"wait_export"}));
        assert!(
            AudioData::load_wav(wav)
                .unwrap()
                .samples
                .iter()
                .any(|sample| sample.abs() > 0.01)
        );
        let midi = app.directory.path().join("Blue 7!.mid");
        app.command(json!({"command":"file_dialog", "kind":"export_midi", "path":midi}));
        let exported = menu(&mut app, "File", "Export MIDI…");
        assert_eq!(exported["exported_file"], json!(midi));
        assert!(std::fs::read(midi).unwrap().starts_with(b"MThd"));
        app.quit();
    }
}

#[test]
fn mouse_name_limit_and_blank_apply_do_not_damage_the_original_name() {
    let mut app = Harness::new();
    let original = app.inspect();
    menu(&mut app, "File", "Rename project…");
    app.click("Keyboard");
    app.click("Clear");
    app.click("Space");
    let blank = app.click("Apply name");
    assert_eq!(blank["project"], original["project"]);
    assert_eq!(blank["history"], original["history"]);
    menu(&mut app, "File", "Rename project…");
    app.click("Keyboard");
    mouse_text(&mut app, "ab");
    // Advance the harness clock beyond egui's multi-click interval before
    // placing a caret; a rapid synthetic click would select the whole word.
    for _ in 0..12 {
        app.inspect();
    }
    let draft = app.inspect();
    let text = draft["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "ab")
        .unwrap();
    app.command(json!({"command":"pointer", "phase":"click",
        "x":text["min"][0].as_f64().unwrap()+1.0,
        "y":(text["min"][1].as_f64().unwrap()+text["max"][1].as_f64().unwrap())*0.5}));
    mouse_text(&mut app, "x");
    assert_eq!(app.click("Apply name")["project"]["name"], "xab");
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        original["project"]
    );
    menu(&mut app, "File", "Rename project…");
    app.click("Keyboard");
    mouse_text(&mut app, &"a".repeat(130));
    let bounded = app.click("Apply name");
    assert_eq!(bounded["project"]["name"], "a".repeat(128));
    assert_eq!(bounded["history"]["undo"].as_array().unwrap().len(), 1);
    app.quit();
}

#[test]
fn existing_long_unicode_names_survive_open_apply_cancel_and_can_be_replaced() {
    let mut app = Harness::new();
    let mut project = daw_core::Project::new();
    project.name = "Cafe\u{301} 🎹 ".repeat(24);
    let path = app.directory.path().join("Existing.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));
    for action in ["Apply name", "Cancel"] {
        menu(&mut app, "File", "Rename project…");
        app.click("Keyboard");
        let unchanged = app.click(action);
        assert_eq!(unchanged["project"], original["project"]);
        assert_eq!(unchanged["history"], original["history"]);
    }
    menu(&mut app, "File", "Rename project…");
    app.click("Keyboard");
    // The initial Unicode selection is replaced without slicing a UTF-8 code
    // point or leaving a suffix from a name longer than the new-input limit.
    mouse_text(&mut app, "New 2");
    let replaced = app.click("Apply name");
    assert_eq!(replaced["project"]["name"], "New 2");
    assert_eq!(replaced["file_dialog_defaults"]["save"], "Existing.rill");
    assert_eq!(replaced["file_dialog_defaults"]["wav"], "Existing.wav");
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        original["project"]
    );
    app.quit();
}
