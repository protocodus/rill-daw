//! Native parameter drafts are editable using actual mouse clicks only.
mod common;

use common::Harness;
use daw_core::{
    AutomationParameter, AutomationPoint, AutomationTarget, Clip, Instrument, PluginParameterValue,
    Project,
};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text:?}: {}", state["visible_labels"]))
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn channel<'a>(state: &'a Value, view: &str, id: u64) -> &'a Value {
    state[view]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(state[field], before[field], "Draft changed {field}");
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

fn right_click(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "button":"secondary", "x":x, "y":y}))
}

fn toggle_keypad(app: &mut Harness, state: &Value) -> Value {
    let heading = label(state, "Value");
    let left = heading["min"][0].as_f64().unwrap();
    let top = heading["min"][1].as_f64().unwrap();
    let toggle = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|row| {
            let (x, y) = center(row);
            row["text"] == "123" && x >= left && x <= left + 240.0 && y > top && y < top + 130.0
        })
        .unwrap_or_else(|| {
            panic!(
                "Missing exact-entry keypad toggle: {}",
                state["visible_labels"]
            )
        });
    let (x, y) = center(toggle);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn open_control(app: &mut Harness, view: &str, id: u64, name: &str) -> Value {
    let before = app.inspect();
    let opened = right_click(app, &channel(&before, view, id)[name]);
    unchanged(&opened, &before);
    let opened = toggle_keypad(app, &opened);
    unchanged(&opened, &before);
    for text in [
        "7",
        "0",
        ".",
        "±",
        "Clear",
        "Backspace",
        "Apply value",
        "Cancel",
    ] {
        label(&opened, text);
    }
    opened
}

fn enter_mouse(app: &mut Harness, before: &Value, number: &str) -> Value {
    unchanged(&app.click("Clear"), before);
    let mut state = app.inspect();
    for digit in number.trim_start_matches('-').chars() {
        state = app.click(&digit.to_string());
        unchanged(&state, before);
    }
    if number.starts_with('-') {
        state = app.click("±");
        unchanged(&state, before);
    }
    state
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn one_edit(state: &Value, before: &Value) {
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
}

fn fixture(app: &mut Harness, compact: bool, automated: bool) -> (u64, u64, Option<u64>) {
    let mut project = Project::new();
    let source = project.add_audio_track("Mouse channel");
    project.track_mut(source).unwrap().gain = 0.75;
    project.track_mut(source).unwrap().pan = 0.173456;
    project.track_mut(source).unwrap().collapsed = compact;
    project.add_audio_track("Other channel");
    let bus = project.add_bus("Room");
    project.add_send(source, bus, 0.25, false).unwrap();
    let lane = automated.then(|| {
        let lane = project
            .add_automation_track_with_value(
                "Pan movement",
                AutomationTarget {
                    track: source,
                    parameter: AutomationParameter::TrackPan,
                },
                0.25,
            )
            .unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(8.0, 0.25),
            AutomationPoint::new(12.0, 0.75),
        ];
        lane
    });
    let path = app.directory.path().join("mouse-parameters.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    (source, bus, lane)
}

fn send_value_rect(state: &Value) -> &Value {
    let (_, level_y) = center(label(state, "Level"));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            row["text"]
                .as_str()
                .and_then(|text| text.trim_end_matches('×').trim().parse::<f64>().ok())
                .is_some()
                && (center(row).1 - level_y).abs() < 4.0
        })
        .max_by(|left, right| center(left).0.total_cmp(&center(right).0))
        .unwrap_or_else(|| panic!("Missing send level: {}", state["visible_labels"]))
}

#[test]
fn mixer_gain_pan_and_send_accept_precise_mouse_only_drafts() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (source, bus, _) = fixture(&mut app, false, false);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":760, "pixels_per_point":density}),
        );
        let before = app.click("Mixer");
        let opened = open_control(&mut app, "mixer", source, "gain");
        assert!(
            opened["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| {
                    label.as_str().is_some_and(|text| {
                        text.starts_with("Range: -60 to ") && text.ends_with(" dB")
                    })
                })
        );
        enter_mouse(&mut app, &before, "-12.5");
        let gained = app.click("Apply value");
        near(
            track(&gained, source)["gain"].as_f64().unwrap(),
            10_f64.powf(-12.5 / 20.0),
        );
        one_edit(&gained, &before);
        assert_eq!(undo(&mut app)["project"], before["project"]);

        let before = app.inspect();
        let opened = open_control(&mut app, "mixer", source, "pan");
        assert!(visible(&opened, "Range: -100 to 100 %"));
        enter_mouse(&mut app, &before, "23.4567");
        let panned = app.click("Apply value");
        near(track(&panned, source)["pan"].as_f64().unwrap(), 0.234567);
        one_edit(&panned, &before);
        assert_eq!(undo(&mut app)["project"], before["project"]);

        app.click("Mouse channel");
        let before = app.click("Routing");
        let opened = right_click(&mut app, send_value_rect(&before));
        assert!(visible(&opened, "Range: 0 to 16 ×"));
        toggle_keypad(&mut app, &opened);
        enter_mouse(&mut app, &before, "0.375");
        let sent = app.click("Apply value");
        let send = track(&sent, source)["sends"]
            .as_array()
            .unwrap()
            .iter()
            .find(|send| send["target_bus"] == bus)
            .unwrap();
        near(send["gain"].as_f64().unwrap(), 0.375);
        one_edit(&sent, &before);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn compact_header_and_channel_properties_keep_the_same_mouse_entry() {
    let mut app = Harness::new();
    let (source, _, _) = fixture(&mut app, true, false);
    app.command(json!({"command":"viewport", "width":1100, "height":720}));
    app.click("Other channel");
    let before = app.inspect();
    open_control(&mut app, "timeline", source, "pan");
    enter_mouse(&mut app, &before, "-12.345");
    let panned = app.click("Apply value");
    near(track(&panned, source)["pan"].as_f64().unwrap(), -0.12345);
    assert_eq!(panned["selected_track"], before["selected_track"]);
    assert_eq!(track(&panned, source)["collapsed"], true);
    one_edit(&panned, &before);
    assert_eq!(undo(&mut app)["project"], before["project"]);

    app.click("Mixer");
    let before = app.click("Mouse channel");
    assert_eq!(before["popups"]["properties"], true);
    let (_, gain_y) = center(label(&before, "Gain"));
    let gain = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|row| {
            row["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" dB"))
                && (center(row).1 - gain_y).abs() < 12.0
        })
        .unwrap();
    let opened = right_click(&mut app, gain);
    toggle_keypad(&mut app, &opened);
    enter_mouse(&mut app, &before, "-9.875");
    let gained = app.click("Apply value");
    near(
        track(&gained, source)["gain"].as_f64().unwrap(),
        10_f64.powf(-9.875 / 20.0),
    );
    one_edit(&gained, &before);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}

#[test]
fn automated_pan_mouse_drafts_reject_invalid_values_and_preserve_the_curve() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let (source, _, lane) = fixture(&mut app, true, true);
        let lane = lane.unwrap();
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        let before = app.inspect();
        let curve = track(&before, lane)["automation"]["points"].clone();
        let opened = open_control(&mut app, "timeline", source, "pan");
        for text in ["Clear", "Apply value", "Cancel", "Override automation"] {
            let rect = label(&opened, text);
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
            assert!(rect["max"][0].as_f64().unwrap() <= 800.0);
            assert!(rect["min"][1].as_f64().unwrap() >= 0.0);
            assert!(rect["max"][1].as_f64().unwrap() <= 600.0);
        }
        unchanged(&app.click("Apply value"), &before);
        open_control(&mut app, "timeline", source, "pan");
        for number in ["101", "12..5"] {
            enter_mouse(&mut app, &before, number);
            let rejected = app.click("Apply value");
            unchanged(&rejected, &before);
            label(&rejected, "Clear");
        }
        enter_mouse(&mut app, &before, "25.125");
        unchanged(&app.click("Cancel"), &before);
        open_control(&mut app, "timeline", source, "pan");
        enter_mouse(&mut app, &before, "25.125");
        let outside = app.command(json!({"command":"pointer", "phase":"click", "x":790, "y":590}));
        unchanged(&outside, &before);
        open_control(&mut app, "timeline", source, "pan");
        let drafted = enter_mouse(&mut app, &before, "25.125");
        let hidden = toggle_keypad(&mut app, &drafted);
        unchanged(&hidden, &before);
        assert!(!visible(&hidden, "Clear"));
        assert!(visible(&hidden, "25.125"));
        let reopened = toggle_keypad(&mut app, &hidden);
        unchanged(&reopened, &before);
        assert!(visible(&reopened, "25.125"));
        let applied = app.click("Apply value");
        near(track(&applied, source)["pan"].as_f64().unwrap(), 0.25125);
        near(
            track(&applied, lane)["automation"]["override_value"]
                .as_f64()
                .unwrap(),
            0.625625,
        );
        assert_eq!(track(&applied, lane)["automation"]["points"], curve);
        one_edit(&applied, &before);
        open_control(&mut app, "timeline", source, "pan");
        let resumed = app.click("Resume automation");
        assert!(track(&resumed, lane)["automation"]["override_value"].is_null());
        assert_eq!(track(&resumed, lane)["automation"]["points"], curve);
        assert_eq!(undo(&mut app)["project"], applied["project"]);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn plugin_normalized_parameter_accepts_mouse_digits_without_claiming_physical_units() {
    // Reuse the production metadata/state IPC fixture. This test exercises
    // host parameter controls, not native plug-in DSP or its custom editor.
    let worker = tempfile::tempdir().unwrap();
    let source = worker.path().join("parameter_host.rs");
    std::fs::write(&source, include_str!("common/parameter_host.rs")).unwrap();
    let host = worker.path().join(if cfg!(windows) {
        "parameter-host.exe"
    } else {
        "parameter-host"
    });
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = std::process::Command::new(compiler)
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&host)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "metadata fixture: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let plugins = common::plugin_fixture();
    let mut app = Harness::with_plugin_host(plugins.path(), &host);
    let mut project = Project::new();
    let source = project.add_midi_track("Mouse synth");
    project
        .set_instrument(
            source,
            Instrument::Vst3 {
                path: plugins
                    .path()
                    .join("RillFixture.vst3")
                    .to_string_lossy()
                    .into_owned(),
                class_id: Some("11111111111111111111111111111111".into()),
            },
        )
        .unwrap();
    project.track_mut(source).unwrap().instrument_parameters =
        vec![PluginParameterValue { id: 7, value: 0.73 }];
    project
        .track_mut(source)
        .unwrap()
        .clips
        .push(Clip::midi("Mouse phrase", 0.0, 4.0, vec![]));
    let path = app.directory.path().join("mouse-plugin.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":760}));
    app.click("Mouse phrase");
    app.click("Window");
    app.click("Instrument");
    app.click("Automate…");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let before = loop {
        let state = app.inspect();
        assert!(state["native_editor"]["error"].is_null());
        if state["native_editor"]["parameter_count"].as_u64().unwrap() >= 200 {
            break state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "metadata did not arrive"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(before["native_editor"]["selected"], 7);
    assert!(visible(&before, "Plug-in unit · Hz"));
    let dial = &before["native_editor"]["parameter_control"]["dial"];
    let opened = right_click(&mut app, dial);
    assert!(visible(&opened, "Range: 0 to 100 %"));
    toggle_keypad(&mut app, &opened);
    enter_mouse(&mut app, &before, "61.23456789");
    let applied = app.click("Apply value");
    let actual = applied["native_editor"]["selected_value"].as_f64().unwrap();
    assert!((actual - 0.6123456789).abs() < 1e-12, "{actual}");
    one_edit(&applied, &before);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}
