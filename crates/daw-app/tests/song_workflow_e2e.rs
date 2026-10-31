//! Compose a complete short song through the running app, then deliver and recall it.
mod common;

use common::Harness;
use daw_core::{AudioData, ClipContent, Project};
use serde_json::{Value, json};
use std::path::Path;

const SAMPLE_RATE: u32 = 48_000;
const PHRASE_FRAMES: usize = SAMPLE_RATE as usize * 4;
const SONG_FRAMES: usize = SAMPLE_RATE as usize * 16;

fn menu(app: &mut Harness, name: &str, action: &str) -> Value {
    app.click(name);
    // The shared driver prefers the foreground menu and scrolls its commands
    // into view. Hidden arranger tools are no longer duplicate visible labels.
    app.click(action)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn notes(state: &Value, clip: usize) -> &Value {
    &state["project"]["tracks"][3]["clips"][clip]["content"]["Midi"]["notes"]
}

fn draw_note(app: &mut Harness, beat: f64, pitch: u8) -> Value {
    let state = app.inspect();
    let view = &state["midi_editor"];
    let x = view["plot"]["min"][0].as_f64().unwrap()
        + (beat - view["start_beat"].as_f64().unwrap()) * view["pixels_per_beat"].as_f64().unwrap();
    let y = view["plot"]["min"][1].as_f64().unwrap()
        + (127.5 - f64::from(pitch) - view["row_offset"].as_f64().unwrap())
            * view["row_height"].as_f64().unwrap();
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}))
}

fn select_clip(app: &mut Harness, id: u64) -> Value {
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"];
    let (x, y) = center(rect);
    let state = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(state["selected_clip"], id);
    state
}

fn header_control(app: &mut Harness, track: u64, name: &str, fraction: f64) -> Value {
    let state = app.inspect();
    let rect = &state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()[name];
    let (x, y) = center(rect);
    let target = rect["min"][0].as_f64().unwrap()
        + fraction * (rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap());
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":target, "to_y":y}))
}

/// Original deterministic sources, made in the test rather than shipped samples
/// or an installed instrument. Their arrangement and all edits use the real UI.
fn write_phrase(path: &Path, voice: usize) {
    let mut noise = 0x9e37_79b9_u32;
    let samples = (0..PHRASE_FRAMES)
        .map(|frame| {
            let t = frame as f64 / f64::from(SAMPLE_RATE);
            let beat = t * 2.0;
            let phase = std::f64::consts::TAU;
            match voice {
                0 => {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    let random = f64::from(noise) / f64::from(u32::MAX) * 2.0 - 1.0;
                    let kick_t = (beat % 2.0) * 0.5;
                    let kick = (phase * (49.0 * kick_t + 2.2 * (1.0 - (-28.0 * kick_t).exp())))
                        .sin()
                        * (-18.0 * kick_t).exp()
                        * 0.4;
                    let snare_t = ((beat + 1.0) % 2.0) * 0.5;
                    let snare = random * (-32.0 * snare_t).exp() * 0.16;
                    let hat_t = (beat % 0.5) * 0.5;
                    (kick + snare + random * (-120.0 * hat_t).exp() * 0.055) as f32
                }
                1 => {
                    let root = if beat < 4.0 { 55.0 } else { 65.406_391 };
                    let local = (beat % 1.0) * 0.5;
                    let envelope = (local * 160.0).min(1.0) * (-6.5 * local).exp();
                    ((phase * root * t).sin() + 0.2 * (phase * root * 2.0 * t).sin()) as f32
                        * envelope as f32
                        * 0.2
                }
                _ => {
                    let pitches = if beat < 4.0 {
                        [220.0, 261.625_565, 329.627_557, 440.0]
                    } else {
                        [261.625_565, 329.627_557, 391.995_436, 523.251_131]
                    };
                    let pitch = pitches[(beat * 2.0) as usize % 4];
                    let local = (beat % 0.5) * 0.5;
                    let envelope = (local * 200.0).min(1.0) * (-10.0 * local).exp();
                    ((phase * pitch * local).sin() + 0.22 * (phase * pitch * 3.0 * local).sin())
                        as f32
                        * envelope as f32
                        * 0.15
                }
            }
        })
        .collect();
    AudioData {
        sample_rate: SAMPLE_RATE,
        channels: 1,
        samples,
    }
    .write_wav(path)
    .unwrap();
}

fn export_exact(app: &mut Harness, path: &Path) -> AudioData {
    menu(app, "File", "Export WAV…");
    if app.inspect()["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
    app.click("Export…");
    let done = app.command(json!({"command":"wait_export"}));
    assert_eq!(done["exported_file"], json!(path));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn eight_bar_song_can_be_arranged_varied_balanced_and_recalled_without_source_files() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    let sources: Vec<_> = ["Drums.wav", "Bass.wav", "Glass.wav"]
        .iter()
        .map(|name| app.directory.path().join(name))
        .collect();

    // Build three independent eight-bar parts by repeating two-bar phrases.
    for (index, source) in sources.iter().enumerate() {
        write_phrase(source, index);
        let imported = app.command(json!({"command":"drop", "paths":[source], "beat":0.0}));
        assert_eq!(imported["project"]["tempo"], 120.0);
        for _ in 0..3 {
            app.command(json!({"command":"key", "key":"D", "cmd":true}));
        }
        let state = app.inspect();
        let clips = state["project"]["tracks"][index]["clips"]
            .as_array()
            .unwrap();
        assert_eq!(clips.len(), 4);
        for (phrase, clip) in clips.iter().enumerate() {
            assert_eq!(clip["start"], phrase as f64 * 8.0);
            assert_eq!(clip["length"], 8.0);
        }
    }

    // Draw a MIDI guide alongside the audible WAV parts. Rill intentionally
    // needs a user-chosen VST3 to sound MIDI; this test installs no instrument.
    menu(&mut app, "Track", "Add MIDI track");
    app.key("Escape");
    menu(&mut app, "Window", "Clip Editor");
    for (beat, pitch) in [(0.0, 69), (0.75, 72), (1.5, 76), (2.5, 79)] {
        draw_note(&mut app, beat, pitch);
    }
    let state = app.inspect();
    assert_eq!(notes(&state, 0).as_array().unwrap().len(), 4);
    menu(&mut app, "Window", "Arrangement");
    for _ in 0..6 {
        menu(&mut app, "Edit", "Ghost copy");
    }
    let repeated = menu(&mut app, "Edit", "Duplicate");
    let clips = repeated["project"]["tracks"][3]["clips"]
        .as_array()
        .unwrap();
    assert_eq!(clips.len(), 8);
    assert!(clips[7]["link_group"].is_null());
    for (bar, clip) in clips.iter().enumerate() {
        assert_eq!(clip["start"], bar as f64 * 4.0);
        assert_eq!(clip["length"], 4.0);
        if bar < 7 {
            assert_eq!(clip["link_group"], clips[0]["link_group"]);
        }
    }
    let ending = repeated["selected_clip"].as_u64().unwrap();
    select_clip(&mut app, ending);
    menu(&mut app, "Window", "Clip Editor");
    let before = app.inspect();
    let varied = draw_note(&mut app, 3.25, 81);
    assert_eq!(notes(&varied, 7).as_array().unwrap().len(), 5);
    assert_eq!(notes(&varied, 0), notes(&before, 0));
    assert_eq!(
        app.command(json!({"command":"key", "key":"Z", "cmd":true}))["project"],
        before["project"]
    );
    assert_eq!(
        app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}))["project"],
        varied["project"]
    );

    // Balance from the arrangement headers and verify each gesture is undoable.
    menu(&mut app, "Window", "Arrangement");
    let state = app.inspect();
    let bass = state["project"]["tracks"][1]["id"].as_u64().unwrap();
    let glass = state["project"]["tracks"][2]["id"].as_u64().unwrap();
    let before = state["project"].clone();
    let balanced = header_control(&mut app, bass, "gain", 0.86);
    assert!(balanced["project"]["tracks"][1]["gain"].as_f64().unwrap() < 1.0);
    assert_eq!(
        app.command(json!({"command":"key", "key":"Z", "cmd":true}))["project"],
        before
    );
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    let state = header_control(&mut app, glass, "pan", 0.65);
    assert!(state["project"]["tracks"][2]["pan"].as_f64().unwrap() > 0.2);
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    project.validate().unwrap();
    assert_eq!(project.end_beat(), 32.0);

    let preview_path = app.directory.path().join("Glass Steps preview.wav");
    let preview = export_exact(&mut app, &preview_path);
    assert_eq!(preview.frames(), SONG_FRAMES);
    assert_eq!(preview.channels, 2);
    assert_eq!(preview.sample_rate, SAMPLE_RATE);
    assert!(
        preview
            .samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() < 1.0)
    );
    // Every bar contains real musical output, including the last repeated bar.
    for bar in preview
        .samples
        .as_chunks::<{ SAMPLE_RATE as usize * 4 }>()
        .0
    {
        let energy: f64 = bar.iter().map(|sample| f64::from(*sample).powi(2)).sum();
        assert!(energy > 1.0, "An arranged bar was silent: {energy}");
    }

    let bundle = app.directory.path().join("Glass Steps.rill");
    let saved = app.command(json!({"command":"save", "path":bundle}));
    assert_eq!(saved["modified"], false);
    for source in &sources {
        std::fs::remove_file(source).unwrap();
    }
    // A new process cannot reuse the old waveform/render cache. Portable assets
    // alone must reproduce the song, the linked notes, and its independent ending.
    let mut app = app.restart();
    let restored = app.command(json!({"command":"open", "path":bundle}));
    assert_eq!(restored["modified"], false);
    assert_eq!(
        restored["project"]["tracks"][3],
        saved["project"]["tracks"][3]
    );
    let recalled: Project = serde_json::from_value(restored["project"].clone()).unwrap();
    recalled.validate().unwrap();
    for track in recalled.tracks.iter().take(3) {
        for clip in &track.clips {
            let ClipContent::Audio { path, .. } = &clip.content else {
                panic!()
            };
            assert!(Path::new(path).is_file());
        }
    }
    let final_path = app.directory.path().join("Glass Steps.wav");
    let recalled_audio = export_exact(&mut app, &final_path);
    assert_eq!(recalled_audio.samples, preview.samples);

    // Optional local capture retains the actual app-produced deliverables without
    // adding generated media or machine-specific paths to the source tree.
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        let destination = Path::new(&destination);
        std::fs::create_dir_all(destination).unwrap();
        std::fs::copy(&bundle, destination.join("Glass Steps.rill")).unwrap();
        std::fs::copy(&final_path, destination.join("Glass Steps.wav")).unwrap();
    }
    app.quit();
}
