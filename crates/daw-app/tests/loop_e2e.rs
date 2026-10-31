//! Loop controls are exercised through the production application's actual UI.
mod common;
use common::Harness;
use serde_json::{Value, json};

fn field_center(state: &Value, prefix: &str) -> (f64, f64) {
    let field = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            label["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("Missing {prefix}: {}", state["visible_labels"]));
    (
        (field["min"][0].as_f64().unwrap() + field["max"][0].as_f64().unwrap()) / 2.0,
        (field["min"][1].as_f64().unwrap() + field["max"][1].as_f64().unwrap()) / 2.0,
    )
}

fn type_beat(app: &mut Harness, prefix: &str, value: &str) -> Value {
    let (x, y) = field_center(&app.inspect(), prefix);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":value}));
    app.key("Enter")
}

fn edit(app: &mut Harness, label: &str) -> Value {
    app.click("Edit");
    app.click(label)
}

fn assert_range(state: &Value, start: f64, end: f64, enabled: bool) {
    assert_eq!(state["project"]["loop_start"], start);
    assert_eq!(state["project"]["loop_end"], end);
    assert_eq!(state["project"]["loop_enabled"], enabled);
}

#[test]
fn loop_indicator_clears_ruler_labels_without_shifting_the_canvas_when_toggled() {
    let mut app = Harness::new();
    for density in [1.0, 2.0] {
        app.command(
            json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}),
        );
        for dark in [true, false] {
            let off = app.command(json!({"command":"system_theme","dark":dark}));
            app.click("Loop");
            let on = app.command(json!({"command":"inspect","paint":true}));
            let palette = if dark {
                daw_ui::theme::DARK
            } else {
                daw_ui::theme::LIGHT
            };
            let fill = json!(palette.accent.gamma_multiply(0.6).to_array());
            let canvas_top = on["timeline"]["canvas"]["min"][1].as_f64().unwrap();
            let strip = on["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|shape| {
                    shape["fill"] == fill && shape["rect"]["max"][1].as_f64().unwrap() <= canvas_top
                })
                .expect("visible loop indicator below ruler labels");
            let strip_top = strip["rect"]["min"][1].as_f64().unwrap();
            let clock = on["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|label| label["text"] == "0:00")
                .expect("visible clock label");
            assert!(
                clock["max"][1].as_f64().unwrap() + 8.0 <= strip_top,
                "loop indicator crowds the clock at {density}x: {clock}, {strip}"
            );
            assert_eq!(off["timeline"]["canvas"], on["timeline"]["canvas"]);
            app.click("Loop");
        }
    }
    app.quit();
}

#[test]
fn loop_range_toggle_undo_archive_reopen_and_full_song_export() {
    let mut app = Harness::new();
    let source = app.directory.path().join("loop-source.wav");
    daw_core::AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 48_000 * 3],
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source]}));
    let state = app.click("Range");
    assert_range(&state, 0.0, 16.0, false);
    assert_range(
        &type_beat(&mut app, "Start beat ", "3.25"),
        2.25,
        16.0,
        false,
    );
    assert_range(&type_beat(&mut app, "End beat ", "6.5"), 2.25, 5.5, false);
    app.key("Escape");
    let enabled = app.click("Loop");
    assert_range(&enabled, 2.25, 5.5, true);
    let range = app.click("Range");
    for label in ["Start beat 3.25", "End beat 6.50"] {
        assert!(
            range["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == label),
            "Missing exact range field {label}: {}",
            range["visible_labels"]
        );
    }
    assert_eq!(range["project"], enabled["project"]);
    assert_eq!(range["history"], enabled["history"]);
    app.key("Escape");
    assert_range(&edit(&mut app, "Undo"), 2.25, 5.5, false);
    assert_range(&edit(&mut app, "Undo"), 2.25, 16.0, false);
    assert_range(&edit(&mut app, "Undo"), 0.0, 16.0, false);
    edit(&mut app, "Redo");
    edit(&mut app, "Redo");
    assert_range(&edit(&mut app, "Redo"), 2.25, 5.5, true);

    let archive = app.directory.path().join("loop-range.rill");
    app.command(json!({"command":"save","path":archive}));
    let disabled = app.click("Loop");
    assert_range(&disabled, 2.25, 5.5, false);
    let reopened = app.command(json!({"command":"open","path":archive}));
    assert_range(&reopened, 2.25, 5.5, true);
    let project = daw_core::Project::load(&archive).unwrap();
    project.validate().unwrap();
    assert_eq!(
        (project.loop_start, project.loop_end, project.loop_enabled),
        (2.25, 5.5, true)
    );

    // Export remains a complete song, independent of the live transport loop.
    let output = app.directory.path().join("whole-song.wav");
    app.command(json!({"command":"export","path":output}));
    app.command(json!({"command":"wait_export"}));
    let rendered = daw_core::AudioData::load_wav(output).unwrap();
    assert_eq!(rendered.frames(), 48_000 * 3 + 9_600);
    assert!(rendered.samples.iter().any(|sample| sample.abs() > 0.05));
    app.quit();
}

#[test]
fn loop_range_drag_is_one_undo_step_and_invalid_text_stays_bounded() {
    let mut app = Harness::new();
    let state = app.click("Range");
    let (x, y) = field_center(&state, "Start beat ");
    app.command(json!({"command":"pointer","phase":"down","x":x,"y":y}));
    app.command(json!({"command":"pointer","phase":"move","x":x+12.0,"y":y}));
    app.command(json!({"command":"pointer","phase":"move","x":x+24.0,"y":y}));
    let dragged = app.command(json!({"command":"pointer","phase":"up","x":x+24.0,"y":y}));
    let start = dragged["project"]["loop_start"].as_f64().unwrap();
    assert!(start > 0.0 && start < 16.0, "{}", dragged["project"]);
    assert_eq!(dragged["history"]["undo"].as_array().unwrap().len(), 1);
    app.key("Escape");
    assert_range(&edit(&mut app, "Undo"), 0.0, 16.0, false);
    assert_range(&edit(&mut app, "Redo"), start, 16.0, false);
    app.click("Range");
    let negative = type_beat(&mut app, "Start beat ", "-100");
    assert_range(&negative, 0.0, 16.0, false);
    let reversed = type_beat(&mut app, "End beat ", "-100");
    let end = reversed["project"]["loop_end"].as_f64().unwrap();
    assert!(end > 0.0 && end <= 16.0);
    let excessive = type_beat(&mut app, "End beat ", "100000000");
    assert_eq!(excessive["project"]["loop_end"], daw_core::MAX_BEAT);
    let before_invalid = excessive["project"].clone();
    let nonfinite = type_beat(&mut app, "End beat ", "NaN");
    assert_eq!(nonfinite["project"], before_invalid);
    let project: daw_core::Project = serde_json::from_value(nonfinite["project"].clone()).unwrap();
    project.validate().unwrap();
    app.quit();
}

#[test]
fn loop_endpoint_text_entry_keeps_step_buttons_still_and_never_steps_on_a_double_click() {
    for (width, density, dark) in [(800, 1.0, false), (1440, 2.0, true)] {
        let mut app = Harness::new();
        app.command(
            json!({"command":"viewport","width":width,"height":900,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("Range");
        for (prefix, replacement, expected_start, expected_end) in [
            ("Start beat ", "3.25", 2.25, 16.0),
            ("End beat ", "6.5", 2.25, 5.5),
        ] {
            let before = app.inspect();
            let (x, y) = field_center(&before, prefix);
            let buttons = |state: &Value| {
                ["−", "+"].map(|symbol| {
                    let button = state["menu_label_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|label| {
                            label["text"] == symbol
                                && ((label["min"][1].as_f64().unwrap()
                                    + label["max"][1].as_f64().unwrap())
                                    * 0.5
                                    - y)
                                    .abs()
                                    < 1.0
                        })
                        .expect("endpoint step button");
                    (button["min"][0].as_f64().unwrap() + button["max"][0].as_f64().unwrap()) * 0.5
                })
            };
            let original_buttons = buttons(&before);
            for _ in 0..2 {
                let focused = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
                assert_eq!(
                    focused["project"], before["project"],
                    "Focusing {prefix} must not step it"
                );
                assert_eq!(focused["history"], before["history"]);
                for (original, actual) in original_buttons.into_iter().zip(buttons(&focused)) {
                    assert!(
                        (original - actual).abs() < 0.01,
                        "{prefix} step button moved during text entry: {original} -> {actual} at {width}px/{density}x"
                    );
                }
            }
            // Numeric multi-click selects the complete value without a select-all shortcut.
            app.command(json!({"command":"text","text":replacement}));
            let edited = app.key("Enter");
            assert_range(&edited, expected_start, expected_end, false);
            assert_eq!(
                edited["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
        }
        app.key("Escape");
        assert_range(&edit(&mut app, "Undo"), 2.25, 16.0, false);
        assert_range(&edit(&mut app, "Undo"), 0.0, 16.0, false);
        app.quit();
    }
}

fn loop_drag_fixture(width: u32, density: f64, dark: bool, recording: bool) -> Harness {
    let mut app = if recording {
        Harness::with_virtual_audio(None, None)
    } else {
        Harness::new()
    };
    let source = app.directory.path().join("loop-drag.wav");
    daw_core::AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.05; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = daw_core::Project::new();
    project.loop_enabled = true;
    project.loop_start = 2.0;
    project.loop_end = 10.0;
    let audio = project.add_audio_track("Loop audio");
    project
        .track_mut(audio)
        .unwrap()
        .clips
        .push(daw_core::Clip::audio(
            "Audio phrase",
            0.0,
            4.0,
            source.to_string_lossy(),
        ));
    let midi = project.add_midi_track("Loop MIDI");
    project
        .track_mut(midi)
        .unwrap()
        .clips
        .push(daw_core::Clip::midi(
            "MIDI phrase",
            0.0,
            16.0,
            vec![daw_core::Note::new(60, 4.0, 1.0, 90)],
        ));
    let path = app.directory.path().join("loop-drag.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(
        json!({"command":"viewport","width":width,"height":900,"pixels_per_point":density}),
    );
    app.command(json!({"command":"system_theme","dark":dark}));
    app.click("Fit");
    app
}

fn rect_center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn loop_drag_context_unchanged(before: &Value, after: &Value) {
    for field in [
        "selected_track",
        "selected_clip",
        "selected_clips",
        "selection_span",
    ] {
        assert_eq!(
            after[field], before[field],
            "Loop endpoint drag changed {field}"
        );
    }
    assert_eq!(after["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(after["timeline"]["cursor"], before["timeline"]["cursor"]);
    assert_eq!(after["timeline"]["canvas"], before["timeline"]["canvas"]);
}

fn drag_loop_handle(app: &mut Harness, handle: &str, delta_beats: f64) -> (Value, Value) {
    let before = app.inspect();
    let (x, y) = rect_center(&before["timeline"][handle]);
    let dx = delta_beats * before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"down","x":x,"y":y}));
    let mut held = before;
    for fraction in [0.4, 0.75, 1.0] {
        held = app.command(json!({"command":"pointer","phase":"move","x":x+dx*fraction,"y":y}));
    }
    let released = app.command(json!({"command":"pointer","phase":"up","x":x+dx,"y":y}));
    (held, released)
}

#[test]
fn loop_strip_endpoints_drag_live_with_snap_and_separate_undo_without_editing_music() {
    for (width, density, dark) in [(800, 1.0, false), (1440, 2.0, true)] {
        let mut app = loop_drag_fixture(width, density, dark, false);
        app.click("Edit");
        let selected = app.click("Select all clips");
        let before = app.command(json!({"command":"inspect","paint":true}));
        assert_eq!(before["project"], selected["project"]);
        assert_eq!(before["history"], selected["history"]);
        assert_eq!(before["selected_clips"].as_array().unwrap().len(), 2);
        for handle in ["loop_start_handle", "loop_end_handle"] {
            let rect = &before["timeline"][handle];
            assert!(
                !rect.is_null(),
                "Visible loop endpoint needs a real mouse target"
            );
            assert!(
                rect["min"][1].as_f64().unwrap()
                    >= before["timeline"]["ruler"]["max"][1].as_f64().unwrap() + 8.0
            );
            let (x, y) = rect_center(rect);
            let hover = app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
            assert_eq!(hover["project"], before["project"]);
            assert_eq!(hover["history"], before["history"]);
            let clicked = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
            assert_eq!(clicked["project"], before["project"]);
            assert_eq!(clicked["history"], before["history"]);
            loop_drag_context_unchanged(&before, &clicked);
        }
        for (handle, delta, start, end) in [
            ("loop_start_handle", 1.13, 3.25, 10.0),
            ("loop_end_handle", 0.68, 3.25, 10.75),
        ] {
            let before = app.inspect();
            let (held, released) = drag_loop_handle(&mut app, handle, delta);
            assert_range(&held, start, end, true);
            assert_range(&released, start, end, true);
            loop_drag_context_unchanged(&before, &held);
            loop_drag_context_unchanged(&before, &released);
            let undo = released["history"]["undo"].as_array().unwrap();
            assert_eq!(
                undo.len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
            assert_eq!(undo.last(), Some(&json!("Loop range")));
            let undone = edit(&mut app, "Undo");
            assert_eq!(undone["project"], before["project"]);
            loop_drag_context_unchanged(&before, &undone);
            assert_eq!(edit(&mut app, "Redo")["project"], released["project"]);
        }
        // Snap is changed through the visible selector; no modifier is needed.
        let before = app.inspect();
        app.click("Snap 1/16");
        let snap_off = app.click("Off");
        assert_eq!(snap_off["project"], before["project"]);
        assert_eq!(snap_off["history"], before["history"]);
        let (held, released) = drag_loop_handle(&mut app, "loop_end_handle", 0.37);
        for state in [&held, &released] {
            assert_eq!(state["project"]["loop_start"], 3.25);
            assert!((state["project"]["loop_end"].as_f64().unwrap() - 11.12).abs() < 1e-4);
            loop_drag_context_unchanged(&before, state);
        }
        assert_eq!(
            released["history"]["undo"].as_array().unwrap().len(),
            before["history"]["undo"].as_array().unwrap().len() + 1
        );
        assert_eq!(edit(&mut app, "Undo")["project"], before["project"]);
        app.quit();
    }
}

fn zoom_loop_view(app: &mut Harness) -> Value {
    let state = app.inspect();
    let labels = state["label_rects"].as_array().unwrap();
    let fit = labels.iter().find(|label| label["text"] == "Fit").unwrap();
    let button = labels
        .iter()
        .filter(|label| {
            label["text"] == "+"
                && (rect_center(label).1 - rect_center(fit).1).abs() < 4.0
                && rect_center(label).0 < rect_center(fit).0
        })
        .max_by(|a, b| rect_center(a).0.total_cmp(&rect_center(b).0))
        .unwrap();
    let (x, y) = rect_center(button);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

#[test]
fn loop_strip_handles_keep_a_positive_range_and_never_appear_at_clipped_edges() {
    let mut app = loop_drag_fixture(1440, 1.0, false, false);
    let before = app.inspect();
    let (_, zero) = drag_loop_handle(&mut app, "loop_start_handle", -3.0);
    assert_range(&zero, 0.0, 10.0, true);
    loop_drag_context_unchanged(&before, &zero);
    assert_eq!(edit(&mut app, "Undo")["project"], before["project"]);
    let (_, crossed) = drag_loop_handle(&mut app, "loop_start_handle", 10.0);
    assert!((crossed["project"]["loop_start"].as_f64().unwrap() - 9.999).abs() < 1e-8);
    assert_eq!(crossed["project"]["loop_end"], 10.0);
    assert_eq!(edit(&mut app, "Undo")["project"], before["project"]);
    let (_, crossed) = drag_loop_handle(&mut app, "loop_end_handle", -9.0);
    assert_eq!(crossed["project"]["loop_start"], 2.0);
    assert!((crossed["project"]["loop_end"].as_f64().unwrap() - 2.001).abs() < 1e-8);
    assert_eq!(edit(&mut app, "Undo")["project"], before["project"]);

    // Centered zoom exposes the continuation of the loop, not a new endpoint.
    let mut clipped = app.inspect();
    for _ in 0..10 {
        clipped = zoom_loop_view(&mut app);
        if clipped["timeline"]["loop_start_handle"].is_null()
            && clipped["timeline"]["loop_end_handle"].is_null()
        {
            break;
        }
    }
    let timeline = &clipped["timeline"];
    let visible_start = timeline["start_beat"].as_f64().unwrap();
    let visible_end = visible_start
        + (timeline["canvas"]["max"][0].as_f64().unwrap()
            - timeline["canvas"]["min"][0].as_f64().unwrap())
            / timeline["pixels_per_beat"].as_f64().unwrap();
    assert!(visible_start > 2.0 && visible_end < 10.0);
    assert!(timeline["loop_start_handle"].is_null());
    assert!(timeline["loop_end_handle"].is_null());
    assert!(!timeline["loop_indicator"].is_null());
    let rect = &timeline["loop_indicator"];
    let (_, y) = rect_center(rect);
    for (x, dx) in [
        (rect["min"][0].as_f64().unwrap() + 1.0, 40.0),
        (rect["max"][0].as_f64().unwrap() - 1.0, -40.0),
    ] {
        let moved = app
            .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+dx,"to_y":y}));
        assert_eq!(moved["project"], clipped["project"]);
        assert_eq!(moved["history"], clipped["history"]);
    }
    app.quit();
}

#[test]
fn loop_strip_endpoint_drag_is_disabled_during_recording() {
    let mut app = loop_drag_fixture(1100, 1.0, true, true);
    app.click("Audio phrase");
    app.click("Window");
    app.click("Recording");
    app.click("Arm recording");
    app.key("Escape");
    app.click("Record");
    let wait_phase = |app: &mut Harness, phase: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let state = app.inspect();
            if state["recording"]["phase"] == phase {
                break state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Recording did not reach {phase}: {state}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    let recording = wait_phase(&mut app, "recording");
    for handle in ["loop_start_handle", "loop_end_handle"] {
        let (_, released) = drag_loop_handle(&mut app, handle, 1.0);
        assert_eq!(released["project"], recording["project"]);
        assert_eq!(released["history"], recording["history"]);
        assert_eq!(released["recording"]["phase"], "recording");
    }
    app.click("Stop");
    wait_phase(&mut app, "idle");
    app.quit();
}
