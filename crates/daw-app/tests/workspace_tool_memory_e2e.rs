//! Pointer tools belong to their editing workspace, including through utilities.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let midi = project.add_midi_track("Tool memory synth");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Tool memory phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let source = app.directory.path().join("tool-memory.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Tool memory audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Tool memory recording",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("tool-memory.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Tool memory phrase");
    app
}

fn window(app: &mut Harness, label: &str) -> Value {
    app.click("Window");
    app.click(label)
}

fn close(app: &mut Harness, title: &str) -> Value {
    let state = app.inspect();
    let title = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == title)
        .expect("visible popup title");
    let x = (title["min"][0].as_f64().unwrap() + title["max"][0].as_f64().unwrap()) / 2.0;
    let y = (title["min"][1].as_f64().unwrap() + title["max"][1].as_f64().unwrap()) / 2.0;
    let bounds = state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bounds| {
            bounds["min"][0].as_f64().unwrap() < x
                && bounds["max"][0].as_f64().unwrap() > x
                && bounds["min"][1].as_f64().unwrap() < y
                && bounds["max"][1].as_f64().unwrap() > y
        })
        .expect("popup bounds");
    let x = bounds["max"][0].as_f64().unwrap() - (y - bounds["min"][1].as_f64().unwrap());
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(
            state[field], before[field],
            "Tool navigation changed {field}"
        );
    }
}

#[test]
fn drawing_notes_then_back_restores_select_and_allows_immediate_mouse_trim() {
    let mut app = fixture();
    window(&mut app, "Clip Editor");
    let state = app.click("Pencil");
    let editor = &state["midi_editor"];
    let row = editor["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|pitch| pitch == 64)
        .unwrap() as f64;
    let x = editor["plot"]["min"][0].as_f64().unwrap()
        + (2.0 - editor["start_beat"].as_f64().unwrap())
            * editor["pixels_per_beat"].as_f64().unwrap();
    let y = editor["plot"]["min"][1].as_f64().unwrap()
        + (row + 0.5 - editor["row_offset"].as_f64().unwrap())
            * editor["row_height"].as_f64().unwrap();
    let drawn = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(
        drawn["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let before = app.click("Back");
    assert_eq!(before["view"], "Arrangement");
    assert_eq!(before["timeline"]["tool"], "Select");
    unchanged(&before, &drawn);
    let clip = &before["timeline"]["clips"][0]["rect"];
    let x = clip["max"][0].as_f64().unwrap() - 2.0;
    let y = (clip["min"][1].as_f64().unwrap() + clip["max"][1].as_f64().unwrap()) / 2.0;
    let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let trimmed = app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":x-scale, "to_y":y}));
    let clip = &trimmed["project"]["tracks"][0]["clips"][0];
    assert_eq!(
        clip["length"], 3.0,
        "Returning from Pencil must permit a trim, not draw"
    );
    assert_eq!(
        clip["content"],
        before["project"]["tracks"][0]["clips"][0]["content"]
    );
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], before["project"]);
    let restored = window(&mut app, "Clip Editor");
    assert_eq!(restored["midi_editor"]["tool"], "Pencil");
    app.quit();
}

#[test]
fn arranger_midi_and_audio_keep_distinct_tools_through_navigation_and_popups() {
    let mut app = fixture();
    for (width, dark) in [(800, false), (1440, true)] {
        app.command(json!({"command":"viewport", "width":width, "height":900}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        let original = app.click("Cut");
        let notes = window(&mut app, "Clip Editor");
        if width == 800 {
            assert_eq!(notes["midi_editor"]["tool"], "Select");
        }
        let notes = app.click("Pencil");
        assert_eq!(notes["midi_editor"]["tool"], "Pencil");
        let audio = app.click("Next");
        assert_eq!(audio["view"], "Clip");
        assert_eq!(
            audio["selected_clip"],
            audio["project"]["tracks"][1]["clips"][0]["id"]
        );
        if width == 800 {
            assert_eq!(audio["audio_editor"]["tool"], "Cursor");
        }
        assert_eq!(app.click("Fade in")["audio_editor"]["tool"], "Fade in");
        assert_eq!(app.click("Previous")["midi_editor"]["tool"], "Pencil");

        let notes = app.inspect();
        assert_eq!(app.click("Properties")["popups"]["properties"], true);
        assert_eq!(window(&mut app, "Library")["popups"]["library"], true);
        assert_eq!(
            close(&mut app, "Device library")["popups"]["properties"],
            true
        );
        let returned = close(&mut app, "Properties of Tool memory phrase");
        assert_eq!(returned["view"], "Clip");
        assert_eq!(returned["midi_editor"]["tool"], "Pencil");
        for field in ["selected_track", "selected_clip", "selected_notes"] {
            assert_eq!(returned[field], notes[field]);
        }
        unchanged(&returned, &original);
        assert_eq!(app.click("Next")["audio_editor"]["tool"], "Fade in");
        app.click("Previous");
        assert_eq!(app.click("Arranger")["timeline"]["tool"], "Cut");
        app.click("Mixer");
        let returned = app.click("Arranger");
        assert_eq!(returned["timeline"]["tool"], "Cut");
        unchanged(&returned, &original);

        // Letter and legacy numeric shortcuts update the current workspace's
        // remembered choice, exactly like its visible pointer-tool controls.
        window(&mut app, "Clip Editor");
        assert_eq!(app.key("Q")["midi_editor"]["tool"], "Select");
        assert_eq!(app.key("2")["midi_editor"]["tool"], "Pencil");
        assert_eq!(app.click("Arranger")["timeline"]["tool"], "Cut");
        assert_eq!(app.key("P")["timeline"]["tool"], "Cursor");
        assert_eq!(
            window(&mut app, "Clip Editor")["midi_editor"]["tool"],
            "Pencil"
        );
        assert_eq!(app.click("Arranger")["timeline"]["tool"], "Cursor");
        unchanged(&app.inspect(), &original);
    }
    app.quit();
}
