//! Named track densities are persistent view preferences, not song edits.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Note, Project,
};
use serde_json::{Value, json};

fn unchanged(before: &Value, after: &Value) {
    for key in ["project", "history", "revision", "modified"] {
        assert_eq!(
            before[key], after[key],
            "A density/view action changed {key}"
        );
    }
}

fn item<'a>(state: &'a Value, list: &str, track: u64) -> &'a Value {
    state["timeline"][list]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["track"] == track)
        .unwrap()
}

fn point(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value, secondary: bool) -> Value {
    let (x, y) = point(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,
        "button":if secondary { "secondary" } else { "primary" }}))
}

fn preset(app: &mut Harness, name: &str) -> Value {
    app.click("View");
    app.click("Track density");
    app.click(name)
}

fn full_track_count(state: &Value) -> usize {
    let canvas = &state["timeline"]["canvas"];
    state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| {
            track["folder"] != true
                && track["header"]["min"][1].as_f64().unwrap() >= canvas["min"][1].as_f64().unwrap()
                && track["header"]["max"][1].as_f64().unwrap() <= canvas["max"][1].as_f64().unwrap()
        })
        .count()
}

#[test]
fn laptop_density_presets_keep_channel_actions_and_remember_the_view_without_edits() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let folder = project.add_folder("Drums");
    let first = project.add_audio_track("Kick");
    let second = project.add_audio_track("Snare");
    project.set_parent_folder(first, Some(folder)).unwrap();
    project.set_parent_folder(second, Some(folder)).unwrap();
    for index in 0..10 {
        project.add_audio_track(format!("Channel {}", index + 3));
    }
    let path = app.directory.path().join("density.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let initial = app.command(json!({"command":"viewport","width":1280,"height":800}));
    assert_eq!(initial["timeline"]["density"], "Compact");
    assert_eq!(initial["timeline"]["row_height"], 64.0);

    for (dark, scale) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":scale}),
        );
        let loop_start = app.inspect();
        preset(&mut app, "Compact");
        let compact = app.inspect();
        unchanged(&loop_start, &compact);
        let compact_count = full_track_count(&compact);
        assert!(
            compact_count >= 6,
            "A laptop must show at least six complete source tracks, saw {compact_count}"
        );
        for id in [first, second] {
            let track = item(&compact, "tracks", id);
            assert_eq!(track["depth"], 1);
            for section in ["mix", "meter", "record_arm", "details"] {
                assert!(
                    !track["sections"][section].is_null(),
                    "Compact {section} missing for {id}"
                );
            }
            for control in ["gain", "pan", "mute", "solo"] {
                let bounds = &item(&compact, "channels", id)[control];
                assert!(
                    bounds["max"][1].as_f64().unwrap() - bounds["min"][1].as_f64().unwrap() >= 20.0
                );
            }
            let grip = &track["reorder_grip"];
            assert!(grip["max"][0].as_f64().unwrap() - grip["min"][0].as_f64().unwrap() >= 13.0);
        }
        let gain = item(&compact, "channels", first)["gain"].clone();
        let menu = click_rect(&mut app, &gain, true);
        unchanged(&loop_start, &menu);
        assert!(
            menu["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == "Automate")
        );
        app.key("Escape");
        let compact = app.inspect();
        let details = item(&compact, "tracks", first)["sections"]["details"].clone();
        let channel = click_rect(&mut app, &details, false);
        unchanged(&loop_start, &channel);
        assert_eq!(channel["popups"]["properties"], true);
        assert_eq!(channel["selected_track"], first);
        app.click("Arranger");

        let state = app.inspect();
        let muted = click_rect(&mut app, &item(&state, "channels", first)["mute"], false);
        assert_eq!(
            muted["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|track| track["id"] == first)
                .unwrap()["mute"],
            true
        );
        app.click("Edit");
        let undone = app.click("Undo");
        assert_eq!(undone["project"], initial["project"]);
        // Undo creates a redo branch; density changes must preserve that branch.
        let baseline = app.inspect();
        preset(&mut app, "Detailed");
        let detailed = app.inspect();
        unchanged(&baseline, &detailed);
        assert_eq!(detailed["timeline"]["row_height"], 144.0);
        let detailed_count = full_track_count(&detailed);
        assert!(detailed_count < compact_count);
        eprintln!(
            "1280×800 @ {scale}×: {compact_count} complete source tracks in Compact, {detailed_count} in Detailed"
        );
        assert!(!item(&detailed, "tracks", first)["sections"]["routing"].is_null());
        preset(&mut app, "Standard");
        let standard = app.inspect();
        unchanged(&baseline, &standard);
        assert_eq!(standard["timeline"]["row_height"], 89.0);
        assert_eq!(standard["timeline"]["preferred_density"], "Standard");
    }
    let mut app = app.restart();
    app.command(json!({"command":"open","path":path}));
    let restored = app.inspect();
    assert_eq!(restored["timeline"]["row_height"], 89.0);
    assert_eq!(restored["timeline"]["preferred_density"], "Standard");
    assert_eq!(restored["project"], serde_json::to_value(project).unwrap());
    app.quit();
}

fn row_height(state: &Value, id: u64) -> f64 {
    let r = &item(state, "tracks", id)["header"];
    r["max"][1].as_f64().unwrap() - r["min"][1].as_f64().unwrap()
}

fn occupancy(state: &Value, id: u64, beat: f64, color: [u8; 3]) -> bool {
    let timeline = &state["timeline"];
    let x = timeline["canvas"]["min"][0].as_f64().unwrap()
        + (beat - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap();
    let header = &item(state, "tracks", id)["header"];
    let (top, bottom) = (
        header["min"][1].as_f64().unwrap(),
        header["max"][1].as_f64().unwrap(),
    );
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|shape| {
            let r = &shape["rect"];
            let y0 = r["min"][1].as_f64().unwrap();
            let y1 = r["max"][1].as_f64().unwrap();
            shape["fill"] == json!([color[0], color[1], color[2], 255])
                && y0 >= top
                && y1 <= bottom
                && y0 <= (top + bottom) * 0.5
                && y1 >= (top + bottom) * 0.5
                && y1 > y0
                && y1 - y0 < 12.0
                && r["min"][0].as_f64().unwrap() <= x
                && x <= r["max"][0].as_f64().unwrap()
        })
}

#[test]
fn every_track_chevron_collapses_to_a_colored_timeline_overview_and_recalls_its_state() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Audio.wav");
    AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.18; 8000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let folder = project.add_folder("Ensemble");
    let audio = project.add_audio_track("Voice");
    let midi = project.add_midi_track("Keys");
    let bus = project.add_bus("Room");
    project.set_parent_folder(audio, Some(folder)).unwrap();
    project.set_parent_folder(midi, Some(folder)).unwrap();
    let lane = project
        .add_automation_track(
            "Keys gain",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(lane).unwrap().collapsed = false;
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Breath",
        1.0,
        1.0,
        source.to_string_lossy(),
    ));
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Chord",
        3.0,
        2.0,
        vec![Note::new(64, 0.0, 1.0, 90)],
    ));
    project.create_automation_clip(lane, 6.0, 1.0).unwrap();
    project
        .automation_lane_mut(lane)
        .unwrap()
        .points
        .push(AutomationPoint::new(8.0, 0.6));
    let colors = [
        (folder, [185, 97, 211]),
        (audio, [47, 152, 231]),
        (midi, [112, 212, 45]),
        (bus, [224, 139, 66]),
        (lane, [78, 182, 136]),
    ];
    for (id, color) in colors {
        project.track_mut(id).unwrap().color = color;
    }
    let path = app.directory.path().join("Track overviews.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let before = app.set_track_density("Standard");
    let visible = |state: &Value, id| {
        state["timeline"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["track"] == id)
    };
    for (id, color) in colors.into_iter().skip(1) {
        let initial = app.inspect();
        assert!(item(&initial, "tracks", id)["disclosure"].is_object());
        app.click_track_disclosure(id);
        let state = app.command(json!({"command":"inspect","paint":true}));
        assert!((20.0..=22.0).contains(&row_height(&state, id)));
        assert!(item(&state, "tracks", id)["sections"]["mix"].is_null());
        if id == audio || id == midi {
            let title = if id == audio { "Breath" } else { "Chord" };
            assert!(
                state["painted_text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|text| text["text"] != title)
            );
        } else if id == lane {
            assert!(item(&state, "automation", id)["clips"][0]["plot"].is_null());
        }
        for beat in [1.5, 3.5, 6.5, 8.0, 9.5] {
            let expected = (id == audio && beat == 1.5)
                || (id == midi && beat == 3.5)
                || (id == lane && (beat == 6.5 || beat == 8.0));
            assert_eq!(
                occupancy(&state, id, beat, color),
                expected,
                "track {id}, beat {beat}"
            );
        }
        if id != bus {
            let clip = if id == lane {
                &item(&state, "automation", id)["clips"][0]
            } else {
                item(&state, "clips", id)
            };
            let (x, y) = point(&clip["overview"]);
            let editor =
                app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
            assert_eq!(editor["selected_clip"], clip["id"]);
            assert_eq!(editor["project"], state["project"]);
            if id == lane {
                assert_eq!(editor["automation_editor"]["clip"], clip["id"]);
            } else {
                assert_eq!(editor["view"], "Clip");
                assert!(
                    editor[if id == audio {
                        "audio_editor"
                    } else {
                        "midi_editor"
                    }]
                    .is_object()
                );
            }
            let back = app.click("Back");
            assert_eq!(row_height(&back, id), row_height(&state, id));
        }
    }
    app.click_track_disclosure(folder);
    let hidden = app.command(json!({"command":"inspect","paint":true}));
    assert!(row_height(&hidden, folder) <= 22.0);
    assert!(!visible(&hidden, audio) && !visible(&hidden, midi) && !visible(&hidden, lane));
    for (beat, expected) in [
        (1.5, true),
        (2.5, false),
        (3.5, true),
        (6.5, true),
        (8.0, true),
        (9.5, false),
    ] {
        assert_eq!(
            occupancy(&hidden, folder, beat, colors[0].1),
            expected,
            "folder at beat {beat}"
        );
    }
    app.command(json!({"command":"save","path":path}));
    let recalled = app.command(json!({"command":"open","path":path}));
    let decoded: Project = serde_json::from_value(recalled["project"].clone()).unwrap();
    // Each open extracts the portable WAV into a fresh scratch directory.
    let wav = decoded.audio_paths().next().unwrap();
    assert_eq!(std::fs::read(wav).unwrap(), std::fs::read(&source).unwrap());
    if let daw_core::ClipContent::Audio { path, .. } =
        &mut project.track_mut(audio).unwrap().clips[0].content
    {
        *path = wav.to_owned();
    }
    for track in &mut project.tracks {
        track.collapsed = true;
    }
    assert_eq!(decoded, project);
    assert!(!visible(&recalled, midi));
    app.click_track_disclosure(folder);
    for (id, _) in colors.into_iter().skip(1) {
        assert!(row_height(&app.inspect(), id) <= 22.0);
        app.click_track_disclosure(id);
        assert_eq!(row_height(&app.inspect(), id), row_height(&before, id));
    }
    let expanded = app.inspect();
    for track in &mut project.tracks {
        track.collapsed = false;
    }
    assert_eq!(expanded["project"], serde_json::to_value(project).unwrap());
    for id in [audio, midi] {
        assert!(
            expanded["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .any(|clip| clip["track"] == id && clip["content_rect"].is_object())
        );
    }
    assert!(item(&expanded, "automation", lane)["clips"][0]["plot"].is_object());
    app.quit();
}
