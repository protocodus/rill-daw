//! An optional clip title leaves its space to music and remains easy to restore.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn model(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn region(state: &Value, id: u64) -> &Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn point(rect: &Value, x: f64, y: f64) -> (f64, f64) {
    let at = |axis: usize, fraction: f64| {
        rect["min"][axis].as_f64().unwrap()
            + (rect["max"][axis].as_f64().unwrap() - rect["min"][axis].as_f64().unwrap()) * fraction
    };
    (at(0, x), at(1, y))
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64), secondary: bool) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1,
        "button":if secondary {"secondary"} else {"primary"}}))
}

fn context_name(app: &mut Harness, id: u64, name: &str) -> Value {
    let state = app.inspect();
    let at = point(&region(&state, id)["rect"], 0.5, 0.8);
    pointer(app, "click", at, true);
    app.click("Rename…");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.key("Backspace");
    if !name.is_empty() {
        app.command(json!({"command":"text","text":name}));
    }
    app.click("Apply name");
    app.inspect()
}

fn history(app: &mut Harness, redo: bool) -> Value {
    app.click("Edit");
    app.click(if redo { "Redo" } else { "Undo" })
}

#[test]
fn audio_and_midi_names_can_be_hidden_restored_and_recalled_without_losing_clip_editing() {
    for (dark, scale) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let source = app.directory.path().join("source.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.1; 192_000],
        }
        .write_wav(&source)
        .unwrap();
        let source_bytes = std::fs::read(&source).unwrap();
        let mut project = Project::new();
        let audio = project.add_audio_track("Audio track");
        let audio_clip = Clip::audio("Audio name", 1.0, 4.0, source.to_string_lossy());
        let audio_id = audio_clip.id;
        project.track_mut(audio).unwrap().clips.push(audio_clip);
        let midi = project.add_midi_track("MIDI track");
        let midi_clip = Clip::midi(
            "MIDI name",
            1.0,
            4.0,
            vec![Note::new(60, 0.0, 1.0, 90), Note::new(67, 2.0, 1.0, 100)],
        );
        let midi_id = midi_clip.id;
        project.track_mut(midi).unwrap().clips.push(midi_clip);
        let path = app.directory.path().join("unnamed.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":scale}),
        );

        for (id, blank) in [(audio_id, ""), (midi_id, "   ")] {
            let before = app.inspect();
            let old_content_top = region(&before, id)["content_rect"]["min"][1]
                .as_f64()
                .unwrap();
            let old_name = model(&before, id)["name"].clone();
            let cleared = context_name(&mut app, id, blank);
            assert_eq!(model(&cleared, id)["name"], "");
            assert_eq!(
                cleared["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
            let clip = region(&cleared, id);
            assert_eq!(clip["named_header"], false);
            assert!(
                clip["title_rect"]["max"][1].as_f64().unwrap()
                    - clip["title_rect"]["min"][1].as_f64().unwrap()
                    <= 3.5
            );
            assert!(old_content_top - clip["content_rect"]["min"][1].as_f64().unwrap() >= 16.0);
            assert!(clip["link_icon"].is_null());
            assert_eq!(model(&history(&mut app, false), id)["name"], old_name);
            assert_eq!(model(&history(&mut app, true), id)["name"], "");

            let state = app.inspect();
            let at = point(&region(&state, id)["name_hit_rect"], 0.5, 0.25);
            let restoring = pointer(&mut app, "double_click", at, false);
            assert_eq!(restoring["view"], "Arrangement");
            assert!(
                restoring["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "Apply name")
            );
            app.command(json!({"command":"text","text":"Border restored"}));
            let restored = app.click("Apply name");
            assert_eq!(model(&restored, id)["name"], "Border restored");
            assert_eq!(region(&app.inspect(), id)["named_header"], true);

            context_name(&mut app, id, "");
            let restored = context_name(&mut app, id, "Context restored");
            assert_eq!(model(&restored, id)["name"], "Context restored");
            let empty = context_name(&mut app, id, "");
            if id == audio_id {
                let handle = &region(&empty, id)["fade_in_handle"];
                // This starts inside the generous rename band, on a real fade
                // handle. The existing handle keeps pointer ownership.
                let fade = point(handle, 0.5, 0.1);
                let zoom = empty["timeline"]["pixels_per_beat"].as_f64().unwrap();
                let faded = app.command(json!({"command":"pointer","phase":"drag","x":fade.0,"y":fade.1,"to_x":fade.0+zoom*0.5,"to_y":fade.1}));
                assert!(
                    (model(&faded, id)["content"]["Audio"]["fade_in"]
                        .as_f64()
                        .unwrap()
                        - 0.5)
                        .abs()
                        < 1e-6
                );
                assert!(
                    !faded["visible_labels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|label| label == "Apply name")
                );
                history(&mut app, false);
            }
            let empty = app.inspect();
            let body = point(&region(&empty, id)["content_rect"], 0.5, 0.7);
            let opened = pointer(&mut app, "double_click", body, false);
            assert_eq!(
                opened["view"], "Clip",
                "Body double-click still opens the editor"
            );
            assert_eq!(opened["selected_clip"], id);
            app.click("Arranger");

            let before = app.inspect();
            let body = point(&region(&before, id)["content_rect"], 0.5, 0.85);
            let zoom = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
            let moved = app.command(json!({"command":"pointer","phase":"drag","x":body.0,"y":body.1,"to_x":body.0+zoom,"to_y":body.1}));
            assert!((model(&moved, id)["start"].as_f64().unwrap() - 2.0).abs() < 1e-6);
            history(&mut app, false);
            let before = app.inspect();
            let edge = point(&region(&before, id)["rect"], 1.0, 0.85);
            let trimmed = app.command(json!({"command":"pointer","phase":"drag","x":edge.0-1.0,"y":edge.1,"to_x":edge.0-1.0-zoom*0.5,"to_y":edge.1}));
            assert!((model(&trimmed, id)["length"].as_f64().unwrap() - 3.5).abs() < 1e-6);
            history(&mut app, false);
            assert_eq!(model(&app.inspect(), id)["name"], "");
        }
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        app.command(json!({"command":"save","path":path}));
        for _ in 0..200 {
            if app.inspect()["modified"] == false {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut app = app.restart();
        app.command(json!({"command":"open","path":path}));
        let restored = app.inspect();
        for id in [audio_id, midi_id] {
            assert_eq!(model(&restored, id)["name"], "");
            assert_eq!(region(&restored, id)["named_header"], false);
        }
        app.quit();
    }
}
