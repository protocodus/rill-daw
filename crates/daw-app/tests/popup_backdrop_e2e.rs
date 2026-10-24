//! Every popup draws its active surface above one dimmed, contextual backdrop.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

const WIDTH: f64 = 1100.0;
const HEIGHT: f64 = 760.0;

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn painted(app: &mut Harness) -> Value {
    // Let native opening/closing fades settle before checking surface opacity.
    for _ in 0..12 {
        app.inspect();
    }
    app.command(json!({"command":"inspect", "paint":true}))
}

fn dim_layers(state: &Value) -> Vec<usize> {
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| {
            let [left, top, right, bottom] = bounds(&shape["rect"]);
            let alpha = shape["fill"][3].as_u64().unwrap();
            (left <= 0.1
                && top <= 0.1
                && right >= WIDTH - 0.1
                && bottom >= HEIGHT - 0.1
                && alpha > 0
                && alpha < 255)
                .then_some(index)
        })
        .collect()
}

fn assert_backdrop(state: &Value, foreground: &str) {
    let dim = dim_layers(state);
    assert_eq!(dim.len(), 1, "{foreground} needs one full-window dim layer");
    assert_eq!(
        state["painted_rects"][dim[0]]["fill"],
        json!([
            0,
            0,
            0,
            if state["effective_theme"] == "Dark" {
                104
            } else {
                64
            }
        ]),
        "the backdrop uses the shared theme's dim strength"
    );
    let label = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == foreground)
        .unwrap_or_else(|| panic!("Missing {foreground}: {}", state["visible_labels"]));
    let [left, top, right, bottom] = bounds(label);
    let (x, y) = ((left + right) * 0.5, (top + bottom) * 0.5);
    assert!(
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .skip(dim[0] + 1)
            .any(|shape| {
                let [left, top, right, bottom] = bounds(&shape["rect"]);
                left < x
                    && x < right
                    && top < y
                    && y < bottom
                    && right - left > 80.0
                    && bottom - top > 30.0
                    && shape["fill"][3] == 255
            }),
        "{foreground} must have an opaque active surface painted above the dim"
    );
}

fn assert_clear(app: &mut Harness) -> Value {
    let state = painted(app);
    assert!(
        dim_layers(&state).is_empty(),
        "closed popups must leave no dim"
    );
    state
}

fn window(app: &mut Harness, label: &str) {
    app.click("Window");
    app.click(label);
}

fn fixture(app: &mut Harness) -> u64 {
    let mut project = Project::new();
    let track = project.add_midi_track("Backdrop synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Backdrop phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("backdrop.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Backdrop phrase");
    track
}

#[test]
fn utility_backdrops_preserve_context_and_clear_after_nested_navigation() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":WIDTH, "height":HEIGHT, "pixels_per_point":density}));
        fixture(&mut app);
        let original = assert_clear(&mut app);

        for (menu, title) in [
            ("Instrument", "Instrument of Backdrop synth"),
            ("Effects", "Effects of Backdrop synth"),
            ("Routing", "Routing of Backdrop synth"),
            ("Automation", "Add automation"),
            ("Properties", "Properties of Backdrop phrase"),
            ("Library", "Device library"),
            ("Setup", "Setup"),
        ] {
            window(&mut app, menu);
            let state = painted(&mut app);
            assert_backdrop(&state, title);
            assert!(state["popup_context"]["shapes"].as_u64().unwrap() > 0);
            assert_eq!(state["popup_context"]["source"], "Arrangement");
            assert_eq!(state["project"], original["project"]);
            app.key("Escape");
            assert_clear(&mut app);
        }

        window(&mut app, "Routing");
        app.click("Master output");
        assert_backdrop(&painted(&mut app), "Master output");
        app.key("Escape");
        assert_backdrop(&painted(&mut app), "Routing of Backdrop synth");
        app.key("Escape");
        assert_clear(&mut app);

        window(&mut app, "Effects");
        for _ in 0..3 {
            app.click("+ Insert");
            let nested = painted(&mut app);
            assert_backdrop(&nested, "Device library");
            assert_eq!(
                nested["popup_context"]["source"],
                "Effects · Backdrop synth"
            );
            app.key("Escape");
            assert_backdrop(&painted(&mut app), "Effects of Backdrop synth");
        }
        app.key("Escape");
        let restored = assert_clear(&mut app);
        assert_eq!(restored["project"], original["project"]);
        assert_eq!(restored["selected_clip"], original["selected_clip"]);
        app.quit();
    }
}

#[test]
fn menus_and_pickers_keep_their_contents_above_the_dim_and_dismiss_normally() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":WIDTH, "height":HEIGHT}));
    let track = fixture(&mut app);
    let original = assert_clear(&mut app)["project"].clone();

    app.click("File");
    assert_backdrop(&painted(&mut app), "New");
    app.key("Escape");
    assert_clear(&mut app);

    app.click("Ch 1 > Master");
    assert_backdrop(&painted(&mut app), "Master");
    app.key("Escape");
    assert_clear(&mut app);

    let state = app.inspect();
    let snap = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|label| label.as_str().filter(|label| label.starts_with("Snap ")))
        .unwrap();
    app.click(snap);
    assert_backdrop(&painted(&mut app), "1/32");
    app.command(json!({"command":"pointer", "phase":"click", "x":1090, "y":750}));
    assert_clear(&mut app);

    let state = app.inspect();
    let strip = state["timeline"]["track_colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|strip| strip["track"] == track)
        .unwrap();
    let [left, top, _, bottom] = bounds(&strip["rect"]);
    app.command(json!({"command":"pointer", "phase":"click", "x":left+1.5, "y":(top+bottom)*0.5}));
    assert_backdrop(&painted(&mut app), "Color of Backdrop synth");
    app.key("Escape");
    assert_clear(&mut app);

    let state = app.inspect();
    let clip = &state["timeline"]["clips"][0]["rect"];
    let [left, top, right, bottom] = bounds(clip);
    app.command(
        json!({"command":"pointer", "phase":"click", "button":"secondary",
        "x":(left+right)*0.5, "y":(top+bottom)*0.5}),
    );
    assert_backdrop(&painted(&mut app), "Create linked clone");
    app.key("Escape");
    assert_clear(&mut app);

    app.click("Mixer");
    app.click("Channel");
    assert_backdrop(&painted(&mut app), "Open channel");
    app.click("Effects");
    assert_backdrop(&painted(&mut app), "Effects of Backdrop synth");
    app.key("Escape");
    let restored = assert_clear(&mut app);
    assert_eq!(restored["view"], "Mixer");
    assert_eq!(restored["project"], original);
    app.quit();
}

#[test]
fn save_confirmation_uses_one_dim_layer_and_restores_the_utility_on_cancel() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":WIDTH, "height":HEIGHT}));
    app.click("Track");
    app.click("Add audio track");
    window(&mut app, "Properties");
    let original = app.inspect();
    app.click("File");
    app.click("New");
    common::wait_visible(&mut app, "Save changes?");
    let prompt = painted(&mut app);
    assert_eq!(prompt["document_guard"]["phase"], "confirm");
    assert_backdrop(&prompt, "Save changes?");
    app.click("Cancel");
    let restored = painted(&mut app);
    assert_backdrop(&restored, "Track · New audio");
    assert_eq!(restored["project"], original["project"]);
    assert_eq!(restored["popups"], original["popups"]);
    app.key("Escape");
    assert_clear(&mut app);
    app.quit();
}
