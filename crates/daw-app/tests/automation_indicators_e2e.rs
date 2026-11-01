//! Effective native controls, visible takeover, and mouse-only resume paths.
mod common;
use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn lane(state: &Value, id: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()["automation"]
}
fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}
fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn enter_number(app: &mut Harness, rect: &Value, text: &str) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}));
    app.key("Enter")
}

#[test]
fn native_pan_send_and_arranger_indicate_override_and_resume_from_visible_controls() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Source");
        let bus = project.add_bus("Room");
        project.add_send(source, bus, 0.25, false).unwrap();
        let pan = project
            .add_automation_track(
                "Pan envelope",
                AutomationTarget {
                    track: source,
                    parameter: AutomationParameter::TrackPan,
                },
            )
            .unwrap();
        let send = project
            .add_automation_track(
                "Room envelope",
                AutomationTarget {
                    track: source,
                    parameter: AutomationParameter::SendGain { bus },
                },
            )
            .unwrap();
        project.automation_lane_mut(pan).unwrap().points = vec![
            AutomationPoint::new(2.0, 0.75),
            AutomationPoint::new(4.0, 0.25),
        ];
        project.automation_lane_mut(send).unwrap().points = vec![
            AutomationPoint::new(2.0, 0.05),
            AutomationPoint::new(4.0, 0.10),
        ];
        let original = project.clone();
        let path = app.directory.path().join("indicator-controls.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":850,"pixels_per_point":density}),
        );
        app.click("Mixer");
        let properties = app.click("Source");
        // The first point applies before its own beat; merely opening a control
        // must not write that automated value into its stored manual setting.
        assert_eq!(properties["project"]["tracks"][0]["pan"], 0.0);
        label(&properties, "Pan automation");
        let panned = enter_number(&mut app, label(&properties, "50.0%"), "25");
        assert!((lane(&panned, pan)["override_value"].as_f64().unwrap() - 0.625).abs() < 1e-6);
        label(&panned, "Overridden");
        let mixer = app.click("Mixer");
        label(&mixer, "Overridden");
        let source_kind = &mixer["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == source)
            .unwrap()["source_label"];
        assert!(
            source_kind["max"][1].as_f64().unwrap() - source_kind["min"][1].as_f64().unwrap()
                < 20.0,
            "Automation status must not wrap the Audio source kind"
        );
        app.click("Source");
        let resumed = menu(&mut app, "Overridden", "Resume automation");
        assert!(lane(&resumed, pan)["override_value"].is_null());
        label(&resumed, "50.0%");
        assert_eq!(resumed["project"]["tracks"][0]["pan"], 0.25);
        assert_eq!(
            lane(&resumed, pan)["points"],
            serde_json::to_value(&original.automation_lane(pan).unwrap().points).unwrap()
        );

        let editing = menu(&mut app, "Automated", "Edit automation");
        assert_eq!(editing["popups"]["automation"], true);
        assert_eq!(
            editing["popups"]["properties"], false,
            "Edit automation opens one workspace"
        );
        assert_eq!(editing["selected_track"], pan);
        app.click("Mixer");
        app.click("Source");
        let routing = app.click("Routing");
        let level_y = center(label(&routing, "Level")).1;
        let numeric = routing["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|rect| {
                rect["text"]
                    .as_str()
                    .and_then(|text| text.strip_suffix('×'))
                    .and_then(|text| text.parse::<f64>().ok())
                    .is_some()
                    && (center(rect).1 - level_y).abs() < 4.0
            })
            .max_by(|a, b| center(a).0.total_cmp(&center(b).0))
            .unwrap_or_else(|| panic!("No send value: {}", routing["visible_labels"]));
        // Send readouts now open deliberate exact entry with a single click.
        let (x, y) = center(numeric);
        let exact = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        label(&exact, "Source → Room · Send level");
        app.click("123");
        app.click("Clear");
        for key in ["0", ".", "4"] {
            app.click(key);
        }
        let changed = app.click("Apply value");
        assert!((lane(&changed, send)["override_value"].as_f64().unwrap() - 0.025).abs() < 1e-6);
        label(&changed, "Overridden");
        let resumed = menu(&mut app, "Overridden", "Resume automation");
        assert!(lane(&resumed, send)["override_value"].is_null());
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert!(lane(&undone, send)["override_value"].is_number());
        app.click("Arranger");
        let state = app.inspect();
        label(&state, "Overridden");
        // Lane status opens the same resume action without opening an editor.
        let resumed = menu(&mut app, "Overridden", "Resume automation");
        assert!(lane(&resumed, send)["override_value"].is_null());
        assert_eq!(
            lane(&resumed, send)["points"],
            serde_json::to_value(&original.automation_lane(send).unwrap().points).unwrap()
        );
        app.quit();
    }
}
