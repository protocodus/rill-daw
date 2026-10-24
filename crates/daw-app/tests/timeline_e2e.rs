mod common;
use common::Harness;
use daw_core::{Clip, Project};
use serde_json::{Value, json};

fn fixture(app: &mut Harness, tracks: usize, start: f64) -> Value {
    let mut project = Project::new();
    for index in 0..tracks {
        let track = project.add_midi_track(format!("Track {index:02}"));
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            format!("Clip {index:02}"),
            start,
            8.0,
            vec![],
        ));
    }
    let path = app.directory.path().join("timeline.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}

fn number(state: &Value, key: &str) -> f64 {
    state["timeline"][key].as_f64().unwrap()
}
fn point(state: &Value) -> (f64, f64) {
    let rect = &state["timeline"]["canvas"];
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        rect["min"][1].as_f64().unwrap() + 50.0,
    )
}
fn check_fit(state: &Value) {
    let view = &state["timeline"];
    let bounds = &view["bounds"];
    let rect = &view["canvas"];
    let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
    let height = rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap();
    let left = (bounds["start_beat"].as_f64().unwrap() - number(state, "start_beat"))
        * number(state, "pixels_per_beat");
    let right = (bounds["end_beat"].as_f64().unwrap() - number(state, "start_beat"))
        * number(state, "pixels_per_beat");
    assert!(
        left >= -0.01 && right <= width + 0.01,
        "Horizontal fit: {view}"
    );
    let bottom = (bounds["end_row"].as_f64().unwrap() - number(state, "row_offset"))
        * number(state, "row_height");
    assert!(bottom <= height + 0.01, "Vertical fit: {view}");
}

#[test]
fn arranger_fit_button_and_shortcut_reveal_late_content_and_all_tracks() {
    let mut app = Harness::new();
    let original = fixture(&mut app, 20, 64.0)["project"].clone();
    let fitted = app.click("Fit");
    check_fit(&fitted);
    assert!(number(&fitted, "start_beat") > 60.0);
    assert!(number(&fitted, "row_height") < 84.0);
    assert!(
        fitted["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Track 19")
    );
    let (x, y) = point(&fitted);
    app.command(json!({"command":"scroll","x":x,"y":y,"delta":120.0,"ctrl":true}));
    let fitted_again = app.key("F");
    check_fit(&fitted_again);
    assert_eq!(fitted_again["project"], original);
    assert_eq!(fitted_again["device_open"], false);
    app.quit();
}

#[test]
fn arranger_pan_zoom_and_keyboard_navigation_leave_clips_unchanged() {
    let mut app = Harness::new();
    fixture(&mut app, 12, 0.0);
    let before = app.inspect();
    let original = before["project"].clone();
    let (x, y) = point(&before);
    let left = before["timeline"]["canvas"]["min"][0].as_f64().unwrap();
    let anchor = number(&before, "start_beat") + (x - left) / number(&before, "pixels_per_beat");
    let zoomed = app.command(json!({"command":"scroll","x":x,"y":y,"delta":100.0,"ctrl":true}));
    assert!(number(&zoomed, "pixels_per_beat") > number(&before, "pixels_per_beat"));
    let after_anchor =
        number(&zoomed, "start_beat") + (x - left) / number(&zoomed, "pixels_per_beat");
    assert!((anchor - after_anchor).abs() < 1e-4);
    let vertical =
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":100.0,"ctrl":true,"shift":true}));
    assert!(number(&vertical, "row_height") > number(&zoomed, "row_height"));
    let shifted = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-90.0,"shift":true}));
    assert!(number(&shifted, "start_beat") > number(&vertical, "start_beat"));
    let scrolled = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-90.0}));
    assert!(number(&scrolled, "row_offset") > number(&shifted, "row_offset"));
    let panned = app.command(json!({"command":"pointer","phase":"drag","button":"middle","x":x,"y":y,"to_x":x-70.0,"to_y":y-20.0}));
    assert!(number(&panned, "start_beat") > number(&scrolled, "start_beat"));
    let alt = app.command(
        json!({"command":"pointer","phase":"drag","alt":true,"x":x,"y":y,"to_x":x-70.0,"to_y":y}),
    );
    assert!(number(&alt, "start_beat") > number(&panned, "start_beat"));
    let dragged_zoom = app.command(json!({"command":"pointer","phase":"drag","button":"secondary","ctrl":true,"x":x,"y":y,"to_x":x+40.0,"to_y":y}));
    assert!(number(&dragged_zoom, "pixels_per_beat") > number(&alt, "pixels_per_beat"));
    let keyed = app.key("Plus");
    assert!(number(&keyed, "pixels_per_beat") > number(&dragged_zoom, "pixels_per_beat"));
    let keyed_y = app.command(json!({"command":"key","key":"Minus","shift":true}));
    assert!(number(&keyed_y, "row_height") < number(&keyed, "row_height"));
    assert_eq!(
        keyed_y["project"], original,
        "Navigation must never edit a clip"
    );
    app.quit();
}

#[test]
fn arranger_common_tool_shortcuts_draw_cut_duplicate_erase_and_seek() {
    let mut app = Harness::new();
    fixture(&mut app, 1, 0.0);
    let state = app.inspect();
    let rect = &state["timeline"]["canvas"];
    let left = rect["min"][0].as_f64().unwrap();
    let y = rect["min"][1].as_f64().unwrap() + 30.0;
    let x = left + number(&state, "pixels_per_beat") * 2.0;
    app.key("E");
    let split = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        split["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    app.key("R");
    let copied = app.command(json!({"command":"pointer","phase":"click","x":left+30.0,"y":y}));
    assert_eq!(
        copied["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    app.key("Z");
    let erased = app.command(json!({"command":"pointer","phase":"click","x":left+30.0,"y":y}));
    assert_eq!(
        erased["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    app.key("P");
    let cursor = app.command(json!({"command":"pointer","phase":"click","x":left+number(&state,"pixels_per_beat")*4.0,"y":y}));
    assert!((number(&cursor, "cursor") - 4.0).abs() < 1e-6);
    app.key("W");
    let drawn = app.command(json!({"command":"pointer","phase":"click","x":left+number(&state,"pixels_per_beat")*12.0,"y":y}));
    assert!(
        drawn["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|clip| clip["start"] == 12.0)
    );
    let selected = app.key("Q");
    assert_eq!(selected["timeline"]["tool"], "Select");
    app.quit();
}
