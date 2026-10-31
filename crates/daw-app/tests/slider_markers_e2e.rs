//! Slider landmarks are painted on the real handle and follow native edits.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Marker {
    Minimum,
    Maximum,
    Center,
    Zero,
    Ordinary,
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

fn point(value: &Value) -> [f64; 2] {
    [number(&value[0]), number(&value[1])]
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 0.6
}

fn scale<'a>(state: &'a Value, name: &str) -> &'a Value {
    if state["view"] == "Mixer" {
        &state["mixer"]["channels"][0][name]
    } else {
        &state["timeline"]["channels"][0][name]
    }
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn assert_marker(state: &Value, scale_name: &str, expected: Marker) {
    let geometry = scale(state, scale_name);
    let center = point(&geometry["handle_center"]);
    let triangles: Vec<_> = state["painted_paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|path| {
            let points: Vec<_> = path["points"].as_array()?.iter().map(point).collect();
            if points.len() != 3 || path["closed"] != true || number(&path["fill"][3]) == 0.0 {
                return None;
            }
            let bounds = points.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |[left, top, right, bottom], point| {
                    [
                        left.min(point[0]),
                        top.min(point[1]),
                        right.max(point[0]),
                        bottom.max(point[1]),
                    ]
                },
            );
            let [left, top, right, bottom] = bounds;
            (near([(left + right) * 0.5, (top + bottom) * 0.5], center)
                && (1.0..=10.0).contains(&(right - left))
                && (1.0..=10.0).contains(&(bottom - top)))
            .then_some(points)
        })
        .collect();
    let circles: Vec<_> = state["painted_circles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|circle| {
            near(point(&circle["center"]), center)
                && (0.5..=5.0).contains(&number(&circle["radius"]))
        })
        .collect();
    match expected {
        Marker::Minimum | Marker::Maximum => {
            assert_eq!(
                triangles.len(),
                1,
                "Missing {expected:?} triangle at {center:?}"
            );
            assert!(circles.is_empty(), "Endpoint must take priority over zero");
            let direction = if matches!(expected, Marker::Minimum) {
                1.0
            } else {
                -1.0
            };
            assert_eq!(
                triangles[0]
                    .iter()
                    .filter(|point| (point[1] - center[1]) * direction > 0.1)
                    .count(),
                1,
                "{expected:?} must point {} even on a horizontal slider: {:?}",
                if direction > 0.0 { "down" } else { "up" },
                triangles[0],
            );
        }
        Marker::Center | Marker::Zero => {
            assert!(triangles.is_empty());
            assert_eq!(
                circles.len(),
                1,
                "Missing {expected:?} circle at {center:?}"
            );
            let circle = circles[0];
            if matches!(expected, Marker::Zero) {
                assert_eq!(number(&circle["fill"][3]), 0.0, "Zero must be hollow");
                assert!(number(&circle["stroke"][3]) > 0.0);
                assert!(number(&circle["stroke_width"]) > 0.0);
            } else {
                assert!(number(&circle["fill"][3]) > 0.0, "Midpoint must be filled");
                assert_eq!(number(&circle["stroke_width"]), 0.0);
            }
        }
        Marker::Ordinary => {
            assert!(triangles.is_empty() && circles.is_empty());
            assert!(
                state["painted_lines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|line| {
                        let a = point(&line["points"][0]);
                        let b = point(&line["points"][1]);
                        near([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5], center)
                            && (number(&line["stroke_width"]) - 1.5).abs() < 0.01
                    }),
                "Ordinary values retain a visible center index"
            );
        }
    }
}

#[test]
fn mixer_fader_markers_follow_mouse_edits_and_undo_without_changing_stored_values() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let track = project.add_audio_track("Slider landmarks");
        project.track_mut(track).unwrap().gain = 0.6;
        let path = app.directory.path().join("slider-landmarks.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":900, "pixels_per_point":density}),
        );
        app.click("Mixer");
        let original = painted(&mut app);
        assert_marker(&original, "fader_scale", Marker::Ordinary);

        // Rotary pan uses an arc/index, covered by mixer_pan_knob_e2e;
        // endpoint triangles and unity/midpoint glyphs belong to sliders.
        for (field, geometry, fraction, expected, marker) in [
            ("gain", "fader_scale", 0.0, 0.0, Marker::Minimum),
            ("gain", "fader_scale", 1.0, 1.5, Marker::Maximum),
            ("gain", "fader_scale", 0.5, 0.75, Marker::Center),
            ("gain", "fader_scale", 2.0 / 3.0, 1.0, Marker::Zero),
        ] {
            let before = painted(&mut app);
            let control = scale(&before, geometry);
            let min = point(&control["min_position"]);
            let max = point(&control["max_position"]);
            let x = min[0] + (max[0] - min[0]) * fraction;
            let y = min[1] + (max[1] - min[1]) * fraction;
            let edited = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
            assert!((number(&edited["project"]["tracks"][0][field]) - expected).abs() < 1e-5);
            let visible = painted(&mut app);
            assert_marker(&visible, geometry, marker);
            for key in ["project", "history", "modified"] {
                assert_eq!(visible[key], edited[key], "Painting a marker changed {key}");
            }
            let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
            assert_eq!(undone["project"], original["project"]);
            let restored = painted(&mut app);
            assert_marker(&restored, "fader_scale", Marker::Ordinary);
        }
        app.quit();
    }
}

#[test]
fn arranger_markers_distinguish_unity_gain_from_midpoint_in_standard_and_compact_rows() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let track = project.add_audio_track("Header landmarks");
        project.track_mut(track).unwrap().gain = 1.0;
        let path = app.directory.path().join("header-landmarks.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        for compact in [false, true] {
            if compact {
                app.click("Header landmarks");
                app.click("Window");
                app.click("Properties");
                app.click("Compact track");
                app.key("Escape");
            }
            let original = painted(&mut app);
            assert_eq!(original["timeline"]["channels"][0]["compact"], compact);
            assert_marker(&original, "pan_scale", Marker::Zero);
            assert_marker(&original, "gain_scale", Marker::Zero);
            let maximum_db = f64::from(20.0 * 1.5_f32.log10());
            for (field, geometry, fraction, marker) in [
                ("pan", "pan_scale", 0.0, Marker::Minimum),
                ("pan", "pan_scale", 1.0, Marker::Maximum),
                ("gain", "gain_scale", 0.0, Marker::Minimum),
                ("gain", "gain_scale", 0.5, Marker::Center),
                ("gain", "gain_scale", 1.0, Marker::Maximum),
            ] {
                let before = painted(&mut app);
                let control = scale(&before, geometry);
                let min = point(&control["min_position"]);
                let max = point(&control["max_position"]);
                let x = min[0] + (max[0] - min[0]) * fraction;
                let y = min[1] + (max[1] - min[1]) * fraction;
                let edited =
                    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
                let expected = if field == "pan" {
                    fraction * 2.0 - 1.0
                } else if fraction == 0.0 {
                    0.0
                } else {
                    10.0_f64.powf((-60.0 + fraction * (60.0 + maximum_db)) / 20.0)
                };
                assert!((number(&edited["project"]["tracks"][0][field]) - expected).abs() < 1e-5);
                let visible = painted(&mut app);
                assert_marker(&visible, geometry, marker);
                for key in ["project", "history", "modified"] {
                    assert_eq!(visible[key], edited[key]);
                }
                let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
                assert_eq!(undone["project"], original["project"]);
                let restored = painted(&mut app);
                assert_marker(&restored, "pan_scale", Marker::Zero);
                assert_marker(&restored, "gain_scale", Marker::Zero);
            }
        }
        app.quit();
    }
}
