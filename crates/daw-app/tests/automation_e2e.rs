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
    let bottom = rect["max"][1].as_f64().unwrap();
    let view = &state["automation_editor"];
    (
        left + (beat - view["start_beat"].as_f64().unwrap())
            * view["pixels_per_beat"].as_f64().unwrap(),
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

fn bounded_actions(state: &Value, names: &[&str]) {
    for name in names {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == *name)
            .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]));
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
        assert!(rect["max"][0].as_f64().unwrap() <= 1100.0);
        assert!(rect["max"][1].as_f64().unwrap() <= 720.0);
        assert!(rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap() >= 13.0);
    }
}

#[test]
fn automation_navigation_fit_and_musical_beat_editing_share_editor_controls() {
    use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget};
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track(
        "Long source name 🎹 with enough detail to exercise the aligned bounded selector",
    );
    let lane_name = "Long automation lane 🎹 with enough detail to remain readable without pushing its editing controls off screen";
    let lane = project
        .add_automation_track(
            lane_name,
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![
        AutomationPoint::new(32.0, 0.25),
        AutomationPoint::new(40.0, 0.5),
        AutomationPoint::new(64.0, 0.75),
    ];
    let path = app.directory.path().join("navigation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click(lane_name);
    menu(&mut app, "Window", "Automation");
    let baseline = app.inspect();
    let fit_contains = |state: &Value| {
        let view = &state["automation_editor"];
        assert!(view["start_beat"].as_f64().unwrap() <= 32.0);
        assert!(view["start_beat"].as_f64().unwrap() > 30.0);
        assert!(view["end_beat"].as_f64().unwrap() >= 64.0);
        assert!(view["end_beat"].as_f64().unwrap() < 66.0);
    };
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(
            json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let fitted = app.click("Fit");
        fit_contains(&fitted);
        bounded_actions(
            &fitted,
            &[
                "Step",
                "Linear",
                "Smooth",
                "Enabled",
                "Fit",
                "Add parameter",
                "Delete point",
            ],
        );
        for (button, alt, ctrl) in [
            ("middle", false, false),
            ("primary", true, false),
            ("secondary", false, true),
        ] {
            let fitted = app.click("Fit");
            let (x, y) = position(&fitted, 40.0, 0.5);
            let moved = app.command(
                json!({"command":"pointer","phase":"drag","button":button,"alt":alt,"ctrl":ctrl,
                "x":x,"y":y,"to_x":x + if ctrl { 90.0 } else { -90.0 },"to_y":y-25.0}),
            );
            assert_ne!(
                moved["automation_editor"]["start_beat"],
                fitted["automation_editor"]["start_beat"]
            );
            assert_eq!(
                moved["project"], baseline["project"],
                "Navigation starting on a point must not edit it"
            );
            assert_eq!(moved["history"], baseline["history"]);
        }
        let fitted = app.click("Fit");
        let (x, y) = position(&fitted, 40.0, 0.5);
        let zoomed = app.command(json!({"command":"scroll","x":x,"y":y,"delta":120.0,"ctrl":true}));
        assert!(
            zoomed["automation_editor"]["pixels_per_beat"]
                .as_f64()
                .unwrap()
                > fitted["automation_editor"]["pixels_per_beat"]
                    .as_f64()
                    .unwrap()
        );
        let unchanged = app.command(
            json!({"command":"scroll","x":x,"y":y,"delta":120.0,"ctrl":true,"shift":true}),
        );
        assert_eq!(
            unchanged["automation_editor"]["pixels_per_beat"],
            zoomed["automation_editor"]["pixels_per_beat"]
        );
        assert_eq!(
            unchanged["automation_editor"]["plot"],
            zoomed["automation_editor"]["plot"]
        );
        assert_eq!(unchanged["project"], baseline["project"]);
        let fitted = app.click("Fit");
        let (x, y) = position(&fitted, 40.0, 0.5);
        let selected = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        assert_eq!(selected["automation_editor"]["point_beat"], 41.0);
        let zoomed = app.key("Equals");
        assert!(
            zoomed["automation_editor"]["pixels_per_beat"]
                .as_f64()
                .unwrap()
                > fitted["automation_editor"]["pixels_per_beat"]
                    .as_f64()
                    .unwrap()
        );
        fit_contains(&app.key("F"));
    }
    let state = app.inspect();
    let rect = &state["automation_editor"]["point_beat_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"45"}));
    let typed = app.key("Enter");
    assert_eq!(
        typed["project"]["tracks"][1]["automation"]["points"][1]["beat"],
        44.0
    );
    assert_eq!(typed["automation_editor"]["point_beat"], 45.0);
    menu(&mut app, "Edit", "Undo");
    assert_eq!(app.inspect()["project"], baseline["project"]);
    let dragged = drag(&mut app, (40.0, 0.5), (44.0, 0.8));
    assert_eq!(
        dragged["project"]["tracks"][1]["automation"]["points"][1]["beat"],
        44.0
    );
    assert!(
        (dragged["project"]["tracks"][1]["automation"]["points"][1]["value"]
            .as_f64()
            .unwrap()
            - 0.8)
            .abs()
            < 0.001
    );
    let deleted = app.key("Delete");
    assert_eq!(
        deleted["project"]["tracks"][1]["automation"]["points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    menu(&mut app, "Edit", "Undo");
    menu(&mut app, "Edit", "Undo");
    assert_eq!(app.inspect()["project"], baseline["project"]);
    let creation = app.click("Add parameter");
    bounded_actions(
        &creation,
        &["Track", "Parameter", "Volume", "Add automation track"],
    );
    app.key("Escape");
    app.quit();
}

#[test]
fn reopening_same_lane_id_refits_changed_points_and_restores_a_valid_selection() {
    use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget};
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let lane = project
        .add_automation_track(
            "Volume lane",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![
        AutomationPoint::new(32.0, 0.25),
        AutomationPoint::new(64.0, 0.75),
    ];
    let path = app.directory.path().join("reopened.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Volume lane");
    let fitted = menu(&mut app, "Window", "Automation");
    let (x, y) = position(&fitted, 64.0, 0.75);
    let selected = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(selected["automation_editor"]["point_beat"], 65.0);
    let zoomed = app.key("Equals");
    assert!(
        zoomed["automation_editor"]["pixels_per_beat"]
            .as_f64()
            .unwrap()
            > fitted["automation_editor"]["pixels_per_beat"]
                .as_f64()
                .unwrap()
    );

    // External edits retain the lane ID but replace its point IDs and range.
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![
        AutomationPoint::new(96.0, 0.4),
        AutomationPoint::new(104.0, 0.6),
    ];
    project.save(&path).unwrap();
    let reopened = app.command(json!({"command":"open","path":path}));
    assert!(reopened["automation_editor"].is_null());
    assert!(reopened["automation_plot_rect"].is_null());
    app.click("Volume lane");
    let refitted = menu(&mut app, "Window", "Automation");
    let view = &refitted["automation_editor"];
    assert!(view["start_beat"].as_f64().unwrap() <= 96.0);
    assert!(view["start_beat"].as_f64().unwrap() > 95.0);
    assert!(view["end_beat"].as_f64().unwrap() >= 104.0);
    assert!(view["end_beat"].as_f64().unwrap() < 105.0);
    assert_eq!(view["point_beat"], 97.0);
    let deleted = app.click("Delete point");
    let points = deleted["project"]["tracks"][1]["automation"]["points"]
        .as_array()
        .unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0]["beat"], 104.0);
    app.quit();
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
    app.click("More");
    let state = app.click("Remove insert");
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
