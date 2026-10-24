//! Audio editing through the production executable, with rendered sample oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn open(length: f64, drums: bool) -> Harness {
    let mut app = Harness::new();
    let path = app.directory.path().join("audio-source.wav");
    let samples = (0..100000)
        .flat_map(|frame| {
            let sample = if drums {
                if (12000..14400).contains(&frame) || (60000..62400).contains(&frame) {
                    0.8
                } else if (36000..38400).contains(&frame) {
                    0.1
                } else {
                    0.0
                }
            } else {
                0.1 + frame as f32 / 400000.0
            };
            [sample, -sample * 0.5]
        })
        .collect();
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples,
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Audio test");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Audio gestures",
        0.0,
        length,
        path.to_string_lossy(),
    ));
    let project_path = app.directory.path().join("audio.daw.json");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open","path":project_path}));
    app.click("Audio gestures");
    app.click("Window");
    app.click("Clip Editor");
    assert!(!app.inspect()["audio_editor"].is_null());
    app
}
fn clips(state: &Value) -> Vec<Value> {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .clone()
}
fn audio(state: &Value) -> Value {
    clips(state)[0]["content"]["Audio"].clone()
}
fn point(state: &Value, beat: f64) -> (f64, f64) {
    let view = &state["audio_editor"];
    (
        view["rect"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap(),
        (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) / 2.0,
    )
}
fn at(app: &mut Harness, beat: f64) -> Value {
    let point = point(&app.inspect(), beat);
    app.command(json!({"command":"pointer","phase":"click","x":point.0,"y":point.1}))
}
fn undo(app: &mut Harness) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true}))
}
fn wait(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.inspect();
        if state["audio_editor"]["pending"] == false {
            assert_eq!(state["audio_editor"]["error"], "", "{state}");
            return state;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

#[test]
fn cursor_clip_bounds_snap_cut_copy_paste_duplicate_and_undo() {
    let mut app = open(4.13, false);
    near(
        app.click("Clip end")["audio_editor"]["cursor"]
            .as_f64()
            .unwrap(),
        4.13,
    );
    near(
        app.click("Clip start")["audio_editor"]["cursor"]
            .as_f64()
            .unwrap(),
        0.0,
    );
    near(
        clips(&app.click("Snap end"))[0]["length"].as_f64().unwrap(),
        4.25,
    );
    undo(&mut app);
    at(&mut app, 1.0);
    let trimmed = app.click("Set start");
    near(clips(&trimmed)[0]["start"].as_f64().unwrap(), 1.0);
    near(clips(&trimmed)[0]["offset"].as_f64().unwrap(), 1.0);
    undo(&mut app);
    at(&mut app, 2.0);
    near(
        clips(&app.click("Set end"))[0]["length"].as_f64().unwrap(),
        2.0,
    );
    undo(&mut app);
    app.click("Copy");
    app.click("Clip end");
    let pasted = app.click("Paste");
    assert_eq!(clips(&pasted).len(), 2);
    near(clips(&pasted)[1]["start"].as_f64().unwrap(), 4.13);
    assert_ne!(clips(&pasted)[0]["id"], clips(&pasted)[1]["id"]);
    let duplicate = app.click("Duplicate clip");
    assert_eq!(clips(&duplicate).len(), 3);
    undo(&mut app);
    undo(&mut app);
    app.key("E");
    let cut = at(&mut app, 1.5);
    assert_eq!(clips(&cut).len(), 2);
    near(clips(&cut)[0]["length"].as_f64().unwrap(), 1.5);
    undo(&mut app);
    app.key("P");
    at(&mut app, 2.0);
    assert_eq!(clips(&app.click("Split at cursor")).len(), 2);
    app.quit();
}

#[test]
fn fades_curves_reverse_gain_normalize_and_export_match_source() {
    let mut app = open(4.0, false);
    app.key("I");
    near(audio(&at(&mut app, 1.0))["fade_in"].as_f64().unwrap(), 1.0);
    app.click("Linear");
    app.click("Equal power");
    app.key("O");
    near(audio(&at(&mut app, 3.0))["fade_out"].as_f64().unwrap(), 1.0);
    app.click("Linear");
    app.click("S curve");
    let reversed = app.click("Reverse");
    assert_eq!(audio(&reversed)["reverse"], true);
    let rect = &reversed["audio_editor"]["gain_rect"];
    let x = (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) / 2.0;
    let y = (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"-6"}));
    let gain = app.key("Enter");
    near(audio(&gain)["gain"].as_f64().unwrap(), 10.0_f64.powf(-0.3));
    app.click("Normalize");
    let normalized = wait(&mut app);
    let gain = audio(&normalized)["gain"].as_f64().unwrap();
    near(gain * (0.1 + 95999.0 / 400000.0), 10.0_f64.powf(-0.05));
    app.click("Normalize");
    near(audio(&wait(&mut app))["gain"].as_f64().unwrap(), gain);
    let rendered = export(&mut app, "edited.wav");
    assert_eq!(rendered.channels, 2);
    assert_eq!(rendered.sample_rate, 48000);
    for frame in [1200, 6000, 24000, 48000, 84000, 95000] {
        let beat = frame as f64 / 24000.0;
        let attack = daw_core::FadeCurve::EqualPower.value(beat) as f64;
        let release = daw_core::FadeCurve::SCurve.value(4.0 - beat) as f64;
        let expected = (0.1 + (95999 - frame) as f64 / 400000.0) * gain * attack * release;
        assert!((rendered.samples[frame * 2] as f64 - expected).abs() < 2e-5);
        assert!((rendered.samples[frame * 2 + 1] as f64 + expected * 0.5).abs() < 2e-5);
    }
    let before = std::fs::read(app.directory.path().join("audio-source.wav")).unwrap();
    let save = app.directory.path().join("edited.rill");
    app.command(json!({"command":"save","path":save}));
    let restored = Project::load(save).unwrap();
    let restored_clip = &restored.tracks[0].clips[0];
    assert_eq!(
        serde_json::to_value(&restored_clip.content).unwrap()["Audio"]["reverse"],
        true
    );
    assert_eq!(
        std::fs::read(app.directory.path().join("audio-source.wav")).unwrap(),
        before
    );
    app.quit();
}

#[test]
fn transient_algorithm_sensitivity_preview_and_split_are_undoable_and_preserve_audio() {
    let mut app = open(4.0, true);
    let original = export(&mut app, "before.wav");
    app.click("Transients");
    app.click("Analyze");
    let low = wait(&mut app);
    assert_eq!(low["audio_editor"]["preview"].as_array().unwrap().len(), 2);
    assert_eq!(clips(&low).len(), 1, "Preview must not split");
    app.click("Energy rise");
    app.click("Amplitude gate");
    let state = app.inspect();
    let rect = &state["audio_editor"]["sensitivity_rect"];
    let high=app.command(json!({"command":"pointer","phase":"click","x":rect[2].as_f64().unwrap()-1.0,"y":(rect[1].as_f64().unwrap()+rect[3].as_f64().unwrap())/2.0}));
    assert!(high["audio_editor"]["sensitivity"].as_f64().unwrap() > 0.9);
    app.click("Analyze");
    let preview = wait(&mut app);
    assert_eq!(preview["audio_editor"]["preview"], json!([0.5, 1.5, 2.5]));
    let split = app.click("Split transients");
    assert_eq!(clips(&split).len(), 4);
    let after = export(&mut app, "after.wav");
    assert_eq!(original.samples, after.samples);
    assert_eq!(clips(&undo(&mut app)).len(), 1);
    app.quit();
}

#[test]
fn audio_pan_zoom_and_fit_reset_both_axes_without_changing_clip() {
    let mut app = open(4.0, false);
    let before = app.inspect();
    let original = before["project"].clone();
    let initial_reference = before["audio_editor"]["amplitude_reference"]
        .as_f64()
        .unwrap();
    near(initial_reference, 0.1 + 95999.0 / 400000.0);
    let (x, y) = point(&before, 2.0);
    let zoomed = app.command(json!({"command":"scroll","x":x,"y":y,"delta":100.0,"ctrl":true}));
    assert!(
        zoomed["audio_editor"]["pixels_per_beat"].as_f64().unwrap()
            > before["audio_editor"]["pixels_per_beat"].as_f64().unwrap()
    );
    let vertical =
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":100.0,"ctrl":true,"shift":true}));
    assert!(
        vertical["audio_editor"]["row_height"].as_f64().unwrap()
            > before["audio_editor"]["row_height"].as_f64().unwrap()
    );
    app.command(json!({"command":"pointer","phase":"drag","button":"middle","x":x,"y":y,"to_x":x-100.0,"to_y":y-40.0}));
    for fit in [false, true] {
        let state = if fit {
            app.command(json!({"command":"click","label":"Fit","occurrence":1}))
        } else {
            app.key("F")
        };
        let view = &state["audio_editor"];
        let width = view["rect"][2].as_f64().unwrap() - view["rect"][0].as_f64().unwrap();
        let height = view["rect"][3].as_f64().unwrap() - view["rect"][1].as_f64().unwrap();
        assert!(
            (4.0 - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap()
                <= width + 0.01
        );
        assert!(
            (view["rows"].as_f64().unwrap() - view["row_offset"].as_f64().unwrap())
                * view["row_height"].as_f64().unwrap()
                <= height + 0.01
        );
        assert_eq!(state["project"], original);
    }
    // Editing gain must leave the amplitude reference/channel centers stable;
    // an explicit Fit then uses the new audible peak rather than unity scale.
    let state = app.inspect();
    let rect = &state["audio_editor"]["gain_rect"];
    let x = (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5;
    let y = (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"6"}));
    let changed = app.key("Enter");
    near(
        changed["audio_editor"]["amplitude_reference"]
            .as_f64()
            .unwrap(),
        initial_reference,
    );
    let fitted = app.command(json!({"command":"click","label":"Fit","occurrence":1}));
    near(
        fitted["audio_editor"]["amplitude_reference"]
            .as_f64()
            .unwrap(),
        initial_reference * 10.0_f64.powf(0.3),
    );
    assert_eq!(fitted["project"], changed["project"]);
    app.quit();
}
