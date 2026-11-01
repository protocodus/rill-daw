//! Clip orientation and shared commands survive occurrence and media changes.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

const PHRASE: &str = "Repeated phrase · 夜のシンセ with a deliberately long name";

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(state[field], before[field], "Orientation changed {field}");
    }
}

fn open_clip(app: &mut Harness, clip: u64) -> Value {
    let state = app.inspect();
    let target = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["id"] == clip)
        .unwrap();
    let (x, y) = center(&target["visible_rect"]);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    app.click("Window");
    app.click("Clip Editor")
}

#[test]
fn repeated_clip_shows_song_span_local_ruler_and_stable_properties_without_losing_canvas() {
    let mut app = Harness::new();
    let source = app.directory.path().join("context.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 192_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.beats_per_bar = 4;
    let midi = project.add_midi_track("Evening lead");
    let first = Clip::midi(PHRASE, 0.0, 8.0, vec![Note::new(60, 2.0, 1.0, 100)]);
    let first_id = first.id;
    let repeated = Clip::midi(PHRASE, 32.0, 8.0, vec![Note::new(64, 2.0, 1.0, 100)]);
    let repeated_id = repeated.id;
    project
        .track_mut(midi)
        .unwrap()
        .clips
        .extend([first, repeated]);
    let audio = project.add_audio_track("Rhythm source");
    let audio_clip = Clip::audio("Rhythm phrase", 48.0, 8.0, source.to_string_lossy());
    let audio_id = audio_clip.id;
    project.track_mut(audio).unwrap().clips.push(audio_clip);
    let path = app.directory.path().join("clip-context.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));

    for (width, density, dark) in [(800, 1.0, false), (800, 2.0, true), (1440, 1.0, true)] {
        app.command(
            json!({"command":"viewport", "width":width, "height":900, "pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.click("Fit");
        let mut properties = None;
        let mut local_canvas_top = None;
        for (id, context, global_start) in [
            (first_id, "Song 1:1–3:1 · Evening lead", Some(0.0)),
            (repeated_id, "Song 9:1–11:1 · Evening lead", Some(32.0)),
            (audio_id, "Song 13:1–15:1 · Rhythm source", None),
        ] {
            let state = open_clip(&mut app, id);
            unchanged(&state, &original);
            assert_eq!(state["selected_clip"], id);
            let command = label(&state, "Properties");
            let identity = label(&state, context);
            assert!(identity["min"][0].as_f64().unwrap() >= 0.0);
            let painted = app.command(json!({"command":"inspect", "paint":true}));
            let rendered_context = painted["painted_text"]
                .as_array()
                .unwrap()
                .iter()
                .find(|text| text["text"] == context)
                .and_then(|text| text["rendered_text"].as_str())
                .expect("The context caption is painted");
            let song_span = context.split(" · ").next().unwrap();
            assert!(
                rendered_context.starts_with(song_span),
                "The complete global span must be rendered beside header actions: {rendered_context:?}, expected {song_span:?}"
            );
            assert!(identity["max"][0].as_f64().unwrap() < command["min"][0].as_f64().unwrap());
            assert!(command["max"][0].as_f64().unwrap() <= f64::from(width));
            if let Some(expected) = &properties {
                assert_eq!(command, expected, "Properties moved between clip kinds");
            }
            properties = Some(command.clone());

            if width < 1000 {
                let ai = state["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|label| {
                        label["text"] == "AI"
                            && (label["min"][1].as_f64().unwrap()
                                - command["min"][1].as_f64().unwrap())
                            .abs()
                                < 1.0
                    })
                    .expect("The clip header keeps its own compact scoped AI action");
                let (x, y) = center(ai);
                let scoped =
                    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
                assert_eq!(
                    scoped["assistant"]["scope"]["kind"],
                    if global_start.is_some() {
                        "midi_clip"
                    } else {
                        "audio_clip"
                    }
                );
                unchanged(&scoped, &original);
                let returned = app.key("Escape");
                assert_eq!(returned["selected_clip"], id);
                unchanged(&returned, &original);
            }

            let expected_ruler_hint = if global_start.is_some() {
                "The editor ruler starts at local bar 1 for this clip."
            } else {
                "The editor ruler shows song bars and beats."
            };
            let (x, y) = center(identity);
            app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
            let mut saw_ruler_hint = false;
            for _ in 0..80 {
                let hovered = app.inspect();
                unchanged(&hovered, &original);
                if hovered["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| {
                        label.as_str().is_some_and(|text| {
                            text.contains("\nSong position:") && text.ends_with(expected_ruler_hint)
                        })
                    })
                {
                    assert_eq!(
                        hovered["visible_labels"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|label| label.as_str() == Some(context))
                            .count(),
                        1,
                        "The header caption must not be repeated in separate elision tooltips",
                    );
                    saw_ruler_hint = true;
                    break;
                }
            }
            assert!(
                saw_ruler_hint,
                "The context hint must describe this clip's actual ruler: {expected_ruler_hint}"
            );

            if let Some(global_start) = global_start {
                let editor = &state["midi_editor"];
                assert_eq!(editor["time_basis"], "clip-local");
                assert_eq!(editor["clip_global_start"], global_start);
                assert_eq!(editor["clip_global_end"], global_start + 8.0);
                let paint = app.command(json!({"command":"inspect", "paint":true}));
                assert!(
                    paint["painted_text"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|text| text["text"] == "Local")
                );
                let plot = &editor["plot"];
                let top = plot["min"][1].as_f64().unwrap();
                assert!(
                    plot["max"][1].as_f64().unwrap() - top > 350.0,
                    "Context displaced the piano roll"
                );
                if let Some(expected) = local_canvas_top {
                    assert_eq!(top, expected, "An occurrence changed canvas height");
                }
                local_canvas_top = Some(top);
                let local_position = 2.0;
                let x = plot["min"][0].as_f64().unwrap()
                    + (local_position - editor["start_beat"].as_f64().unwrap())
                        * editor["pixels_per_beat"].as_f64().unwrap();
                let (_, y) = center(&editor["ruler"]);
                let moved =
                    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
                assert!(
                    (moved["midi_editor"]["cursor"].as_f64().unwrap()
                        - global_start
                        - local_position)
                        .abs()
                        < 1e-6
                );
                assert!(
                    (moved["midi_editor"]["local_cursor"].as_f64().unwrap() - local_position).abs()
                        < 1e-6
                );
                unchanged(&moved, &original);
            }

            let popup = app.click("Properties");
            assert_eq!(popup["popups"]["properties"], true);
            assert_eq!(popup["inspector_page"], "Clip");
            assert_eq!(popup["backdrop_blur"], json!([]));
            let returned = app.key("Escape");
            unchanged(&returned, &original);
            assert_eq!(returned["selected_clip"], id);
            assert_eq!(label(&returned, "Properties"), properties.as_ref().unwrap());
            app.click("Back");
        }
    }
    app.quit();
}
