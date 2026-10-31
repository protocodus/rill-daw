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
