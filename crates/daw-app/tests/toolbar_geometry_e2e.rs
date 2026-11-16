//! Shared toolbar spacing survives real editor layouts, wrapping and mouse use.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Note, Project,
};
use eframe::egui::Color32;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Editor {
    Arranger,
    Midi,
    Audio,
    Automation,
}

impl Editor {
    fn tools(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Arranger => &[
                ("Select", "Select"),
                ("Cursor", "Cursor"),
                ("Pencil", "Pencil"),
                ("Cut", "Cut"),
                ("Join", "Glue"),
                ("Erase", "Erase"),
                ("Duplicate", "Duplicate"),
                ("Ghost copy", "Ghost"),
                ("Stretch", "Stretch"),
                ("Slip", "Slip"),
            ],
            Self::Midi => &[
                ("Select", "Select"),
                ("Cursor", "Cursor"),
                ("Pencil", "Pencil"),
                ("Multi-draw", "MultiDraw"),
                ("Cut", "Cut"),
                ("Glue", "Glue"),
                ("Erase", "Erase"),
                ("Duplicate", "Duplicate"),
            ],
            Self::Audio => &[
                ("Select", "Select"),
                ("Cursor", "Cursor"),
                ("Cut", "Cut"),
                ("Stretch", "Stretch"),
                ("Slip", "Slip"),
                ("Warp", "Warp"),
                ("Fade in", "Fade in"),
                ("Fade out", "Fade out"),
            ],
            Self::Automation => &[("Fit song", ""), ("Fit", "")],
        }
    }

    fn open(self, app: &mut Harness) {
        app.click("Arranger");
        app.click("Select");
        match self {
            Self::Arranger => {}
            Self::Midi | Self::Audio => {
                app.click(if matches!(self, Self::Midi) {
                    "Geometry MIDI"
                } else {
                    "Geometry audio"
                });
                app.click("Window");
                app.click("Clip Editor");
            }
            Self::Automation => {
                app.click("Geometry automation");
                app.click("Window");
                app.click("Automation");
            }
        }
    }

    fn tool(self, state: &Value) -> &Value {
        match self {
            Self::Arranger => &state["timeline"]["tool"],
            Self::Midi => &state["midi_editor"]["tool"],
            Self::Audio => &state["audio_editor"]["tool"],
            Self::Automation => &Value::Null,
        }
    }

    fn canvas_top(self, state: &Value) -> f64 {
        match self {
            Self::Arranger => state["timeline"]["ruler"]["min"][1].as_f64().unwrap(),
            Self::Midi => state["midi_editor"]["ruler"]["min"][1].as_f64().unwrap(),
            Self::Audio => state["audio_editor"]["rect"][1].as_f64().unwrap(),
            Self::Automation => state["automation_editor"]["plot"]["min"][1]
                .as_f64()
                .unwrap(),
        }
    }
}

fn fixture() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let midi = project.add_midi_track("Keys");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Geometry MIDI",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 0.5, 90), Note::new(67, 2.0, 0.5, 90)],
    ));
    let source = app.directory.path().join("geometry.wav");
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![0.1; 88_200],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Geometry audio",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let automation = project
        .add_automation_track(
            "Geometry automation",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.automation_lane_mut(automation).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.5),
        AutomationPoint::new(4.0, 0.8),
    ];
    let path = app.directory.path().join("toolbar-geometry.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app
}

type Bounds = [f64; 4];

fn bounds(rect: &Value) -> Bounds {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn center(rect: Bounds) -> (f64, f64) {
    ((rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5)
}

fn contains(outer: Bounds, inner: Bounds) -> bool {
    inner[0] >= outer[0] - 0.1
        && inner[1] >= outer[1] - 0.1
        && inner[2] <= outer[2] + 0.1
        && inner[3] <= outer[3] + 0.1
}

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "song_session",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            state[field], before[field],
            "Tool/view control changed {field}"
        );
    }
}

fn enclosing_hit(state: &Value, text: Bounds) -> Bounds {
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shape| bounds(&shape["rect"]))
        .filter(|rect| {
            contains(*rect, text)
                && rect[3] - rect[1] >= 21.0
                && rect[3] - rect[1] <= 35.0
                && rect[2] - rect[0] <= text[2] - text[0] + 80.0
        })
        .min_by(|a, b| ((a[2] - a[0]) * (a[3] - a[1])).total_cmp(&((b[2] - b[0]) * (b[3] - b[1]))))
        .unwrap_or_else(|| panic!("No native hover target encloses {text:?}"))
}

/// Union the actual vector glyph paint, excluding the surrounding button.
/// Internal glyph margins vary by symbol, so exact atom spacing is covered by
/// the shared component renderer tests, while this checks the composed app.
fn glyph(state: &Value, region: Bounds, expected: &Value) -> Bounds {
    let mut pieces = Vec::new();
    for field in ["painted_paths", "painted_lines"] {
        for shape in state[field].as_array().unwrap() {
            let points = shape["points"].as_array().unwrap();
            if points.is_empty() {
                continue;
            }
            let mut rect = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for point in points {
                let x = point[0].as_f64().unwrap();
                let y = point[1].as_f64().unwrap();
                rect[0] = rect[0].min(x);
                rect[1] = rect[1].min(y);
                rect[2] = rect[2].max(x);
                rect[3] = rect[3].max(y);
            }
            if contains(region, rect) && shape["stroke_width"].as_f64().unwrap_or(0.0) > 0.0 {
                assert_eq!(
                    &shape["stroke"], expected,
                    "Toolbar glyph uses 80% of its unchanged label color"
                );
                pieces.push(rect);
            }
        }
    }
    for shape in state["painted_circles"].as_array().unwrap() {
        let x = shape["center"][0].as_f64().unwrap();
        let y = shape["center"][1].as_f64().unwrap();
        let r = shape["radius"].as_f64().unwrap();
        let rect = [x - r, y - r, x + r, y + r];
        if contains(region, rect) {
            let color = if shape["stroke_width"].as_f64().unwrap_or(0.0) > 0.0 {
                &shape["stroke"]
            } else {
                &shape["fill"]
            };
            assert_eq!(color, expected);
            pieces.push(rect);
        }
    }
    for shape in state["painted_rects"].as_array().unwrap() {
        let rect = bounds(&shape["rect"]);
        if contains(region, rect) && shape["stroke_width"].as_f64().unwrap_or(0.0) > 0.0 {
            assert_eq!(&shape["stroke"], expected);
            pieces.push(rect);
        }
    }
    assert!(
        !pieces.is_empty(),
        "A visible vector icon must paint inside {region:?}"
    );
    pieces
        .into_iter()
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
        .unwrap()
}

fn check_control(app: &mut Harness, name: &str, width: i32, canvas_top: f64) -> Bounds {
    let state = app.inspect();
    let (x, y) = center(bounds(label(&state, name)));
    app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    let painted = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|text| text["text"] == name)
        .unwrap_or_else(|| panic!("Missing visible label {name}"));
    assert_eq!(
        painted["rendered_text"], name,
        "Toolbar labels stay complete"
    );
    let text = bounds(&painted["rect"]);
    let hit = enclosing_hit(&state, text);
    assert!(
        hit[0] >= 0.0 && hit[2] <= f64::from(width) && hit[3] <= canvas_top,
        "{name} stays above the editor canvas at width {width}: {hit:?}"
    );
    assert!(
        hit[3] - hit[1] < 34.0,
        "Toolbar control {name} uses the requested compact density"
    );
    let foreground = &painted["colors"][0];
    assert_eq!(foreground[3], 255, "The label retains full opacity");
    let color = Color32::from_rgba_premultiplied(
        foreground[0].as_u64().unwrap() as u8,
        foreground[1].as_u64().unwrap() as u8,
        foreground[2].as_u64().unwrap() as u8,
        255,
    )
    .gamma_multiply(0.8);
    let icon = glyph(
        &state,
        [hit[0], hit[1], text[0] - 1.0, hit[3]],
        &json!(color.to_array()),
    );
    assert!(
        (center(text).1 - center(hit).1).abs() <= 1.0,
        "{name} label is vertically centered"
    );
    assert!(
        (center(icon).1 - center(text).1).abs() <= 2.0,
        "{name} glyph and label share an axis: {icon:?} {text:?}"
    );
    let gap = text[0] - icon[2];
    assert!(
        (2.0..=8.0).contains(&gap),
        "{name} icon stays close to its label: gap {gap}"
    );
    let vertical = (hit[3] - hit[1] - (text[3] - text[1])) * 0.5;
    let left = icon[0] - hit[0];
    let right = hit[2] - text[2];
    assert!(
        left > vertical && right > vertical,
        "{name} has more horizontal than vertical padding: left{left},right{right},vertical{vertical}"
    );
    hit
}

#[test]
fn toolbar_glyphs_labels_and_targets_align_wrap_and_remain_clickable_across_editors() {
    let mut app = fixture();
    for dark in [false, true] {
        app.command(json!({"command":"system_theme","dark":dark}));
        for density in [1.0, 2.0] {
            for width in [800, 1100, 1200, 1480] {
                app.command(json!({"command":"viewport","width":width,"height":1000,"pixels_per_point":density}));
                for editor in [
                    Editor::Arranger,
                    Editor::Midi,
                    Editor::Audio,
                    Editor::Automation,
                ] {
                    editor.open(&mut app);
                    let before = app.inspect();
                    if matches!(editor, Editor::Audio) {
                        let gain = &before["audio_editor"]["gain_rect"];
                        let gain_center =
                            (gain[1].as_f64().unwrap() + gain[3].as_f64().unwrap()) * 0.5;
                        let gain_height = gain[3].as_f64().unwrap() - gain[1].as_f64().unwrap();
                        assert!(
                            gain_height < 34.0,
                            "Audio gain follows the compact command row"
                        );
                        // Compare other controls only when they occupy the same wrapped row.
                        let pitch = bounds(label(&before, "Pitch"));
                        if gain[1].as_f64().unwrap() < pitch[3]
                            && pitch[1] < gain[3].as_f64().unwrap()
                        {
                            assert!(
                                (gain_center - center(pitch).1).abs() <= 1.0,
                                "Audio gain and pitch labels share their vertical center"
                            );
                        }
                        let select = bounds(label(&before, "Select"));
                        if gain[1].as_f64().unwrap() < select[3]
                            && select[1] < gain[3].as_f64().unwrap()
                        {
                            assert!(
                                (gain_center - center(select).1).abs() <= 1.0,
                                "Audio gain cannot pull later controls below the tool baseline"
                            );
                        }
                    }
                    let mut targets: Vec<(&str, Bounds)> = Vec::new();
                    if !matches!(editor, Editor::Automation) {
                        for action in ["Tool help", "View", "Fit"] {
                            let rect = bounds(label(&before, action));
                            // Arranger zoom/Fit live in the fixed ruler corner,
                            // below its command toolbar. Other editor Fit controls
                            // are part of their command bar above the ruler.
                            let bottom = if matches!(editor, Editor::Arranger) && action == "Fit" {
                                before["viewport"][1].as_f64().unwrap()
                            } else {
                                editor.canvas_top(&before)
                            };
                            assert!(
                                rect[0] >= 0.0 && rect[2] <= f64::from(width) && rect[3] <= bottom,
                                "{editor:?}: trailing {action} remains visible at {width}, density{density}: {rect:?}, canvas top {}",
                                editor.canvas_top(&before)
                            );
                        }
                    }
                    for &(name, mode) in editor.tools() {
                        let target =
                            check_control(&mut app, name, width, editor.canvas_top(&before));
                        for &(other, previous) in &targets {
                            assert!(
                                target[2] <= previous[0]
                                    || target[0] >= previous[2]
                                    || target[3] <= previous[1]
                                    || target[1] >= previous[3],
                                "{editor:?}: {name} overlaps {other} at {width}, density{density}"
                            );
                            if target[1] < previous[3] && previous[1] < target[3] {
                                assert!(
                                    (center(target).1 - center(previous).1).abs() <= 1.0,
                                    "{editor:?}: same-row controls share their vertical center"
                                );
                            }
                        }
                        targets.push((name, target));
                        let clicked = app.click(name);
                        if !matches!(editor, Editor::Automation) {
                            assert_eq!(editor.tool(&clicked), mode);
                        }
                        unchanged(&clicked, &before);
                    }
                }
            }
        }
    }
    app.quit();
}
