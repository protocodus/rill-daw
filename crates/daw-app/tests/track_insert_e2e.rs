//! Track context actions insert on the clicked lane, including unselected tracks.
mod common;

use common::Harness;
use daw_core::{AudioData, AutomationParameter, AutomationTarget, Project};
use serde_json::{Value, json};

fn center(rect: &Value, axis: usize) -> f64 {
    (rect["min"][axis].as_f64().unwrap() + rect["max"][axis].as_f64().unwrap()) * 0.5
}

fn beat_x(state: &Value, beat: f64) -> f64 {
    let timeline = &state["timeline"];
    timeline["canvas"]["min"][0].as_f64().unwrap()
        + (beat - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap()
}

fn context_menu(app: &mut Harness, track: u64, lane_beat: Option<f64>) -> Value {
    let before = app.inspect();
    let row = before["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["track"] == track)
        .unwrap();
    let (x, y) = match lane_beat {
        Some(beat) => (beat_x(&before, beat), center(&row["header"], 1)),
        None => (center(&row["name"], 0), center(&row["name"], 1)),
    };
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let menu = app.inspect();
    assert_eq!(menu["project"], before["project"]);
    assert_eq!(menu["history"], before["history"]);
    menu
}

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

#[test]
fn track_menus_insert_samples_and_clips_at_the_lane_click_or_header_playhead() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1080}));
    let mut project = Project::new();
    let other = project.add_audio_track("Other channel");
    let audio = project.add_audio_track("Recordings");
    let midi = project.add_instrument_track("Piano");
    let automation = project
        .add_automation_track(
            "Piano pan",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackPan,
            },
        )
        .unwrap();
    project.track_mut(automation).unwrap().collapsed = false;
    let bus = project.add_bus("Returns");
    let folder = project.add_folder("Band");
    let path = app.directory.path().join("insertion.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app.sixteenth_snap();
    let cursor = 2.25;
    let x = beat_x(&state, cursor);
    let y = center(&state["timeline"]["ruler"], 1);
    let positioned = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(positioned["timeline"]["cursor"], cursor);
    let source = app.directory.path().join("Snare.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.2; 4_800],
    }
    .write_wav(&source)
    .unwrap();

    for track in [midi, automation, audio] {
        for lane_beat in [None, Some(6.32)] {
            let before = app.click("Other channel");
            assert_eq!(before["selected_track"], other);
            let menu = context_menu(&mut app, track, lane_beat);
            let sample = track == audio;
            assert_eq!(has(&menu, "Insert sample…"), sample);
            assert_eq!(has(&menu, "Insert clip"), !sample);
            let inserted = if sample {
                app.command(json!({"command":"file_dialog","kind":"import_audio","path":null}));
                let cancelled = app.click("Insert sample…");
                assert_eq!(cancelled["project"], before["project"]);
                assert_eq!(cancelled["history"], before["history"]);
                context_menu(&mut app, track, lane_beat);
                app.command(json!({"command":"file_dialog","kind":"import_audio","path":source}));
                app.click("Insert sample…");
                app.command(json!({"command":"wait_audio_import","timeout_ms":10000}))
            } else {
                app.click("Insert clip")
            };
            let tracks = inserted["project"]["tracks"].as_array().unwrap();
            assert_eq!(tracks.len(), project.tracks.len());
            let destination = tracks.iter().find(|row| row["id"] == track).unwrap();
            let lane = if track == automation {
                &destination["automation"]
            } else {
                destination
            };
            let clips = lane["clips"].as_array().unwrap();
            assert_eq!(clips.len(), 1);
            let clip = &clips[0];
            assert_eq!(
                clip["start"],
                if lane_beat.is_some() { 6.25 } else { cursor }
            );
            assert_eq!(inserted["selected_track"], track);
            assert_eq!(inserted["selected_clip"], clip["id"]);
            if sample {
                let imported =
                    AudioData::load_wav(clip["content"]["Audio"]["path"].as_str().unwrap())
                        .unwrap();
                assert_eq!(imported.frames(), 4800);
            } else {
                assert_eq!(clip["length"], 4.0);
                if track == midi {
                    assert_eq!(clip["content"]["Midi"]["notes"], json!([]));
                } else {
                    assert!(!clip["points"].as_array().unwrap().is_empty());
                }
            }
            app.click("Edit");
            let undone = app.click("Undo");
            assert_eq!(undone["project"], before["project"]);
            assert_eq!(undone["history"]["undo"], before["history"]["undo"]);
        }
    }
    for track in [bus, folder] {
        let menu = context_menu(&mut app, track, None);
        assert!(has(&menu, "Add track"));
        assert!(!has(&menu, "Insert clip"));
        assert!(!has(&menu, "Insert sample…"));
        app.key("Escape");
    }
    app.quit();
}

#[test]
fn collapse_and_expand_automation_together_by_channel_folder_or_whole_arrangement() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1400,"height":1000}));
    let mut project = Project::new();
    let folder = project.add_folder("Drums");
    let kick = project.add_audio_track("Kick");
    project.set_parent_folder(kick, Some(folder)).unwrap();
    let bass = project.add_midi_track("Bass");
    let mut lane = |track, name, parameter| {
        project
            .add_automation_track(name, AutomationTarget { track, parameter })
            .unwrap()
    };
    let gain = lane(kick, "Kick gain", AutomationParameter::TrackGain);
    let pan = lane(kick, "Kick pan", AutomationParameter::TrackPan);
    let bass_gain = lane(bass, "Bass gain", AutomationParameter::TrackGain);
    project.track_mut(pan).unwrap().collapsed = false;
    project.track_mut(bass_gain).unwrap().collapsed = false;
    let path = app.directory.path().join("batch-heights.rill");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open","path":path}));
    context_menu(&mut app, kick, None);
    let compact = app.click("Collapse all automation tracks");
    let collapsed = |state: &Value, id| {
        state["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == id)
            .unwrap()["collapsed"]
            == true
    };
    assert!(collapsed(&compact, gain) && collapsed(&compact, pan));
    assert!(!collapsed(&compact, bass_gain));
    assert_eq!(
        app.command(json!({"command":"key","cmd":true,"key":"Z"}))["project"],
        initial["project"]
    );
    context_menu(&mut app, folder, None);
    let expanded = app.click("Expand all automation tracks");
    assert!(!collapsed(&expanded, gain) && !collapsed(&expanded, pan));
    app.click("Tracks");
    let all = app.click("Collapse all automation tracks");
    assert!([gain, pan, bass_gain].iter().all(|id| collapsed(&all, *id)));
    let reopened = {
        app.command(json!({"command":"save","path":path}));
        app.command(json!({"command":"open","path":path}))
    };
    assert_eq!(all["project"], reopened["project"]);
    app.quit();
}
