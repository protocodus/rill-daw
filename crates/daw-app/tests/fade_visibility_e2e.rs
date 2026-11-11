//! Fade reveal and the actual submitted envelope veil through the running app.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn region(state: &Value, id: u64) -> &Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1}))
}
fn undo(app: &mut Harness) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true}))
}
fn preview_mesh(state: &Value, rect: &Value) {
    let mesh = state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mesh| {
            let bounds = &mesh["bounds"];
            mesh["valid"] == true
                && mesh["max_alpha"].as_u64().is_some_and(|a| a > 0 && a <= 32)
                && bounds[0].as_f64().unwrap() >= rect["min"][0].as_f64().unwrap() - 1.0
                && bounds[1].as_f64().unwrap() >= rect["min"][1].as_f64().unwrap() - 1.0
                && bounds[2].as_f64().unwrap() <= rect["max"][0].as_f64().unwrap() + 1.0
                && bounds[3].as_f64().unwrap() <= rect["max"][1].as_f64().unwrap() + 1.0
        })
        .expect("A subtle valid mesh must actually be submitted within the attenuated clip region");
    assert!(mesh["vertices"].as_u64().unwrap() <= 4100);
}

#[test]
fn fade_handles_reveal_near_corners_keep_drag_ownership_and_leave_subtle_applied_previews() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let source = app.directory.path().join("fade.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.3; 192_000],
        }
        .write_wav(&source)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Fade visibility");
        let clip = Clip::audio("", 1.0, 4.0, source.to_string_lossy());
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let overlap = project.add_audio_track("Overlap");
        let incoming = Clip::audio("Incoming", 2.0, 4.0, source.to_string_lossy());
        let incoming_id = incoming.id;
        project.track_mut(overlap).unwrap().clips.extend([
            Clip::audio("Outgoing", 0.0, 6.0, source.to_string_lossy()),
            incoming,
        ]);
        let path = app.directory.path().join("fades.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":if dark {2.0} else {1.0}}));
        for editor in [false, true] {
            if editor {
                let state = app.inspect();
                let body = &region(&state, id)["content_rect"];
                let at = (center(body).0, body["max"][1].as_f64().unwrap() - 3.0);
                app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":at.0,"y":at.1}));
                assert_eq!(app.click("Edit audio")["view"], "Clip");
            }
            pointer(&mut app, "move", (1279.0, 0.0));
            let before = app.inspect();
            let view = if editor {
                &before["audio_editor"]
            } else {
                region(&before, id)
            };
            assert_eq!(view["fade_handles_visible"], json!([false, false]));
            assert!(view["fade_preview"].is_null());
            let at = center(&view["fade_handles"][0]);
            let hovered = pointer(&mut app, "move", at);
            let view = if editor {
                &hovered["audio_editor"]
            } else {
                region(&hovered, id)
            };
            assert_eq!(view["fade_handles_visible"], json!([true, false]));
            assert_eq!(hovered["project"], before["project"]);
            assert_eq!(hovered["history"], before["history"]);
            let ppb = before[if editor { "audio_editor" } else { "timeline" }]["pixels_per_beat"]
                .as_f64()
                .unwrap();
            pointer(&mut app, "down", at);
            let end = (at.0 + ppb * 0.75, at.1 + 100.0);
            let held = pointer(&mut app, "move", end);
            let view = if editor {
                &held["audio_editor"]
            } else {
                region(&held, id)
            };
            assert_eq!(
                view["fade_handles_visible"][0], true,
                "Owned drag stays visible outside reveal band"
            );
            pointer(&mut app, "up", end);
            pointer(&mut app, "move", (1279.0, 0.0));
            let faded = app.command(json!({"command":"inspect","paint":true}));
            let view = if editor {
                &faded["audio_editor"]
            } else {
                region(&faded, id)
            };
            assert_eq!(view["fade_handles_visible"], json!([false, false]));
            assert!(view["fade_preview"].is_object());
            preview_mesh(&faded, &view["fade_preview"]);
            assert!(
                (faded["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["fade_in"]
                    .as_f64()
                    .unwrap()
                    - 0.75)
                    .abs()
                    < 1e-6
            );
            assert_eq!(
                faded["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
            if !editor {
                let clip_rect = &region(&faded, id)["rect"];
                let edge = (
                    clip_rect["min"][0].as_f64().unwrap() + 1.0,
                    clip_rect["max"][1].as_f64().unwrap() - 3.0,
                );
                app.command(
                    json!({"command":"pointer","phase":"drag","x":edge.0,"y":edge.1,
                    "to_x":edge.0+ppb*0.25,"to_y":edge.1}),
                );
                let state = app.inspect();
                // Anchor wheel zoom to this clip, keeping its attenuated start
                // visible rather than centering a sparse song's empty right side.
                let anchor = center(&region(&state, id)["content_rect"]);
                app.command(
                    json!({"command":"scroll","x":anchor.0,"y":anchor.1,"delta":100.0,"ctrl":true}),
                );
                pointer(&mut app, "move", (1279.0, 0.0));
                let zoomed = app.command(json!({"command":"inspect","paint":true}));
                assert!(
                    (zoomed["project"]["tracks"][0]["clips"][0]["start"]
                        .as_f64()
                        .unwrap()
                        - 1.25)
                        .abs()
                        < 1e-6
                );
                let surface = region(&zoomed, id);
                let preview = &surface["fade_preview"];
                assert!(
                    preview["min"][0].as_f64().unwrap()
                        >= surface["rect"]["min"][0].as_f64().unwrap() - 1.0
                );
                assert!(
                    preview["max"][0].as_f64().unwrap()
                        < surface["content_rect"]["max"][0].as_f64().unwrap()
                );
                let expected_end = surface["rect"]["min"][0].as_f64().unwrap()
                    + 0.5 * zoomed["timeline"]["pixels_per_beat"].as_f64().unwrap();
                assert!(
                    (preview["max"][0].as_f64().unwrap() - expected_end).abs() < 2.0,
                    "Trimmed fade keeps its original source endpoint after zoom"
                );
                assert!(
                    preview["min"][1].as_f64().unwrap()
                        >= surface["content_rect"]["min"][1].as_f64().unwrap()
                );
                preview_mesh(&zoomed, preview);
                assert_eq!(
                    undo(&mut app)["project"],
                    faded["project"],
                    "Trim undo preserves the preceding fade"
                );
            }
            assert_eq!(undo(&mut app)["project"], before["project"]);
            let clear = app.inspect();
            assert!(
                if editor {
                    &clear["audio_editor"]
                } else {
                    region(&clear, id)
                }["fade_preview"]
                    .is_null()
            );
            if editor {
                app.click("Arranger");
            }
        }
        let idle = app.inspect();
        let view = region(&idle, incoming_id);
        assert_eq!(view["crossfade_handle_visible"], false);
        let at = center(&view["crossfade_handle"]);
        assert_eq!(
            region(&pointer(&mut app, "move", at), incoming_id)["crossfade_handle_visible"],
            true
        );
        let ppb = idle["timeline"]["pixels_per_beat"].as_f64().unwrap();
        pointer(&mut app, "down", at);
        let end = (at.0 + ppb * 0.5, at.1 + 50.0);
        assert_eq!(
            region(&pointer(&mut app, "move", end), incoming_id)["crossfade_handle_visible"],
            true
        );
        pointer(&mut app, "up", end);
        let away = pointer(&mut app, "move", (1279.0, 0.0));
        assert_eq!(
            region(&away, incoming_id)["crossfade_handle_visible"],
            false
        );
        assert_eq!(undo(&mut app)["project"], idle["project"]);
        app.quit();
    }
}
