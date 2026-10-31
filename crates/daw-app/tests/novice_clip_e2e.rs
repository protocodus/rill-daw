//! Direct clip timing and discoverable pattern creation in the production app.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, MAX_BEAT, Note, Project};
use serde_json::{Value, json};

fn window(app: &mut Harness, name: &str) -> Value {
    app.click("Window");
    app.click(name)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn properties(app: &mut Harness, id: u64) -> Value {
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"];
    let (x, y) = center(rect);
    let state = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(state["selected_clip"], id);
    window(app, "Properties")
}

fn edit_timing(app: &mut Harness, name: &str, value: &str) -> Value {
    let state = app.inspect();
    let rects = state["label_rects"].as_array().unwrap();
    let label = rects
        .iter()
        .rev()
        .find(|rect| rect["text"] == name)
        .unwrap();
    let y = center(label).1;
    let number = rects
        .iter()
        .rev()
        .find(|rect| {
            let timing_text = rect["text"].as_str().is_some_and(|text| {
                if name == "Start" {
                    text.starts_with("Beat ")
                } else {
                    text.ends_with(" beats")
                }
            });
            timing_text
                && (center(rect).1 - y).abs() < 12.0
                && rect["min"][0].as_f64().unwrap() > label["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| panic!("Missing {name} input: {}", state["visible_labels"]));
    let (x, y) = center(number);
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

fn open_fixture(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("clip-workflow.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}))
}

#[test]
fn midi_timing_controls_move_trim_validate_and_recall_without_changing_ghost_peers() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("MIDI timing");
    let mut source = Clip::midi(
        "Phrase",
        2.0,
        4.0,
        vec![Note::new(60, 0.5, 1.0, 100), Note::new(67, 3.0, 1.0, 85)],
    );
    source.offset = 0.5;
    source.source_rate = 1.5;
    let first = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let peer = project.clone_clip(track, first).unwrap();
    project.trim_clip(track, peer, 7.0, 2.0).unwrap();
    open_fixture(&mut app, &project);
    let before = properties(&mut app, first);
    assert_eq!(before["modified"], false);

    let moved = edit_timing(&mut app, "Start", "9");
    assert_eq!(clip(&moved, first)["start"], 8.0, "Start is one-based");
    for field in ["offset", "length", "source_rate", "content", "link_group"] {
        assert_eq!(clip(&moved, first)[field], clip(&before, first)[field]);
    }
    assert_eq!(clip(&moved, peer), clip(&before, peer));
    assert_eq!(moved["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], moved["project"]);

    let trimmed = edit_timing(&mut app, "Duration", "2.5");
    assert_eq!(clip(&trimmed, first)["length"], 2.5);
    for field in ["start", "offset", "source_rate", "content", "link_group"] {
        assert_eq!(clip(&trimmed, first)[field], clip(&moved, first)[field]);
    }
    assert_eq!(clip(&trimmed, peer), clip(&before, peer));
    assert_eq!(trimmed["history"]["undo"].as_array().unwrap().len(), 2);
    assert_eq!(undo(&mut app, false)["project"], moved["project"]);
    assert_eq!(undo(&mut app, true)["project"], trimmed["project"]);

    let invalid = edit_timing(&mut app, "Duration", "1000000");
    assert_eq!(invalid["project"], trimmed["project"]);
    assert_eq!(invalid["history"], trimmed["history"]);
    assert_eq!(invalid["error"], true);
    app.click("Dismiss");

    // Shared precise timing bounds Start by the existing duration, so an
    // excessive value clamps to the latest valid occurrence placement.
    let clamped = edit_timing(&mut app, "Start", "1000001");
    let bounded = clip(&clamped, first);
    let length = clip(&trimmed, first)["length"].as_f64().unwrap();
    assert_eq!(bounded["start"], MAX_BEAT - length);
    assert_eq!(bounded["start"].as_f64().unwrap() + length, MAX_BEAT);
    for field in ["offset", "length", "source_rate", "content", "link_group"] {
        assert_eq!(bounded[field], clip(&trimmed, first)[field]);
    }
    assert_eq!(clip(&clamped, peer), clip(&trimmed, peer));
    assert_eq!(clamped["error"], false);
    assert_eq!(
        clamped["history"]["undo"].as_array().unwrap().len(),
        trimmed["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(undo(&mut app, false)["project"], trimmed["project"]);
    assert_eq!(undo(&mut app, true)["project"], clamped["project"]);
    assert_eq!(undo(&mut app, false)["project"], trimmed["project"]);

    let path = app.directory.path().join("edited-timing.rill");
    let saved = app.command(json!({"command":"save", "path":path}));
    assert_eq!(saved["modified"], false);
    app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":path}));
    assert_eq!(reopened["project"], trimmed["project"]);
    assert_eq!(reopened["modified"], false);
    app.quit();
}

#[test]
fn reversed_audio_duration_uses_trim_source_mapping_and_preserves_its_ghost_peer() {
    let mut app = Harness::new();
    let path = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: vec![0.1; 480000],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Audio timing");
    let mut source = Clip::audio("Reversed", 2.0, 4.0, path.to_string_lossy());
    source.offset = 3.0;
    source.source_rate = 0.75;
    if let ClipContent::Audio { reverse, .. } = &mut source.content {
        *reverse = true;
    }
    let first = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let peer = project.clone_clip(track, first).unwrap();
    open_fixture(&mut app, &project);
    let before = properties(&mut app, first);
    let trimmed = edit_timing(&mut app, "Duration", "2");
    assert_eq!(clip(&trimmed, first)["length"], 2.0);
    assert_eq!(clip(&trimmed, first)["offset"], 4.5);
    assert_eq!(clip(&trimmed, first)["source_rate"], 0.75);
    assert_eq!(
        clip(&trimmed, first)["content"],
        clip(&before, first)["content"]
    );
    assert_eq!(clip(&trimmed, peer), clip(&before, peer));
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], trimmed["project"]);
    let moved = edit_timing(&mut app, "Start", "5");
    assert_eq!(clip(&moved, first)["start"], 4.0);
    assert_eq!(clip(&moved, first)["offset"], 4.5);
    assert_eq!(
        clip(&moved, first)["content"],
        clip(&before, first)["content"]
    );
    assert_eq!(clip(&moved, peer), clip(&before, peer));
    app.quit();
}

#[test]
fn add_pattern_uses_the_selected_midi_lane_and_cursor_and_can_create_a_track() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.add_midi_track("Blank MIDI");
    open_fixture(&mut app, &project);
    let state = app.click("Blank MIDI");
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Double-click to add a pattern")
    );
    let canvas = &state["timeline"]["canvas"];
    let x = canvas["min"][0].as_f64().unwrap()
        + (6.0 - state["timeline"]["start_beat"].as_f64().unwrap())
            * state["timeline"]["pixels_per_beat"].as_f64().unwrap();
    // Keep the original direct blank-lane gesture alongside the visible action.
    let lane_y = center(&state["timeline"]["track_colors"][0]["rect"]).1;
    let drawn =
        app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":lane_y}));
    assert_eq!(drawn["project"]["tracks"][0]["clips"][0]["start"], 6.0);
    assert_eq!(drawn["project"]["tracks"][0]["clips"][0]["length"], 4.0);
    assert_eq!(undo(&mut app, false)["project"], state["project"]);

    let y = canvas["min"][1].as_f64().unwrap() - 16.0;
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    let added = app.click("Add pattern");
    assert_eq!(added["project"]["tracks"].as_array().unwrap().len(), 1);
    let pattern = &added["project"]["tracks"][0]["clips"][0];
    assert_eq!(pattern["start"], 6.0);
    assert_eq!(pattern["length"], 4.0);
    assert_eq!(added["selected_clip"], pattern["id"]);
    assert_eq!(undo(&mut app, false)["project"], state["project"]);
    assert_eq!(undo(&mut app, true)["project"], added["project"]);
    undo(&mut app, false);

    let empty = Project::new();
    open_fixture(&mut app, &empty);
    let created = app.click("Add pattern");
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(created["project"]["tracks"][0]["kind"], "Midi");
    assert_eq!(created["project"]["tracks"][0]["clips"][0]["length"], 4.0);
    assert_eq!(created["popups"]["library"], true);
    app.quit();
}

#[test]
fn automatic_arranger_fit_keeps_sparse_tracks_readable_and_still_reveals_every_row() {
    let mut app = Harness::new();
    let ordinary_height = f64::from(daw_ui::TRACK_HEADER_HEIGHT);
    for tracks in [1, 2, 12] {
        let mut project = Project::new();
        for index in 0..tracks {
            let track = project.add_midi_track(format!("Lane {index}"));
            project.track_mut(track).unwrap().clips.push(Clip::midi(
                format!("Pattern {index}"),
                48.0,
                8.0,
                vec![],
            ));
        }
        let original = open_fixture(&mut app, &project)["project"].clone();
        for keyboard in [false, true] {
            let state = if keyboard {
                let previous = app.inspect();
                let (x, y) = center(&previous["timeline"]["canvas"]);
                app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
                app.key("F")
            } else {
                app.click("Fit")
            };
            let view = &state["timeline"];
            let height = view["row_height"].as_f64().unwrap();
            assert!(
                height <= ordinary_height,
                "Automatic Fit enlarged sparse rows beyond the ordinary header: {view}"
            );
            if tracks <= 2 {
                assert_eq!(height, ordinary_height);
            }
            let canvas_height = view["canvas"]["max"][1].as_f64().unwrap()
                - view["canvas"]["min"][1].as_f64().unwrap();
            assert!(height * tracks as f64 <= canvas_height + 0.01);
            let canvas_width = view["canvas"]["max"][0].as_f64().unwrap()
                - view["canvas"]["min"][0].as_f64().unwrap();
            let left = (48.0 - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap();
            let right = (56.0 - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap();
            assert!(left >= -0.01 && right <= canvas_width + 0.01);
            assert!(
                state["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == &format!("Lane {}", tracks - 1))
            );
            assert_eq!(state["project"], original);
        }
    }
    app.quit();
}
