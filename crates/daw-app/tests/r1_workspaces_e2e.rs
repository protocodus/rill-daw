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

fn task_content(device: &str, task: &str) -> &'static [&'static str] {
    match (device, task) {
        ("r1-mini", "Oscillators") => &["OSCILLATOR 1", "OSCILLATOR 2", "SOURCE MIX"],
        ("r1-mini", "Voices") => &["VOICE ALLOCATION", "PITCH RESPONSE"],
        ("r1-mini" | "r1-sampler", "Modulation") => &["LFO", "MODULATION DEPTH"],
        ("r1-mini" | "r1-sampler", "Filter & amp") => {
            &["FILTER", "FILTER ENVELOPE", "AMP ENVELOPE"]
        }
        ("r1-sampler", "Sample") => &["REGION", "LOOP"],
        ("r1-sampler", "Pitch & voices") => &["PITCH", "VOICES"],
        ("r1-spacer", "Echo") => &["Routing", "Echo on", "Reverb on", "Taps"],
        ("r1-spacer", "Reverb") => &["Routing", "Echo on", "Reverb on", "Seed"],
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

fn assert_tab_titles(state: &Value, pages: &[&str], page_controls: &[&str]) {
    let labels = state["label_rects"].as_array().unwrap();
    // Start at the device header to inspect its own paint, above the arranger.
    let header_end = labels
        .iter()
        .rposition(|entry| entry["text"] == "Randomize")
        .unwrap();
    let device_labels = &labels[header_end + 1..];
    let find = |title: &str| {
        device_labels
            .iter()
            .find(|entry| entry["text"] == title)
            .unwrap_or_else(|| panic!("Missing device label {title}"))
    };
    let first = find(pages[0]);
    let tab_y = center(first).1;
    let row: Vec<_> = device_labels
        .iter()
        .filter(|entry| {
            let top = entry["min"][1].as_f64().unwrap();
            let bottom = entry["max"][1].as_f64().unwrap();
            (top..=bottom).contains(&tab_y)
        })
        .map(|entry| entry["text"].as_str().unwrap())
        .collect();
    assert_eq!(row, pages, "Only page titles belong on the tab strip");
    for &caption in page_controls {
        let control = find(caption);
        visible_in_window(state, control);
        assert!(
            control["min"][1].as_f64().unwrap() > first["max"][1].as_f64().unwrap(),
            "{caption} belongs below the tab strip"
        );
    }
}

fn assert_tab_paint(state: &Value, pages: &[&str], selected: &str) {
    for &page in pages {
        let title = label(state, page);
        assert!(
            state["painted_text"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text["text"] == page
                    && text["rect"]["min"] == title["min"]
                    && text["rect"]["max"] == title["max"]
                    && text["colors"]
                        .as_array()
                        .is_some_and(|colors| colors.iter().any(|ink| ink[3] == 204))),
            "{page} title at 80% opacity"
        );
        let (title_x, title_y) = center(title);
        let behind_title = |shape: &Value| {
            let rect = &shape["rect"];
            let (x, y) = center(rect);
            (x - title_x).abs() <= 1.0
                && (y - title_y).abs() <= f64::from(daw_ui::tokens::SELECTION_GLOW_EXTENT)
                && rect["min"][0].as_f64().unwrap() < title["min"][0].as_f64().unwrap()
                && rect["min"][1].as_f64().unwrap() < title["min"][1].as_f64().unwrap()
        };
        let has_expected_fill = state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|shape| behind_title(shape))
            .any(|shape| {
                if page == selected {
                    shape["fill"] == state["theme_colors"]["panel"]
                } else {
                    shape["fill"][3] == (255.0_f32 * 0.245).round() as u8
                }
            });
        let halo = state["painted_meshes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mesh| {
                behind_title(mesh)
                    && mesh["transparent_vertices"].as_u64().unwrap() > 0
                    && mesh["max_alpha"].as_u64().unwrap() > 0
            });
        assert!(
            has_expected_fill,
            "{page} face color or opacity, selected {selected}"
        );
        assert_eq!(halo, page == selected, "{page} glow, selected {selected}");
    }
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
            // Below the caption, within the height of a row of controls.
            entry["text"] == text && (at_x - x).abs() < 50.0 && at_y > top && at_y < top + 110.0
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
        (
            "r1-mini",
            true,
            &["Oscillators", "Filter & amp", "Modulation", "Voices"],
            "Volume",
        ),
        (
            "r1-sampler",
            true,
            &["Sample", "Pitch & voices", "Filter & amp", "Modulation"],
            "Volume",
        ),
        ("r1-drums", true, &[], "Kit volume"),
        ("r1-filter", false, &[], "Output"),
        ("r1-equalizer", false, &[], "Output"),
        ("r1-saturator", false, &[], "Output"),
        ("r1-compressor", false, &[], "Output"),
        ("r1-modulator", false, &[], "Output"),
        ("r1-spacer", false, &["Echo", "Reverb"], "Output"),
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
                assert_tab_titles(&state, pages, task_content(device, page));
                assert_eq!(
                    label(&state, output),
                    &output_rect,
                    "{device}/{page} moved output"
                );
                visible_in_window(&state, label(&state, page));
                if device == "r1-spacer" {
                    let painted = app.command(json!({"command":"inspect","paint":true}));
                    assert_tab_paint(&painted, pages, page);
                }
                if instrument {
                    assert_eq!(
                        state["native_editor"]["performance"]["keyboard"], performance["keyboard"],
                        "{device}/{page} moved keyboard"
                    );
                }
            }
            if instrument && dark && !pages.is_empty() {
                app.command(json!({"command":"viewport","width":800,"height":960}));
                let compact = app.inspect();
                let level = label(&compact, output);
                let keyboard = &compact["native_editor"]["performance"]["keyboard"];
                for page in pages {
                    let state = app.click(page);
                    assert_tab_titles(&state, pages, task_content(device, page));
                    assert_eq!(
                        label(&state, output),
                        level,
                        "{device}/{page} compact level"
                    );
                    assert_eq!(
                        &state["native_editor"]["performance"]["keyboard"], keyboard,
                        "{device}/{page} compact keyboard"
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
    let before = app.click("Modulation");
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
    unchanged(&app.click("Oscillators"), &edited);
    unchanged(&app.click("Modulation"), &edited);
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

    // The default Saw disables pulse width, but its stored value and range
    // must still be readable without changing the oscillator.
    let disabled = label(&before, "Width").clone();
    assert_knob_readouts(&before, &disabled, &["5", "95", "50 %"], false);
    let over_disabled = pointer(&mut app, "move", center(&disabled));
    assert_knob_readouts(&over_disabled, &disabled, &["5", "95", "50 %"], true);
    unchanged(&over_disabled, &before);

    app.click("Filter & amp");
    let before = pointer(&mut app, "move", (2.0, 2.0));
    let caption = label(&before, "Cutoff").clone();
    let output = label(&before, "Volume").clone();
    let values = ["20", "20k", "2.00 kHz"];
    assert_knob_readouts(&before, &caption, &values, false);

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

    // The switch sits on the row's center line, as Feel's middle option does.
    let sync = (
        center(label(&left, "Sync")).0,
        center(label(&left, "Dotted")).1,
    );
    pointer(&mut app, "click", sync);
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
fn the_selected_drum_pad_shows_its_entire_signal_path_and_silent_level() {
    let mut app = Harness::new();
    let before = open_device(&mut app, "r1-drums", true, false);
    for stage in [
        "TRIGGER",
        "REGION",
        "TUNING",
        "PITCH ENV",
        "AMP ENVELOPE",
        "TONE",
    ] {
        visible_in_window(&before, label(&before, stage));
    }
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

#[test]
fn filter_circuit_and_both_cutoff_sources_share_one_editable_panel() {
    use daw_devices::devices::filter::{ENV_AMOUNT, LFO_DEPTH};
    let mut app = Harness::new();
    let before = open_device(&mut app, "r1-filter", false, false);
    for (left, right) in [("CIRCUIT", "FILTER"), ("LFO", "ENVELOPE FOLLOWER")] {
        let a = center(label(&before, left));
        let b = center(label(&before, right));
        assert!(a.0 < b.0 && (a.1 - b.1).abs() < 1.0, "{left}, {right}");
    }
    let circuit_y = center(label(&before, "CIRCUIT")).1;
    let lfo_y = center(label(&before, "LFO")).1;
    assert!(circuit_y < lfo_y && lfo_y < center(label(&before, "Mix")).1);

    for (caption, parameter) in [("Depth", LFO_DEPTH), ("Amount", ENV_AMOUNT)] {
        let control = label(&before, caption);
        visible_in_window(&before, control);
        let (x, y) = center(control);
        let y = y + 12.0 + f64::from(daw_ui::KnobSize::Standard.diameter()) * 0.5;
        let edited = app.command(json!({"command":"pointer","phase":"drag",
            "x":x,"y":y,"to_x":x,"to_y":y-36.0}));
        let parameters = edited["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap();
        assert!(
            parameters
                .iter()
                .any(|p| p["id"] == parameter && p["value"].as_f64().unwrap() > 0.5),
            "{caption} drag missed: {parameters:?}"
        );
        assert_eq!(edited["history"]["undo"][0], "Effect parameter");
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert_eq!(undone["project"], before["project"]);
    }
    app.command(json!({"command":"viewport","width":800,"height":960}));
    let compact = app.inspect();
    for (kind, slope) in [("High-pass", "12 dB"), ("Notch", "24 dB")] {
        let kind = label(&compact, kind);
        let slope = label(&compact, slope);
        assert!(kind["max"][0].as_f64().unwrap() < slope["min"][0].as_f64().unwrap());
        assert!((center(kind).1 - center(slope).1).abs() < 1.0);
    }
    let high_pass = center(label(&compact, "High-pass"));
    let resonance = label(&compact, "Resonance");
    let drive = label(&compact, "Drive");
    assert_eq!(resonance["max"][1], drive["max"][1], "Resonance wrapped");
    app.click("Clean");
    let fixed = app.click("Transistor Ladder");
    // The popup can scroll when the circuit chooser takes focus.
    let (old_left, old_top) = center(label(&compact, "CIRCUIT"));
    let (new_left, new_top) = center(label(&fixed, "CIRCUIT"));
    let x_shift = new_left - old_left;
    let y_shift = new_top - old_top;
    for caption in ["Type", "Slope", "Depth", "Mix"] {
        let (x, y) = center(label(&compact, caption));
        let (new_x, new_y) = center(label(&fixed, caption));
        assert!(
            (new_x - x - x_shift).abs() <= 1.0 && (new_y - y - y_shift).abs() <= 1.0,
            "{caption} moved within the panel"
        );
    }
    let inactive = (high_pass.0 + x_shift, high_pass.1 + y_shift);
    unchanged(&pointer(&mut app, "click", inactive), &fixed);
    app.quit();
}

#[test]
fn equalizer_band_and_analyzer_choices_share_a_row_above_the_fixed_frequency_dial() {
    let mut app = Harness::new();
    let before = open_device(&mut app, "r1-equalizer", false, true);
    let band = center(label(&before, "Band"));
    for caption in ["Mode", "Analyzer"] {
        let choice = center(label(&before, caption));
        assert!(
            band.0 < choice.0 && (band.1 - choice.1).abs() < 1.0,
            "{caption}"
        );
    }
    let frequency = label(&before, "Frequency");
    assert!(center(frequency).1 > band.1);
    for (band, shape) in [("HP", "Slope"), ("2", "Type"), ("LP", "Slope")] {
        let selected = common::click_last_label(&mut app, band);
        unchanged(&selected, &before);
        assert_eq!(label(&selected, "Frequency"), frequency);
        assert!(center(label(&selected, shape)).0 < center(frequency).0);
    }
    app.quit();
}

#[test]
fn narrow_equalizer_wraps_its_global_choices_and_keeps_the_last_controls_reachable() {
    use daw_devices::devices::equalizer::ANALYZER;
    let mut app = Harness::new();
    open_device(&mut app, "r1-equalizer", false, false);
    app.command(json!({"command":"viewport","width":800,"height":960}));
    let before = app.inspect();
    let band_y = center(label(&before, "Band")).1;
    let mode_y = center(label(&before, "Mode")).1;
    let analyzer_y = center(label(&before, "Analyzer")).1;
    assert!(band_y < mode_y && (mode_y - analyzer_y).abs() < 1.0);
    visible_in_window(&before, label(&before, "Q"));
    let analyzer = |state: &Value| {
        state["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == ANALYZER)
            .and_then(|parameter| parameter["value"].as_f64())
    };
    let changed = app.click("Input");
    assert_eq!(analyzer(&changed), Some(0.5));
    // Output also labels the trim at the bottom of the device.
    let source = changed["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == "Output" && (center(entry).1 - analyzer_y).abs() < 1.0)
        .expect("rightmost analyzer choice");
    let restored = pointer(&mut app, "click", center(source));
    assert_eq!(analyzer(&restored), Some(1.0));
    app.quit();
}
