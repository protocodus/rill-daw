//! Track readouts stay concise without turning rounded captions into song edits.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn click_label(app: &mut Harness, text: &str, double: bool) -> Value {
    let state = app.inspect();
    let rect = label(&state, text);
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":if double {"double_click"} else {"click"}, "x":x, "y":y}))
}

fn channel(state: &Value, track: u64) -> &Value {
    state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()
}

fn unchanged(state: &Value, original: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(
            state[field], original[field],
            "Inspecting a channel changed {field}"
        );
    }
}

fn assert_text_clear_of_meter(state: &Value, text: &str, track: u64) {
    let painted = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| shape["text"] == text)
        .unwrap_or_else(|| panic!("Missing painted {text}"));
    let meter = &channel(state, track)["meter"];
    assert!(
        painted["rect"]["max"][0].as_f64().unwrap() <= meter["min"][0].as_f64().unwrap() - 3.0,
        "{text} overlaps its output meter: {painted}, {meter}"
    );
}

#[test]
fn header_captions_stay_bounded_and_exact_popup_dismissal_preserves_precision() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let mut tracks = Vec::new();
    for (name, gain, pan) in [
        ("Reference", 0.5, 0.12345679),
        ("Quiet", 0.00005, -1.0),
        ("High", 2.0, 1.0),
        ("Silent", 0.0, -0.5),
    ] {
        let track = project.add_bus(name);
        project.track_mut(track).unwrap().gain = gain;
        project.track_mut(track).unwrap().pan = pan;
        tracks.push(track);
    }
    let fixture = app.directory.path().join("readable-headers.rill");
    project.save(&fixture).unwrap();
    let original = app.command(json!({"command":"open", "path":fixture}));
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [1100, 1480] {
            app.command(json!({"command":"viewport", "width":width, "height":900, "pixels_per_point":density}));
            for (track, gain_label, pan_label, gain) in [
                (tracks[0], "-6.0 dB", "R12", 0.5_f32),
                (tracks[1], "-86.0 dB", "L100", 0.00005_f32),
                (tracks[2], "6.0 dB", "R100", 2.0_f32),
                (tracks[3], "−∞ dB", "L50", 0.0_f32),
            ] {
                let state = app.command(json!({"command":"inspect", "paint":true}));
                assert_eq!(channel(&state, track)["compact"], false);
                for text in [gain_label, pan_label] {
                    label(&state, text);
                    assert_text_clear_of_meter(&state, text, track);
                }
                unchanged(&state, &original);
                click_label(&mut app, gain_label, false);
                let focused = app.command(json!({"command":"inspect", "paint":true}));
                let precise = daw_ui::exact_value_text(f64::from(daw_ui::channel_gain_to_db(gain)));
                let field = label(&focused, &precise);
                assert!(field["min"][0].as_f64().unwrap() >= 0.0);
                assert!(field["max"][0].as_f64().unwrap() < f64::from(width));
                unchanged(&focused, &original);
                click_label(&mut app, &precise, false);
                let clicked_input = app.inspect();
                label(&clicked_input, "Apply value");
                unchanged(&clicked_input, &original);
                unchanged(&app.click("Cancel"), &original);
                click_label(&mut app, gain_label, false);
                app.command(json!({"command":"text", "text":"-24.12345"}));
                // Outside dismissal cancels a draft, including for stored
                // gain values outside the ordinary editing range.
                // Row heights can move a channel beneath fixed coordinates.
                // Dismiss in the live timeline canvas, away from track actions.
                let canvas = &state["timeline"]["canvas"];
                let dismissed = app.command(json!({"command":"pointer", "phase":"click",
                    "x":canvas["max"][0].as_f64().unwrap()-12.0,
                    "y":canvas["max"][1].as_f64().unwrap()-12.0}));
                unchanged(&dismissed, &original);
                assert_eq!(dismissed["view"], "Arrangement");
                assert_eq!(dismissed["popups"]["properties"], false);
                click_label(&mut app, pan_label, false);
                unchanged(&app.click("Apply value"), &original);
                click_label(&mut app, pan_label, false);
                app.command(json!({"command":"text", "text":"55.6789"}));
                unchanged(&app.key("Escape"), &original);
                if gain == 0.5 {
                    click_label(&mut app, gain_label, false);
                    unchanged(&app.click("Apply value"), &original);
                }
            }
        }
    }
    app.quit();
}

#[test]
fn concise_header_fields_still_accept_precise_values_and_undo_them() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_bus("Precise channel");
    project.track_mut(track).unwrap().gain = 0.5;
    project.track_mut(track).unwrap().pan = 0.12345679;
    let fixture = app.directory.path().join("precise-header.rill");
    project.save(&fixture).unwrap();
    let original = app.command(json!({"command":"open", "path":fixture}));
    click_label(&mut app, "-6.0 dB", false);
    app.command(json!({"command":"text", "text":"-12.34567"}));
    let gained = app.key("Enter");
    let gain = gained["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!((gain - 10.0_f64.powf(-12.34567 / 20.0)).abs() < 1e-7);
    assert_eq!(
        gained["project"]["tracks"][0]["pan"],
        original["project"]["tracks"][0]["pan"]
    );
    assert_eq!(gained["history"]["undo"].as_array().unwrap().len(), 1);
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], original["project"]);
    assert_eq!(undone["modified"], original["modified"]);
    app.click("Edit");
    let redone = app.click("Redo");
    assert_eq!(redone["project"], gained["project"]);

    click_label(&mut app, "R12", false);
    app.command(json!({"command":"text", "text":"-23.45678"}));
    let panned = app.click("Apply value");
    assert!((panned["project"]["tracks"][0]["pan"].as_f64().unwrap() + 0.2345678).abs() < 1e-7);
    assert_eq!(
        panned["project"]["tracks"][0]["gain"],
        gained["project"]["tracks"][0]["gain"]
    );
    assert_eq!(panned["history"]["undo"].as_array().unwrap().len(), 2);
    app.click("Edit");
    assert_eq!(app.click("Undo")["project"], gained["project"]);
    app.quit();
}
