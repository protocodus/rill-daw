//! Workspace views, transport commands and playback guides stay distinct.
mod common;

use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn background<'a>(state: &'a Value, text: &str) -> &'a Value {
    let [left, top, right, bottom] = bounds(label(state, text));
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| {
            let [x0, y0, x1, y1] = bounds(&shape["rect"]);
            x0 <= left + 0.5
                && y0 <= top + 0.5
                && x1 >= right - 0.5
                && y1 >= bottom - 0.5
                && x1 - x0 <= right - left + 90.0
                && y1 - y0 >= 24.0
                && y1 - y0 <= 44.0
        })
        .min_by(|a, b| {
            let area = |shape: &Value| {
                let [x0, y0, x1, y1] = bounds(&shape["rect"]);
                (x1 - x0) * (y1 - y0)
            };
            area(a).total_cmp(&area(b))
        })
        .unwrap_or_else(|| panic!("No control background for {text}"))
}

fn same_row(state: &Value, labels: &[&str]) {
    let centers: Vec<_> = labels
        .iter()
        .map(|text| {
            let [_, top, _, bottom] = bounds(label(state, text));
            (top + bottom) * 0.5
        })
        .collect();
    assert!(
        centers
            .iter()
            .all(|center| (center - centers[0]).abs() <= 2.0),
        "Related controls split across rows: {labels:?}: {centers:?}"
    );
}

#[test]
fn main_bar_keeps_related_controls_together_and_distinguishes_commands_from_views() {
    let mut app = Harness::new();
    let before = app.inspect();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1100, 1480, 1600] {
            app.command(json!({"command":"viewport", "width":width,
                "height":900, "pixels_per_point":density}));
            app.command(json!({"command":"pointer", "phase":"move", "x":780.0, "y":880.0}));
            let state = app.command(json!({"command":"inspect", "paint":true}));
            for field in ["project", "history", "selected_track", "selected_clip"] {
                assert_eq!(
                    state[field], before[field],
                    "Toolbar layout changed {field}"
                );
            }
            same_row(&state, &["Arranger", "Mixer"]);
            same_row(&state, &["Start", "Play", "Stop", "Record"]);
            same_row(&state, &["001 . 01 . 00", "120.0 BPM", "4/4"]);
            same_row(&state, &["Click", "Loop", "Range"]);
            if width >= 1100 {
                same_row(
                    &state,
                    &["Arranger", "Start", "120.0 BPM", "Click", "Range"],
                );
            }
            let mut rectangles: Vec<(&str, [f64; 4])> = Vec::new();
            for text in [
                "Arranger", "Mixer", "Start", "Play", "Stop", "Record", "Click", "Loop", "Range",
            ] {
                let control = background(&state, text);
                let rect = bounds(&control["rect"]);
                assert!(rect[0] >= 0.0 && rect[1] >= 40.0, "{text}: {rect:?}");
                assert!(
                    rect[2] <= f64::from(width) && rect[3] < 240.0,
                    "{text}: {rect:?}"
                );
                for (other, [left, top, right, bottom]) in &rectangles {
                    assert!(
                        rect[2] <= *left
                            || rect[0] >= *right
                            || rect[3] <= *top
                            || rect[1] >= *bottom,
                        "{text} overlaps {other} at width {width}"
                    );
                }
                rectangles.push((text, rect));
            }
            for text in ["Start", "Play", "Stop", "Record"] {
                let control = background(&state, text);
                assert!(
                    control["fill"][3].as_u64().unwrap() > 0,
                    "{text} is a filled command"
                );
                assert_eq!(control["stroke_width"], 0.0);
            }
            for text in ["Mixer", "Click", "Loop"] {
                let control = background(&state, text);
                assert_eq!(
                    control["fill"][3], 0,
                    "Idle {text} uses the quiet view outline"
                );
                assert!(control["stroke_width"].as_f64().unwrap() > 0.0);
            }
            let tool = bounds(label(&state, "Select"));
            assert!(
                rectangles.iter().all(|(_, rect)| rect[3] <= tool[1]),
                "Editor tools stay in their own row below the main bar"
            );
        }
    }
    app.quit();
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn view_navigation_and_playback_guides_preserve_their_existing_transport_and_edit_semantics() {
    let mut app = Harness::with_virtual_audio(None, None);
    let path = app.directory.path().join("Toolbar audio.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| {
                let sample = (frame as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin() * 0.2;
                [sample, sample * 0.5]
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    app.command(json!({"command":"drop", "paths":[path]}));
    let before = app.key("E");
    let baseline = export(&mut app, "Before guides.wav");
    for view in ["Mixer", "Arranger"] {
        let state = app.click(view);
        for field in ["project", "history", "selected_track", "selected_clip"] {
            assert_eq!(state[field], before[field]);
        }
    }
    assert_eq!(app.inspect()["timeline"]["tool"], "Cut");
    app.click("Play");
    let deadline = Instant::now() + Duration::from_secs(10);
    let playing = loop {
        let state = app.inspect();
        if state["engine"]["playing"] == true {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "Transport did not start: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let clicked = app.click("Click");
    assert_eq!(clicked["project"]["metronome_enabled"], true);
    assert_eq!(
        clicked["engine"]["engine_id"],
        playing["engine"]["engine_id"]
    );
    assert_eq!(
        clicked["engine"]["generation"],
        playing["engine"]["generation"]
    );
    app.click("Pause");
    app.click("Start");
    let stopped = app.click("Stop");
    assert_eq!(stopped["timeline"]["cursor"], 0.0);
    let looped = app.click("Loop");
    assert_eq!(looped["project"]["loop_enabled"], true);
    let range = app.click("Range");
    assert!(
        range["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text
                .as_str()
                .is_some_and(|text| text.starts_with("Start beat ")))
    );
    assert_eq!(range["project"], looped["project"]);
    assert_eq!(range["history"], looped["history"]);
    app.key("Escape");
    assert_eq!(
        export(&mut app, "With guides.wav").samples,
        baseline.samples,
        "The metronome and live loop guide never enter song export"
    );
    app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    let restored = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(restored["history"]["undo"], before["history"]["undo"]);
    assert_eq!(restored["timeline"]["tool"], "Cut");
    app.quit();
}
