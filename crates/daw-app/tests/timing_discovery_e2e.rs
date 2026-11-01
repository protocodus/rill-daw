//! Loop scope and exact time conversions are discoverable with mouse controls.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn fixture() -> Harness {
    let mut app = Harness::new();
    let source = app.directory.path().join("Timing.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.05; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 112.0;
    project.beats_per_bar = 3;
    project.loop_start = 1.25;
    project.loop_end = 5.5;
    project.loop_enabled = false;
    let audio = project.add_audio_track("Timing audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Fractional clip",
        3.25,
        7.75,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("Timing MIDI");
    project
        .track_mut(midi)
        .unwrap()
        .clips
        .push(Clip::midi("Thirty seconds", 0.0, 56.0, vec![]));
    let path = app.directory.path().join("Timing.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app
}

fn has(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn range_step(app: &mut Harness, field: &str, symbol: &str) -> Value {
    let state = app.inspect();
    let labels = state["menu_label_rects"].as_array().unwrap();
    let field = labels
        .iter()
        .find(|label| label["text"].as_str().is_some_and(|s| s.starts_with(field)))
        .unwrap();
    let center_y = |label: &Value| {
        (label["min"][1].as_f64().unwrap() + label["max"][1].as_f64().unwrap()) * 0.5
    };
    let button = labels
        .iter()
        .find(|label| label["text"] == symbol && (center_y(label) - center_y(field)).abs() < 2.0)
        .unwrap();
    app.command(json!({"command":"pointer", "phase":"click",
        "x":(button["min"][0].as_f64().unwrap()+button["max"][0].as_f64().unwrap())*0.5,
        "y":center_y(button)}))
}

#[test]
fn loop_arrangement_and_endpoint_steps_preserve_selection_cursor_and_have_separate_undo() {
    let mut app = fixture();
    app.click("Fractional clip");
    let before = app.inspect();
    let looped = menu(&mut app, "Range", "Loop playback over arrangement");
    assert_eq!(looped["project"]["loop_start"], 0.0);
    assert_eq!(looped["project"]["loop_end"], 56.0);
    assert_eq!(looped["project"]["loop_enabled"], true);
    assert_eq!(looped["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(looped["selected_clip"], before["selected_clip"]);
    assert_eq!(looped["timeline"]["cursor"], before["timeline"]["cursor"]);
    assert_eq!(looped["history"]["undo"], json!(["Loop arrangement"]));
    assert_eq!(
        menu(&mut app, "Range", "Loop playback over arrangement")["history"],
        looped["history"]
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], looped["project"]);
    let range = app.click("Range");
    assert!(has(&range, "0:30.000 · Bar 19 · Beat 3.00"));
    assert!(has(&range, "0:30.000 · 18.667 bars"));
    let step1 = range_step(&mut app, "End beat", "+");
    assert_eq!(step1["project"]["loop_end"], 56.25);
    let step2 = range_step(&mut app, "End beat", "+");
    assert_eq!(step2["project"]["loop_end"], 56.5);
    assert_eq!(step2["history"]["undo"].as_array().unwrap().len(), 3);
    app.click("Arranger");
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"]["loop_end"], 56.25);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"]["loop_end"], 56.0);
    let range = app.click("Range");
    let unchanged = range_step(&mut app, "Start beat", "−");
    assert_eq!(unchanged["project"], range["project"]);
    assert_eq!(unchanged["history"], range["history"]);
    app.quit();
}

#[test]
fn clip_and_loop_time_readouts_fit_both_appearances_and_do_not_edit_values() {
    for dark in [false, true] {
        let mut app = fixture();
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1440] {
            app.command(json!({"command":"viewport", "width":width, "height":800}));
            app.click("Fractional clip");
            let before = app.inspect();
            let opened = menu(&mut app, "Window", "Properties");
            let window = opened["window_rects"].as_array().unwrap().last().unwrap();
            // The elapsed/bar summaries follow the editable clip fields.
            // Reach them with the body's wheel scroll, leaving tabs pinned.
            app.command(json!({"command":"scroll", "delta":-10000.0,
                "x":window["max"][0].as_f64().unwrap()-2.0*f64::from(daw_ui::tokens::SPACE_LG),
                "y":window["max"][1].as_f64().unwrap()-f64::from(daw_ui::tokens::SPACE_XL)}));
            let mut props = app.inspect();
            for _ in 0..20 {
                props = app.inspect();
            }
            assert_eq!(props["window_rects"], opened["window_rects"]);
            for label in [
                "0:01.741 · Bar 2 · Beat 1.25",
                "0:05.893 · Bar 4 · Beat 3.00",
                "0:04.152 · 2.583 bars",
            ] {
                let rect = props["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|rect| rect["text"] == label)
                    .unwrap_or_else(|| panic!("Missing elapsed readout {label}"));
                assert!(rect["min"][0].as_f64().unwrap() >= window["min"][0].as_f64().unwrap());
                assert!(rect["max"][0].as_f64().unwrap() <= window["max"][0].as_f64().unwrap());
                assert!(rect["min"][1].as_f64().unwrap() >= window["min"][1].as_f64().unwrap());
                assert!(rect["max"][1].as_f64().unwrap() <= window["max"][1].as_f64().unwrap());
                assert!(
                    rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap()
                        >= f64::from(daw_ui::tokens::FONT_BODY),
                    "Clipped elapsed readout {label}: {rect}"
                );
            }
            assert_eq!(props["project"], before["project"]);
            assert_eq!(props["history"], before["history"]);
            app.click("Arranger");
            let looped = menu(&mut app, "Range", "Loop playback over arrangement");
            let indicator = &looped["timeline"]["loop_indicator"];
            let lane = &looped["timeline"]["loop_lane"];
            assert!(!indicator.is_null());
            assert!(indicator["min"][1].as_f64().unwrap() >= lane["min"][1].as_f64().unwrap());
            assert!(indicator["max"][1].as_f64().unwrap() <= lane["max"][1].as_f64().unwrap());
            assert!(
                looped["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s
                        .as_str()
                        .is_some_and(|s| s.starts_with("Loop 1:1.00–19:3.00")))
            );
            app.click("Range");
            let viewed = app.inspect();
            assert_eq!(viewed["project"], looped["project"]);
            assert_eq!(viewed["history"], looped["history"]);
            app.click("Arranger");
        }
        app.quit();
    }
}
