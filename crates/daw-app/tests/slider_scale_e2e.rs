//! Native slider paint and mouse edits share exact range and handle positions.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
struct Point {
    x: f64,
    y: f64,
}
impl Point {
    fn from(value: &Value) -> Self {
        Self {
            x: value[0].as_f64().unwrap(),
            y: value[1].as_f64().unwrap(),
        }
    }
    fn between(self, other: Self, fraction: f64) -> Self {
        Self {
            x: self.x + (other.x - self.x) * fraction,
            y: self.y + (other.y - self.y) * fraction,
        }
    }
    fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug)]
struct Scale {
    min: Point,
    max: Point,
    handle: Point,
}
impl Scale {
    fn from(value: &Value) -> Self {
        assert!(value.is_object(), "Missing slider scale: {value}");
        Self {
            min: Point::from(&value["min_position"]),
            max: Point::from(&value["max_position"]),
            handle: Point::from(&value["handle_center"]),
        }
    }
    fn vertical(self) -> bool {
        (self.max.y - self.min.y).abs() > (self.max.x - self.min.x).abs()
    }
    fn length(self) -> f64 {
        self.min.distance(self.max)
    }
}

fn mixer_channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}
fn arrangement_channel(state: &Value, id: u64) -> &Value {
    state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}
fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}
fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} (±{tolerance})"
    );
}

fn caption_below(state: &Value, scale: Scale, text: &str) {
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                if label["text"] != text {
                    return false;
                }
                let min = Point::from(&label["min"]);
                let max = Point::from(&label["max"]);
                let center = min.between(max, 0.5);
                min.x >= scale.min.x - 10.0
                    && max.x <= scale.max.x + 10.0
                    && center.y > scale.min.y
                    && center.y < scale.min.y + 24.0
            }),
        "Caption {text:?} must be visible under {scale:?}: {}",
        state["visible_labels"]
    );
}

fn caption_left_at(state: &Value, point: Point, text: &str) {
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                if label["text"] != text {
                    return false;
                }
                let min = Point::from(&label["min"]);
                let max = Point::from(&label["max"]);
                let center = min.between(max, 0.5);
                max.x < point.x
                    && point.x - max.x < 40.0
                    && (center.y - point.y).abs() < 1.0
                    && max.y - min.y >= 10.0
            }),
        "Endpoint {text:?} must be visibly aligned to {point:?}"
    );
}
fn click_fraction(app: &mut Harness, scale: Scale, fraction: f64) -> Value {
    let point = scale.min.between(scale.max, fraction);
    app.command(json!({"command":"pointer", "phase":"click", "x":point.x, "y":point.y}))
}

/// These assertions observe real painter lines/rectangles, independently of the
/// geometry returned by the component. Pointer edits below also prove that the
/// displayed positions address the parameter's actual native hit target.
fn assert_paint_alignment(state: &Value, scale: Scale, fraction: f64, ticks: &[f64]) {
    let expected = scale.min.between(scale.max, fraction);
    assert!(
        scale.handle.distance(expected) <= 0.7,
        "Handle {scale:?} must be centered on {fraction}"
    );
    let lines = state["painted_lines"]
        .as_array()
        .expect("opt-in painted line capture");
    let has_cross_line = |point: Point| {
        lines.iter().any(|line| {
            let a = Point::from(&line["points"][0]);
            let b = Point::from(&line["points"][1]);
            let width = a.distance(b);
            if !(2.0..=40.0).contains(&width) {
                return false;
            }
            if scale.vertical() {
                (a.y - point.y).abs() <= 0.7
                    && (b.y - point.y).abs() <= 0.7
                    && a.x.min(b.x) <= point.x + 12.0
                    && a.x.max(b.x) >= point.x - 12.0
            } else {
                (a.x - point.x).abs() <= 0.7
                    && (b.x - point.x).abs() <= 0.7
                    && a.y.min(b.y) <= point.y + 12.0
                    && a.y.max(b.y) >= point.y - 12.0
            }
        })
    };
    for tick in ticks {
        assert!(
            has_cross_line(scale.min.between(scale.max, *tick)),
            "Missing painted tick {tick} on {scale:?}"
        );
    }
    assert!(
        has_cross_line(scale.handle),
        "The handle needs a visible center index: {scale:?}"
    );
    assert!(
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| {
                let min = Point::from(&shape["rect"]["min"]);
                let max = Point::from(&shape["rect"]["max"]);
                let center = min.between(max, 0.5);
                (4.0..=40.0).contains(&(max.x - min.x))
                    && (4.0..=40.0).contains(&(max.y - min.y))
                    && shape["stroke_width"].as_f64().unwrap_or(0.0) > 0.0
                    && center.distance(scale.handle) <= 0.7
            }),
        "The painted handle must be centered on its index: {scale:?}"
    );
}

fn fixture(app: &mut Harness) -> u64 {
    let mut project = Project::new();
    let id = project.add_audio_track("Measured channel");
    project.track_mut(id).unwrap().gain = 0.75;
    let path = app.directory.path().join("slider-scales.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    id
}

#[test]
fn mixer_ranges_ticks_and_handle_indices_match_mouse_edits_at_multiple_sizes() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let id = fixture(&mut app);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        app.click("Mixer");
        let initial = painted(&mut app);
        let pan = Scale::from(&mixer_channel(&initial, id)["pan_scale"]);
        caption_below(&initial, pan, "L");
        caption_below(&initial, pan, "R");
        caption_below(&initial, pan, "Pan C");
        assert_paint_alignment(&initial, pan, 0.5, &[0.0, 0.5, 1.0]);
        for fraction in [0.0, 0.5, 1.0] {
            let pan = Scale::from(&mixer_channel(&painted(&mut app), id)["pan_scale"]);
            let edited = click_fraction(&mut app, pan, fraction);
            near(
                track(&edited, id)["pan"].as_f64().unwrap(),
                fraction * 2.0 - 1.0,
                1e-5,
            );
            let after = painted(&mut app);
            let pan = Scale::from(&mixer_channel(&after, id)["pan_scale"]);
            assert_paint_alignment(&after, pan, fraction, &[0.0, 0.5, 1.0]);
            caption_below(
                &after,
                pan,
                if fraction == 0.0 {
                    "Pan L100"
                } else if fraction == 1.0 {
                    "Pan R100"
                } else {
                    "Pan C"
                },
            );
        }
        let mut lengths = Vec::new();
        for (height, fractions) in [
            (600, vec![0.0, 0.2, 0.4, 0.5, 0.6, 0.8, 1.0]),
            (1100, (0..=10).map(|i| f64::from(i) / 10.0).collect()),
        ] {
            app.command(json!({"command":"viewport", "width":1100, "height":height}));
            let state = painted(&mut app);
            let fader = Scale::from(&mixer_channel(&state, id)["fader_scale"]);
            caption_left_at(&state, fader.min, "−∞");
            caption_left_at(&state, fader.max, "+3.5 dB");
            lengths.push(fader.length());
            for fraction in [0.0, 0.2, 0.5, 0.8, 1.0] {
                let fader = Scale::from(&mixer_channel(&painted(&mut app), id)["fader_scale"]);
                let edited = click_fraction(&mut app, fader, fraction);
                near(
                    track(&edited, id)["gain"].as_f64().unwrap(),
                    fraction * 1.5,
                    1e-5,
                );
                let state = painted(&mut app);
                assert_paint_alignment(
                    &state,
                    Scale::from(&mixer_channel(&state, id)["fader_scale"]),
                    fraction,
                    &fractions,
                );
            }
        }
        assert!(
            lengths[1] > lengths[0] + 300.0,
            "The fader should use additional height: {lengths:?}"
        );
        app.quit();
    }
}

#[test]
fn arrangement_compact_controls_keep_range_and_current_value_visible_and_editable() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let id = fixture(&mut app);
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}),
        );
        for compact in [false, true] {
            if compact {
                app.click("Measured channel");
                app.click("Window");
                app.click("Properties");
                app.click("Compact track");
                app.key("Escape");
            }
            let state = painted(&mut app);
            let channel = arrangement_channel(&state, id);
            assert_eq!(channel["compact"], compact);
            let pan = Scale::from(&channel["pan_scale"]);
            caption_below(&state, pan, "L");
            caption_below(&state, pan, "R");
            if compact {
                let gain = Scale::from(&channel["gain_scale"]);
                caption_below(&state, gain, "−∞");
                caption_below(&state, gain, "+3.5");
            }
            for fraction in [0.0, 0.5, 1.0] {
                let pan = Scale::from(&arrangement_channel(&painted(&mut app), id)["pan_scale"]);
                let edited = click_fraction(&mut app, pan, fraction);
                near(
                    track(&edited, id)["pan"].as_f64().unwrap(),
                    fraction * 2.0 - 1.0,
                    1e-5,
                );
                let state = painted(&mut app);
                assert_paint_alignment(
                    &state,
                    Scale::from(&arrangement_channel(&state, id)["pan_scale"]),
                    fraction,
                    &[0.0, 0.5, 1.0],
                );
                if compact {
                    caption_below(
                        &state,
                        Scale::from(&arrangement_channel(&state, id)["pan_scale"]),
                        if fraction == 0.0 {
                            "L100"
                        } else if fraction == 1.0 {
                            "R100"
                        } else {
                            "C"
                        },
                    );
                }
            }
            assert!(
                pan.length() > 20.0,
                "Even compact controls retain usable travel: {pan:?}"
            );
        }
        app.quit();
    }
}
