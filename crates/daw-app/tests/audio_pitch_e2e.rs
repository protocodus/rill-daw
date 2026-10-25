//! Pitch controls through the running app, checked against exported audio and
//! portable project recall. No desktop, physical device, or native plugin UI.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn fixture() -> (Harness, u64, std::path::PathBuf) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1800, "height":1000}));
    let source = app.directory.path().join("original-tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..96_000)
            .flat_map(|frame| {
                let sample =
                    (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0).sin() as f32 * 0.25;
                [sample, -0.5 * sample]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Tone channel");
    let clip = Clip::audio("Tone", 0.0, 4.0, source.to_string_lossy());
    let id = clip.id;
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("pitch.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Tone");
    (app, id, source)
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn processing(state: &Value, id: u64) -> &Value {
    &clip(state, id)["content"]["Audio"]["processing"]
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn close_pitch(app: &mut Harness) {
    for _ in 0..3 {
        if !visible(&app.inspect(), "Transpose") {
            return;
        }
        app.key("Escape");
    }
    assert!(
        !visible(&app.inspect(), "Transpose"),
        "pitch popup must dismiss"
    );
}

fn open_pitch(app: &mut Harness, id: u64) -> Value {
    close_pitch(app);
    let state = app.inspect();
    let pitch = processing(&state, id)["transpose_semitones"]
        .as_f64()
        .unwrap();
    app.click(&if pitch == 0.0 {
        "Pitch".into()
    } else {
        format!("Pitch {pitch:+.2} st")
    })
}

fn set_pitch(app: &mut Harness, id: u64, pitch: f64) -> Value {
    let state = open_pitch(app, id);
    let rect = &state["audio_transpose_rect"];
    let x = (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5;
    let y = (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":pitch.to_string()}));
    app.key("Enter");
    close_pitch(app);
    let state = app.inspect();
    near(
        processing(&state, id)["transpose_semitones"]
            .as_f64()
            .unwrap(),
        pitch,
    );
    state
}

fn set_algorithm(app: &mut Harness, id: u64, algorithm: &str) -> Value {
    let state = app.inspect();
    let previous = processing(&state, id)["algorithm"].as_str().unwrap();
    if previous != algorithm {
        open_pitch(app, id);
        app.click(previous);
        app.click(algorithm);
        close_pitch(app);
    }
    let state = app.inspect();
    assert_eq!(processing(&state, id)["algorithm"], algorithm);
    state
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    close_pitch(app);
    let path = app.directory.path().join(name);
    app.click("File");
    let state = app.click("Export WAV…");
    if state["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
    app.click("Export…");
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["error"], false, "{state}");
    AudioData::load_wav(path).unwrap()
}

fn assert_tone(audio: &AudioData, seconds: f64, semitones: f64) {
    assert_eq!(audio.channels, 2);
    assert_eq!(
        audio.frames(),
        (seconds * f64::from(audio.sample_rate)).round() as usize
    );
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    let from = audio.frames() / 4;
    let count = audio.frames() / 2;
    let channel: Vec<_> = audio
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .skip(from)
        .take(count)
        .map(|frame| frame[0])
        .collect();
    let crossings = channel
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let frequency = crossings as f64 * f64::from(audio.sample_rate) / count as f64;
    let expected = 440.0 * 2.0_f64.powf(semitones / 12.0);
    assert!(
        (frequency - expected).abs() < 4.0,
        "expected {expected} Hz, measured {frequency}"
    );
    let rms = (channel
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / count as f64)
        .sqrt();
    assert!(
        (0.05..0.5).contains(&rms),
        "pitch output lost or over-amplified: rms={rms}"
    );
    let stereo_error = audio
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .skip(from)
        .take(count)
        .map(|frame| (frame[1] + frame[0] * 0.5).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        stereo_error < 0.005,
        "stereo relationship changed: {stereo_error}"
    );
}

fn select_clip(app: &mut Harness, id: u64) {
    close_pitch(app);
    app.click("Arranger");
    app.key("Q");
    let state = app.key("F");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
}

#[test]
fn both_algorithms_transpose_up_down_and_fine_tune_without_changing_duration_or_source() {
    for algorithm in ["Signalsmith", "Bungee"] {
        let (mut app, id, source) = fixture();
        let source_bytes = std::fs::read(&source).unwrap();
        app.click("Window");
        let state = app.click("Clip Editor");
        let original = clip(&state, id).clone();
        assert!(
            !visible(&state, "Transpose"),
            "processing options appear only on demand"
        );
        set_algorithm(&mut app, id, algorithm);
        for pitch in [12.0, -12.0, 7.25] {
            let state = set_pitch(&mut app, id, pitch);
            for field in ["start", "length", "offset", "source_rate"] {
                assert_eq!(clip(&state, id)[field], original[field]);
            }
            let audio = export(&mut app, &format!("{algorithm}-{pitch}.wav"));
            assert_tone(&audio, 2.0, pitch);
        }
        open_pitch(&mut app, id);
        app.click("Reset pitch");
        close_pitch(&mut app);
        let state = app.inspect();
        assert_eq!(processing(&state, id)["transpose_semitones"], 0.0);
        assert_eq!(
            processing(&state, id)["algorithm"],
            algorithm,
            "reset retains the selected engine"
        );
        assert_tone(&export(&mut app, "reset.wav"), 2.0, 0.0);
        assert_eq!(
            std::fs::read(source).unwrap(),
            source_bytes,
            "pitch edits must never rewrite source media"
        );
        app.quit();
    }
}

#[test]
fn properties_pitch_updates_ghosts_supports_undo_and_recalls_from_a_portable_project() {
    let (mut app, original, source) = fixture();
    app.click("Edit");
    let ghost = app.click("Ghost copy")["selected_clip"].as_u64().unwrap();
    app.click("Edit");
    let duplicate = app.click("Duplicate")["selected_clip"].as_u64().unwrap();
    select_clip(&mut app, ghost);
    app.click("Window");
    let before = app.click("Properties");
    let changed = set_pitch(&mut app, ghost, 12.37);
    for id in [original, ghost] {
        assert_eq!(processing(&changed, id)["transpose_semitones"], 12.37);
    }
    assert_eq!(processing(&changed, duplicate)["transpose_semitones"], 0.0);
    assert_eq!(
        changed["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    for id in [original, ghost] {
        assert_eq!(processing(&undone, id)["transpose_semitones"], 0.0);
    }
    let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    for id in [original, ghost] {
        assert_eq!(processing(&redone, id)["transpose_semitones"], 12.37);
    }
    let changed = set_algorithm(&mut app, ghost, "Bungee");
    assert_eq!(processing(&changed, original)["algorithm"], "Bungee");
    assert_eq!(processing(&changed, duplicate)["algorithm"], "Signalsmith");
    let archive = app.directory.path().join("portable-pitch.rill");
    app.command(json!({"command":"save", "path":archive}));
    std::fs::remove_file(source).unwrap();
    let mut app = app.restart();
    let restored = app.command(json!({"command":"open", "path":archive}));
    for id in [original, ghost, duplicate] {
        assert_eq!(processing(&restored, id), processing(&changed, id));
        assert_eq!(
            clip(&restored, id)["link_group"],
            clip(&changed, id)["link_group"]
        );
        let path = clip(&restored, id)["content"]["Audio"]["path"]
            .as_str()
            .unwrap();
        assert_eq!(AudioData::load_wav(path).unwrap().frames(), 96_000);
    }
    select_clip(&mut app, ghost);
    app.click("Window");
    app.click("Clip Editor");
    let state = set_pitch(&mut app, ghost, -7.25);
    assert_eq!(processing(&state, original)["transpose_semitones"], -7.25);
    assert_eq!(processing(&state, duplicate)["transpose_semitones"], 0.0);
    app.quit();
}

#[test]
fn arranger_stretch_and_audio_pitch_use_the_same_selected_engine_in_export() {
    for algorithm in ["Signalsmith", "Bungee"] {
        let (mut app, id, _) = fixture();
        app.click("Window");
        app.click("Clip Editor");
        set_algorithm(&mut app, id, algorithm);
        set_pitch(&mut app, id, 12.0);
        let state = app.click("Arranger");
        let rect = &state["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == id)
            .unwrap()["rect"];
        let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
        let x = rect["max"][0].as_f64().unwrap() - 2.0;
        let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
        app.key("S");
        let state = app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":x+width*0.5, "to_y":y}));
        near(clip(&state, id)["length"].as_f64().unwrap(), 6.0);
        near(clip(&state, id)["source_rate"].as_f64().unwrap(), 2.0 / 3.0);
        assert_eq!(processing(&state, id)["algorithm"], algorithm);
        assert_eq!(processing(&state, id)["transpose_semitones"], 12.0);
        assert_tone(&export(&mut app, "stretch-pitch.wav"), 3.0, 12.0);
        let state = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(clip(&state, id)["source_rate"], 1.0);
        assert_eq!(processing(&state, id)["transpose_semitones"], 12.0);
        assert_tone(&export(&mut app, "undo-stretch.wav"), 2.0, 12.0);
        app.quit();
    }
}
