//! Native-menu automation through a real isolated editor child and production
//! application UI. The child models native actions without a desktop or audio
//! device; host menu composition/dispatch is covered by native host unit tests.
mod common;

use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Instrument, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Fixture {
    worker: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let worker = tempfile::tempdir().unwrap();
        let source = worker.path().join("native_parameter_host.rs");
        let metadata =
            include_str!("common/parameter_host.rs").replace("fn main() {", "fn metadata_main() {");
        std::fs::write(
            &source,
            format!(
                "{metadata}\n{}",
                include_str!("common/native_parameter_editor.rs")
            ),
        )
        .unwrap();
        let host = worker.path().join(if cfg!(windows) {
            "native-parameter-host.exe"
        } else {
            "native-parameter-host"
        });
        let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let result = Command::new(compiler)
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "fixture compilation: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self {
            worker,
            plugins: common::plugin_fixture(),
            host,
        }
    }

    fn app(&self) -> (Harness, u64, u64, [u64; 2]) {
        let mut app = Harness::with_plugin_host(self.plugins.path(), &self.host);
        let mut project = Project::new();
        let track = project.add_midi_track("Neon keys 🎹");
        let plugin = self
            .plugins
            .path()
            .join("RillFixture.vst3")
            .to_string_lossy()
            .into_owned();
        project
            .set_instrument(
                track,
                Instrument::Vst3 {
                    path: plugin.clone(),
                    class_id: Some("11111111111111111111111111111111".into()),
                },
            )
            .unwrap();
        let source = project.track_mut(track).unwrap();
        source.instrument_state = vec![40];
        source
            .clips
            .push(Clip::midi("Neon phrase", 0.0, 8.0, vec![]));
        for name in ["Echo A", "Echo B"] {
            let mut slot = EffectSlot::new(
                name,
                Effect::Vst3 {
                    path: plugin.clone(),
                    class_id: Some("33333333333333333333333333333333".into()),
                },
            );
            slot.state = vec![30];
            source.inserts.push(slot);
        }
        let effects = [source.inserts[0].id, source.inserts[1].id];
        let other = project.add_midi_track("Other channel");
        project
            .track_mut(other)
            .unwrap()
            .clips
            .push(Clip::midi("Other phrase", 0.0, 8.0, vec![]));
        let path = app.directory.path().join("native-automation.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.click("Neon phrase");
        (app, track, other, effects)
    }

    fn action(&self, app: &mut Harness, actions: &str) {
        let state = app.inspect();
        let pid = state["native_editor"]["pid"]
            .as_u64()
            .expect("live isolated editor");
        let prefix = self.worker.path().join(format!("editor-{pid}"));
        let handled = prefix.with_extension("handled");
        let _ = std::fs::remove_file(&handled);
        let temporary = prefix.with_extension("pending");
        std::fs::write(&temporary, actions).unwrap();
        std::fs::rename(&temporary, prefix.with_extension("actions")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !handled.exists() {
            assert!(
                Instant::now() < deadline,
                "isolated editor did not consume native action"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn wait(app: &mut Harness, condition: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        assert!(
            state["native_editor"]["error"].is_null(),
            "editor failed: {}",
            state["native_editor"]
        );
        if condition(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "editor transition timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn menu(app: &mut Harness, item: &str) -> Value {
    app.click("Window");
    app.click(item)
}

fn open_instrument(app: &mut Harness) -> Value {
    menu(app, "Instrument");
    app.click("Open editor");
    wait(app, |state| {
        state["native_editor"]["open"] == true && state["native_editor"]["parameter_count"] == 3
    })
}

fn lanes(state: &Value) -> Vec<&Value> {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| !track["automation"].is_null())
        .collect()
}

fn assert_lane(lane: &Value, track: u64, effect: Option<u64>, initial: f64) {
    assert_eq!(
        lane["automation"]["target"],
        json!({"track":track,"parameter":{"VstParameter":{"effect":effect,"id":0}}})
    );
    let value = lane["automation"]["points"][0]["value"].as_f64().unwrap();
    assert!(
        (value - initial).abs() < 0.0001,
        "initial value {value}, expected {initial}"
    );
}

#[test]
fn native_instrument_action_targets_its_owner_reuses_undoes_and_recalls() {
    let fixture = Fixture::new();
    let (mut app, track, other, _) = fixture.app();
    let opened = open_instrument(&mut app);
    let pid = opened["native_editor"]["pid"].clone();
    assert_ne!(pid.as_u64().unwrap(), std::process::id() as u64);
    menu(&mut app, "Arrangement");
    let selected = app.click("Other phrase");
    assert_eq!(selected["selected_track"], other);
    fixture.action(&mut app, "change 0 0.77\nautomate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    assert_lane(lanes(&created)[0], track, None, 0.77);
    assert_eq!(lanes(&created)[0]["name"], "Neon keys 🎹 · Filter Cutoff");
    let lane = lanes(&created)[0]["id"].clone();
    assert_eq!(created["selected_track"], lane);
    assert_eq!(created["popups"]["automation"], true);
    assert_eq!(created["native_editor"]["pid"], pid);
    assert_eq!(created["native_editor"]["last_touched"], 0);

    menu(&mut app, "Arrangement");
    app.click("Other phrase");
    fixture.action(&mut app, "automate 0\n");
    let reused = wait(&mut app, |state| state["selected_track"] == lane);
    assert_eq!(lanes(&reused).len(), 1);
    assert_lane(lanes(&reused)[0], track, None, 0.77);
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert!(lanes(&undone).is_empty(), "reopen must add no Undo entry");
    assert_eq!(
        undone["project"]["tracks"][0]["instrument_state"],
        json!([77])
    );
    assert_eq!(
        undone["project"]["tracks"][0]["instrument_parameters"],
        json!([])
    );
    assert_eq!(undone["native_editor"]["pid"], pid);
    let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    assert_eq!(lanes(&redone).len(), 1);
    let path = app.directory.path().join("native-recalled.rill");
    app.command(json!({"command":"save", "path":path}));
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":path}));
    assert_lane(lanes(&recalled)[0], track, None, 0.77);
    assert_eq!(
        recalled["project"]["tracks"][0]["instrument_state"],
        json!([77])
    );
    app.click("Neon phrase");
    open_instrument(&mut app);
    fixture.action(&mut app, "automate 0\n");
    let reopened = wait(&mut app, |state| state["selected_track"] == lane);
    assert_eq!(lanes(&reopened).len(), 1);
    assert_lane(lanes(&reopened)[0], track, None, 0.77);
    app.quit();
}

#[test]
fn native_effect_action_keeps_instance_channel_context_and_state() {
    let fixture = Fixture::new();
    let (mut app, track, other, effects) = fixture.app();
    for (index, expected) in [0.26, 0.64].into_iter().enumerate() {
        menu(&mut app, "Arrangement");
        app.click("Neon phrase");
        menu(&mut app, "Effects");
        app.command(json!({"command":"click", "label":"Open editor", "occurrence":index}));
        wait(&mut app, |state| {
            state["native_editor"]["open"] == true
                && state["native_editor"]["effect"] == effects[index]
        });
        menu(&mut app, "Arrangement");
        assert_eq!(app.click("Other phrase")["selected_track"], other);
        fixture.action(&mut app, &format!("change 0 {expected}\nautomate 0\n"));
        let created = wait(&mut app, |state| lanes(state).len() == index + 1);
        assert_lane(
            lanes(&created)[index],
            track,
            Some(effects[index]),
            expected,
        );
        assert_eq!(
            lanes(&created)[index]["name"],
            format!(
                "Neon keys 🎹 · Echo {} · Filter Cutoff",
                if index == 0 { "A" } else { "B" }
            )
        );
    }
    let path = app.directory.path().join("native-effects.rill");
    let saved = app.command(json!({"command":"save", "path":path}));
    assert_eq!(
        saved["project"]["tracks"][0]["inserts"][0]["state"],
        json!([26])
    );
    assert_eq!(
        saved["project"]["tracks"][0]["inserts"][1]["state"],
        json!([64])
    );
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":path}));
    assert_eq!(lanes(&recalled).len(), 2);
    assert_lane(lanes(&recalled)[0], track, Some(effects[0]), 0.26);
    assert_lane(lanes(&recalled)[1], track, Some(effects[1]), 0.64);
    app.quit();
}

#[test]
fn invalid_native_requests_and_document_confirmation_do_not_create_lanes() {
    let fixture = Fixture::new();
    let (mut app, track, _, _) = fixture.app();
    open_instrument(&mut app);
    fixture.action(
        &mut app,
        "automate 43\nautomate 999\nautomate 0 -0.1\nautomate 0 1.1\nchange 7 0.66\n",
    );
    let rejected = wait(&mut app, |state| {
        state["native_editor"]["last_touched"] == 7
    });
    assert!(lanes(&rejected).is_empty());
    assert_eq!(
        rejected["project"]["tracks"][0]["instrument_parameters"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.click("File");
    app.click("New");
    common::wait_visible(&mut app, "Don't save");
    fixture.action(&mut app, "automate 0\nchange 7 0.67\n");
    let pending = wait(&mut app, |state| {
        state["project"]["tracks"][0]["instrument_parameters"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value["id"] == 7 && value["value"] == 0.67)
            })
    });
    assert!(
        lanes(&pending).is_empty(),
        "late native action must not edit a transitioning document"
    );
    app.click("Cancel");
    fixture.action(&mut app, "automate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    assert_lane(lanes(&created)[0], track, None, 0.4);
    app.quit();
}

#[test]
fn opaque_native_preset_is_captured_before_lane_undo_and_recalled() {
    let fixture = Fixture::new();
    let (mut app, track, _, _) = fixture.app();
    let opened = open_instrument(&mut app);
    let pid = opened["native_editor"]["pid"].clone();
    fixture.action(&mut app, "preset 0 0.91\nautomate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    assert_lane(lanes(&created)[0], track, None, 0.91);
    assert_eq!(
        created["project"]["tracks"][0]["instrument_state"],
        json!([91])
    );
    assert_eq!(
        created["project"]["tracks"][0]["instrument_parameters"],
        json!([])
    );
    assert!(
        created["native_editor"]["last_touched"].is_null(),
        "fixture must not manufacture a parameter gesture for a preset"
    );
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert!(lanes(&undone).is_empty());
    assert_eq!(
        undone["project"]["tracks"][0]["instrument_state"],
        json!([91]),
        "Undo lane creation must preserve the already selected native preset"
    );
    assert_eq!(
        undone["native_editor"]["pid"], pid,
        "lane-only Undo must retain the editor instance"
    );
    assert_eq!(undone["history"]["undo"], json!(["Change plug-in preset"]));

    let path = app.directory.path().join("opaque-preset.rill");
    app.command(json!({"command":"save", "path":path}));
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":path}));
    assert!(lanes(&recalled).is_empty());
    assert_eq!(
        recalled["project"]["tracks"][0]["instrument_state"],
        json!([91])
    );
    app.click("Neon phrase");
    let reopened = open_instrument(&mut app);
    assert_eq!(reopened["native_editor"]["selected_value"], 0.91);
    fixture.action(&mut app, "automate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    assert_lane(lanes(&created)[0], track, None, 0.91);
    app.command(json!({"command":"save", "path":path}));
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":path}));
    assert_lane(lanes(&recalled)[0], track, None, 0.91);
    assert_eq!(
        recalled["project"]["tracks"][0]["instrument_state"],
        json!([91])
    );
    app.quit();
}

#[test]
fn native_parameter_gestures_override_owned_automation_and_preserve_it_after_snapshot() {
    let fixture = Fixture::new();
    let (mut app, track, other, _) = fixture.app();
    let opened = open_instrument(&mut app);
    let pid = opened["native_editor"]["pid"].clone();
    fixture.action(&mut app, "automate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    let curve = lanes(&created)[0]["automation"]["points"].clone();
    assert_lane(lanes(&created)[0], track, None, 0.4);
    menu(&mut app, "Arrangement");
    assert_eq!(app.click("Other phrase")["selected_track"], other);
    fixture.action(&mut app, "change 0 0.77\n");
    let changed = wait(&mut app, |state| {
        lanes(state)[0]["automation"]["override_value"]
            .as_f64()
            .is_some_and(|value| (value - 0.77).abs() < 1e-6)
    });
    assert_eq!(lanes(&changed)[0]["automation"]["points"], curve);
    assert_eq!(changed["native_editor"]["pid"], pid);
    assert_eq!(
        changed["selected_track"], other,
        "Native ownership must not follow selection"
    );

    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Instrument");
    let browser = app.click("Automate…");
    assert_eq!(browser["native_editor"]["automation_state"], "Overridden");
    assert!(
        browser["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Resume automation")
    );
    let saved = app.directory.path().join("native-override.rill");
    let snapshot = app.command(json!({"command":"save", "path":saved}));
    assert_eq!(
        snapshot["project"]["tracks"][0]["instrument_state"],
        json!([77])
    );
    assert_eq!(
        snapshot["project"]["tracks"][0]["instrument_parameters"],
        json!([])
    );
    assert!(
        (lanes(&snapshot)[0]["automation"]["override_value"]
            .as_f64()
            .unwrap()
            - 0.77)
            .abs()
            < 1e-6
    );
    let resumed = app.click("Resume automation");
    assert!(lanes(&resumed)[0]["automation"]["override_value"].is_null());
    assert_eq!(resumed["native_editor"]["automation_state"], "Automated");
    assert_eq!(resumed["native_editor"]["pid"], pid);
    assert!((resumed["native_editor"]["selected_value"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(lanes(&resumed)[0]["automation"]["points"], curve);
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":saved}));
    assert!(
        (lanes(&recalled)[0]["automation"]["override_value"]
            .as_f64()
            .unwrap()
            - 0.77)
            .abs()
            < 1e-6
    );
    assert_eq!(lanes(&recalled)[0]["automation"]["points"], curve);
    app.quit();
}

#[test]
fn delayed_browser_parameter_acknowledgement_does_not_retake_resumed_automation() {
    let fixture = Fixture::new();
    let (mut app, track, _, _) = fixture.app();
    let opened = open_instrument(&mut app);
    let pid = opened["native_editor"]["pid"].as_u64().unwrap();
    fixture.action(&mut app, "automate 0\n");
    let created = wait(&mut app, |state| lanes(state).len() == 1);
    assert_lane(lanes(&created)[0], track, None, 0.4);
    assert!(lanes(&created)[0]["automation"]["override_value"].is_null());
    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Instrument");
    app.click("Automate…");

    // A delayed native worker still acknowledges the browser's already-owned
    // edit. Keep that reply pending until after the visible Resume action.
    let prefix = fixture.worker.path().join(format!("editor-{pid}"));
    let hold = prefix.with_extension("hold-acks");
    std::fs::write(&hold, []).unwrap();
    let overridden = app.click("Override automation");
    assert_eq!(
        overridden["native_editor"]["automation_state"],
        "Overridden"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(prefix.with_extension("log"))
        .unwrap()
        .contains("\"command\":\"set_parameter\"")
    {
        assert!(Instant::now() < deadline, "native setter was not received");
        std::thread::sleep(Duration::from_millis(10));
    }
    let resumed = app.click("Resume automation");
    assert!(lanes(&resumed)[0]["automation"]["override_value"].is_null());
    let undo_steps = resumed["history"]["undo"].as_array().unwrap().len();
    std::fs::remove_file(hold).unwrap();

    // The actor emits this real gesture after draining its acknowledgements,
    // proving the app consumed the late reply before assertions are sampled.
    fixture.action(&mut app, "change 7 0.61\n");
    let acknowledged = wait(&mut app, |state| {
        state["native_editor"]["last_touched"] == 7
    });
    assert!(lanes(&acknowledged)[0]["automation"]["override_value"].is_null());
    assert_eq!(
        acknowledged["native_editor"]["automation_state"],
        "Automated"
    );
    assert_eq!(acknowledged["native_editor"]["pid"], pid);
    assert_eq!(
        acknowledged["history"]["undo"].as_array().unwrap().len(),
        undo_steps + 1,
        "Only the genuine Resonance gesture may add an Undo entry"
    );
    assert!(
        (acknowledged["native_editor"]["selected_value"]
            .as_f64()
            .unwrap()
            - 0.4)
            .abs()
            < 1e-6
    );

    fixture.action(&mut app, "change 0 0.77\n");
    let manual = wait(&mut app, |state| {
        lanes(state)[0]["automation"]["override_value"]
            .as_f64()
            .is_some_and(|value| (value - 0.77).abs() < 1e-6)
    });
    assert_eq!(manual["native_editor"]["automation_state"], "Overridden");
    assert_lane(lanes(&manual)[0], track, None, 0.4);
    app.quit();
}
