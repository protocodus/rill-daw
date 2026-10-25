//! Strip silence through the real app, with independent source/WAV checks.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn fixture(reverse: bool, ghost: bool) -> (Harness, u64) {
    let mut app = Harness::new();
    let path = app.directory.path().join("silence-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..96000)
            .flat_map(|frame| {
                let v = if (12000..30000).contains(&frame) || (60000..84000).contains(&frame) {
                    0.4
                } else {
                    0.001
                };
                [v, -v * 0.5]
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Silence editing");
    project.track_mut(track).unwrap().gain = 1.0;
    let mut clip = Clip::audio("Quiet spaces", 1.0, 4.0, path.to_string_lossy());
    if let ClipContent::Audio { reverse: value, .. } = &mut clip.content {
        *value = reverse;
    }
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    if ghost {
        project.clone_clip(track, id).unwrap();
    }
    let path = app.directory.path().join("silence.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Quiet spaces");
    app.click("Window");
    app.click("Clip Editor");
    (app, id)
}
fn preview(app: &mut Harness) -> Value {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let state = app.inspect();
        assert!(state["audio_silence"]["error"].is_null(), "{state}");
        if state["audio_silence"]["pending"] == false {
            return state;
        }
        assert!(
            Instant::now() < until,
            "Silence analysis timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn open(app: &mut Harness) -> Value {
    app.click("Process");
    app.click("Strip silence…");
    preview(app)
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn chord(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key","key":key,"cmd":true}))
}
fn number(app: &mut Harness, index: usize, value: f64) -> Value {
    let state = app.inspect();
    let rect = &state["audio_silence"]["controls"][index];
    let x = (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) / 2.0;
    let y = (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    chord(app, "A");
    app.command(json!({"command":"text","text":value.to_string()}));
    app.key("Enter");
    preview(app)
}
fn spans(state: &Value) -> Vec<(f64, f64)> {
    state["audio_silence"]["retained"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_f64().unwrap(), r[1].as_f64().unwrap()))
        .collect()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn preview_apply_keeps_ghost_and_source_audio_with_one_undo_and_portable_recall() {
    let (mut app, id) = fixture(false, true);
    let before = app.inspect()["project"].clone();
    let baseline = export(&mut app, "before.wav");
    let state = open(&mut app);
    assert_eq!(
        state["project"], before,
        "Preview must not edit the document"
    );
    let regions = spans(&state);
    assert_eq!(regions.len(), 2);
    for (actual, expected) in regions.iter().zip([(1.49, 2.26), (3.49, 4.51)]) {
        near(actual.0, expected.0);
        near(actual.1, expected.1);
    }
    let edited = app.click("Apply strip silence");
    assert!(edited["audio_silence"].is_null());
    let clips = edited["project"]["tracks"][0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 3);
    let original_ghost = before["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] != id)
        .unwrap();
    assert!(clips.contains(original_ghost));
    let output = export(&mut app, "after.wav");
    assert_eq!(output.frames(), baseline.frames());
    for frame in 0..output.frames() {
        let beat = frame as f64 / 24000.0;
        let keep = beat >= 5.0
            || regions
                .iter()
                .any(|(start, end)| beat + 1e-8 >= *start && beat < *end - 1e-8);
        for channel in 0..2 {
            let expected = if keep {
                baseline.samples[frame * 2 + channel]
            } else {
                0.0
            };
            assert!(
                (output.samples[frame * 2 + channel] - expected).abs() < 2e-6,
                "frame {frame}, channel {channel}"
            );
        }
    }
    assert_eq!(chord(&mut app, "Z")["project"], before);
    assert_eq!(export(&mut app, "undo.wav").samples, baseline.samples);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(export(&mut app, "redo.wav").samples, output.samples);
    let path = app.directory.path().join("stripped.rill");
    app.command(json!({"command":"save","path":path}));
    let mut app = app.restart();
    app.command(json!({"command":"open","path":path}));
    assert_eq!(export(&mut app, "recalled.wav").samples, output.samples);
    app.quit();
}

#[test]
fn settings_reanalyze_and_cancel_blocks_background_keyboard_edits() {
    let (mut app, _) = fixture(false, false);
    let before = app.inspect()["project"].clone();
    open(&mut app);
    let state = number(&mut app, 2, 0.0);
    assert_eq!(spans(&state), vec![(1.5, 2.25), (3.5, 4.5)]);
    let state = number(&mut app, 1, 2000.0);
    assert_eq!(spans(&state), vec![(1.0, 5.0)]);
    let state = number(&mut app, 1, 100.0);
    assert_eq!(spans(&state).len(), 2);
    let state = number(&mut app, 0, -96.0);
    assert_eq!(spans(&state), vec![(1.0, 5.0)]);
    app.key("Delete");
    assert_eq!(app.inspect()["project"], before);
    let state = app.key("Escape");
    assert!(state["audio_silence"].is_null());
    assert_eq!(state["project"], before);
    assert_eq!(state["view"], "Clip");
    open(&mut app);
    let state = app.click("Cancel");
    assert!(state["audio_silence"].is_null());
    assert_eq!(state["project"], before);
    app.quit();
}

#[test]
fn reversed_source_preview_and_retained_audio_follow_playback_order() {
    let (mut app, _) = fixture(true, false);
    let baseline = export(&mut app, "reverse-before.wav");
    open(&mut app);
    let state = number(&mut app, 2, 0.0);
    assert_eq!(spans(&state), vec![(1.5, 2.5), (3.75, 4.5)]);
    app.click("Apply strip silence");
    let output = export(&mut app, "reverse-after.wav");
    for frame in 0..output.frames() {
        let beat = frame as f64 / 24000.0;
        let keep = (1.5..2.5).contains(&beat) || (3.75..4.5).contains(&beat);
        for channel in 0..2 {
            let expected = if keep {
                baseline.samples[frame * 2 + channel]
            } else {
                0.0
            };
            assert!(
                (output.samples[frame * 2 + channel] - expected).abs() < 2e-6,
                "frame {frame}, channel {channel}"
            );
        }
    }
    app.quit();
}

#[test]
fn stripping_the_whole_clip_returns_to_arranger_and_undo_restores_it() {
    let (mut app, _) = fixture(false, false);
    let before = app.inspect()["project"].clone();
    let baseline = export(&mut app, "whole-before.wav");
    open(&mut app);
    let state = number(&mut app, 0, 0.0);
    assert!(spans(&state).is_empty());
    let state = app.click("Apply strip silence");
    assert_eq!(state["view"], "Arrangement");
    assert!(state["audio_silence"].is_null());
    assert!(
        state["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(chord(&mut app, "Z")["project"], before);
    assert_eq!(export(&mut app, "whole-undo.wav").samples, baseline.samples);
    app.quit();
}
