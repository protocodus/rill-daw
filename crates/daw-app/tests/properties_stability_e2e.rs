//! Properties keeps mouse step targets stable when the first stretch is applied.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(midi: bool, width: u32, height: u32) -> (Harness, u64) {
    let mut app = Harness::new();
    let source = app.directory.path().join("original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 192_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = if midi {
        project.add_midi_track("MIDI timing")
    } else {
        project.add_audio_track("Audio timing")
    };
    let mut clip = if midi {
        Clip::midi(
            "Stable properties",
            2.0,
            4.0,
            vec![Note::new(60, 1.0, 1.0, 90), Note::new(67, 3.0, 1.0, 80)],
        )
    } else {
        Clip::audio("Stable properties", 2.0, 4.0, source.to_string_lossy())
    };
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("properties-stability.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":width, "height":height}));
    app.click("Select");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    let selected = pointer(&mut app, center(rect));
    assert_eq!(selected["selected_clip"], id);
    app.click("Window");
    app.click("Properties");
    (app, id)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn pointer(app: &mut Harness, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer", "phase":"click", "x":at.0, "y":at.1}))
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn duration_button<'a>(state: &'a Value, symbol: &str) -> &'a Value {
    let row = label(state, "Duration");
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|candidate| {
            candidate["text"] == symbol
                && (center(candidate).1 - center(row).1).abs() < 2.0
                && candidate["min"][0].as_f64().unwrap() > row["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| panic!("Missing Duration {symbol}: {}", state["label_rects"]))
}

fn same_rect(actual: &Value, expected: &Value) {
    for edge in ["min", "max"] {
        for axis in 0..2 {
            assert!(
                (actual[edge][axis].as_f64().unwrap() - expected[edge][axis].as_f64().unwrap())
                    .abs()
                    <= 1.0,
                "Properties target moved: {expected} -> {actual}"
            );
        }
    }
}

fn assert_geometry(state: &Value, before: &Value, width: u32, height: u32) {
    assert_eq!(
        state["popups"]["properties"], true,
        "Properties closed: {state}"
    );
    // The final Stretch label is the readout row, after the Duration mode menu.
    for text in ["Duration", "Source offset", "Stretch"] {
        same_rect(label(state, text), label(before, text));
    }
    for symbol in ["−", "+"] {
        let button = duration_button(state, symbol);
        same_rect(button, duration_button(before, symbol));
        assert!(button["min"][0].as_f64().unwrap() >= 0.0);
        assert!(button["min"][1].as_f64().unwrap() >= 0.0);
        assert!(button["max"][0].as_f64().unwrap() <= width as f64);
        assert!(button["max"][1].as_f64().unwrap() <= height as f64);
    }
}

fn assert_readout(state: &Value, duration: f64) {
    let text = format!("{:.1}%", 100.0 * duration / 4.0);
    let readout = label(state, &text);
    assert!((center(readout).1 - center(label(state, "Stretch")).1).abs() < 2.0);
}

fn repeat_stretch_steps(midi: bool) {
    for (width, height) in [(800, 600), (1440, 1000)] {
        let (mut app, id) = fixture(midi, width, height);
        let original = app.inspect();
        assert_eq!(
            original["project"]["tracks"][0]["clips"][0]["source_rate"],
            1.0
        );
        assert_readout(&original, 4.0);
        assert_geometry(&original, &original, width, height);
        let choice = app.click("Trim");
        let stretch = choice["menu_label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "Stretch")
            .unwrap();
        let chosen = pointer(&mut app, center(stretch));
        assert_eq!(chosen["project"], original["project"]);
        assert_eq!(chosen["history"], original["history"]);
        assert_readout(&chosen, 4.0);
        // Choosing a mode can scroll its focused row into view in the short
        // audio popup. Freeze targets after that distinct navigation action,
        // while the clip is still unstretched and its readout is still 100%.
        assert_geometry(&chosen, &chosen, width, height);
        let plus = center(duration_button(&chosen, "+"));
        let minus = center(duration_button(&chosen, "−"));

        let mut snapshots = vec![original["project"].clone()];
        let baseline_undo = original["history"]["undo"].as_array().unwrap().len();
        // Reuse the original coordinates, as a person repeatedly pressing +/−
        // does. A shifted popup must not turn a later click into another edit.
        for (index, (at, duration)) in [(plus, 5.0), (plus, 6.0), (minus, 5.0), (minus, 4.0)]
            .into_iter()
            .enumerate()
        {
            let changed = pointer(&mut app, at);
            let clip = &changed["project"]["tracks"][0]["clips"][0];
            assert_eq!(clip["id"], id);
            assert_eq!(clip["length"], duration);
            assert!(
                (clip["source_rate"].as_f64().unwrap() - 4.0 / duration).abs() < 1e-12,
                "Wrong stretch rate for duration {duration}: {clip}"
            );
            let mut expected = original["project"].clone();
            expected["tracks"][0]["clips"][0]["length"] = json!(duration);
            expected["tracks"][0]["clips"][0]["source_rate"] = clip["source_rate"].clone();
            if !midi {
                // Stretch makes the original source fade window explicit so
                // subsequent duration edits preserve its non-destructive shape.
                expected["tracks"][0]["clips"][0]["content"]["Audio"]["envelope"] =
                    json!({"length":4.0, "offset":0.0});
            }
            assert_eq!(
                changed["project"], expected,
                "A duration click preserves the source window and unrelated properties"
            );
            assert_eq!(changed["selected_clip"], id);
            assert_eq!(
                changed["history"]["undo"].as_array().unwrap().len(),
                baseline_undo + index + 1
            );
            assert_geometry(&changed, &chosen, width, height);
            assert_readout(&changed, duration);
            snapshots.push(changed["project"].clone());
        }
        for index in (0..4).rev() {
            app.click("Edit");
            let undone = app.click("Undo");
            assert_eq!(
                undone["project"], snapshots[index],
                "One Undo per mouse step"
            );
            assert_eq!(
                undone["history"]["undo"].as_array().unwrap().len(),
                baseline_undo + index
            );
        }
        for expected in snapshots.iter().skip(1) {
            app.click("Edit");
            assert_eq!(app.click("Redo")["project"], *expected);
        }
        app.quit();
    }
}

#[test]
fn audio_properties_keep_first_and_repeated_stretch_steps_stable_at_small_and_large_sizes() {
    repeat_stretch_steps(false);
}

#[test]
fn midi_properties_keep_first_and_repeated_stretch_steps_stable_at_small_and_large_sizes() {
    repeat_stretch_steps(true);
}
