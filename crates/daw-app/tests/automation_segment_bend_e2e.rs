//! Bend the rendered line directly in every automation editing surface.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationLane, AutomationParameter, AutomationPoint, AutomationTarget, Clip,
    Project,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Surface {
    Arrangement,
    Pattern,
    Editor,
}

fn lane(state: &Value, track: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == track)
        .unwrap()["automation"]
}

fn curve(state: &Value, track: u64, surface: Surface) -> &Value {
    let lane = lane(state, track);
    if matches!(surface, Surface::Pattern) {
        &lane["clips"][0]
    } else {
        lane
    }
}

fn diagram(state: &Value, track: u64) -> &Value {
    state["timeline"]["automation"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["track"] == track)
        .unwrap()
}

fn fixture(surface: Surface, dark: bool, density: f64) -> (Harness, u64) {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let audio = app.directory.path().join("held.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.25; 48_000 * 6],
    }
    .write_wav(&audio)
    .unwrap();
    project.track_mut(source).unwrap().clips.push(Clip::audio(
        "Held source",
        0.0,
        12.0,
        audio.to_string_lossy(),
    ));
    let track = project
        .add_automation_track(
            "Shape volume",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let points = |start| {
        vec![
            AutomationPoint::new(start, 0.2),
            AutomationPoint::new(start + 4.0, 0.8),
            AutomationPoint::new(start + 8.0, 0.3),
        ]
    };
    let lane = project.automation_lane_mut(track).unwrap();
    // Editing one segment must not replace the whole lane's interpolation.
    lane.points = points(2.0);
    if matches!(surface, Surface::Pattern) {
        project.create_automation_clip(track, 1.0, 10.0).unwrap();
        let clip = &mut project.automation_lane_mut(track).unwrap().clips[0];
        clip.points = points(0.0);
    }
    let path = app.directory.path().join("bend-line.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    app.command(
        json!({"command":"viewport", "width":1200, "height":900, "pixels_per_point":density}),
    );
    if matches!(surface, Surface::Editor) {
        app.click("Shape volume");
        app.click("Window");
        app.click("Automation");
    } else {
        // Arrangement lanes and patterns edit their curves with Pencil.
        app.click("Pencil");
    }
    (app, track)
}

fn at_line_quarter(state: &Value, track: u64, surface: Surface, segment: usize) -> (f64, f64) {
    let data = curve(state, track, surface);
    let a = &data["points"][segment];
    let b = &data["points"][segment + 1];
    let beat = a["beat"].as_f64().unwrap() * 0.75 + b["beat"].as_f64().unwrap() * 0.25;
    let value = a["value"].as_f64().unwrap() * 0.75 + b["value"].as_f64().unwrap() * 0.25;
    let (rect, start, ppb) = match surface {
        Surface::Arrangement => (
            &diagram(state, track)["rect"],
            state["timeline"]["start_beat"].as_f64().unwrap(),
            state["timeline"]["pixels_per_beat"].as_f64().unwrap(),
        ),
        Surface::Pattern => (
            &diagram(state, track)["clips"][0]["plot"],
            data["offset"].as_f64().unwrap(),
            state["timeline"]["pixels_per_beat"].as_f64().unwrap(),
        ),
        Surface::Editor => (
            &state["automation_editor"]["plot"],
            state["automation_editor"]["start_beat"].as_f64().unwrap(),
            state["automation_editor"]["pixels_per_beat"]
                .as_f64()
                .unwrap(),
        ),
    };
    let left = rect["min"][0].as_f64().unwrap();
    let top = rect["min"][1].as_f64().unwrap();
    let bottom = rect["max"][1].as_f64().unwrap();
    (left + (beat - start) * ppb, bottom - value * (bottom - top))
}

fn history(app: &mut Harness, action: &str) -> Value {
    app.click("Edit");
    app.click(action)
}

fn direct_bends_preserve_endpoints(surface: Surface) {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let (mut app, track) = fixture(surface, dark, density);
        let initial = app.inspect();
        assert_eq!(
            curve(&initial, track, surface)["interpolation"],
            "Linear",
            "New curves must start with straight segments"
        );
        for (segment, delta) in [(0, 18.0), (0, -18.0), (1, 18.0), (1, -18.0)] {
            let before = app.inspect();
            assert_eq!(before["project"], initial["project"]);
            let from = at_line_quarter(&before, track, surface, segment);
            // The quarter segment is away from both endpoint circles and the
            // midpoint diamond: this exercises dragging the line itself.
            app.command(json!({"command":"pointer", "phase":"down", "x":from.0, "y":from.1}));
            app.command(
                json!({"command":"pointer", "phase":"move", "x":from.0, "y":from.1+delta*0.5}),
            );
            app.command(json!({"command":"pointer", "phase":"move", "x":from.0, "y":from.1+delta}));
            app.command(json!({"command":"pointer", "phase":"up", "x":from.0, "y":from.1+delta}));
            let after = app.inspect();
            let original = curve(&before, track, surface);
            let changed = curve(&after, track, surface);
            let amount = changed["points"][segment]["curve"].as_f64().unwrap();
            let expected_sign = if segment == 0 {
                delta.signum()
            } else {
                -delta.signum()
            };
            assert!(
                amount * expected_sign > 0.05 && amount.abs() <= 1.0,
                "{surface:?} segment {segment} delta {delta}: curve {amount}"
            );
            assert_eq!(changed["interpolation"], original["interpolation"]);
            let mut expected = original["points"].clone();
            expected[segment]["curve"] = json!(amount);
            expected[segment]["interpolation"] = json!("Spline");
            assert_eq!(
                changed["points"], expected,
                "Only the chosen outgoing shape may change"
            );
            if matches!(surface, Surface::Pattern) {
                assert_eq!(
                    lane(&after, track)["points"],
                    lane(&before, track)["points"]
                );
            }
            let parsed: Vec<AutomationPoint> =
                serde_json::from_value(changed["points"].clone()).unwrap();
            let mut evaluator: AutomationLane =
                serde_json::from_value(lane(&after, track).clone()).unwrap();
            evaluator.clips.clear();
            evaluator.points = parsed.clone();
            for point in &parsed {
                let value = evaluator.arrangement_value_at(point.beat).unwrap();
                assert!(
                    (value - point.value).abs() < 1e-6,
                    "Bending moved an endpoint"
                );
            }
            assert_eq!(
                after["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1,
                "One drag must produce one undo action"
            );
            assert_eq!(history(&mut app, "Undo")["project"], before["project"]);
            assert_eq!(history(&mut app, "Redo")["project"], after["project"]);
            if segment == 1 && delta < 0.0 {
                let local_beat = parsed[segment].beat * 0.75 + parsed[segment + 1].beat * 0.25;
                let value = evaluator.arrangement_value_at(local_beat).unwrap();
                let song_beat = local_beat
                    + if matches!(surface, Surface::Pattern) {
                        changed["start"].as_f64().unwrap() - changed["offset"].as_f64().unwrap()
                    } else {
                        0.0
                    };
                let wav = app.directory.path().join("bent.wav");
                app.command(json!({"command":"export", "path":wav}));
                app.command(json!({"command":"wait_export"}));
                let audio = AudioData::load_wav(wav).unwrap();
                let frame = (song_beat * 24_000.0).round() as usize;
                for channel in 0..2 {
                    assert!(
                        (audio.samples[frame * 2 + channel] - 0.25 * 1.5 * value).abs() < 0.00003,
                        "{surface:?}: exported audio must use the displayed segment bend"
                    );
                }
                let path = app.directory.path().join("bent-saved.rill");
                app.command(json!({"command":"save", "path":path}));
                app = app.restart();
                let recalled = app.command(json!({"command":"open", "path":path}));
                assert_eq!(lane(&recalled, track), lane(&after, track));
            } else {
                assert_eq!(history(&mut app, "Undo")["project"], before["project"]);
            }
        }
        app.quit();
    }
}

#[test]
fn direct_arranger_lines_bend_in_both_directions_without_moving_neighbors() {
    direct_bends_preserve_endpoints(Surface::Arrangement);
}

#[test]
fn direct_pattern_lines_bend_in_both_directions_without_changing_the_base_lane() {
    direct_bends_preserve_endpoints(Surface::Pattern);
}

#[test]
fn direct_editor_lines_bend_in_both_directions_and_survive_undo_and_reload() {
    direct_bends_preserve_endpoints(Surface::Editor);
}
