//! Clip titles stay separate from musical previews through actual app paint.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, Note, Project};
use eframe::egui::{Color32, Rgba};
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
    inner[0] >= outer[0] - 0.51
        && inner[1] >= outer[1] - 0.51
        && inner[2] <= outer[2] + 0.51
        && inner[3] <= outer[3] + 0.51
}

fn same_pixels(a: [f64; 4], b: [f64; 4], scale: f64) -> bool {
    a.into_iter()
        .zip(b)
        .all(|(a, b)| (a - b).abs() <= 0.51 / scale)
}

fn paint_color(value: &Value) -> Color32 {
    let [r, g, b, a] = std::array::from_fn(|i| value[i].as_u64().unwrap() as u8);
    Color32::from_rgba_premultiplied(r, g, b, a)
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |c: Color32| {
        let c = Rgba::from(c);
        0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b()
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn audio_and_midi_clips_keep_titles_and_previews_separate_in_both_appearances() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Clip source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..96_000)
            .map(|i| if i % 48 < 24 { -0.5 } else { 0.5 })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio surface");
    project.track_mut(audio).unwrap().color = [40, 199, 107];
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio clip",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI surface");
    project.track_mut(midi).unwrap().color = [103, 65, 228];
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
    project.track_mut(compact).unwrap().color = project.track(midi).unwrap().color;
    let mut muted = Clip::midi(
        "Compact clip",
        0.0,
        4.0,
        vec![Note::new(127, 0.0, 4.0, 100)],
    );
    muted.muted = true;
    project.track_mut(compact).unwrap().clips.push(muted);
    let path = app.directory.path().join("clip-surfaces.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open", "path":path}));
    wait_until(&mut app, |state| state["waveform"]["ready_count"] == 1);

    for (dark, density, width, rows) in
        [(false, 1.0, 1100, "Overview"), (true, 2.0, 800, "Compact")]
    {
        app.command(
            json!({"command":"viewport", "width":width, "height":760, "pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.click("View");
        app.click("Track density");
        app.click(rows);
        for name in ["MIDI clip", "Audio clip", "Compact clip"] {
            app.click(name);
            let state = app.command(json!({"command":"inspect", "paint":true}));
            assert_eq!(state["project"], before["project"]);
            assert_eq!(state["history"], before["history"]);
            assert_eq!(state["timeline"]["density"], rows);
            let shapes = state["painted_rects"].as_array().unwrap();
            let mut active_chroma = 0;
            let mut muted_chroma = 0;
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
                let fill_at = |area| {
                    let shape = shapes
                        .iter()
                        .find(|shape| {
                            same_pixels(bounds(&shape["rect"]), area, density)
                                && shape["fill"][3] == 255
                        })
                        .expect("event bodies and title bands are opaque");
                    paint_color(&shape["fill"])
                };
                let fill = fill_at(rect);
                let band = fill_at(bounds(&clip["name_hit_rect"]));
                let title_text = state["painted_text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|text| contains(title, bounds(&text["rect"])))
                    .expect("a visible title must survive the density change");
                assert!(contrast(paint_color(&title_text["colors"][0]), band) >= 4.5);
                if clip["track"] == audio {
                    assert!(
                        band.g() > band.r().saturating_add(60) && fill.g() > fill.r(),
                        "green event: {band:?} / {fill:?}"
                    );
                    let waveform = state["waveform"]["painted"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|p| p["clip"] == clip["id"] && p["view"] == "arranger")
                        .expect("the audio preview must remain visible at this density");
                    assert!(waveform["vertices"].as_u64().unwrap() > 0);
                    let wave_rect = &waveform["rect"];
                    assert!(wave_rect[1].as_f64().unwrap() >= content[1] - 0.5);
                    assert!(wave_rect[3].as_f64().unwrap() <= content[3] + 0.5);
                } else {
                    let chroma = i32::from(band.b()) - i32::from(band.g());
                    if clip["muted"] == true {
                        muted_chroma = chroma;
                    } else {
                        active_chroma = chroma;
                        assert!(
                            chroma > 60 && fill.b() > fill.g(),
                            "violet event: {band:?} / {fill:?}"
                        );
                    }
                    let notes: Vec<_> = shapes
                        .iter()
                        .filter(|shape| {
                            let b = bounds(&shape["rect"]);
                            shape["fill"][3] == 255 && contains(content, b) && b[2] - b[0] > 5.0
                        })
                        .collect();
                    assert_eq!(notes.len(), if clip["track"] == midi { 3 } else { 1 });
                    for note in notes {
                        let ink = paint_color(&note["fill"]);
                        if clip["muted"] == true {
                            assert_ne!(ink, fill, "muted notes must remain visible");
                        } else {
                            assert!(contrast(ink, fill) >= 3.0, "note {ink:?} on body {fill:?}");
                        }
                    }
                }
            }
            if name == "Audio clip" {
                assert!(
                    muted_chroma < active_chroma,
                    "muting softens the track color"
                );
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
