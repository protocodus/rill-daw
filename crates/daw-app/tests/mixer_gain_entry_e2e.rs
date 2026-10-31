//! Exact Mixer gain entry uses the same dB control as the track headers.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

const RATE: usize = 48_000;

fn gain_rect(state: &Value, gain: f64) -> &Value {
    let expected_db = format!("{:.1} dB", daw_ui::channel_gain_to_db(gain as f32));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == expected_db)
        .unwrap_or_else(|| {
            panic!(
                "Missing concise {expected_db} readout: {}",
                state["visible_labels"]
            )
        })
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn type_gain(app: &mut Harness, current_gain: f64, db: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(gain_rect(&state, current_gain));
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":db}));
    app.key("Enter")
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

fn exported(app: &mut Harness, filename: &str) -> AudioData {
    let path = app.directory.path().join(filename);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn assert_level(audio: &AudioData, gain: f64) {
    assert_eq!(audio.sample_rate, RATE as u32);
    assert_eq!(audio.channels, 2);
    assert!(
        audio.samples[..RATE * 2]
            .iter()
            .all(|sample| (f64::from(*sample) - 0.25 * gain).abs() < 1e-6)
    );
    assert!(
        audio.samples[RATE * 2..]
            .iter()
            .all(|sample| *sample == 0.0)
    );
}

#[test]
fn precise_gain_entry_undo_recall_and_export_use_the_same_channel_value() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let source = app.directory.path().join("gain-reference.wav");
        AudioData {
            sample_rate: RATE as u32,
            channels: 1,
            samples: vec![0.25; RATE],
        }
        .write_wav(&source)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Precise level");
        let channel = project.track_mut(track).unwrap();
        channel.gain = 0.75;
        channel
            .clips
            .push(Clip::audio("Reference", 0.0, 2.0, source.to_string_lossy()));
        let fixture = app.directory.path().join("gain-reference.rill");
        project.save(&fixture).unwrap();
        app.command(json!({"command":"open","path":fixture}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}),
        );
        let original = app.click("Mixer");
        assert_level(&exported(&mut app, "before.wav"), 0.75);

        let edited = type_gain(&mut app, 0.75, "-6.3");
        let expected = 10.0_f64.powf(-6.3 / 20.0);
        near(
            edited["project"]["tracks"][0]["gain"].as_f64().unwrap(),
            expected,
        );
        assert_eq!(
            edited["project"]["tracks"][0]["clips"],
            original["project"]["tracks"][0]["clips"]
        );
        assert_eq!(undo(&mut app, false)["project"], original["project"]);
        assert_eq!(undo(&mut app, true)["project"], edited["project"]);
        assert_level(&exported(&mut app, "after.wav"), expected);

        // File Save as / Open still use their normal document paths after a
        // numeric edit; reopening must not recompute or round the stored gain.
        let bundle = app.directory.path().join("exact-level.rill");
        app.command(json!({"command":"file_dialog","kind":"save","path":bundle}));
        app.click("File");
        app.click("Save as…");
        app.command(json!({"command":"wait_project"}));
        let mut app = app.restart();
        app.command(json!({"command":"file_dialog","kind":"open","path":bundle}));
        app.click("File");
        let reopened = app.click("Open…");
        let reopened = if reopened["project_pending"] == true {
            app.command(json!({"command":"wait_project"}))
        } else {
            reopened
        };
        near(
            reopened["project"]["tracks"][0]["gain"].as_f64().unwrap(),
            expected,
        );
        let restored = exported(&mut app, "recalled.wav");
        assert_level(&restored, expected);
        app.quit();
    }
}

#[test]
fn viewing_extreme_values_and_middle_dragging_readout_or_fader_do_not_edit() {
    let mut app = Harness::new();
    let mut project = Project::new();
    for (name, gain) in [("High", 2.0), ("Quiet", 0.00005), ("Silent", 0.0)] {
        let id = project.add_bus(name);
        project.track_mut(id).unwrap().gain = gain;
    }
    let fixture = app.directory.path().join("stored-values.rill");
    project.save(&fixture).unwrap();
    app.command(json!({"command":"open","path":fixture}));
    app.command(json!({"command":"viewport","width":800,"height":600}));
    let original = app.click("Mixer");
    let unchanged = |state: &Value| {
        assert_eq!(state["project"], original["project"]);
        assert_eq!(state["history"], original["history"]);
        assert_eq!(state["selected_track"], original["selected_track"]);
    };
    for dark in [false, true] {
        unchanged(&app.command(json!({"command":"system_theme","dark":dark})));
        unchanged(&app.inspect());
    }

    let state = app.command(json!({"command":"inspect","paint":true}));
    let readout = gain_rect(&state, 2.0).clone();
    let fader = state["mixer"]["channels"][0]["fader_scale"]["rail"].clone();
    assert!(fader.is_object(), "first channel fader rail");
    for target in [&readout, &fader] {
        let (x, y) = center(target);
        unchanged(
            &app.command(json!({"command":"pointer","phase":"down","button":"middle","x":x,"y":y})),
        );
        unchanged(&app.command(
            json!({"command":"pointer","phase":"move","button":"middle","x":x+24.0,"y":y+20.0}),
        ));
        unchanged(&app.command(
            json!({"command":"pointer","phase":"up","button":"middle","x":x+24.0,"y":y+20.0}),
        ));
        unchanged(&app.inspect());
    }
    let edited = type_gain(&mut app, 2.0, "-60");
    assert_eq!(edited["project"]["tracks"][0]["gain"], 0.0);
    assert_eq!(undo(&mut app, false)["project"], original["project"]);
    app.quit();
}
