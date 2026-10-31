//! The real editing canvases share a visible cursor without changing edit state.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Canvas {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    start: f64,
    scale: f64,
}

impl Canvas {
    fn from(state: &Value, view: &str) -> Self {
        let view = &state[view];
        let rect = &view[if !view["plot"].is_null() {
            "plot"
        } else if !view["canvas"].is_null() {
            "canvas"
        } else {
            "rect"
        }];
        let (left, top, right, bottom) = if rect.is_array() {
            (
                number(&rect[0]),
                number(&rect[1]),
                number(&rect[2]),
                number(&rect[3]),
            )
        } else {
            (
                number(&rect["min"][0]),
                number(&rect["min"][1]),
                number(&rect["max"][0]),
                number(&rect["max"][1]),
            )
        };
        Self {
            left,
            top,
            right,
            bottom,
            start: number(&view["start_beat"]),
            scale: number(&view["pixels_per_beat"]),
        }
    }

    fn x(self, beat: f64) -> f64 {
        self.left + (beat - self.start) * self.scale
    }
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

fn luminance(color: &Value) -> f64 {
    [0.2126, 0.7152, 0.0722]
        .into_iter()
        .enumerate()
        .map(|(index, weight)| {
            let channel = number(&color[index]) / 255.0;
            weight
                * if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn visible_cursor(state: &Value, canvas: Canvas, beat: f64, density: f64) -> Value {
    let x = canvas.x(beat);
    assert!(x > canvas.left && x < canvas.right);
    let layers: Vec<_> = state["painted_lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|line| {
            let a = &line["points"][0];
            let b = &line["points"][1];
            (number(&a[0]) - x).abs() <= 1.0 / density
                && (number(&b[0]) - x).abs() <= 1.0 / density
                && number(&a[1]).min(number(&b[1])) <= canvas.top + 1.0
                && number(&a[1]).max(number(&b[1])) >= canvas.bottom - 1.0
                && number(&line["clip"]["min"][1]) <= canvas.top + 1.0
                && number(&line["clip"]["max"][1]) >= canvas.bottom - 1.0
        })
        .collect();
    let core = layers
        .iter()
        .copied()
        .filter(|line| number(&line["stroke_width"]) >= 2.5 && line["stroke"][3] == 255)
        .min_by(|a, b| number(&a["stroke_width"]).total_cmp(&number(&b["stroke_width"])))
        .unwrap_or_else(|| {
            panic!("Missing opaque cursor core across the canvas at {x}: {layers:?}")
        });
    assert!(
        layers.iter().any(|line| {
            number(&line["stroke_width"]) >= number(&core["stroke_width"]) * 2.0
                && (1.0..255.0).contains(&number(&line["stroke"][3]))
        }),
        "Cursor needs a wider translucent glow: {layers:?}"
    );
    let cursor_luminance = luminance(&core["stroke"]);
    let background_luminance = luminance(&state["theme_colors"]["background"]);
    let contrast = (cursor_luminance.max(background_luminance) + 0.05)
        / (cursor_luminance.min(background_luminance) + 0.05);
    assert!(
        contrast >= 3.0,
        "Cursor contrast must remain strong in both themes: {contrast}, {core}"
    );
    core["stroke"].clone()
}

fn fixture(app: &mut Harness) {
    let source = app.directory.path().join("cursor.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.25; 192_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let midi = project.add_midi_track("Cursor keys");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Cursor notes",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 8.0, 100)],
    ));
    let audio = project.add_audio_track("Cursor audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Cursor waveform",
        0.0,
        8.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("cursor.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(
            state[field], before[field],
            "Cursor placement changed {field}"
        );
    }
}

#[test]
fn cursor_is_visible_over_clips_and_empty_space_in_each_editor_without_editing() {
    let mut app = Harness::new();
    fixture(&mut app);
    let original = app.inspect();
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            app.command(json!({"command":"system_theme", "dark":dark}));
            app.command(json!({"command":"viewport", "width":1100, "height":900, "pixels_per_point":density}));
            let mut shared_color = Value::Null;
            for (view, clip, beat) in [
                ("timeline", None, 3.0),
                ("midi_editor", Some("Cursor notes"), 4.0),
                ("audio_editor", Some("Cursor waveform"), 5.0),
            ] {
                app.click("Arranger");
                app.click("Select");
                if let Some(clip) = clip {
                    app.click(clip);
                    app.click("Window");
                    app.click("Clip Editor");
                }
                app.click("Cursor");
                let before = app.inspect();
                let canvas = Canvas::from(&before, view);
                let y = (canvas.top + canvas.bottom) * 0.5;
                app.command(
                    json!({"command":"pointer", "phase":"click", "x":canvas.x(beat), "y":y}),
                );
                let state = app.command(json!({"command":"inspect", "paint":true}));
                assert!((number(&state[view]["cursor"]) - beat).abs() < 1e-6);
                unchanged(&state, &original);
                let color = visible_cursor(&state, Canvas::from(&state, view), beat, density);
                if shared_color.is_null() {
                    shared_color = color;
                } else {
                    assert_eq!(
                        color, shared_color,
                        "Editors must share the same cursor color"
                    );
                }
            }
        }
    }
    app.quit();
}
