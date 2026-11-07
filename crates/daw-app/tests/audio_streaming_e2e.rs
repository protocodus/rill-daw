//! Real large-file import, admitted source modes, canvas badges and bounded
//! engine playback. The fixture is written in small chunks, never a large Vec.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::Path,
};

const RATE: u32 = 48_000;
const BEAT: usize = 24_000;
const WINDOW: usize = BEAT * 4;
const LARGE_FRAMES: usize = 4_194_304 + 1024;
const SOURCE_OFFSET: usize = LARGE_FRAMES - WINDOW;

fn source_frame(frame: usize) -> [f32; 2] {
    let ramp = (frame % 1_048_576) as f32 / 16_777_216.0;
    [0.125 + ramp, -0.25 + ramp]
}

fn write_large_source(path: &Path) {
    let bytes = (LARGE_FRAMES * 8) as u32;
    let mut file = BufWriter::new(File::create(path).unwrap());
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&3_u16.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&RATE.to_le_bytes()).unwrap();
    file.write_all(&(RATE * 8).to_le_bytes()).unwrap();
    file.write_all(&8_u16.to_le_bytes()).unwrap();
    file.write_all(&32_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    let mut chunk = Vec::with_capacity(8192 * 8);
    for first in (0..LARGE_FRAMES).step_by(8192) {
        chunk.clear();
        for frame in first..(first + 8192).min(LARGE_FRAMES) {
            for sample in source_frame(frame) {
                chunk.extend_from_slice(&sample.to_le_bytes());
            }
        }
        file.write_all(&chunk).unwrap();
    }
    file.flush().unwrap();
    assert!(bytes as usize > 32 * 1024 * 1024);
}

fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}
fn clip_view(state: &Value, id: u64) -> &Value {
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
fn has(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label.as_str().is_some_and(|label| label.contains(text)))
}
fn unchanged(state: &Value, before: &Value) {
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
}
fn overlaps(a: &Value, b: &Value) -> bool {
    b.is_object()
        && (0..2).all(|axis| {
            a["min"][axis].as_f64().unwrap() < b["max"][axis].as_f64().unwrap()
                && a["max"][axis].as_f64().unwrap() > b["min"][axis].as_f64().unwrap()
        })
}
fn hover(app: &mut Harness, id: u64) -> Value {
    let state = app.inspect();
    let (x, y) = center(&clip_view(&state, id)["visible_rect"]);
    app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
    for _ in 0..90 {
        let state = app.command(json!({"command":"inspect","paint":true}));
        if has(&state, daw_ui::DISK_STREAMED_AUDIO) {
            return state;
        }
    }
    panic!("Streamed clip {id} did not expose its storage tooltip");
}
fn callback_pcm(app: &mut Harness, start: usize, frames: usize, reverse: bool) -> Value {
    let state = app.command(json!({"command":"pump_audio","frames":frames}));
    assert_eq!(state["audio_output"].as_array().unwrap().len(), frames);
    for (offset, stereo) in state["audio_output"].as_array().unwrap().iter().enumerate() {
        let expected = source_frame(
            SOURCE_OFFSET
                + if reverse {
                    WINDOW - 1 - start - offset
                } else {
                    start + offset
                },
        );
        for (channel, expected_sample) in expected.iter().copied().enumerate() {
            assert!(
                (stereo[channel].as_f64().unwrap() as f32 - expected_sample).abs() < 1e-7,
                "Disk PCM frame{} leg{channel} was not the original source",
                start + offset
            );
        }
    }
    assert_eq!(state["engine"]["render_errors"], 0);
    state
}

#[test]
fn disk_badges_admission_portable_recall_seek_and_reverse_preserve_large_source_pcm() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let external = app.directory.path().join("Long disk source.wav");
    write_large_source(&external);
    app.command(json!({"command":"file_dialog","kind":"import_audio","path":external}));
    app.click("File");
    app.click("Import audio…");
    let imported = app.command(json!({"command":"wait_audio_import","timeout_ms":30000}));
    assert_eq!(imported["error"], false, "{}", imported["status"]);
    // Loading the external-reference fixture also reserves subprocess-assigned
    // IDs before this test creates further occurrences in its own process.
    let legacy = app.directory.path().join("streaming-fixture.json");
    project(&imported).save_legacy_json(&legacy).unwrap();
    let mut song = Project::load(&legacy).unwrap();
    assert_eq!(song.tracks.len(), 1);
    let track = song.tracks[0].id;
    let first = song.tracks[0].clips[0].id;
    let private = match &song.tracks[0].clips[0].content {
        ClipContent::Audio { path, .. } => path.clone(),
        _ => panic!("Audio import"),
    };
    assert_ne!(Path::new(&private), external);
    assert!(std::fs::metadata(&private).unwrap().len() > 32 * 1024 * 1024);
    song.tempo = 120.0;
    song.track_mut(track).unwrap().gain = 1.0;
    {
        let clip = song.clip_mut(track, first).unwrap();
        clip.name = "Long disk source".into();
        clip.length = 4.0;
        clip.offset = SOURCE_OFFSET as f64 / BEAT as f64;
    }
    let ghost = song.clone_clip(track, first).unwrap();
    let blank = Clip::audio("", 8.0, 4.0, &private);
    let blank_id = blank.id;
    song.track_mut(track).unwrap().clips.push(blank);
    let small = app.directory.path().join("Small memory source.wav");
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: vec![0.015625; WINDOW * 2],
    }
    .write_wav(&small)
    .unwrap();
    let small_track = song.add_audio_track("Small source");
    let small_clip = Clip::audio("Small resident", 0.0, 4.0, small.to_string_lossy());
    let small_id = small_clip.id;
    song.track_mut(small_track).unwrap().mute = true;
    song.track_mut(small_track).unwrap().clips.push(small_clip);
    // Explicit legacy references are a test fixture; the public Save below
    // must make the real portable ZIP and privately retain all source media.
    song.save_legacy_json(&legacy).unwrap();
    app.command(json!({"command":"open","path":legacy}));
    wait_until(&mut app, |state| {
        clip_view(state, first)["source_mode"] == "disk"
            && clip_view(state, small_id)["source_mode"] == "memory"
    });
    for (dark, density) in [(false, 1.0), (true, 1.0), (false, 2.0), (true, 2.0)] {
        // Each case represents a native display at launch. A fresh context
        // avoids carrying unrelated tooltip state across a DPI transition.
        app = app.restart();
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1280,"height":900,"pixels_per_point":density}),
        );
        app.command(json!({"command":"open","path":legacy}));
        wait_until(&mut app, |state| {
            clip_view(state, first)["source_mode"] == "disk"
                && clip_view(state, small_id)["source_mode"] == "memory"
        });
        let before = app.inspect();
        for id in [first, ghost, blank_id] {
            let view = clip_view(&before, id);
            let badge = &view["disk_badge"];
            assert_eq!(view["source_mode"], "disk");
            assert!(
                badge.is_object(),
                "A visible streamed clip needs its drive badge"
            );
            assert!(!overlaps(badge, &view["name_hit_rect"]));
            for handle in view["fade_handles"].as_array().into_iter().flatten() {
                assert!(!overlaps(badge, handle));
            }
            let hovered = hover(&mut app, id);
            unchanged(&hovered, &before);
        }
        assert_eq!(clip_view(&before, blank_id)["named_header"], false);
        assert_eq!(clip_view(&before, small_id)["source_mode"], "memory");
        assert!(clip_view(&before, small_id)["disk_badge"].is_null());
        assert!(
            before["audio_sources"]["resident_pcm_bytes"]
                .as_u64()
                .unwrap()
                < 2 * 1024 * 1024,
            "Large source metadata must not retain a complete decoded sample Vec"
        );
    }
    app = app.restart();
    app.command(json!({"command":"viewport","width":1280,"height":900,"pixels_per_point":1.0}));
    app.command(json!({"command":"open","path":legacy}));
    let state = app.inspect();
    let (x, y) = center(&clip_view(&state, first)["visible_rect"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    let properties = app.click("Properties");
    assert!(has(&properties, daw_ui::DISK_STREAMED_AUDIO));
    app.key("Escape");
    let archive = app.directory.path().join("Portable disk song.rill");
    app.command(json!({"command":"save","path":archive}));
    let mut signature = [0; 4];
    File::open(&archive)
        .unwrap()
        .read_exact(&mut signature)
        .unwrap();
    assert_eq!(&signature, b"PK\x03\x04");
    for original in [external.as_path(), Path::new(&private), small.as_path()] {
        std::fs::remove_file(original).unwrap();
    }
    app = app.restart();
    let reopened = app.command(json!({"command":"open","path":archive}));
    assert_eq!(reopened["error"], false, "{}", reopened["status"]);
    let recalled = project(&reopened);
    let recalled_path = match &recalled.clip(track, first).unwrap().content {
        ClipContent::Audio { path, .. } => path,
        _ => unreachable!(),
    };
    assert_ne!(recalled_path, &private);
    assert!(Path::new(recalled_path).exists());
    let shared_group = song.clip(track, first).unwrap().link_group.unwrap();
    for id in [first, ghost, blank_id] {
        let original = song.clip(track, id).unwrap();
        let restored = recalled.clip(track, id).unwrap();
        assert_eq!(restored.name, original.name);
        assert_eq!(restored.start, original.start);
        assert_eq!(restored.length, original.length);
        assert_eq!(restored.offset, original.offset);
        assert_eq!(restored.source_rate, original.source_rate);
        assert_eq!(restored.link_group, original.link_group);
    }
    assert_eq!(
        recalled.clip(track, first).unwrap().link_group,
        Some(shared_group)
    );
    assert_eq!(
        recalled.clip(track, ghost).unwrap().link_group,
        Some(shared_group)
    );
    wait_until(&mut app, |state| {
        clip_view(state, first)["source_mode"] == "disk"
    });
    app.click("Start");
    app.click("Play");
    wait_until(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let streamed = callback_pcm(&mut app, 0, 4096, false);
    let memory = &streamed["engine"]["audio_streaming"];
    assert!(memory["streamed_clips"].as_u64().unwrap() >= 3);
    assert!(memory["window_bytes"].as_u64().unwrap() <= 1024 * 1024);
    assert!(memory["resident_bytes"].as_u64().unwrap() < 2 * 1024 * 1024);
    assert!(memory["disk_frames_read"].as_u64().unwrap() > 0);
    app.click("Cursor");
    let view = app.inspect();
    let ruler = &view["timeline"]["ruler"];
    let x = view["timeline"]["canvas"]["min"][0].as_f64().unwrap()
        + (1.0 - view["timeline"]["start_beat"].as_f64().unwrap())
            * view["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let (_, y) = center(ruler);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    wait_until(&mut app, |state| {
        (state["engine"]["position_beats"].as_f64().unwrap() - 1.0).abs() < 1e-7
    });
    callback_pcm(&mut app, BEAT, 2048, false);
    app.click("Stop");
    app.click("Select");
    let view = app.inspect();
    let (x, y) = center(&clip_view(&view, first)["visible_rect"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    app.click("Clip Editor");
    app.click("Process");
    app.click("Reverse");
    let reversed = wait_until(&mut app, |state| state["audio_editor"]["pending"] == false);
    let reversed_song = project(&reversed);
    for id in [first, ghost] {
        assert!(matches!(
            &reversed_song.clip(track, id).unwrap().content,
            ClipContent::Audio { reverse: true, .. }
        ));
    }
    app.click("Start");
    app.click("Play");
    wait_until(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    callback_pcm(&mut app, 0, 4096, true);
    app.click("Stop");
    let wav = app.directory.path().join("Streamed reverse.wav");
    app.command(json!({"command":"export","path":wav}));
    app.command(json!({"command":"wait_export"}));
    let output = AudioData::load_wav(wav).unwrap();
    for frame in [
        0,
        8191,
        8192,
        BEAT,
        WINDOW - 1,
        WINDOW,
        WINDOW + BEAT,
        2 * WINDOW,
        3 * WINDOW - 1,
    ] {
        let expected = source_frame(if frame < 2 * WINDOW {
            SOURCE_OFFSET + WINDOW - 1 - frame % WINDOW
        } else {
            frame - 2 * WINDOW
        });
        for (channel, expected_sample) in expected.iter().copied().enumerate() {
            assert!(
                (output.samples[frame * 2 + channel] - expected_sample).abs() < 1e-7,
                "Portable streamed export differs at frame{frame} leg{channel}"
            );
        }
    }
    app.quit();
}
