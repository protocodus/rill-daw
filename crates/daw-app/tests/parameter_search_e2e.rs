//! Search-to-automation through the production app and its isolated metadata IPC.
//! The temporary Rust worker supplies deterministic metadata/state only; actual
//! native VST execution remains covered by the opt-in plugin integration tests.
mod common;

use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Instrument, PluginParameterValue, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Fixture {
    _worker: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let worker = tempfile::tempdir().unwrap();
        let source = worker.path().join("parameter_host.rs");
        std::fs::write(&source, include_str!("common/parameter_host.rs")).unwrap();
        let host = worker.path().join(if cfg!(windows) {
            "parameter-host.exe"
        } else {
            "parameter-host"
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
            _worker: worker,
            plugins: common::plugin_fixture(),
            host,
        }
    }

    fn app(&self) -> (Harness, u64, [u64; 2]) {
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
        source.instrument_state = vec![60];
        source.instrument_parameters = vec![PluginParameterValue { id: 7, value: 0.73 }];
        source
            .clips
            .push(Clip::midi("Neon phrase", 0.0, 8.0, vec![]));
        for (name, saved, value) in [("Echo A", 10, 0.21), ("Echo B", 90, 0.82)] {
            let mut effect = EffectSlot::new(
                name,
                Effect::Vst3 {
                    path: plugin.clone(),
                    class_id: Some("33333333333333333333333333333333".into()),
                },
            );
            effect.state = vec![saved];
            effect.parameters = vec![PluginParameterValue { id: 7, value }];
            source.inserts.push(effect);
        }
        let effects = [source.inserts[0].id, source.inserts[1].id];
        let path = app.directory.path().join("parameters.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.click("Neon phrase");
        (app, track, effects)
    }
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn menu(app: &mut Harness, item: &str) -> Value {
    app.click("Window");
    app.click(item)
}

fn ready(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        assert!(
            state["native_editor"]["error"].is_null(),
            "metadata load: {}",
            state["native_editor"]
        );
        if state["native_editor"]["parameter_count"].as_u64().unwrap() >= 200 {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "metadata did not arrive: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn query(app: &mut Harness, text: &str) -> Value {
    app.command(json!({"command":"key", "key":"F", "cmd":true}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    if text.is_empty() {
        app.key("Backspace")
    } else {
        app.command(json!({"command":"text", "text":text}))
    }
}

fn lanes(state: &Value) -> Vec<&Value> {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| !track["automation"].is_null())
        .collect()
}

fn assert_lane(lane: &Value, track: u64, effect: Option<u64>, parameter: u32, initial: f64) {
    assert_eq!(
        lane["automation"]["target"],
        json!({"track":track,"parameter":{"VstParameter":{"effect":effect,"id":parameter}}})
    );
    let value = lane["automation"]["points"][0]["value"].as_f64().unwrap();
    assert!(
        (value - initial).abs() < 0.0001,
        "lane initial {value}, expected {initial}"
    );
}

fn double_click(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"] == label)
        .unwrap();
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}))
}

fn drag_parameter_knob(app: &mut Harness, state: &Value, delta_y: f64) -> Value {
    let center = &state["native_editor"]["parameter_control"]["center"];
    let x = center[0].as_f64().expect("parameter knob center");
    let y = center[1].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x,"to_y":y+delta_y}))
}

#[test]
fn instrument_search_handles_empty_results_keyboard_activation_reuse_and_recall() {
    let fixture = Fixture::new();
    for (dark, width, height, density) in [(false, 1100, 760, 1.0), (true, 800, 600, 2.0)] {
        let (mut app, track, _) = fixture.app();
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        menu(&mut app, "Instrument");
        app.click("Automate…");
        // The chooser requests focus itself; typing requires no extra search click.
        app.command(json!({"command":"text","text":"FILTER cut"}));
        let selected = ready(&mut app);
        assert_eq!(selected["native_editor"]["selected"], 7);
        assert!(
            (selected["native_editor"]["selected_value"]
                .as_f64()
                .unwrap()
                - 0.73)
                .abs()
                < 1e-6
        );
        assert!(!visible(&selected, "Output Meter"));
        assert_eq!(selected["native_editor"]["open"], false);

        let read_only = query(&mut app, "Output Meter");
        assert!(
            read_only["native_editor"]["selected"].is_null(),
            "read-only parameters must not be offered for automation"
        );

        let empty = query(&mut app, "nothing matches this parameter");
        assert!(empty["native_editor"]["selected"].is_null());
        let after_enter = app.key("Enter");
        assert!(
            lanes(&after_enter).is_empty(),
            "an empty search must never activate a stale selection"
        );
        assert_eq!(after_enter["popups"]["instrument"], true);

        let unicode = query(&mut app, "ω motion");
        assert_eq!(unicode["native_editor"]["selected"], 9000);
        let created = app.key("Enter");
        assert_eq!(lanes(&created).len(), 1);
        assert_lane(lanes(&created)[0], track, None, 9000, 0.5);
        let lane_id = lanes(&created)[0]["id"].clone();
        assert_eq!(created["popups"]["automation"], true);

        app.key("Escape");
        query(&mut app, "");
        app.click("Automated");
        let automated = app.inspect();
        assert_eq!(automated["native_editor"]["selected"], 9000);
        assert!(!visible(&automated, "Filter Cutoff"));
        let reused = app.key("Enter");
        assert_eq!(lanes(&reused).len(), 1);
        assert_eq!(reused["selected_track"], lane_id);

        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert!(lanes(&undone).is_empty());
        let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
        assert_eq!(lanes(&redone).len(), 1);
        let path = app.directory.path().join("searched.rill");
        app.command(json!({"command":"save","path":path}));
        let mut app = app.restart();
        let recalled = app.command(json!({"command":"open","path":path}));
        assert_lane(lanes(&recalled)[0], track, None, 9000, 0.5);
        app.quit();
    }
}

#[test]
fn modified_result_navigation_does_not_change_selection_or_create_automation() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app();
    menu(&mut app, "Instrument");
    app.click("Automate…");
    ready(&mut app);
    let original = query(&mut app, "Filter");
    assert_eq!(original["native_editor"]["selected"], 7);
    for modifier in ["shift", "ctrl", "alt", "cmd"] {
        for key in ["ArrowDown", "Enter"] {
            let mut command = json!({"command":"key", "key":key, "release_modifiers":true});
            command[modifier] = json!(true);
            let state = app.command(command);
            assert_eq!(
                state["native_editor"]["selected"], 7,
                "{modifier}+{key} changed the result"
            );
            assert_eq!(state["native_editor"]["parameter_query"], "Filter");
            assert_eq!(state["project"], original["project"]);
            assert!(lanes(&state).is_empty());
            assert_eq!(state["popups"]["instrument"], true);
        }
    }
    assert_eq!(app.key("ArrowDown")["native_editor"]["selected"], 8);
    let created = app.key("Enter");
    assert_lane(lanes(&created)[0], track, None, 8, 0.5);
    app.quit();
}

#[test]
fn insert_search_uses_exact_instance_and_supports_arrows_double_click_and_last_touched() {
    let fixture = Fixture::new();
    let (mut app, track, effects) = fixture.app();
    menu(&mut app, "Effects");
    app.command(json!({"command":"click","label":"Automate…","occurrence":0}));
    ready(&mut app);
    let selected = query(&mut app, "Filter");
    assert_eq!(selected["native_editor"]["selected"], 7);
    let stepped = app.key("ArrowDown");
    assert_eq!(stepped["native_editor"]["selected"], 8);
    let created = app.key("Enter");
    assert_lane(lanes(&created)[0], track, Some(effects[0]), 8, 0.25);

    app.key("Escape");
    let selected = query(&mut app, "Filter Cutoff");
    assert!(
        (selected["native_editor"]["selected_value"]
            .as_f64()
            .unwrap()
            - 0.21)
            .abs()
            < 1e-6
    );
    let touched = drag_parameter_knob(&mut app, &selected, -30.0);
    assert_eq!(touched["native_editor"]["last_touched"], 7);
    let expected = touched["native_editor"]["selected_value"].as_f64().unwrap();
    let cleared = query(&mut app, "");
    assert_eq!(cleared["native_editor"]["parameter_query"], "");
    app.click("Last touched");
    let created = double_click(&mut app, "Filter Cutoff");
    assert_eq!(lanes(&created).len(), 2);
    assert_lane(lanes(&created)[1], track, Some(effects[0]), 7, expected);

    // Returning to the same source channel and a different instance must not
    // reuse the first insert's metadata values, even though its VST class matches.
    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Effects");
    app.command(json!({"command":"click","label":"Automate…","occurrence":1}));
    ready(&mut app);
    let second = query(&mut app, "Filter Cutoff");
    assert_eq!(second["native_editor"]["effect"], effects[1]);
    assert!((second["native_editor"]["selected_value"].as_f64().unwrap() - 0.82).abs() < 1e-6);
    let created = app.key("Enter");
    assert_eq!(lanes(&created).len(), 3);
    assert_lane(lanes(&created)[2], track, Some(effects[1]), 7, 0.82);
    assert_eq!(created["native_editor"]["open"], false);
    app.quit();
}

#[test]
fn add_automation_parameter_search_uses_saved_instance_values_and_reuses_existing_lane() {
    let fixture = Fixture::new();
    let (mut app, track, effects) = fixture.app();
    menu(&mut app, "Automation");
    app.click("Volume");
    app.click("VST instrument parameter");
    common::wait_visible(&mut app, "Filter Cutoff");
    query(&mut app, "No such parameter");
    let empty = app.key("Enter");
    assert!(
        lanes(&empty).is_empty(),
        "empty plugin search must not create a volume lane"
    );
    query(&mut app, "Filter Cutoff");
    let created = app.key("Enter");
    assert_lane(lanes(&created)[0], track, None, 7, 0.73);

    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Automation");
    app.click("Volume");
    app.click("01 · Echo A · Parameter");
    common::wait_visible(&mut app, "Filter Cutoff");
    query(&mut app, "Filter Cutoff");
    let created = app.key("Enter");
    assert_eq!(lanes(&created).len(), 2);
    assert_lane(lanes(&created)[1], track, Some(effects[0]), 7, 0.21);

    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Automation");
    app.click("Volume");
    app.click("02 · Echo B · Parameter");
    common::wait_visible(&mut app, "Filter Cutoff");
    query(&mut app, "Filter Cutoff");
    let created = app.key("Enter");
    assert_eq!(lanes(&created).len(), 3);
    assert_lane(lanes(&created)[2], track, Some(effects[1]), 7, 0.82);
    let id = lanes(&created)[2]["id"].clone();
    menu(&mut app, "Arrangement");
    app.click("Neon phrase");
    menu(&mut app, "Automation");
    app.click("Volume");
    app.click("02 · Echo B · Parameter");
    common::wait_visible(&mut app, "Filter Cutoff");
    query(&mut app, "Filter Cutoff");
    let reused = app.key("Enter");
    assert_eq!(reused["selected_track"], id);
    assert_eq!(lanes(&reused).len(), 3);
    app.quit();
}

#[test]
fn parameter_metadata_error_stays_in_the_browser_without_creating_a_lane() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app();
    let path = app.directory.path().join("parameters.rill");
    let mut project = Project::load(&path).unwrap();
    let Instrument::Vst3 { class_id, .. } = &mut project.track_mut(track).unwrap().instrument
    else {
        unreachable!()
    };
    *class_id = Some("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF".into());
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Neon phrase");
    menu(&mut app, "Instrument");
    app.click("Automate…");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["native_editor"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("Fixture metadata unavailable"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "missing metadata failure: {}",
            state["native_editor"]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    query(&mut app, "Filter Cutoff");
    let failed = app.key("Enter");
    assert!(lanes(&failed).is_empty());
    assert!(visible(&failed, "Retry"));
    assert_eq!(failed["native_editor"]["open"], false);
    app.key("Escape");
    assert!(visible(&app.inspect(), "Open editor"));
    app.quit();
}

#[test]
fn parameter_controls_show_automation_and_manual_override_with_resume_undo_and_recall() {
    use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget};
    let fixture = Fixture::new();
    for (effect_index, dark) in [(None, false), (Some(0), true)] {
        let (mut app, track, effects) = fixture.app();
        let effect = effect_index.map(|index| effects[index]);
        let path = app.directory.path().join("parameters.rill");
        let mut project = Project::load(&path).unwrap();
        let lane = project
            .add_automation_track_with_value(
                "Cutoff movement",
                AutomationTarget {
                    track,
                    parameter: AutomationParameter::VstParameter { effect, id: 7 },
                },
                0.2,
            )
            .unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(8.0, 0.2),
            AutomationPoint::new(12.0, 0.8),
        ];
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":760,
            "pixels_per_point":if dark {2.0} else {1.0}}));
        app.click("Neon phrase");
        if let Some(index) = effect_index {
            menu(&mut app, "Effects");
            app.command(json!({"command":"click", "label":"Automate…", "occurrence":index}));
        } else {
            menu(&mut app, "Instrument");
            app.click("Automate…");
        }
        ready(&mut app);
        let initial = query(&mut app, "Filter Cutoff");
        let curve = lanes(&initial)[0]["automation"]["points"].clone();
        assert_eq!(initial["native_editor"]["automation_state"], "Automated");
        assert!(
            (initial["native_editor"]["selected_value"].as_f64().unwrap() - 0.2).abs() < 1e-6,
            "Control must show the held first value even before its first node"
        );
        assert!(visible(&initial, "Override automation"));

        let changed = drag_parameter_knob(&mut app, &initial, -30.0);
        let held = lanes(&changed)[0]["automation"]["override_value"]
            .as_f64()
            .unwrap();
        assert!((held - 0.2).abs() > 0.01);
        assert_eq!(changed["native_editor"]["automation_state"], "Overridden");
        assert!(visible(&changed, "Overridden") && visible(&changed, "Resume automation"));
        assert_eq!(lanes(&changed)[0]["automation"]["points"], curve);

        let saved = app.directory.path().join("overridden-parameters.rill");
        app.command(json!({"command":"save", "path":saved}));
        let resumed = app.click("Resume automation");
        assert!(lanes(&resumed)[0]["automation"]["override_value"].is_null());
        assert_eq!(resumed["native_editor"]["automation_state"], "Automated");
        assert!((resumed["native_editor"]["selected_value"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(lanes(&resumed)[0]["automation"]["points"], curve);
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert!(
            (lanes(&undone)[0]["automation"]["override_value"]
                .as_f64()
                .unwrap()
                - held)
                .abs()
                < 1e-6
        );
        let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
        assert!(lanes(&redone)[0]["automation"]["override_value"].is_null());

        let explicit = app.click("Override automation");
        assert!(
            (lanes(&explicit)[0]["automation"]["override_value"]
                .as_f64()
                .unwrap()
                - 0.2)
                .abs()
                < 1e-6
        );
        let mut app = app.restart();
        let recalled = app.command(json!({"command":"open", "path":saved}));
        assert!(
            (lanes(&recalled)[0]["automation"]["override_value"]
                .as_f64()
                .unwrap()
                - held)
                .abs()
                < 1e-6
        );
        assert_eq!(lanes(&recalled)[0]["automation"]["points"], curve);
        app.quit();
    }
}

#[test]
fn knob_numeric_entry_is_precise_automatable_and_recalled_without_keyboard_shortcuts() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app();
    app.command(json!({"command":"viewport","width":1100,"height":850}));
    menu(&mut app, "Instrument");
    app.click("Automate…");
    ready(&mut app);
    let initial = query(&mut app, "Filter Cutoff");
    assert!(visible(&initial, "Normalized value"));
    assert!(visible(&initial, "Plug-in unit · Hz"));
    assert!(visible(&initial, "0 %") && visible(&initial, "100 %"));
    assert_eq!(
        initial["native_editor"]["parameter_control"]["range"],
        json!([0.0, 100.0])
    );
    let before_history = initial["history"]["undo"].as_array().unwrap().len();
    let changed = common::edit_plugin_parameter_value(&mut app, &initial, "61.2345");
    let value = changed["native_editor"]["selected_value"].as_f64().unwrap();
    assert!((value - 0.612345).abs() < 1e-8, "{value}");
    assert_eq!(changed["native_editor"]["last_touched"], 7);
    assert_eq!(
        changed["history"]["undo"].as_array().unwrap().len(),
        before_history + 1
    );
    assert!(
        visible(&changed, "61.2345 %"),
        "{}",
        changed["visible_labels"]
    );

    // The context editor preserves more digits than the compact readout and
    // does not edit the song until its visible Apply action is clicked.
    let center = &changed["native_editor"]["parameter_control"]["center"];
    let editing = app.command(
        json!({"command":"pointer","phase":"click","button":"secondary",
        "x":center[0],"y":center[1]}),
    );
    assert!(visible(&editing, "Range: 0 to 100 %"));
    assert!(visible(&editing, "Apply value"));
    let current_percent = changed["native_editor"]["selected_value"].as_f64().unwrap() * 100.0;
    let current_text = daw_ui::exact_value_text(current_percent);
    let input = editing["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == current_text)
        .unwrap_or_else(|| {
            panic!(
                "Exact-value draft {current_text:?} must be visible: {}",
                editing["visible_labels"]
            )
        });
    let x = (input["min"][0].as_f64().unwrap() + input["max"][0].as_f64().unwrap()) * 0.5;
    let y = (input["min"][1].as_f64().unwrap() + input["max"][1].as_f64().unwrap()) * 0.5;
    let focused = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert!(
        visible(&focused, "Apply value"),
        "Clicking the input must keep its editor open"
    );
    // An invalid numeric draft must stay editable rather than dismissing the menu.
    let invalid = app.command(json!({"command":"text","text":"NaN"}));
    assert!(
        visible(&invalid, "Enter a finite number."),
        "{}",
        invalid["visible_labels"]
    );
    let rejected = app.click("Apply value");
    assert!(visible(&rejected, "Cancel") && visible(&rejected, "Apply value"));
    assert_eq!(rejected["project"], changed["project"]);
    assert_eq!(rejected["history"], changed["history"]);
    let cancelled = app.click("Cancel");
    assert!(!visible(&cancelled, "Apply value"));
    assert_eq!(cancelled["project"], changed["project"]);
    assert_eq!(cancelled["history"], changed["history"]);
    let reopened = app.command(
        json!({"command":"pointer","phase":"click","button":"secondary",
        "x":center[0],"y":center[1]}),
    );
    assert!(
        visible(&reopened, &current_text),
        "Reopening discards the cancelled draft"
    );
    let drafted = app.command(json!({"command":"text","text":"61.23456789"}));
    assert_eq!(drafted["project"], changed["project"]);
    assert_eq!(drafted["history"], changed["history"]);
    let precise = app.click("Apply value");
    let value = precise["native_editor"]["selected_value"].as_f64().unwrap();
    assert!((value - 0.6123456789).abs() < 1e-12, "{value}");

    let created = app.click("Automate parameter");
    assert_lane(lanes(&created)[0], track, None, 7, value);
    let points = lanes(&created)[0]["automation"]["points"].clone();
    app.key("Escape");
    let automated = app.inspect();
    assert_eq!(automated["native_editor"]["automation_state"], "Automated");
    let overridden = common::edit_plugin_parameter_value(&mut app, &automated, "23.4567");
    assert_eq!(
        overridden["native_editor"]["automation_state"],
        "Overridden"
    );
    assert!(visible(&overridden, "Overridden"));
    let override_value = lanes(&overridden)[0]["automation"]["override_value"]
        .as_f64()
        .unwrap();
    assert!((override_value - 0.234567).abs() < 1e-6);
    assert_eq!(lanes(&overridden)[0]["automation"]["points"], points);
    let path = app.directory.path().join("precise-knob.rill");
    app.command(json!({"command":"save","path":path}));
    let resumed = app.click("Resume automation");
    assert!(lanes(&resumed)[0]["automation"]["override_value"].is_null());
    assert!((resumed["native_editor"]["selected_value"].as_f64().unwrap() - value).abs() < 1e-6);
    assert_eq!(lanes(&resumed)[0]["automation"]["points"], points);

    let mut app = app.restart();
    let reopened = app.command(json!({"command":"open","path":path}));
    assert!(
        (lanes(&reopened)[0]["automation"]["override_value"]
            .as_f64()
            .unwrap()
            - override_value)
            .abs()
            < 1e-8
    );
    app.click("Neon phrase");
    menu(&mut app, "Instrument");
    app.click("Automate…");
    ready(&mut app);
    let recalled = query(&mut app, "Filter Cutoff");
    assert_eq!(recalled["native_editor"]["automation_state"], "Overridden");
    assert!(
        (recalled["native_editor"]["parameter_control"]["value"]
            .as_f64()
            .unwrap()
            - 23.4567)
            .abs()
            < 1e-4
    );
    assert_eq!(lanes(&recalled)[0]["automation"]["points"], points);
    app.quit();
}

#[test]
fn stepped_knob_quantizes_edits_only_and_read_only_parameters_cannot_be_edited() {
    let fixture = Fixture::new();
    let (mut app, _, _) = fixture.app();
    menu(&mut app, "Instrument");
    app.click("Automate…");
    ready(&mut app);
    let initial = query(&mut app, "Oscillator Mode");
    assert_eq!(initial["native_editor"]["selected_steps"], 4);
    assert!((initial["native_editor"]["selected_value"].as_f64().unwrap() - 0.37).abs() < 1e-8);
    assert!(visible(&initial, "5 discrete values"));
    for _ in 0..3 {
        let painted = app.inspect();
        assert_eq!(painted["project"], initial["project"]);
        assert_eq!(painted["history"], initial["history"]);
        assert_eq!(painted["native_editor"]["parameter_control"]["value"], 37.0);
    }
    let changed = common::edit_plugin_parameter_value(&mut app, &initial, "38");
    assert_eq!(changed["native_editor"]["selected_value"], 0.5);
    assert_eq!(changed["native_editor"]["parameter_control"]["value"], 50.0);
    assert!(visible(&changed, "50.00 %"));
    let history = changed["history"].clone();
    let project = changed["project"].clone();
    let read_only = query(&mut app, "Output Meter");
    assert!(read_only["native_editor"]["selected"].is_null());
    assert!(read_only["native_editor"]["parameter_control"].is_null());
    assert!(!visible(&read_only, "Normalized value"));
    assert_eq!(read_only["project"], project);
    assert_eq!(read_only["history"], history);
    app.quit();
}
