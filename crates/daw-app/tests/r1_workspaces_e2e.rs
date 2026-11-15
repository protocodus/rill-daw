//! Task navigation and the focused EQ inspector through the running app.
mod common;

use common::Harness;
use daw_core::{Effect, EffectSlot, Instrument, Project};
use serde_json::{Value, json};

fn open_device(app: &mut Harness, device: &str, instrument: bool, dark: bool) -> Value {
    let mut project = Project::new();
    let track = project.add_midi_track("UX track");
    let channel = project.track_mut(track).unwrap();
    if instrument {
        channel.instrument = Instrument::builtin(device);
    } else {
        channel.inserts.push(EffectSlot::new(
            "UX effect",
            Effect::Builtin {
                device: device.into(),
            },
        ));
    }
    let path = app.directory.path().join(format!("{device}.rill"));
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"system_theme","dark":dark}));
    app.command(json!({"command":"viewport","width":1480,"height":960}));
    app.click("UX track");
    app.click("Window");
    app.click(if instrument { "Instrument" } else { "Effects" });
    if !instrument {
        app.click("Open editor");
    }
    common::wait_visible(app, "Save as…");
    app.inspect()
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(state[field], before[field], "Navigation changed {field}");
    }
}

fn task_content(device: &str, task: &str) -> &'static str {
    match (device, task) {
        ("r1-mini", "Sound") => "OSCILLATORS",
        ("r1-mini", "Motion") => "LFO",
        ("r1-sampler", "Sample") | ("r1-drums", "Sample") => "REGION",
        ("r1-sampler", "Shape") => "FILTER ENVELOPE",
        ("r1-sampler", "Motion") => "LFO",
        ("r1-drums", "Shape") => "PITCH ENV",
        ("r1-filter", "Tone") => "CIRCUIT",
        ("r1-filter", "Motion") => "ENVELOPE FOLLOWER",
        ("r1-compressor", "Shape") => "DYNAMICS",
        ("r1-compressor", "Detector") => "SIDECHAIN",
        ("r1-spacer", "Space") => "Pre-delay",
        ("r1-spacer", "Texture") => "Seed",
        _ => panic!("Unknown task {device}/{task}"),
    }
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn pointer(app: &mut Harness, phase: &str, point: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":point.0,"y":point.1}))
}

fn knob_readout<'a>(state: &'a Value, caption: &Value, text: &str) -> Option<&'a Value> {
    let (x, _) = center(caption);
    let top = caption["max"][1].as_f64().unwrap();
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            let (at_x, at_y) = center(entry);
            entry["text"] == text && (at_x - x).abs() < 50.0 && at_y > top && at_y < top + 90.0
        })
}

fn assert_knob_readouts(state: &Value, caption: &Value, values: &[&str], visible: bool) {
    for value in values {
        assert_eq!(
            knob_readout(state, caption, value).is_some(),
            visible,
            "Knob readout {value:?} should be {}: {}",
            if visible { "visible" } else { "hidden" },
            state["visible_labels"],
        );
    }
}

fn visible_in_window(state: &Value, rect: &Value) {
    assert!(
        state["window_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| {
                (0..2).all(|axis| {
                    rect["min"][axis].as_f64().unwrap()
                        >= window["min"][axis].as_f64().unwrap() - 1.0
                        && rect["max"][axis].as_f64().unwrap()
                            <= window["max"][axis].as_f64().unwrap() + 1.0
                })
            }),
        "Control outside popup: {rect}, windows {}",
        state["window_rects"]
    );
    assert!(
        rect["max"][1].as_f64().unwrap() <= 960.0,
        "Below viewport: {rect}"
    );
}

#[test]
fn every_r1_workspace_is_reachable_without_changing_the_song_or_moving_output() {
    let devices: &[(&str, bool, &[&str], &str)] = &[
        ("r1-mini", true, &["Sound", "Motion"], "Volume"),
        ("r1-sampler", true, &["Sample", "Shape", "Motion"], "Volume"),
        ("r1-drums", true, &["Sample", "Shape"], "Kit volume"),
        ("r1-filter", false, &["Tone", "Motion"], "Output"),
        ("r1-equalizer", false, &[], "Output"),
        ("r1-saturator", false, &[], "Output"),
        ("r1-compressor", false, &["Shape", "Detector"], "Output"),
        ("r1-modulator", false, &[], "Output"),
        ("r1-spacer", false, &["Space", "Texture"], "Output"),
    ];
    for dark in [false, true] {
        for &(device, instrument, pages, output) in devices {
            let mut app = Harness::new();
            let before = open_device(&mut app, device, instrument, dark);
            assert_eq!(
                before["effective_theme"],
                if dark { "Dark" } else { "Light" }
            );
            let output_rect = label(&before, output).clone();
            visible_in_window(&before, &output_rect);
            let performance = before["native_editor"]["performance"].clone();
            if instrument {
                visible_in_window(&before, &performance["keyboard"]);
            }
            for page in pages.iter().rev().chain(pages.iter()) {
                let state = app.click(page);
                unchanged(&state, &before);
                visible_in_window(&state, label(&state, task_content(device, page)));
                assert_eq!(
                    label(&state, output),
                    &output_rect,
                    "{device}/{page} moved output"
                );
                visible_in_window(&state, label(&state, page));
                if instrument {
                    assert_eq!(
                        state["native_editor"]["performance"]["keyboard"], performance["keyboard"],
                        "{device}/{page} moved keyboard"
                    );
                }
            }
            app.quit();
        }
    }
}

#[test]
fn equalizer_band_inspection_does_not_enable_bands_or_edit_parameters() {
    let mut app = Harness::new();
    let before = open_device(&mut app, "r1-equalizer", false, true);
    for band in ["HP", "1", "2", "3", "4", "LP", "2"] {
        // The selector's labels are distinct from the plot labels by their
        // interactive position; prefer the final matching label below the plot.
        let state = app.inspect();
        let target = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["text"] == band)
            .max_by(|a, b| {
                a["min"][1]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&b["min"][1].as_f64().unwrap())
            })
            .unwrap_or_else(|| panic!("Missing band {band}"));
        let x = (target["min"][0].as_f64().unwrap() + target["max"][0].as_f64().unwrap()) * 0.5;
        let y = (target["min"][1].as_f64().unwrap() + target["max"][1].as_f64().unwrap()) * 0.5;
        let selected = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        unchanged(&selected, &before);
        visible_in_window(&selected, label(&selected, "Frequency"));
        let heading = match band {
            "HP" => "HIGH-PASS".into(),
            "LP" => "LOW-PASS".into(),
            _ => format!("BAND {band}"),
        };
        visible_in_window(&selected, label(&selected, &heading));
    }
    app.quit();
}

#[test]
fn a_motion_parameter_keeps_its_value_across_tasks_undo_and_reopen() {
    let mut app = Harness::new();
    open_device(&mut app, "r1-mini", true, true);
    let before = app.click("Motion");
    let rate = label(&before, "Rate");
    let x = (rate["min"][0].as_f64().unwrap() + rate["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rate["min"][1].as_f64().unwrap() + rate["max"][1].as_f64().unwrap()) * 0.5 + 29.0;
    let edited = app.command(json!({"command":"pointer","phase":"drag",
        "x":x,"y":y,"to_x":x,"to_y":y-30.0}));
    assert_ne!(
        edited["project"], before["project"],
        "Rate gesture did not edit sound"
    );
    assert_eq!(edited["history"]["undo"][0], "Instrument parameter");
    unchanged(&app.click("Sound"), &edited);
    unchanged(&app.click("Motion"), &edited);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    visible_in_window(&undone, label(&undone, "LFO"));
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], edited["project"]);
    visible_in_window(&redone, label(&redone, "LFO"));
    let path = app.directory.path().join("motion-recall.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["instrument_parameters"],
        edited["project"]["tracks"][0]["instrument_parameters"]
    );
    app.quit();
}

#[test]
fn knob_values_follow_hover_without_moving_the_panel_or_sticking_after_focus() {
    let mut app = Harness::new();
    open_device(&mut app, "r1-mini", true, true);
    let before = pointer(&mut app, "move", (2.0, 2.0));
    let caption = label(&before, "Cutoff").clone();
    let output = label(&before, "Volume").clone();
    let values = ["20", "20k", "2.00 kHz"];
    assert_knob_readouts(&before, &caption, &values, false);

    // The default Saw disables pulse width, but its stored value and range
    // must still be readable without changing the oscillator.
    let disabled = label(&before, "Width").clone();
    assert_knob_readouts(&before, &disabled, &["5", "95", "50 %"], false);
    let over_disabled = pointer(&mut app, "move", center(&disabled));
    assert_knob_readouts(&over_disabled, &disabled, &["5", "95", "50 %"], true);
    unchanged(&over_disabled, &before);

    // The caption, dial and reserved value line form one hover footprint.
    let caption_point = center(&caption);
    let dial_point = (caption_point.0, caption_point.1 + 29.0);
    let hovered = pointer(&mut app, "move", caption_point);
    assert_knob_readouts(&hovered, &caption, &values, true);
    let value_rect = knob_readout(&hovered, &caption, "2.00 kHz")
        .unwrap()
        .clone();
    for point in [dial_point, center(&value_rect), caption_point] {
        let hovered = pointer(&mut app, "move", point);
        assert_knob_readouts(&hovered, &caption, &values, true);
        unchanged(&hovered, &before);
        assert_eq!(label(&hovered, "Cutoff"), &caption);
        assert_eq!(label(&hovered, "Volume"), &output);
        assert_eq!(hovered["window_rects"], before["window_rects"]);
    }

    pointer(&mut app, "click", dial_point);
    let left = pointer(&mut app, "move", (2.0, 2.0));
    assert_knob_readouts(&left, &caption, &values, false);
    unchanged(&left, &before);

    // An owned drag stays readable after the pointer leaves the knob.
    pointer(&mut app, "down", dial_point);
    let outside = (dial_point.0, dial_point.1 - 100.0);
    let dragging = pointer(&mut app, "move", outside);
    assert_knob_readouts(&dragging, &caption, &["20", "20k"], true);
    assert_ne!(dragging["project"], before["project"]);
    let current = dragging["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["text"].as_str())
        .find(|text| text.ends_with("Hz") && knob_readout(&dragging, &caption, text).is_some())
        .expect("Active drag hid its current value")
        .to_owned();
    pointer(&mut app, "up", outside);
    let released = pointer(&mut app, "move", (2.0, 2.0));
    assert_knob_readouts(&released, &caption, &["20", "20k", &current], false);
    assert_eq!(label(&released, "Volume"), &output);
    app.quit();
}

#[test]
fn inline_knob_values_hide_on_leave_and_revealed_choice_values_remain_clickable() {
    let mut app = Harness::new();
    open_device(&mut app, "r1-modulator", false, false);
    let before = pointer(&mut app, "move", (2.0, 2.0));
    let output = label(&before, "Output").clone();
    let output_point = (output["max"][0].as_f64().unwrap() + 18.0, center(&output).1);
    let has_output_value = |state: &Value| {
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                let point = center(entry);
                entry["text"] == "+0.0 dB"
                    && point.0 > output["max"][0].as_f64().unwrap()
                    && point.0 < output["max"][0].as_f64().unwrap() + 120.0
                    && (point.1 - output_point.1).abs() < 15.0
            })
    };
    assert!(!has_output_value(&before));
    let hovered = pointer(&mut app, "move", output_point);
    assert!(
        has_output_value(&hovered),
        "Inline output value did not appear"
    );
    unchanged(&hovered, &before);
    pointer(&mut app, "click", output_point);
    let left = pointer(&mut app, "move", (2.0, 2.0));
    assert!(
        !has_output_value(&left),
        "Inline value stayed visible after focus"
    );
    unchanged(&left, &before);
    assert_eq!(label(&left, "Output"), &output);

    let sync = center(label(&left, "Sync"));
    pointer(&mut app, "click", (sync.0, sync.1 + 36.0));
    let synced = pointer(&mut app, "move", (2.0, 2.0));
    let division = label(&synced, "Division").clone();
    assert_knob_readouts(&synced, &division, &["8 bars", "1/16", "1 bar"], false);
    let hovered = pointer(&mut app, "move", center(&division));
    assert_knob_readouts(&hovered, &division, &["8 bars", "1/16", "1 bar"], true);
    let value = knob_readout(&hovered, &division, "1 bar").unwrap().clone();
    let on_value = pointer(&mut app, "move", center(&value));
    assert_knob_readouts(&on_value, &division, &["1 bar"], true);
    let menu = pointer(&mut app, "click", center(&value));
    let option = label(&menu, "1/8").clone();
    let over_option = pointer(&mut app, "move", center(&option));
    visible_in_window(&over_option, label(&over_option, "1/8"));
    let picked = pointer(&mut app, "click", center(&option));
    assert_ne!(
        picked["project"], synced["project"],
        "Choice menu did not edit Division"
    );
    let hovered = pointer(&mut app, "move", center(&division));
    assert_knob_readouts(&hovered, &division, &["1/8"], true);
    let left = pointer(&mut app, "move", (2.0, 2.0));
    assert_knob_readouts(&left, &division, &["8 bars", "1/16", "1/8"], false);
    assert_eq!(label(&left, "Output"), &output);
    app.quit();
}

#[test]
fn tool_edits_keep_the_drum_task_and_show_the_silent_pad_readout() {
    let mut app = Harness::new();
    open_device(&mut app, "r1-drums", true, false);
    let before = app.click("Shape");
    let session = &before["song_session"];
    let edited = app.command(json!({"command":"song_tool","name":"set_device_parameters",
        "arguments":{"session_id":session["id"],"expected_revision":session["revision"],
            "track_id":before["project"]["tracks"][0]["id"],
            "parameters":[{"parameter":"pad1.volume","value":"-60 dB"}]}}));
    assert_eq!(
        edited["tool_result"]["ok"], true,
        "{}",
        edited["tool_result"]
    );
    visible_in_window(&edited, label(&edited, "PITCH ENV"));
    let volume = label(&edited, "Pad volume");
    let hovered = pointer(
        &mut app,
        "move",
        (volume["max"][0].as_f64().unwrap() + 18.0, center(volume).1),
    );
    visible_in_window(&hovered, label(&hovered, "−∞ dB"));
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    visible_in_window(&undone, label(&undone, "PITCH ENV"));
    app.quit();
}
