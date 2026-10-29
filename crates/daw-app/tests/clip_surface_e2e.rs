//! Clip titles stay separate from musical previews through actual app paint.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use eframe::egui::Color32;
use serde_json::{Value, json};

fn bounds(value: &Value) -> [f64; 4] {
    [
        value["min"][0].as_f64().unwrap(),
        value["min"][1].as_f64().unwrap(),
        value["max"][0].as_f64().unwrap(),
        value["max"][1].as_f64().unwrap(),
    ]
}

fn contains(outer: [f64; 4], inner: [f64; 4]) -> bool {
    inner[0] >= outer[0] - 0.01
        && inner[1] >= outer[1] - 0.01
        && inner[2] <= outer[2] + 0.01
        && inner[3] <= outer[3] + 0.01
}

#[test]
fn audio_and_midi_clips_keep_titles_and_previews_separate_in_both_appearances() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Clip source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio surface");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio clip",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI surface");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI clip",
        0.0,
        4.0,
        vec![
            Note::new(0, 0.0, 1.0, 100),
            Note::new(60, 1.0, 1.0, 100),
            Note::new(127, 2.0, 1.0, 100),
        ],
    ));
    let compact = project.add_midi_track("Compact surface");
    project.track_mut(compact).unwrap().collapsed = true;
    project.track_mut(compact).unwrap().clips.push(Clip::midi(
        "Compact clip",
        0.0,
        4.0,
        vec![Note::new(127, 0.0, 4.0, 100)],
    ));
    let path = app.directory.path().join("clip-surfaces.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open", "path":path}));

    for (dark, density, width) in [(false, 1.0, 1100), (true, 2.0, 800)] {
        app.command(
            json!({"command":"viewport", "width":width, "height":760, "pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme", "dark":dark}));
        for name in ["MIDI clip", "Audio clip", "Compact clip"] {
            app.click(name);
            let state = app.command(json!({"command":"inspect", "paint":true}));
            assert_eq!(state["project"], before["project"]);
            assert_eq!(state["history"], before["history"]);
            for clip in state["timeline"]["clips"].as_array().unwrap() {
                let rect = bounds(&clip["rect"]);
                let title = bounds(&clip["title_rect"]);
                let content = bounds(&clip["content_rect"]);
                assert!(
                    contains(rect, title),
                    "title must stay inside its clip: {clip}"
                );
                assert!(
                    contains(rect, content),
                    "preview must stay inside its clip: {clip}"
                );
                assert!(title[3] <= content[1], "preview must leave the title clear");
                let surfaces: Vec<_> = state["painted_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|shape| bounds(&shape["rect"]) == rect && shape["fill"][3] == 255)
                    .collect();
                assert!(!surfaces.is_empty(), "both clip types need an opaque body");
                if clip["track"] == midi {
                    let track = project.track(midi).unwrap();
                    let color = Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
                    let note_color = json!(color.gamma_multiply(0.85).to_array());
                    let notes: Vec<_> = state["painted_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|shape| {
                            shape["fill"] == note_color && contains(rect, bounds(&shape["rect"]))
                        })
                        .collect();
                    assert_eq!(notes.len(), 3, "all pitches must retain a visible preview");
                    for note in notes {
                        assert!(
                            contains(content, bounds(&note["rect"])),
                            "MIDI notes must not paint over names"
                        );
                    }
                }
            }
            let selected = state["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|clip| clip["id"] == state["selected_clip"])
                .unwrap();
            let expected = project
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|clip| clip.name == name)
                .unwrap();
            assert_eq!(
                selected["id"], expected.id,
                "title remains a usable selection target"
            );
        }
    }
    app.quit();
}
