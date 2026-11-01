//! Parameter rows align real rails, value fields and neighboring commands.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (number(&rect["min"][0]) + number(&rect["max"][0])) * 0.5,
        (number(&rect["min"][1]) + number(&rect["max"][1])) * 0.5,
    )
}

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
}

fn aligned(state: &Value, density: f64, width: f64) -> [Value; 2] {
    let first = center(label(state, "Quantize preview")).1;
    // The preview is a wrapped parameter row inside a card. It inherits the
    // ordinary row gap, while compact toolbars override theirs to SPACE_SM.
    let pitch = f64::from(daw_ui::tokens::PARAMETER_ROW_HEIGHT + daw_ui::tokens::SPACE_MD);
    let tolerance = 0.5 / density;
    let on_row = |name: &str, y: f64| {
        let row = ((y - first) / pitch).round();
        assert!(row >= 0.0);
        assert!(
            (y - first - row * pitch).abs() <= tolerance,
            "{name} is off the row center at {width}px/{density}×: {y}, first {first}, pitch {pitch}",
        );
    };
    for name in [
        "Strength",
        "Swing",
        "Note ends",
        "Apply quantize",
        "Cancel quantize",
    ] {
        let rect = label(state, name);
        assert!(number(&rect["max"][0]) <= width);
        on_row(name, center(rect).1);
    }
    if width < 1000.0 {
        assert!(center(label(state, "Cancel quantize")).1 > first);
    }
    [("strength_rect", "Strength"), ("swing_rect", "Swing")].map(|(field, name)| {
        let bounds = &state["midi_editor"]["quantize"][field];
        let caption = label(state, name);
        let line_y = state["painted_lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| {
                let a = &line["points"][0];
                let b = &line["points"][1];
                (number(&line["stroke_width"]) - 1.5).abs() < 0.01
                    && (number(&a[0]) - number(&b[0])).abs() < 0.01
                    && number(&a[0]) >= number(&bounds["min"][0])
                    && number(&a[0]) <= number(&bounds["max"][0])
                    && number(&a[1]).min(number(&b[1])) >= number(&bounds["min"][1])
                    && number(&a[1]).max(number(&b[1])) <= number(&bounds["max"][1])
            })
            .map(|line| (number(&line["points"][0][1]) + number(&line["points"][1][1])) * 0.5);
        let triangle_y = state["painted_paths"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|path| {
                let points = path["points"].as_array()?;
                if points.len() != 3
                    || path["closed"] != true
                    || number(&path["fill"][3]) == 0.0
                    || !points.iter().all(|point| {
                        number(&point[0]) >= number(&bounds["min"][0])
                            && number(&point[0]) <= number(&bounds["max"][0])
                            && number(&point[1]) >= number(&bounds["min"][1])
                            && number(&point[1]) <= number(&bounds["max"][1])
                    })
                {
                    return None;
                }
                let top = points
                    .iter()
                    .map(|point| number(&point[1]))
                    .fold(f64::INFINITY, f64::min);
                let bottom = points
                    .iter()
                    .map(|point| number(&point[1]))
                    .fold(f64::NEG_INFINITY, f64::max);
                (bottom - top <= 10.0).then_some((top + bottom) * 0.5)
            });
        let rail_y = line_y.or(triangle_y).unwrap_or_else(|| {
            panic!("Missing actual {name} handle index or endpoint marker inside {bounds}")
        });
        on_row(name, rail_y);
        assert!((rail_y - center(caption).1).abs() <= tolerance);
        let field = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| {
                number(&label["min"][0]) >= number(&bounds["max"][0])
                    && number(&label["max"][0]) <= number(&caption["min"][0])
                    && (center(label).1 - rail_y).abs() <= tolerance
            })
            .unwrap_or_else(|| {
                panic!(
                    "Missing centered numeric field for {name}: {}",
                    state["label_rects"]
                )
            });
        field.clone()
    })
}

#[test]
fn quantize_rows_keep_controls_centered_when_wrapping_and_editing_values() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Aligned controls");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Alignment phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.13, 0.4, 90)],
    ));
    let path = app.directory.path().join("row-alignment.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Alignment phrase");
    app.click("Window");
    app.click("Clip Editor");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    let original = app.inspect();
    app.click("Notes");
    app.click("Quantize...");
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [1440.0, 800.0] {
            app.command(json!({"command":"viewport", "width":width, "height":1000, "pixels_per_point":density}));
            let state = app.command(json!({"command":"inspect", "paint":true}));
            let fields = aligned(&state, density, width);
            let (x, y) = center(&fields[0]);
            app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
            let editing = app.command(json!({"command":"inspect", "paint":true}));
            let editing_fields = aligned(&editing, density, width);
            assert!(fields[0]["text"].as_str().unwrap().ends_with('%'));
            assert!(!editing_fields[0]["text"].as_str().unwrap().ends_with('%'));
            app.click("Quantize preview");
            for state in [&state, &editing] {
                for field in ["project", "history", "modified"] {
                    assert_eq!(
                        state[field], original[field],
                        "Layout or numeric focus changed {field}"
                    );
                }
            }
        }
    }
    let cancelled = app.click("Cancel quantize");
    assert_eq!(cancelled["project"], original["project"]);
    app.quit();
}
