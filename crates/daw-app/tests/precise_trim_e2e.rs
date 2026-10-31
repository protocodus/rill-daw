//! Small arranger corrections through the running application, below egui's
//! ordinary click/drag threshold. Uses commands supported by the frozen build.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(app: &mut Harness, off_grid: bool) -> PathBuf {
    let source = app.directory.path().join("precise-trim.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let (start, length) = if off_grid {
        (2.125, 4.0625)
    } else {
        (2.0, 4.0)
    };
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        start,
        length,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI phrase",
        start,
        length,
        vec![Note::new(60, 0.5, 1.0, 100)],
    ));
    let path = app.directory.path().join(if off_grid {
        "off-grid.rill"
    } else {
        "on-grid.rill"
    });
    project.save(&path).unwrap();
    path
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64), shift: bool) -> Value {
    app.command(json!({"command":"pointer", "phase":phase, "x":at.0, "y":at.1, "shift":shift}))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn clip(state: &Value, track: usize) -> &Value {
    &state["project"]["tracks"][track]["clips"][0]
}

fn geometry(state: &Value, track: usize) -> &Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == clip(state, track)["id"])
        .unwrap()
}

fn edge(state: &Value, track: usize, start: bool) -> (f64, f64) {
    let rect = &geometry(state, track)["rect"];
    (
        if start {
            rect["min"][0].as_f64().unwrap() + 2.0
        } else {
            rect["max"][0].as_f64().unwrap() - 2.0
        },
        center(rect).1,
    )
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.000_01,
        "{actual} != {expected}"
    );
}

fn select_all(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Select all clips")
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

fn zoom_out_for_small_grid(app: &mut Harness) -> Value {
    let mut state = app.inspect();
    for _ in 0..12 {
        let scale = state["timeline"]["pixels_per_beat"].as_f64().unwrap();
        if scale <= 16.0 {
            assert!(scale >= 12.0, "A grid step must be 3–4 pixels: {scale}");
            return state;
        }
        let labels = state["label_rects"].as_array().unwrap();
        let fit = labels.iter().find(|label| label["text"] == "Fit").unwrap();
        let minus = labels
            .iter()
            .filter(|label| {
                label["text"] == "−"
                    && (center(label).1 - center(fit).1).abs() < 4.0
                    && center(label).0 < center(fit).0
            })
            .max_by(|a, b| center(a).0.total_cmp(&center(b).0))
            .unwrap();
        let at = center(minus);
        state = pointer(app, "click", at, false);
    }
    panic!("Zoom-out control never reached the required scale");
}

fn source_content(state: &Value, track: usize) -> Value {
    let mut content = clip(state, track)["content"].clone();
    if let Some(audio) = content.get_mut("Audio").and_then(Value::as_object_mut) {
        audio.remove("envelope");
    }
    content
}

fn assert_bounds(state: &Value, track: usize, start: f64, length: f64) {
    near(clip(state, track)["start"].as_f64().unwrap(), start);
    near(clip(state, track)["length"].as_f64().unwrap(), length);
    near(clip(state, track)["source_rate"].as_f64().unwrap(), 1.0);
}

#[test]
fn two_to_five_pixel_audio_and_midi_trims_preserve_group_and_single_undo() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    // Four representative gestures cover both media, both edges, and every
    // distance from 2 to 5 pixels without waiting out the 0.8-second threshold.
    for (track, start, distance) in [
        (0, false, 2.0),
        (0, true, 3.0),
        (1, false, 4.0),
        (1, true, 5.0),
    ] {
        app.command(json!({"command":"open", "path":path}));
        zoom_out_for_small_grid(&mut app);
        let before = select_all(&mut app);
        let from = edge(&before, track, start);
        let direction = if start { 1.0 } else { -1.0 };
        let down = pointer(&mut app, "down", from, false);
        assert_eq!(down["project"], before["project"]);
        assert!(down["timeline"]["trim_feedback"].is_null());
        let held = pointer(&mut app, "move", (from.0 + direction * 2.0, from.1), false);
        assert_bounds(&held, track, if start { 2.25 } else { 2.0 }, 3.75);
        let feedback = &held["timeline"]["trim_feedback"];
        assert_eq!(feedback["clip"], clip(&held, track)["id"]);
        near(feedback["duration_beats"].as_f64().unwrap(), 3.75);
        let released = pointer(
            &mut app,
            "up",
            (from.0 + direction * distance, from.1),
            false,
        );
        assert_bounds(&released, track, if start { 2.25 } else { 2.0 }, 3.75);
        near(
            clip(&released, track)["offset"].as_f64().unwrap(),
            if start { 0.25 } else { 0.0 },
        );
        assert_eq!(
            source_content(&released, track),
            source_content(&before, track)
        );
        assert_eq!(
            released["project"]["tracks"][1 - track],
            before["project"]["tracks"][1 - track]
        );
        assert_eq!(released["selected_clips"], before["selected_clips"]);
        assert_eq!(
            released["view"], "Arrangement",
            "A short trim must not become a double-click"
        );
        assert!(released["timeline"]["trim_feedback"].is_null());
        assert_eq!(released["history"]["undo"], json!(["Trim clip"]));
        let undone = undo(&mut app, false);
        assert_eq!(undone["project"], before["project"]);
        assert_eq!(undone["selected_clips"], before["selected_clips"]);
        assert_eq!(undo(&mut app, true)["project"], released["project"]);
    }
    app.quit();
}

#[test]
fn shift_and_snap_off_trims_apply_the_final_release_position() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    for (track, start, shift, direction) in [
        (0, true, true, 1.0),
        (1, false, false, -1.0),
        (0, false, true, 1.0),
        (1, false, true, 1.0),
    ] {
        app.command(json!({"command":"open", "path":path}));
        if !shift {
            app.click("Snap 1/16");
            app.click("Off");
        }
        let before = select_all(&mut app);
        let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
        let from = edge(&before, track, start);
        pointer(&mut app, "down", from, shift);
        let held = pointer(&mut app, "move", (from.0 + direction * 2.0, from.1), shift);
        assert_bounds(
            &held,
            track,
            2.0 + if start { direction * 2.0 / scale } else { 0.0 },
            4.0 + if start { -direction } else { direction } * 2.0 / scale,
        );
        // Up delivers the final move and release together in one native frame.
        let released = pointer(&mut app, "up", (from.0 + direction * 5.0, from.1), shift);
        assert_bounds(
            &released,
            track,
            2.0 + if start { direction * 5.0 / scale } else { 0.0 },
            4.0 + if start { -direction } else { direction } * 5.0 / scale,
        );
        assert_eq!(
            released["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|track| track["clips"].as_array().unwrap().len())
                .sum::<usize>(),
            2,
            "Shift on an edge must not duplicate clips"
        );
        assert_eq!(released["selected_clips"], before["selected_clips"]);
        assert_eq!(released["history"]["undo"], json!(["Trim clip"]));
        assert_eq!(undo(&mut app, false)["project"], before["project"]);
    }
    app.quit();
}

#[test]
fn off_grid_edge_presses_and_return_to_origin_do_not_change_the_document() {
    let mut app = Harness::new();
    let path = fixture(&mut app, true);
    for (track, start) in [(0, false), (1, true)] {
        app.command(json!({"command":"open", "path":path}));
        let before = select_all(&mut app);
        let from = edge(&before, track, start);
        pointer(&mut app, "down", from, false);
        // More than 0.8 simulated seconds: even a long stationary press must
        // not round an existing off-grid endpoint onto the snap grid.
        for _ in 0..18 {
            let held = app.inspect();
            assert_eq!(held["project"], before["project"]);
            assert_eq!(held["history"], before["history"]);
            assert!(held["timeline"]["trim_feedback"].is_null());
        }
        let released = pointer(&mut app, "up", from, false);
        assert_eq!(released["project"], before["project"]);
        assert_eq!(released["history"], before["history"]);
        let before = select_all(&mut app);
        pointer(&mut app, "down", from, false);
        let moved = pointer(
            &mut app,
            "move",
            (from.0 + if start { 5.0 } else { -5.0 }, from.1),
            false,
        );
        assert_ne!(moved["project"], before["project"]);
        let returned = pointer(&mut app, "up", from, false);
        assert_eq!(returned["project"], before["project"]);
        assert_eq!(returned["history"], before["history"]);
    }
    app.quit();
}

#[test]
fn fade_handles_win_over_edges_and_body_selection_move_copy_and_double_click_still_work() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    app.command(json!({"command":"open", "path":path}));
    let before = select_all(&mut app);
    let from = center(&geometry(&before, 0)["fade_out_handle"]);
    let right = geometry(&before, 0)["rect"]["max"][0].as_f64().unwrap();
    assert!(
        right - from.0 < 10.0,
        "Exercise the overlapping fade/trim hit area"
    );
    pointer(&mut app, "down", from, false);
    let faded = pointer(&mut app, "move", (from.0 - 3.0, from.1), false);
    assert_bounds(&faded, 0, 2.0, 4.0);
    assert_ne!(
        clip(&faded, 0)["content"]["Audio"]["fade_out"],
        clip(&before, 0)["content"]["Audio"]["fade_out"]
    );
    assert!(faded["timeline"]["trim_feedback"].is_null());
    pointer(&mut app, "up", (from.0 - 3.0, from.1), false);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let before = select_all(&mut app);
    let from = center(&geometry(&before, 1)["rect"]);
    let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let moved = app.command(json!({"command":"pointer", "phase":"drag", "x":from.0, "y":from.1, "to_x":from.0+scale, "to_y":from.1}));
    for track in [0, 1] {
        assert_bounds(&moved, track, 3.0, 4.0);
    }
    assert_eq!(moved["selected_clips"], before["selected_clips"]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let copied = app.command(json!({"command":"pointer", "phase":"drag", "x":from.0, "y":from.1, "to_x":from.0+scale, "to_y":from.1, "shift":true}));
    for track in [0, 1] {
        assert_eq!(
            copied["project"]["tracks"][track]["clips"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let clicked = pointer(&mut app, "click", from, false);
    assert_eq!(clicked["selected_clips"].as_array().unwrap().len(), 1);
    assert_eq!(clicked["project"], before["project"]);
    let opened = pointer(&mut app, "double_click", from, false);
    assert_eq!(opened["view"], "Clip");
    assert_eq!(opened["selected_clip"], clip(&before, 1)["id"]);
    assert_eq!(opened["project"], before["project"]);
    app.quit();
}

#[test]
fn canceled_edge_captures_do_not_resume_on_further_held_motion() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    for cancel in ["escape", "focus", "undo", "tool"] {
        app.command(json!({"command":"open", "path":path}));
        let before = select_all(&mut app);
        let from = edge(&before, 1, false);
        pointer(&mut app, "down", from, true);
        let moved = pointer(&mut app, "move", (from.0 - 2.0, from.1), true);
        assert_ne!(moved["project"], before["project"]);
        let canceled = match cancel {
            "escape" => app.key("Escape"),
            "focus" => {
                app.command(json!({"command":"focused", "focused":false}));
                app.command(json!({"command":"focused", "focused":true}))
            }
            "tool" => {
                assert_eq!(app.key("W")["timeline"]["tool"], "Pencil");
                let selected = app.key("Q");
                assert_eq!(selected["timeline"]["tool"], "Select");
                selected
            }
            _ => undo(&mut app, false),
        };
        pointer(&mut app, "move", (from.0 - 5.0, from.1), true);
        let released = pointer(&mut app, "up", (from.0 - 5.0, from.1), true);
        assert_eq!(
            released["project"], canceled["project"],
            "{cancel} must retire edge capture"
        );
        assert!(released["timeline"]["trim_feedback"].is_null());
        if cancel == "undo" {
            assert_eq!(released["project"], before["project"]);
        }
    }
    app.quit();
}

#[test]
fn a_short_edge_trim_after_a_nearby_body_click_does_not_open_the_editor() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    app.command(json!({"command":"open", "path":path}));
    zoom_out_for_small_grid(&mut app);
    let before = app.inspect();
    let rect = &geometry(&before, 1)["rect"];
    let from = (rect["max"][0].as_f64().unwrap() - 8.0, center(rect).1);
    // Seed the real clip click tracker just outside the edge target, within
    // four pixels of the following trim (the double-click distance is six).
    let clicked = pointer(&mut app, "click", (from.0 - 4.0, from.1), false);
    assert_eq!(clicked["selected_clips"].as_array().unwrap().len(), 1);
    pointer(&mut app, "down", from, false);
    pointer(&mut app, "move", (from.0 - 2.0, from.1), false);
    let released = pointer(&mut app, "up", (from.0 - 2.0, from.1), false);
    assert_eq!(released["view"], "Arrangement");
    assert_bounds(&released, 1, 2.0, 3.75);
    assert_eq!(released["history"]["undo"], json!(["Trim clip"]));
    app.quit();
}

#[test]
fn popups_middle_pan_and_clipped_viewport_edges_cannot_start_a_trim() {
    let mut app = Harness::new();
    let path = fixture(&mut app, false);
    app.command(json!({"command":"open", "path":path}));
    let before = select_all(&mut app);
    let from = edge(&before, 1, false);
    app.click("View");
    pointer(&mut app, "down", from, true);
    pointer(&mut app, "move", (from.0 - 4.0, from.1), true);
    let released = pointer(&mut app, "up", (from.0 - 4.0, from.1), true);
    assert_eq!(released["project"], before["project"]);
    assert_eq!(released["history"], before["history"]);
    app.key("Escape");
    let panned = app.command(json!({"command":"pointer", "phase":"drag", "button":"middle", "x":from.0, "y":from.1, "to_x":from.0-192.0, "to_y":from.1}));
    assert_eq!(panned["project"], before["project"]);
    assert_eq!(panned["history"], before["history"]);
    let real = &geometry(&panned, 1)["rect"];
    let visible = &geometry(&panned, 1)["visible_rect"];
    assert!(
        real["min"][0].as_f64().unwrap() < visible["min"][0].as_f64().unwrap(),
        "The real clip edge must be offscreen"
    );
    let clipped_edge = (visible["min"][0].as_f64().unwrap() + 2.0, center(visible).1);
    pointer(&mut app, "down", clipped_edge, true);
    pointer(
        &mut app,
        "move",
        (clipped_edge.0 + 4.0, clipped_edge.1),
        true,
    );
    let released = pointer(&mut app, "up", (clipped_edge.0 + 4.0, clipped_edge.1), true);
    assert_eq!(released["project"], before["project"]);
    assert_eq!(released["history"], before["history"]);
    app.quit();
}
