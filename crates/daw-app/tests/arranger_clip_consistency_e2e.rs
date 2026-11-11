//! Automation clips edit like MIDI/audio clips, new clips take the last
//! selected clip's size, the Grid snap follows the visible grid, and tracks are
//! added from the arranger itself.
mod common;
use common::Harness;
use daw_core::{AutomationParameter, AutomationTarget, Clip, Project};
use serde_json::{Value, json};

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64), button: &str) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1,"button":button}))
}
fn drag(app: &mut Harness, from: (f64, f64), to: (f64, f64)) -> Value {
    app.command(
        json!({"command":"pointer","phase":"drag","x":from.0,"y":from.1,"to_x":to.0,"to_y":to.1}),
    )
}
fn x_at(state: &Value, beat: f64) -> f64 {
    let timeline = &state["timeline"];
    timeline["canvas"]["min"][0].as_f64().unwrap()
        + (beat - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap()
}
fn ppb(state: &Value) -> f64 {
    state["timeline"]["pixels_per_beat"].as_f64().unwrap()
}
fn row_y(state: &Value, track: u64) -> f64 {
    let header = &state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["track"] == track)
        .unwrap()["header"];
    (header["min"][1].as_f64().unwrap() + header["max"][1].as_f64().unwrap()) / 2.0
}
fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}
fn patterns(state: &Value, id: u64) -> &Vec<Value> {
    track(state, id)["automation"]["clips"].as_array().unwrap()
}
fn pattern_rect(state: &Value, index: usize) -> &Value {
    &state["timeline"]["automation"][0]["clips"][index]["rect"]
}
fn inside(rect: &Value, fx: f64, fy: f64) -> (f64, f64) {
    let [x0, y0, x1, y1] = [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ];
    (x0 + (x1 - x0) * fx, y0 + (y1 - y0) * fy)
}

fn fixture(app: &mut Harness) -> (Value, u64, u64) {
    let mut project = Project::new();
    let midi = project.add_midi_track("Keys");
    project
        .track_mut(midi)
        .unwrap()
        .clips
        .push(Clip::midi("Short", 0.0, 2.0, Vec::new()));
    let automation = project
        .add_automation_track(
            "Keys gain",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .create_automation_clip(automation, 4.0, 4.0)
        .unwrap();
    let path = app.directory.path().join("clip-consistency.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    (
        app.command(json!({"command":"open","path":path})),
        midi,
        automation,
    )
}

#[test]
fn automation_clips_select_first_and_follow_the_clip_tools() {
    let mut app = Harness::new();
    let (opened, _, lane) = fixture(&mut app);
    assert_eq!(opened["timeline"]["tool"], "Select");
    let pattern = patterns(&opened, lane)[0]["id"].clone();

    // A Select click in the body selects the pattern and draws nothing.
    let body = inside(pattern_rect(&opened, 0), 0.4, 0.7);
    let selected = pointer(&mut app, "click", body, "primary");
    assert_eq!(selected["project"], opened["project"]);
    assert_eq!(selected["selected_clip"], pattern);

    // Dragging the body moves it on the visible grid.
    let moved = drag(&mut app, body, (body.0 + ppb(&selected), body.1));
    assert_eq!(patterns(&moved, lane)[0]["start"], 5.0);

    // Pencil draws a point inside the pattern.
    app.click("Pencil");
    let drawn = pointer(
        &mut app,
        "click",
        inside(pattern_rect(&moved, 0), 0.5, 0.7),
        "primary",
    );
    assert_eq!(
        patterns(&drawn, lane)[0]["points"]
            .as_array()
            .unwrap()
            .len(),
        patterns(&moved, lane)[0]["points"]
            .as_array()
            .unwrap()
            .len()
            + 1
    );

    // Stretch scales the pattern and its points in time.
    app.click("Stretch");
    let state = app.inspect();
    let from = inside(pattern_rect(&state, 0), 0.5, 0.7);
    let stretched = drag(&mut app, from, (from.0 + 4.0 * ppb(&state), from.1));
    let clip = &patterns(&stretched, lane)[0];
    assert_eq!(
        (clip["start"].as_f64(), clip["length"].as_f64()),
        (Some(5.0), Some(8.0))
    );
    let before = &patterns(&drawn, lane)[0]["points"];
    let after = clip["points"].as_array().unwrap();
    for (old, new) in before.as_array().unwrap().iter().zip(after) {
        assert_eq!(
            new["beat"].as_f64().unwrap(),
            old["beat"].as_f64().unwrap() * 2.0
        );
    }
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        undone["project"], drawn["project"],
        "one stretch is one undo step"
    );
    let restretched = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(restretched["project"], stretched["project"]);

    // Cut splits at the clicked grid position.
    app.click("Cut");
    let cut = pointer(
        &mut app,
        "click",
        (x_at(&restretched, 7.0), body.1),
        "primary",
    );
    let parts = patterns(&cut, lane);
    assert_eq!(parts.len(), 2);
    assert_eq!(
        (parts[0]["start"].as_f64(), parts[0]["length"].as_f64()),
        (Some(5.0), Some(2.0))
    );
    assert_eq!(
        (parts[1]["start"].as_f64(), parts[1]["length"].as_f64()),
        (Some(7.0), Some(6.0))
    );

    // Erase removes the clicked pattern.
    app.click("Erase");
    let erased = pointer(&mut app, "click", (x_at(&cut, 9.0), body.1), "primary");
    assert_eq!(patterns(&erased, lane).len(), 1);
    app.quit();
}

#[test]
fn double_click_creates_clips_at_the_pointer_with_the_last_selected_size() {
    let mut app = Harness::new();
    let (opened, midi, lane) = fixture(&mut app);
    let row = row_y(&opened, midi);

    // Select the two-beat MIDI clip, then double-click empty MIDI lane space.
    pointer(&mut app, "click", (x_at(&opened, 1.0), row), "primary");
    let created = pointer(
        &mut app,
        "double_click",
        (x_at(&opened, 8.1), row),
        "primary",
    );
    let clips = track(&created, midi)["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 2);
    assert_eq!(
        (clips[1]["start"].as_f64(), clips[1]["length"].as_f64()),
        (Some(8.0), Some(2.0))
    );
    assert_eq!(created["selected_clip"], clips[1]["id"]);

    // The same gesture on an automation lane creates a pattern that size.
    let lane_row = row_y(&created, lane);
    let pattern = pointer(
        &mut app,
        "double_click",
        (x_at(&created, 12.1), lane_row),
        "primary",
    );
    let created_patterns = patterns(&pattern, lane);
    assert_eq!(created_patterns.len(), 2);
    let new = created_patterns
        .iter()
        .find(|clip| clip["start"] == 12.0)
        .unwrap();
    assert_eq!(new["length"], 2.0);
    assert_eq!(pattern["selected_clip"], new["id"]);

    // Selecting the four-beat pattern makes the next MIDI clip four beats.
    pointer(
        &mut app,
        "click",
        (x_at(&pattern, 5.0), lane_row),
        "primary",
    );
    let longer = pointer(
        &mut app,
        "double_click",
        (x_at(&pattern, 16.0), row),
        "primary",
    );
    let clips = track(&longer, midi)["clips"].as_array().unwrap();
    assert_eq!(clips.last().unwrap()["length"], 4.0);
    app.quit();
}

#[test]
fn grid_is_the_default_snap_and_follows_the_visible_grid() {
    let mut app = Harness::new();
    let (opened, midi, lane) = fixture(&mut app);
    let row = row_y(&opened, midi);
    // The default arranger zoom shows sixteenth-note lines: drags land on them.
    assert_eq!(ppb(&opened), 48.0);
    app.click("Snap grid");
    let menu = app.inspect();
    app.key("Escape");
    assert!(menu.to_string().contains("Grid"));
    let from = (x_at(&opened, 1.0), row);
    let moved = drag(&mut app, from, (from.0 + 0.4 * 48.0, row));
    assert_eq!(track(&moved, midi)["clips"][0]["start"], 0.5);
    // Zooming out to bar lines coarsens the snap with them.
    let zoomed =
        app.command(json!({"command":"scroll","x":from.0,"y":row,"delta":-400.0,"ctrl":true}));
    let zoom = ppb(&zoomed);
    assert!((2.5..10.0).contains(&zoom), "zoomed out to {zoom}");
    let from = (x_at(&zoomed, 6.0), row_y(&zoomed, lane));
    let moved = drag(&mut app, from, (from.0 + 2.4 * zoom, from.1));
    assert_eq!(
        patterns(&moved, lane)[0]["start"],
        8.0,
        "4 + 2.4 beats lands on the next bar line"
    );
    app.quit();
}

#[test]
fn tracks_are_added_from_the_arranger_footer_and_context_menu() {
    let mut app = Harness::new();
    let (opened, midi, lane) = fixture(&mut app);
    let count = opened["project"]["tracks"].as_array().unwrap().len();
    assert!(opened["timeline"]["add_track"].is_object());

    app.click("Add track…");
    let footer = app.click("Add audio track");
    let tracks = footer["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), count + 1);
    assert_eq!(tracks.last().unwrap()["kind"], "Audio");
    assert_eq!(footer["selected_track"], tracks.last().unwrap()["id"]);

    // Right-click on a track adds the new track right after it.
    pointer(
        &mut app,
        "click",
        (x_at(&footer, 20.0), row_y(&footer, midi)),
        "secondary",
    );
    app.click("Add track");
    let added = app.click("Add bus");
    let order: Vec<_> = added["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["track"].clone())
        .collect();
    let bus = added["selected_track"].clone();
    assert_eq!(track(&added, bus.as_u64().unwrap())["kind"], "Bus");
    let at = order.iter().position(|id| *id == bus).unwrap();
    assert!(
        order[at - 1] == midi || order[at - 1] == lane,
        "the bus follows the right-clicked track: {order:?}"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], footer["project"], "one undo step");

    // An instrument track keeps its place through the instrument chooser.
    pointer(
        &mut app,
        "click",
        (x_at(&undone, 20.0), row_y(&undone, midi)),
        "secondary",
    );
    app.click("Add track");
    app.click("Add instrument track…");
    // Choosing the instrument opens its device panel, so read the order from
    // the project rather than the arranger rows.
    let added = app.click("Add instrument track");
    let order: Vec<_> = added["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].clone())
        .collect();
    let instrument = added["selected_track"].clone();
    assert_eq!(track(&added, instrument.as_u64().unwrap())["kind"], "Midi");
    let at = order.iter().position(|id| *id == instrument).unwrap();
    assert!(
        order[at - 1] == midi || order[at - 1] == lane,
        "the instrument follows the right-clicked track: {order:?}"
    );
    app.quit();
}

fn rename_by_double_click(app: &mut Harness, at: (f64, f64), name: &str) -> Value {
    let editing = pointer(app, "double_click", at, "primary");
    assert_eq!(
        editing["view"], "Arrangement",
        "double-click never opens an editor"
    );
    assert!(
        editing["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Apply name")
    );
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.key("Backspace");
    app.command(json!({"command":"text","text":name}));
    app.click("Apply name")
}

#[test]
fn double_click_renames_every_clip_kind_and_edit_actions_apply_to_patterns() {
    let mut app = Harness::new();
    let (opened, midi, lane) = fixture(&mut app);
    let named =
        rename_by_double_click(&mut app, (x_at(&opened, 1.0), row_y(&opened, midi)), "Riff");
    assert_eq!(track(&named, midi)["clips"][0]["name"], "Riff");
    let body = inside(pattern_rect(&named, 0), 0.5, 0.7);
    let named = rename_by_double_click(&mut app, body, "Swell");
    assert_eq!(patterns(&named, lane)[0]["name"], "Swell");
    assert_eq!(track(&named, midi)["clips"].as_array().unwrap().len(), 1);

    // Edit → Split at cursor splits the selected pattern at the playhead.
    let state = app.command(json!({"command":"pointer","phase":"click","x":x_at(&named, 6.0),"y":named["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.0}));
    let cursor = state["timeline"]["cursor"].as_f64().unwrap();
    assert!(cursor > 4.0 && cursor < 8.0, "playhead at {cursor}");
    pointer(&mut app, "click", body, "primary");
    app.click("Edit");
    let split = app.click("Split at cursor");
    assert_eq!(patterns(&split, lane).len(), 2);
    assert_eq!(
        split["selected_clip"],
        patterns(&split, lane)[1]["id"],
        "the right half stays selected"
    );

    // Edit → Copy and Paste put a pattern on the selected lane at the playhead.
    pointer(
        &mut app,
        "click",
        inside(pattern_rect(&split, 0), 0.5, 0.7),
        "primary",
    );
    app.click("Edit");
    app.click("Copy");
    app.command(json!({"command":"pointer","phase":"click","x":x_at(&split, 20.0),"y":split["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.0}));
    pointer(
        &mut app,
        "click",
        inside(pattern_rect(&split, 0), 0.5, 0.7),
        "primary",
    );
    app.click("Edit");
    let pasted = app.click("Paste");
    let clips = patterns(&pasted, lane);
    assert_eq!(clips.len(), 3);
    assert!(clips.iter().any(|clip| clip["start"] == 20.0));
    app.quit();
}

#[test]
fn renaming_a_ghost_pattern_renames_its_linked_copies() {
    let mut app = Harness::new();
    let (opened, _, lane) = fixture(&mut app);
    let body = inside(pattern_rect(&opened, 0), 0.5, 0.7);
    pointer(&mut app, "click", body, "secondary");
    let ghost = app.click("Ghost copy");
    assert_eq!(patterns(&ghost, lane).len(), 2);
    let renamed =
        rename_by_double_click(&mut app, inside(pattern_rect(&ghost, 1), 0.5, 0.7), "Echo");
    let clips = patterns(&renamed, lane);
    assert_eq!(clips[0]["name"], "Echo");
    assert_eq!(clips[1]["name"], "Echo");
    app.quit();
}
