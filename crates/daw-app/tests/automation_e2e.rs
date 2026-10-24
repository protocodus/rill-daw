//! Edit automation through the actual application, then inspect persisted and
//! rendered results. Pointer positions come from the production curve component.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn menu(app: &mut Harness, menu: &str, label: &str) -> Value {
    app.click(menu);
    if menu == "+ Insert" {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Search effects")
        {
            app.click("Search effects");
            app.command(json!({"command":"text","text":"Rill Fixture"}));
        }
    }
    for _ in 0..8 {
        let state = app.inspect();
        let count = state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| *s == label)
            .count();
        if count > 0 {
            return app.command(json!({"command":"click","label":label,"occurrence":count-1}));
        }
        let last = state["label_rects"]
            .as_array()
            .unwrap()
            .last()
            .expect("open menu items");
        let x = (last["min"][0].as_f64().unwrap() + last["max"][0].as_f64().unwrap()) * 0.5;
        let y = (last["min"][1].as_f64().unwrap() + last["max"][1].as_f64().unwrap()) * 0.5;
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":-120.0}));
    }
    panic!("Missing {label} after scrolling: {}", app.inspect());
}

fn position(state: &Value, beat: f64, value: f64) -> (f64, f64) {
    let rect = &state["automation_plot_rect"];
    let left = rect["min"][0].as_f64().expect("curve must be visible");
    let top = rect["min"][1].as_f64().unwrap();
    let right = rect["max"][0].as_f64().unwrap();
    let bottom = rect["max"][1].as_f64().unwrap();
    (
        left + beat / 8.0 * (right - left),
        bottom - value * (bottom - top),
    )
}

fn add(app: &mut Harness, beat: f64, value: f64) -> Value {
    let (x, y) = position(&app.inspect(), beat, value);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn drag(app: &mut Harness, from: (f64, f64), to: (f64, f64)) -> Value {
    let state = app.inspect();
    let (x, y) = position(&state, from.0, from.1);
    let (to_x, to_y) = position(&state, to.0, to.1);
    app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":to_x,"to_y":to_y}))
}

#[test]
fn automation_points_interpolation_undo_and_session_recovery() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Track", "Add automation track…");
    assert_eq!(state["popups"]["automation"], true);
    let state = app.click("Add automation track");
    assert_eq!(state["project"]["tracks"][1]["kind"], "Automation");
    let state = add(&mut app, 2.0, 0.2);
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["points"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let state = drag(&mut app, (2.0, 0.2), (3.0, 0.8));
    let points = &state["project"]["tracks"][1]["automation"]["points"];
    assert_eq!(points[1]["beat"], 3.0);
    assert!((points[1]["value"].as_f64().unwrap() - 0.8).abs() < 0.001);
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["points"][1]["beat"],
        2.0
    );
    let state = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["points"][1]["beat"],
        3.0
    );
    for mode in ["Step", "Linear", "Smooth"] {
        let state = app.click(mode);
        assert_eq!(
            state["project"]["tracks"][1]["automation"]["interpolation"],
            mode
        );
    }
    let state = app.click("Delete point");
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    menu(&mut app, "Edit", "Undo");
    let state = app.key("Escape");
    assert_eq!(state["popups"]["automation"], false);
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    project.validate().unwrap();
    let saved = app.directory.path().join("scratch/recovery.daw.json");
    let response = app.raw_command(json!({"command":"quit"}));
    assert_eq!(response["ok"], true);
    assert_eq!(Project::load(saved).unwrap(), project);
    app.wait_exit();
}

#[test]
fn automated_volume_changes_export_and_bypass_restores_manual_level() {
    let mut app = Harness::new();
    let source = app.directory.path().join("tone.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..192_000)
            .map(|sample| 0.2 * (std::f32::consts::TAU * 220.0 * sample as f32 / 48_000.0).sin())
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source]}));
    menu(&mut app, "Track", "Add automation track…");
    let state = app.click("Add automation track");
    let initial = state["project"]["tracks"][1]["automation"]["points"][0]["value"]
        .as_f64()
        .unwrap();
    drag(&mut app, (0.0, initial), (0.0, 0.0));
    drag(&mut app, (4.0, initial), (4.0, 0.0));
    add(&mut app, 2.0, 1.0);
    app.click("Smooth");
    let smooth = export(&mut app, "smooth.wav");
    assert!(rms(&smooth, 0.9, 1.1) > rms(&smooth, 0.05, 0.15) * 5.0);
    assert!(rms(&smooth, 2.5, 3.0) < 1e-6, "endpoint must hold zero");
    app.click("Step");
    let step = export(&mut app, "step.wav");
    assert!(
        rms(&step, 0.2, 0.8) < 1e-6,
        "step must hold the first zero point"
    );
    assert!(rms(&step, 1.2, 1.8) > 0.05);
    app.click("Enabled");
    let manual = export(&mut app, "manual.wav");
    assert!(
        rms(&manual, 2.5, 3.0) > 0.03,
        "bypassed automation restores track level"
    );
    assert_eq!(smooth.samples.len(), manual.samples.len());
    app.quit();
}

#[test]
fn parameter_picker_creates_independent_lanes_and_removal_prunes_only_affected_targets() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    app.key("Escape");
    menu(&mut app, "Track", "Add MIDI track");
    menu(&mut app, "Track", "Add bus");
    app.click("New instrument");
    menu(&mut app, "Window", "Effects");
    menu(&mut app, "+ Insert", "Rill Fixture FX");
    app.click("Add to effect chain");
    menu(&mut app, "Window", "Routing");
    menu(&mut app, "+ Send", "New bus");
    let state = app.inspect();
    let target = state["project"]["tracks"][0]["id"].clone();
    let effect = state["project"]["tracks"][0]["inserts"][0]["id"].clone();
    let bus = state["project"]["tracks"][1]["id"].clone();
    let choices = [
        ("Volume", json!("TrackGain")),
        ("Pan", json!("TrackPan")),
        ("Send · New bus", json!({"SendGain":{"bus":bus}})),
        (
            "01 · Rill Fixture FX · Parameter",
            json!({"VstParameter":{"effect":effect,"id":0}}),
        ),
    ];
    for (index, (label, parameter)) in choices.into_iter().enumerate() {
        if index == 0 {
            menu(&mut app, "Track", "Add automation track…");
        } else {
            app.click("Add parameter");
        }
        if label != "Volume" {
            menu(&mut app, "Volume", label);
        }
        if index == 3 {
            menu(&mut app, "Manual ID", "Use manual ID");
            app.click("Manual ID");
        }
        let state = app.click("Add automation track");
        assert_eq!(
            state["project"]["tracks"].as_array().unwrap().len(),
            index + 3,
            "Creating {label}: status={}, labels={}",
            state["status"],
            state["visible_labels"]
        );
        let lane = &state["project"]["tracks"][index + 2]["automation"];
        assert_eq!(lane["target"]["track"], target);
        assert_eq!(lane["target"]["parameter"], parameter);
    }
    let before = app.inspect()["project"].clone();
    // Escape restores Routing, the caller of the parameter picker. Return to
    // the arranger explicitly before selecting a track on its canvas.
    menu(&mut app, "Window", "Arrangement");
    app.click("New instrument");
    menu(&mut app, "Window", "Effects");
    let state = app.click("×");
    assert!(
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 5);
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"], before);
    menu(&mut app, "Window", "Routing");
    let state = app.click("×");
    assert!(
        state["project"]["tracks"][0]["sends"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 5);
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"], before);
    let state = menu(&mut app, "Window", "Mixer");
    assert_eq!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|label| *label == "Mute")
            .count(),
        2
    );
    app.quit();
}

/// Opt-in: use an installed instrument in a default VST scan root. No plug-in
/// binaries are downloaded or required by the ordinary CI scenarios.
#[test]
#[ignore = "requires an installed instrument via RILL_TEST_VST_INSTRUMENT"]
fn installed_vst_parameter_discovery_creates_named_lane_with_current_value() {
    let path = std::env::var_os("RILL_TEST_VST_INSTRUMENT")
        .expect("Set RILL_TEST_VST_INSTRUMENT to an installed VST3 instrument path");
    let path = std::path::PathBuf::from(path);
    let plugin = daw_plugins::VstInstrument::load(&path, 48_000.0, 256).unwrap();
    let parameters = plugin.parameters().unwrap();
    let first = parameters
        .iter()
        .find(|p| !p.read_only)
        .expect("writable VST parameter");
    let selected = parameters
        .iter()
        .find(|p| !p.read_only && p.name != first.name)
        .expect("second named writable VST parameter")
        .clone();
    let first_name = first.name.clone();
    drop(plugin);
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    menu(&mut app, "Window", "Library");
    app.click("Search instruments");
    let name = path.file_stem().unwrap().to_string_lossy();
    app.command(json!({"command":"text","text":name.chars().take(4).collect::<String>()}));
    wait_label(&mut app, &name);
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == name.as_ref())
        .count();
    app.command(json!({"command":"click","label":name,"occurrence":count-1}));
    app.click("More");
    let state = app.click("Assign to selected track");
    assert_eq!(
        std::path::Path::new(
            state["project"]["tracks"][0]["instrument"]["Vst3"]["path"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap(),
        path.canonicalize().unwrap()
    );
    menu(&mut app, "Track", "Add automation track…");
    menu(&mut app, "Volume", "VST instrument parameter");
    wait_label(&mut app, &first_name);
    menu(&mut app, &first_name, &selected.name);
    let state = app.click("Add automation track");
    assert_eq!(state["error"], false, "{state}");
    let track = &state["project"]["tracks"][1];
    assert_eq!(track["name"], format!("New instrument · {}", selected.name));
    assert_eq!(
        track["automation"]["target"]["parameter"]["VstParameter"]["id"],
        selected.id
    );
    for point in track["automation"]["points"].as_array().unwrap() {
        assert!((point["value"].as_f64().unwrap() - selected.value).abs() < 1e-5);
    }
    app.key("Escape");
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"][1], *track);
    app.quit();
}

fn wait_label(app: &mut Harness, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == label)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for {label}: {state}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["error"], false, "{state}");
    AudioData::load_wav(path).unwrap()
}

fn rms(audio: &AudioData, start: f64, end: f64) -> f64 {
    let first = (start * audio.sample_rate as f64) as usize * audio.channels;
    let last = (end * audio.sample_rate as f64) as usize * audio.channels;
    (audio.samples[first..last]
        .iter()
        .map(|s| f64::from(*s).powi(2))
        .sum::<f64>()
        / (last - first) as f64)
        .sqrt()
}
