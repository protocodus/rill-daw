//! Edit automation through the actual application, then inspect persisted and
//! rendered results. Pointer positions come from the production curve component.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, label: &str) -> Value {
    app.click(menu);
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| *s == label)
        .count();
    assert!(count > 0, "Missing {label}: {state}");
    app.command(json!({"command":"click","label":label,"occurrence":count-1}))
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
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["points"][1]["beat"],
        2.0
    );
    let state = menu(&mut app, "Edit", "Redo");
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
