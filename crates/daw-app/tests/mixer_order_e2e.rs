//! Signal-graph presentation and explicit song ordering through the running UI.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn rect_center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn inside(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 1.0
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn channels(state: &Value) -> &[Value] {
    state["mixer"]["channels"].as_array().unwrap()
}

fn order(state: &Value) -> Vec<u64> {
    channels(state)
        .iter()
        .map(|channel| channel["track"].as_u64().unwrap())
        .collect()
}

fn unchanged(state: &Value, baseline: &Value) {
    for field in [
        "project",
        "history",
        "revision",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            state[field], baseline[field],
            "Mixer presentation changed {field}"
        );
    }
}

fn choose_order(app: &mut Harness, current: &str, next: &str) -> Value {
    app.click(current);
    let state = app.click(next);
    assert_eq!(state["mixer"]["order"], next);
    state
}

fn layout(state: &Value, width: f64, height: f64) {
    let viewport = json!({"min":[0.0, 0.0], "max":[width, height]});
    assert!(inside(&viewport, &state["mixer"]["reorder_viewport"]));
    let choice = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == state["mixer"]["order"])
        .expect("The current ordering stays mouse-reachable above the mixer scroll pane");
    assert!(inside(&viewport, choice));
    for pair in channels(state).windows(2) {
        assert!(
            pair[0]["strip"]["max"][0].as_f64().unwrap()
                < pair[1]["strip"]["min"][0].as_f64().unwrap(),
            "Adjacent strips must not overlap"
        );
    }
    let master = &state["mixer"]["master_strip"];
    assert!(
        channels(state).last().unwrap()["strip"]["max"][0]
            .as_f64()
            .unwrap()
            < master["min"][0].as_f64().unwrap(),
        "Master stays after the downstream buses"
    );
    for channel in channels(state) {
        for pair in ["input", "processing", "measuring", "output"].windows(2) {
            assert!(
                channel["stages"][pair[0]]["max"][1].as_f64().unwrap()
                    <= channel["stages"][pair[1]]["min"][1].as_f64().unwrap() + 1.0,
                "Channel stages remain in signal order"
            );
        }
    }
}

#[test]
fn signal_flow_orders_nested_main_and_send_routes_without_editing_the_song() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        // Deliberately place downstream and unused buses before their sources.
        let final_bus = project.add_bus("Final bus");
        let audio = project.add_audio_track("Audio source");
        let unused_bus = project.add_bus("Unused bus");
        let return_bus = project.add_bus("Return bus");
        let midi = project.add_midi_track("MIDI source");
        let first_bus = project.add_bus("First bus");
        project.set_output(audio, Some(first_bus)).unwrap();
        project.add_send(audio, first_bus, 0.25, false).unwrap();
        project.add_send(midi, return_bus, 0.5, true).unwrap();
        project
            .add_send(first_bus, return_bus, 0.25, false)
            .unwrap();
        project.set_output(return_bus, Some(final_bus)).unwrap();
        let song = vec![final_bus, audio, unused_bus, return_bus, midi, first_bus];
        let flow = vec![audio, midi, unused_bus, first_bus, return_bus, final_bus];
        let path = app.directory.path().join("mixer-order.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1600, "height":1100,
            "pixels_per_point":density}));
        let baseline = app.click("Mixer");
        assert_eq!(baseline["mixer"]["order"], "Signal flow");
        assert_eq!(order(&baseline), flow);
        assert_eq!(baseline["mixer"]["reorder_enabled"], false);
        assert!(
            channels(&baseline)
                .iter()
                .all(|channel| channel["reorder_source"].is_null() && channel["drop"].is_null())
        );
        assert_eq!(
            channels(&baseline)
                .iter()
                .map(|channel| channel["channel"].clone())
                .collect::<Vec<_>>(),
            vec![json!(2), json!(5), json!(3), json!(6), json!(4), json!(1)],
            "Channel numbers retain song identities when strip positions change"
        );
        assert_eq!(
            baseline["mixer"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|group| group["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Tracks", "Buses", "Output"]
        );
        layout(&baseline, 1600.0, 1100.0);

        // Dragging is deliberately inactive in the graph-derived view.
        let from = rect_center(&channels(&baseline)[0]["name"]);
        let to = rect_center(&channels(&baseline)[1]["name"]);
        let dragged = app.command(json!({"command":"pointer", "phase":"drag",
            "x":from.0, "y":from.1, "to_x":to.0, "to_y":to.1}));
        assert!(dragged["mixer"]["reorder_preview"].is_null());
        assert_eq!(order(&dragged), flow);
        unchanged(&dragged, &baseline);

        let manual = choose_order(&mut app, "Signal flow", "Song order");
        assert_eq!(order(&manual), song);
        assert_eq!(manual["mixer"]["reorder_enabled"], true);
        assert!(
            channels(&manual)
                .iter()
                .all(|channel| !channel["reorder_source"].is_null() && !channel["drop"].is_null())
        );
        unchanged(&manual, &baseline);
        let restored = choose_order(&mut app, "Song order", "Signal flow");
        assert_eq!(order(&restored), flow);
        unchanged(&restored, &baseline);

        // The ordering choice stays in the fixed header on a narrow screen;
        // horizontal scrolling exposes the final Master without editing routes.
        let narrow = app.command(json!({"command":"viewport", "width":800, "height":600,
            "pixels_per_point":density}));
        layout(&narrow, 800.0, 600.0);
        unchanged(&narrow, &baseline);
        let pane = &narrow["mixer"]["reorder_viewport"];
        app.command(
            json!({"command":"scroll", "x":pane["min"][0].as_f64().unwrap() + 20.0,
            "y":pane["min"][1].as_f64().unwrap() + 20.0, "delta":0.0, "delta_x":-10000.0}),
        );
        for _ in 0..20 {
            app.inspect();
        }
        let scrolled = app.inspect();
        let master = &scrolled["mixer"]["master_strip"];
        let pane = &scrolled["mixer"]["reorder_viewport"];
        assert!(master["min"][0].as_f64().unwrap() >= pane["min"][0].as_f64().unwrap() - 1.0);
        assert!(master["max"][0].as_f64().unwrap() <= pane["max"][0].as_f64().unwrap() + 1.0);
        unchanged(&scrolled, &baseline);
        let narrow_manual = choose_order(&mut app, "Signal flow", "Song order");
        assert_eq!(order(&narrow_manual), song);
        unchanged(&narrow_manual, &baseline);
        app.quit();
    }
}
