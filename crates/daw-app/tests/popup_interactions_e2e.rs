//! Resize, hover and close the actual application windows using the mouse.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use daw_ui::theme;
use serde_json::{Value, json};

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}
fn window(state: &Value) -> [f64; 4] {
    let windows = state["window_rects"].as_array().unwrap();
    assert_eq!(
        windows.len(),
        1,
        "Expected one foreground window: {windows:?}"
    );
    bounds(&windows[0])
}
fn settle(app: &mut Harness) -> Value {
    for _ in 0..5 {
        app.inspect();
    }
    app.command(json!({"command":"inspect","paint":true}))
}
fn close_icon(state: &Value) -> (&Value, (f64, f64)) {
    let [_, top, right, _] = window(state);
    let line = state["painted_lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| {
            let p = &line["points"];
            let (x0, y0, x1, y1) = (
                p[0][0].as_f64().unwrap(),
                p[0][1].as_f64().unwrap(),
                p[1][0].as_f64().unwrap(),
                p[1][1].as_f64().unwrap(),
            );
            x0 > right - 70.0
                && x1 < right
                && y0 > top
                && y1 < top + 70.0
                && (x1 - x0).abs() > 8.0
                && ((x1 - x0).abs() - (y1 - y0).abs()).abs() < 0.2
        })
        .unwrap_or_else(|| panic!("No native close icon in {state}"));
    let p = &line["points"];
    (
        line,
        (
            (p[0][0].as_f64().unwrap() + p[1][0].as_f64().unwrap()) / 2.0,
            (p[0][1].as_f64().unwrap() + p[1][1].as_f64().unwrap()) / 2.0,
        ),
    )
}
fn resize(app: &mut Harness, rect: [f64; 4], dx: f64, dy: f64) -> Value {
    let (x, y) = (rect[2] - 1.0, rect[3] - 1.0);
    app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+dx,"to_y":y+dy}));
    settle(app)
}

#[test]
fn utilities_retain_mouse_sizes_do_not_grow_on_hover_and_show_close_feedback() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1600,"height":1200,"pixels_per_point":density}),
        );
        let mut project = Project::new();
        // A long source name must not force every native title to viewport width.
        let track = project.add_midi_track("Popup synth with a detailed custom name · ".repeat(8));
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Popup phrase",
            0.0,
            4.0,
            vec![Note::new(60, 0.0, 1.0, 100)],
        ));
        let path = app.directory.path().join("popups.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.click("Popup phrase");
        let original = app.inspect()["project"].clone();
        let colors = if dark { theme::DARK } else { theme::LIGHT };
        for (menu, label) in [
            ("Window", "Instrument"),
            ("Window", "Effects"),
            ("Window", "Routing"),
            ("Window", "Automation"),
            ("Window", "Properties"),
            ("Window", "Library"),
            ("Window", "Setup"),
            ("Window", "Recording"),
            ("Edit", "History…"),
            ("Help", "About RILL"),
            ("File", "Export WAV…"),
        ] {
            app.click(menu);
            app.click(label);
            let initial = settle(&mut app);
            let before = window(&initial);
            for (fx, fy) in [(0.1, 0.2), (0.8, 0.7), (0.3, 0.9), (0.5, 0.05), (0.1, 0.2)] {
                app.command(json!({"command":"pointer","phase":"move","x":before[0]+(before[2]-before[0])*fx,"y":before[1]+(before[3]-before[1])*fy}));
                let hovered = settle(&mut app);
                let after = window(&hovered);
                for axis in 0..4 {
                    assert!(
                        (after[axis] - before[axis]).abs() < 1.1,
                        "{label} grew on hover: {before:?} -> {after:?}"
                    );
                }
            }
            let enlarged = resize(&mut app, before, 70.0, 60.0);
            let large = window(&enlarged);
            assert!(
                large[2] - large[0] > before[2] - before[0] + 45.0,
                "{label} ignores horizontal resize: {before:?} -> {large:?}"
            );
            assert!(
                large[3] - large[1] > before[3] - before[1] + 40.0,
                "{label} ignores vertical resize: {before:?} -> {large:?}"
            );
            let shrunk = resize(&mut app, large, -70.0, -60.0);
            let small = window(&shrunk);
            assert!(
                small[2] - small[0] < large[2] - large[0] - 45.0,
                "{label} cannot shrink: {large:?} -> {small:?}"
            );
            assert!(
                small[3] - small[1] < large[3] - large[1] - 40.0,
                "{label} cannot shrink vertically: {large:?} -> {small:?}"
            );
            app.command(json!({"command":"pointer","phase":"move","x":0,"y":0}));
            let idle = settle(&mut app);
            let (line, (x, y)) = close_icon(&idle);
            assert_eq!(
                line["stroke"],
                json!(colors.text.to_array()),
                "{label} idle close"
            );
            app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
            let hovered = settle(&mut app);
            assert_eq!(
                close_icon(&hovered).0["stroke"],
                json!(colors.action.to_array()),
                "{label} close hover is invisible"
            );
            let blur = hovered["backdrop_blur"].as_array().unwrap();
            assert_eq!(blur.len(), 1, "{label} needs one backdrop blur");
            assert!(
                blur[0]["target_size"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|side| side.as_u64().unwrap() <= 512)
            );
            app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
            let closed = settle(&mut app);
            assert!(
                closed["backdrop_blur"].as_array().unwrap().is_empty(),
                "{label} left blur after close"
            );
            assert_eq!(
                closed["project"], original,
                "{label} navigation changed the song"
            );
        }
        app.quit();
    }
}
