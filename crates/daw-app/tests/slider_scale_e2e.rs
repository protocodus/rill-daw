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

/// These assertions observe real painter shapes, independently of the
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
    let has_landmark = |point: Point| {
        state["painted_circles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|circle| {
                Point::from(&circle["center"]).distance(point) <= 0.7
                    && (0.5..=5.0).contains(&circle["radius"].as_f64().unwrap())
                    && (circle["fill"][3].as_u64().unwrap() > 0
                        || circle["stroke_width"].as_f64().unwrap() > 0.0)
            })
            || state["painted_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| {
                    let points = path["points"].as_array().unwrap();
                    if points.len() != 3
                        || path["closed"] != true
                        || path["fill"][3].as_u64().unwrap() == 0
                    {
                        return false;
                    }
                    let mut minimum = Point {
                        x: f64::INFINITY,
                        y: f64::INFINITY,
                    };
                    let mut maximum = Point {
                        x: f64::NEG_INFINITY,
                        y: f64::NEG_INFINITY,
                    };
                    for vertex in points {
                        let vertex = Point::from(vertex);
                        minimum.x = minimum.x.min(vertex.x);
                        minimum.y = minimum.y.min(vertex.y);
                        maximum.x = maximum.x.max(vertex.x);
                        maximum.y = maximum.y.max(vertex.y);
                    }
                    minimum.between(maximum, 0.5).distance(point) <= 0.7
                        && (1.0..=10.0).contains(&(maximum.x - minimum.x))
                        && (1.0..=10.0).contains(&(maximum.y - minimum.y))
                })
    };
    for tick in ticks {
        let position = scale.min.between(scale.max, *tick);
        assert!(
            has_cross_line(position)
                || (position.distance(scale.handle) <= 0.7 && has_landmark(position)),
            "Missing painted tick {tick} on {scale:?}"
        );
    }
    assert!(
        has_cross_line(scale.handle) || has_landmark(scale.handle),
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
fn mixer_fader_range_ticks_and_handle_index_match_mouse_edits_at_multiple_sizes() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let id = fixture(&mut app);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        app.click("Mixer");
        // Mixer Pan is a rotary control; mixer_pan_knob_e2e checks its
        // calibrated arc, painted index, captions and relative mouse edits.
        let mut lengths = Vec::new();
        // At 600 px the larger shared control rows hold the fader at its
        // usable minimum. Give the tall case room beyond those fixed rows.
        for (height, fractions) in [
            (600, vec![0.0, 0.2, 0.4, 0.5, 0.6, 0.8, 1.0]),
            (1300, (0..=10).map(|i| f64::from(i) / 10.0).collect()),
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
fn arrangement_controls_keep_current_values_and_native_endpoint_mapping_without_micro_scales() {
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
            for name in ["gain_scale", "pan_scale"] {
                assert_eq!(
                    channel[name]["ticks"],
                    json!([]),
                    "Header rails should avoid miniature scales"
                );
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
                let channel = arrangement_channel(&state, id);
                let scale = Scale::from(&channel["pan_scale"]);
                assert!(
                    scale
                        .handle
                        .distance(scale.min.between(scale.max, fraction))
                        <= 0.7
                );
                let caption = if fraction == 0.0 {
                    "L100"
                } else if fraction == 1.0 {
                    "R100"
                } else {
                    "C"
                };
                let rect = &channel["pan_readout"];
                assert!(
                    state["painted_text"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|label| {
                            label["text"] == caption
                                && label["rect"]["min"][0].as_f64().unwrap()
                                    >= rect["min"][0].as_f64().unwrap() - 0.5
                                && label["rect"]["max"][0].as_f64().unwrap()
                                    <= rect["max"][0].as_f64().unwrap() + 0.5
                                && label["rect"]["max"][1].as_f64().unwrap()
                                    <= rect["max"][1].as_f64().unwrap() + 0.5
                        }),
                    "The current pan value must remain in its own readout"
                );
            }
            for fraction in [0.0, 0.5, 1.0] {
                let gain = Scale::from(&arrangement_channel(&painted(&mut app), id)["gain_scale"]);
                let edited = click_fraction(&mut app, gain, fraction);
                let db = -60.0 + fraction * (60.0 + f64::from(20.0 * 1.5_f32.log10()));
                let expected = if fraction == 0.0 {
                    0.0
                } else {
                    10.0_f64.powf(db / 20.0)
                };
                near(track(&edited, id)["gain"].as_f64().unwrap(), expected, 1e-5);
                let after = painted(&mut app);
                let gain = Scale::from(&arrangement_channel(&after, id)["gain_scale"]);
                assert!(gain.handle.distance(gain.min.between(gain.max, fraction)) <= 0.7);
            }
            assert!(
                pan.length() > 20.0,
                "Even compact controls retain usable travel: {pan:?}"
            );
        }
        app.quit();
    }
}
