//! Warp markers and audio quantize through the production executable: mouse
//! gestures, undo/redo, save/reopen, the tool path and rendered click timing.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// 48 kHz at 120 BPM: 24000 frames per beat.
const FRAMES_PER_BEAT: f64 = 24_000.0;
/// Source beats of the clicks: each is off the beat grid.
const CLICKS: [f64; 4] = [0.9, 2.1, 2.95, 4.08];
/// Signalsmith's long grains can pull an attack at a rate change by a few
/// milliseconds; the engine tests document the same bound.
const TOLERANCE: f64 = 384.0;

fn fixture() -> (Harness, std::path::PathBuf) {
    let mut app = Harness::new();
    let path = app.directory.path().join("clicks.wav");
    let mut samples = vec![0.0_f32; 150_000 * 2];
    for click in CLICKS {
        let start = (click * FRAMES_PER_BEAT).round() as usize;
        for n in 0..1_200 {
            let value = 0.8
                * (-(n as f64) / 240.0).exp()
                * (std::f64::consts::TAU * 1_500.0 * n as f64 / 48_000.0).cos();
            samples[(start + n) * 2] = value as f32;
            samples[(start + n) * 2 + 1] = value as f32;
        }
    }
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples,
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Drum loop");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Click loop",
        0.0,
        6.0,
        path.to_string_lossy(),
    ));
    let project_path = app.directory.path().join("warp.rill");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open","path":project_path}));
    (app, project_path)
}

fn open_editor(app: &mut Harness) {
    app.click("Click loop");
    app.click("Window");
    app.click("Clip Editor");
    assert!(!app.inspect()["audio_editor"].is_null());
}

fn markers(state: &Value) -> Vec<(f64, f64)> {
    state["project"]["tracks"][0]["clips"][0]["warp_markers"]
        .as_array()
        .map(|markers| {
            markers
                .iter()
                .map(|marker| {
                    (
                        marker["source"].as_f64().unwrap(),
                        marker["beat"].as_f64().unwrap(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn x_at(state: &Value, beat: f64) -> f64 {
    let view = &state["audio_editor"];
    view["rect"][0].as_f64().unwrap()
        + (beat - view["start_beat"].as_f64().unwrap()) * view["pixels_per_beat"].as_f64().unwrap()
}

fn canvas_y(state: &Value) -> f64 {
    let rect = &state["audio_editor"]["rect"];
    (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) / 2.0
}

fn flag(state: &Value, index: u64) -> (f64, f64) {
    let flags = state["audio_editor"]["warp"]["warp_flags"]
        .as_array()
        .unwrap();
    let rect = &flags
        .iter()
        .find(|flag| flag["index"] == index)
        .unwrap_or_else(|| panic!("flag {index}: {flags:?}"))["rect"];
    (
        (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) / 2.0,
        (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) / 2.0,
    )
}

fn wait_for(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if ready(&state) {
            assert_eq!(state["audio_editor"]["error"], "", "{state}");
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            state["audio_editor"]["warp"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

/// First frame reaching half of the local peak near an expected beat.
fn onset(audio: &AudioData, beat: f64) -> f64 {
    assert_eq!(audio.sample_rate, 48_000);
    let channels = audio.channels;
    let frames = audio.samples.len() / channels;
    let center = (beat * FRAMES_PER_BEAT) as usize;
    let window = center.saturating_sub(6_000)..(center + 6_000).min(frames);
    let level = |frame: usize| audio.samples[frame * channels].abs();
    let peak = window.clone().map(level).fold(0.0_f32, f32::max);
    assert!(peak > 0.1, "no click near beat {beat}");
    window
        .into_iter()
        .find(|&frame| level(frame) >= peak * 0.5)
        .unwrap() as f64
}

fn assert_onset(audio: &AudioData, beat: f64) {
    let actual = onset(audio, beat);
    let expected = beat * FRAMES_PER_BEAT;
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "click near beat {beat} rendered at frame {actual}, expected {expected}"
    );
}

fn undo_count(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

#[test]
fn warp_markers_are_added_moved_and_deleted_by_mouse_with_undo_save_and_export() {
    let (mut app, _) = fixture();
    open_editor(&mut app);
    let before = app.inspect();
    app.click("Warp");
    let state = wait_for(&mut app, |state| {
        state["audio_editor"]["warp"]["warp_hits"]
            .as_array()
            .is_some_and(|hits| hits.len() == 4)
    });
    let hits: Vec<f64> = state["audio_editor"]["warp"]["warp_hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit.as_f64().unwrap())
        .collect();
    for (hit, click) in hits.iter().zip(CLICKS) {
        assert!((hit - click).abs() < 1e-3, "{hits:?}");
    }

    // A click a few pixels from the attack snaps the marker onto it; the first
    // marker also pins the clip edges so it can be dragged on its own.
    let x = x_at(&state, 2.1) + 3.0;
    let added =
        app.command(json!({"command":"pointer","phase":"click","x":x,"y":canvas_y(&state)}));
    let listed = markers(&added);
    assert_eq!(listed.len(), 3, "{}", added["project"]);
    assert!((listed[1].0 - 2.1).abs() < 1e-3 && (listed[1].1 - 2.1).abs() < 1e-3);
    assert_eq!((listed[0], listed[2]), ((0.0, 0.0), (6.0, 6.0)));
    assert_eq!(added["history"]["undo"][0], "Add warp marker");
    assert_eq!(undo_count(&added), undo_count(&before) + 1);

    // Shift bypasses the attack snap.
    let shifted = app.command(json!({"command":"pointer","phase":"click","shift":true,
        "x":x_at(&added, 4.5),"y":canvas_y(&added)}));
    assert_eq!(markers(&shifted).len(), 4);
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(markers(&state), listed);

    // Dragging the flag moves only that marker, snapped to the editor grid,
    // as one undo step.
    let (fx, fy) = flag(&state, 1);
    let to = fx + x_at(&state, 2.0) - x_at(&state, 2.1);
    let moved =
        app.command(json!({"command":"pointer","phase":"drag","x":fx,"y":fy,"to_x":to,"to_y":fy}));
    let dragged = markers(&moved);
    assert!((dragged[1].0 - listed[1].0).abs() < 1e-9);
    assert!((dragged[1].1 - 2.0).abs() < 1e-9, "{dragged:?}");
    assert_eq!((dragged[0], dragged[2]), (listed[0], listed[2]));
    assert_eq!(moved["history"]["undo"][0], "Move warp marker");
    assert_eq!(undo_count(&moved), undo_count(&added) + 1);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(markers(&undone), listed);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(markers(&redone), dragged);

    // The rendered click now lands on beat 2; its neighbours stretch linearly
    // between the fixed edge markers.
    let rendered = export(&mut app, "warped.wav");
    assert_onset(&rendered, 2.0);
    assert_onset(&rendered, 0.9 * 2.0 / 2.1);
    assert_onset(&rendered, 2.0 + (2.95 - 2.1) * 4.0 / 3.9);

    // The flag menu deletes; Undo restores it.
    let (fx, fy) = flag(&redone, 1);
    app.command(json!({"command":"pointer","phase":"click","x":fx,"y":fy,"button":"secondary"}));
    let deleted = app.click("Delete warp marker");
    assert_eq!(markers(&deleted).len(), 2);
    let restored = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(markers(&restored), dragged);

    // Saved and reopened projects keep the markers exactly.
    let saved = app.directory.path().join("saved.rill");
    app.command(json!({"command":"save","path":saved}));
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(markers(&reopened), dragged);
    assert_onset(&export(&mut app, "reopened.wav"), 2.0);
    app.quit();
}

#[test]
fn quantize_drawer_previews_and_applies_one_undo_step_with_clicks_on_the_grid() {
    let (mut app, _) = fixture();
    open_editor(&mut app);
    let original = export(&mut app, "original.wav");
    for click in CLICKS {
        assert_onset(&original, click);
    }
    app.click("Process");
    app.click("Quantize audio…");
    let preview = wait_for(&mut app, |state| {
        state["audio_editor"]["warp"]["quantize"]["moved"] == 4
    });
    let quantize = &preview["audio_editor"]["warp"]["quantize"];
    assert_eq!(quantize["open"], true);
    assert!((quantize["largest_move"].as_f64().unwrap() - 0.1).abs() < 1e-3);
    let targets: Vec<f64> = quantize["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| pair[1].as_f64().unwrap())
        .collect();
    assert_eq!(targets, vec![1.0, 2.0, 3.0, 4.0]);
    assert!(
        markers(&preview).is_empty(),
        "Preview must not edit the clip"
    );
    let undo_before = undo_count(&preview);

    let applied = app.click("Apply quantize");
    assert_eq!(markers(&applied).len(), 6);
    assert_eq!(applied["history"]["undo"][0], "Quantize audio");
    assert_eq!(undo_count(&applied), undo_before + 1);
    assert_eq!(applied["audio_editor"]["warp"]["quantize"]["open"], false);
    let rendered = export(&mut app, "quantized.wav");
    for beat in [1.0, 2.0, 3.0, 4.0] {
        assert_onset(&rendered, beat);
    }
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert!(markers(&undone).is_empty());
    assert_eq!(undo_count(&undone), undo_before);

    // "Markers only" places markers on the hits without moving the audio.
    app.click("Process");
    app.click("Quantize audio…");
    wait_for(&mut app, |state| {
        state["audio_editor"]["warp"]["quantize"]["moved"] == 4
    });
    let pinned = app.click("Markers only");
    let listed = markers(&pinned);
    assert_eq!(listed.len(), 6);
    assert!(
        listed
            .iter()
            .all(|(source, beat)| (source - beat).abs() < 1e-9)
    );
    let unchanged = export(&mut app, "pinned.wav");
    for click in CLICKS {
        assert_onset(&unchanged, click);
    }
    app.quit();
}

#[test]
fn quantize_tool_reports_moves_and_typed_markers_round_trip_through_tools() {
    let (mut app, _) = fixture();
    let state = app.inspect();
    let track = state["project"]["tracks"][0]["id"].clone();
    let clip = state["project"]["tracks"][0]["clips"][0]["id"].clone();
    let quantized = app.command(
        json!({"command":"song_tool","name":"audio_process_clip","arguments":{
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "track_id":track,"clip_id":clip,"operation":"quantize","grid_beats":1.0
        }}),
    );
    assert_eq!(quantized["tool_result"]["ok"], true, "{quantized}");
    let warp = &quantized["tool_result"]["result"]["warp"];
    assert_eq!(warp["moved"], 4, "{warp}");
    assert!((warp["largest_move_beats"].as_f64().unwrap() - 0.1).abs() < 1e-3);
    assert!((warp["largest_move_ms"].as_f64().unwrap() - 50.0).abs() < 0.5);
    assert_eq!(undo_count(&quantized), 1);
    let rendered = export(&mut app, "tool.wav");
    for beat in [1.0, 2.0, 3.0, 4.0] {
        assert_onset(&rendered, beat);
    }

    let read = app.command(json!({"command":"song_tool","name":"get_clip","arguments":{
        "track_id":track,"clip_id":clip
    }}));
    assert_eq!(read["tool_result"]["ok"], true, "{read}");
    let audio = &read["tool_result"]["result"]["audio"];
    assert_eq!(audio["warp_markers"].as_array().unwrap().len(), 6);
    assert!((audio["source_end"].as_f64().unwrap() - 6.0).abs() < 1e-9);

    // A typed edit replaces the whole list atomically; an invalid one is
    // rejected without touching the song.
    let state = app.inspect();
    let set = |markers: Value, state: &Value| {
        json!({"command":"song_tool","name":"apply_edits","arguments":{
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "label":"Warp","edits":[{"op":"set_warp_markers","track":track,"clip":clip,"markers":markers}]
        }})
    };
    let rejected = app.command(set(
        json!([{"source":0.0,"beat":0.0},{"source":2.1,"beat":0.1}]),
        &state,
    ));
    assert_eq!(rejected["tool_result"]["ok"], false, "{rejected}");
    assert_eq!(rejected["project"], state["project"]);
    let edited = app.command(set(
        json!([{"source":0.0,"beat":0.0},{"source":2.1,"beat":2.0},{"source":6.0,"beat":6.0}]),
        &state,
    ));
    assert_eq!(edited["tool_result"]["ok"], true, "{edited}");
    assert_eq!(markers(&edited), vec![(0.0, 0.0), (2.1, 2.0), (6.0, 6.0)]);
    assert_onset(&export(&mut app, "typed.wav"), 2.0);
    let cleared = app.command(set(json!([]), &edited));
    assert_eq!(cleared["tool_result"]["ok"], true, "{cleared}");
    assert!(markers(&cleared).is_empty());
    app.quit();
}
