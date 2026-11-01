//! Native percent drafts survive the application's real storage conversions.
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
        .find(|row| row["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text:?}: {}", state["visible_labels"]))
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn retain_draft_then_insert(app: &mut Harness, draft: &str, inserted: &str) -> Value {
    let edited = app.command(json!({"command":"text", "text":draft}));
    label(&edited, draft);
    app.key("ArrowLeft");
    for _ in 0..5 {
        let idle = app.inspect();
        label(&idle, draft);
        for field in ["project", "history", "modified"] {
            assert_eq!(
                idle[field], edited[field],
                "a unit-conversion acknowledgement changed {field}"
            );
        }
    }
    let changed = app.command(json!({"command":"text", "text":"7"}));
    label(&changed, inserted);
    assert_ne!(
        changed["project"], edited["project"],
        "typing at the retained caret must still edit the model"
    );
    let idle = app.inspect();
    label(&idle, inserted);
    assert_eq!(idle["project"], changed["project"]);
    assert_eq!(idle["history"], changed["history"]);
    idle
}

fn undo_to(app: &mut Harness, before: &Value) {
    // The two native text edits may share the existing parameter history group.
    for _ in 0..2 {
        app.click("Edit");
        let undone = app.click("Undo");
        if undone["project"] == before["project"] {
            return;
        }
    }
    panic!("two intentional text edits must undo to the original model");
}

#[test]
fn plugin_percent_draft_and_caret_survive_f64_and_automated_f32_round_trips() {
    let worker = tempfile::tempdir().unwrap();
    let source_file = worker.path().join("parameter_host.rs");
    std::fs::write(&source_file, include_str!("common/parameter_host.rs")).unwrap();
    let host = worker.path().join(if cfg!(windows) {
        "parameter-host.exe"
    } else {
        "parameter-host"
    });
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = std::process::Command::new(compiler)
        .arg("--edition=2024")
        .arg(&source_file)
        .arg("-o")
        .arg(&host)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let plugins = common::plugin_fixture();
    for automated in [false, true] {
        let mut app = Harness::with_plugin_host(plugins.path(), &host);
        let mut project = Project::new();
        let source = project.add_midi_track("Draft synth");
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
            .push(Clip::midi("Draft phrase", 0.0, 4.0, vec![]));
        let lane = automated.then(|| {
            project
                .add_automation_track_with_value(
                    "Filter curve",
                    AutomationTarget {
                        track: source,
                        parameter: AutomationParameter::VstParameter {
                            effect: None,
                            id: 7,
                        },
                    },
                    0.5,
                )
                .unwrap()
        });
        let path = app.directory.path().join("roundtrip-plugin.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"viewport", "width":1100, "height":850}));
        app.click("Draft phrase");
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
                "parameter metadata did not arrive"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let control = &before["native_editor"]["parameter_control"];
        for axis in 0..2 {
            assert!(
                control["number"]["min"][axis].as_f64().unwrap()
                    >= control["clip"]["min"][axis].as_f64().unwrap(),
                "The complete numeric field must remain reachable: {control}"
            );
            assert!(
                control["number"]["max"][axis].as_f64().unwrap()
                    <= control["clip"]["max"][axis].as_f64().unwrap(),
                "The complete numeric field must remain reachable: {control}"
            );
        }
        click_rect(
            &mut app,
            &before["native_editor"]["parameter_control"]["number"],
        );
        let edited = retain_draft_then_insert(&mut app, "61.234500", "61.2345070");
        let stored = track(&edited, source)["instrument_parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == 7)
            .unwrap()["value"]
            .as_f64()
            .unwrap();
        assert!((stored - 0.61234507).abs() < 1e-12);
        if let Some(lane) = lane {
            assert_eq!(
                track(&edited, lane)["automation"]["points"],
                track(&before, lane)["automation"]["points"]
            );
            assert!(track(&edited, lane)["automation"]["override_value"].is_number());
        }
        undo_to(&mut app, &before);
        app.quit();
    }
}

#[test]
fn pan_properties_preserve_native_draft_and_caret_across_f32_storage_and_automation() {
    for automated in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Draft channel");
        project.track_mut(source).unwrap().pan = 0.14;
        let lane = automated.then(|| {
            let lane = project
                .add_automation_track_with_value(
                    "Pan curve",
                    AutomationTarget {
                        track: source,
                        parameter: AutomationParameter::TrackPan,
                    },
                    0.5,
                )
                .unwrap();
            project.automation_lane_mut(lane).unwrap().points = vec![
                AutomationPoint::new(0.0, 0.5),
                AutomationPoint::new(8.0, 0.75),
            ];
            lane
        });
        let path = app.directory.path().join("roundtrip-pan.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"viewport", "width":1100, "height":850}));
        app.click("Mixer");
        let before = app.click("Draft channel");
        let (_, y) = center(label(&before, "Pan"));
        let number = before["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|row| {
                row["text"].as_str().is_some_and(|text| {
                    text.ends_with('%') && text.trim_end_matches('%').parse::<f64>().is_ok()
                }) && (center(row).1 - y).abs() < 4.0
            })
            .expect("visible Pan numeric field");
        click_rect(&mut app, number);
        let edited = retain_draft_then_insert(&mut app, "14.1400", "14.14070");
        assert_eq!(
            track(&edited, source)["pan"].as_f64().unwrap() as f32,
            14.1407_f32 / 100.0
        );
        if let Some(lane) = lane {
            assert_eq!(
                track(&edited, lane)["automation"]["points"],
                track(&before, lane)["automation"]["points"]
            );
            assert!(track(&edited, lane)["automation"]["override_value"].is_number());
        }
        undo_to(&mut app, &before);
        app.quit();
    }
}
