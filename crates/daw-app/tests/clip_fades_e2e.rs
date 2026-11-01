//! Dedicated clip handles exercised through the running production app.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};

fn open(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("fades.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}
fn source(app: &Harness, name: &str, amplitude: f32) -> std::path::PathBuf {
    let path = app.directory.path().join(name);
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..192_000)
            .flat_map(|_| [amplitude, -amplitude * 0.5])
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}
fn center(rect: &Value) -> (f64, f64) {
    assert!(rect.is_object(), "Missing handle: {rect}");
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn clip(state: &Value, track: usize, index: usize) -> &Value {
    &state["project"]["tracks"][track]["clips"][index]
}
fn audio(state: &Value, track: usize, index: usize) -> &Value {
    &clip(state, track, index)["content"]["Audio"]
}
fn diagnostic(state: &Value, id: u64) -> &Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}
fn drag(app: &mut Harness, point: (f64, f64), delta: f64, shift: bool) -> Value {
    app.command(json!({"command":"pointer","phase":"drag","x":point.0,"y":point.1,"to_x":point.0+delta,"to_y":point.1,"shift":shift}))
}
fn undo(app: &mut Harness) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true}))
}
fn redo(app: &mut Harness) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}))
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn all_audio_clips_have_non_destructive_handles_including_compact_reverse_and_stretch() {
    let mut app = Harness::new();
    let path = source(&app, "unchanged-source.wav", 0.4);
    let original_bytes = std::fs::read(&path).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Audio");
    project.track_mut(track).unwrap().gain = 1.0;
    let first = Clip::audio("First", 0.0, 4.0, path.to_string_lossy());
    let first_id = first.id;
    let mut second = Clip::audio("Trimmed", 4.0, 4.0, path.to_string_lossy());
    second.offset = 2.0;
    let second_id = second.id;
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let other = project.add_audio_track("Compact");
    let mut reversed = Clip::audio("Reverse stretched", 0.0, 4.0, path.to_string_lossy());
    reversed.source_rate = 2.0;
    if let ClipContent::Audio { reverse, .. } = &mut reversed.content {
        *reverse = true;
    }
    let reversed_id = reversed.id;
    project.track_mut(other).unwrap().collapsed = true;
    project.track_mut(other).unwrap().mute = true;
    project.track_mut(other).unwrap().clips.push(reversed);
    let before = open(&mut app, &project);
    for id in [first_id, second_id, reversed_id] {
        let handles = &diagnostic(&before, id)["fade_handles"];
        assert_eq!(handles.as_array().unwrap().len(), 2);
        for handle in handles.as_array().unwrap() {
            center(handle);
        }
    }
    let ppb = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let point = center(&diagnostic(&before, first_id)["fade_handles"][0]);
    let faded = drag(&mut app, point, ppb * 1.5, false);
    near(audio(&faded, 0, 0)["fade_in"].as_f64().unwrap(), 1.5);
    assert_eq!(faded["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    assert_eq!(redo(&mut app)["project"], faded["project"]);

    let point = center(&diagnostic(&faded, second_id)["fade_handles"][1]);
    let faded_out = drag(&mut app, point, -ppb * 1.25, false);
    near(audio(&faded_out, 0, 1)["fade_out"].as_f64().unwrap(), 1.25);
    let point = center(&diagnostic(&faded_out, reversed_id)["fade_handles"][0]);
    let reversed = drag(&mut app, point, ppb * 0.5, false);
    near(audio(&reversed, 1, 0)["fade_in"].as_f64().unwrap(), 1.0);
    for (track, index) in [(0, 0), (0, 1), (1, 0)] {
        for field in ["start", "length", "offset", "source_rate"] {
            assert_eq!(
                clip(&reversed, track, index)[field],
                clip(&before, track, index)[field],
                "{field}"
            );
        }
        assert_eq!(
            audio(&reversed, track, index)["path"],
            audio(&before, track, index)["path"]
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    let rendered = export(&mut app, "faded.wav");
    for (beat, gain) in [
        (0.25, 0.25 / 1.5),
        (0.75, 0.5),
        (1.25, 1.25 / 1.5),
        (4.5, 1.0),
        (7.0, 1.0 / 1.25),
        (7.5, 0.5 / 1.25),
    ] {
        let frame = (beat * 24_000.0_f64).round() as usize;
        near(f64::from(rendered.samples[frame * 2]), 0.4 * gain);
        near(f64::from(rendered.samples[frame * 2 + 1]), -0.2 * gain);
    }
    let archive = app.directory.path().join("fades.rill");
    app.command(json!({"command":"save","path":archive}));
    let reopened = app.command(json!({"command":"open","path":archive}));
    near(audio(&reopened, 0, 0)["fade_in"].as_f64().unwrap(), 1.5);
    assert_eq!(rendered.samples, export(&mut app, "recalled.wav").samples);
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    app.quit();
}

#[test]
fn focused_editor_handles_work_without_fade_tools_and_can_remove_fades() {
    let mut app = Harness::new();
    let path = source(&app, "editor.wav", 0.3);
    let mut project = Project::new();
    let track = project.add_audio_track("Editor");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Handles",
        2.0,
        4.0,
        path.to_string_lossy(),
    ));
    open(&mut app, &project);
    app.click("Handles");
    app.click("Window");
    let before = app.click("Clip Editor");
    assert_eq!(before["audio_editor"]["tool"], "Cursor");
    let ppb = before["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
    let point = center(&before["audio_editor"]["fade_handles"][0]);
    let faded = drag(&mut app, point, ppb * 0.63, true);
    near(audio(&faded, 0, 0)["fade_in"].as_f64().unwrap(), 0.63);
    assert_eq!(faded["audio_editor"]["range"], Value::Null);
    assert_eq!(
        faded["audio_editor"]["cursor"],
        before["audio_editor"]["cursor"]
    );
    app.key("Q");
    let state = app.inspect();
    let point = center(&state["audio_editor"]["fade_handles"][1]);
    let faded_out = drag(&mut app, point, -ppb * 1.0, false);
    near(audio(&faded_out, 0, 0)["fade_out"].as_f64().unwrap(), 1.0);
    for field in ["start", "length", "offset"] {
        assert_eq!(clip(&faded_out, 0, 0)[field], clip(&before, 0, 0)[field]);
    }
    let point = center(&faded_out["audio_editor"]["fade_handles"][0]);
    let removed =
        app.command(json!({"command":"pointer","phase":"double_click","x":point.0,"y":point.1}));
    assert_eq!(audio(&removed, 0, 0)["fade_in"], 0.0);
    near(audio(&removed, 0, 0)["fade_out"].as_f64().unwrap(), 1.0);
    assert_eq!(undo(&mut app)["project"], faded_out["project"]);
    app.quit();
}

#[test]
fn overlap_handle_enables_resizes_and_removes_a_crossfade_without_moving_clips() {
    let mut app = Harness::new();
    let outgoing = source(&app, "outgoing.wav", 0.1);
    let incoming = source(&app, "incoming.wav", 0.5);
    let original_bytes = [
        std::fs::read(&outgoing).unwrap(),
        std::fs::read(&incoming).unwrap(),
    ];
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Blend");
    project.track_mut(track).unwrap().gain = 1.0;
    let next = Clip::audio("Incoming", 2.0, 4.0, incoming.to_string_lossy());
    let id = next.id;
    project.track_mut(track).unwrap().clips.extend([
        Clip::audio("Outgoing", 0.0, 6.0, outgoing.to_string_lossy()),
        next,
    ]);
    let before = open(&mut app, &project);
    let ppb = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let point = center(&diagnostic(&before, id)["crossfade_handle"]);
    let blended = drag(&mut app, point, ppb, false);
    near(
        audio(&blended, 0, 1)["crossfade"]["duration"]
            .as_f64()
            .unwrap(),
        1.0,
    );
    for index in 0..2 {
        for field in ["start", "length", "offset"] {
            assert_eq!(
                clip(&blended, 0, index)[field],
                clip(&before, 0, index)[field],
                "{field}"
            );
        }
    }
    assert_eq!(blended["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    assert_eq!(redo(&mut app)["project"], blended["project"]);
    let rendered = export(&mut app, "crossfaded.wav");
    for progress in [0.125, 0.25, 0.5, 0.875] {
        let frame = ((2.0 + progress) * 24_000.0_f64).round() as usize;
        near(
            f64::from(rendered.samples[frame * 2]),
            0.1 * (1.0 - progress) + 0.5 * progress,
        );
    }
    assert_eq!(std::fs::read(&outgoing).unwrap(), original_bytes[0]);
    assert_eq!(std::fs::read(&incoming).unwrap(), original_bytes[1]);
    let point = center(&diagnostic(&app.inspect(), id)["crossfade_handle"]);
    let popup =
        app.command(json!({"command":"pointer","phase":"double_click","x":point.0,"y":point.1}));
    assert_eq!(
        popup["popups"]["crossfade"], true,
        "{} {}",
        popup["popups"], popup["audio_editor"]
    );
    app.key("Escape");
    let state = app.inspect();
    let point = center(&diagnostic(&state, id)["crossfade_handle"]);
    let removed = drag(&mut app, point, -ppb, false);
    assert!(audio(&removed, 0, 1)["crossfade"].is_null());
    assert_eq!(undo(&mut app)["project"], blended["project"]);
    app.click("Window");
    let editor = app.click("Clip Editor");
    let point = center(&editor["audio_editor"]["crossfade_handle"]);
    let ppb = editor["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
    let edited = drag(&mut app, point, -ppb * 0.37, true);
    near(
        audio(&edited, 0, 1)["crossfade"]["duration"]
            .as_f64()
            .unwrap(),
        0.63,
    );
    assert_eq!(edited["audio_editor"]["range"], Value::Null);
    assert_eq!(clip(&edited, 0, 1)["start"], clip(&editor, 0, 1)["start"]);
    let point = center(&edited["audio_editor"]["crossfade_handle"]);
    let popup =
        app.command(json!({"command":"pointer","phase":"double_click","x":point.0,"y":point.1}));
    assert_eq!(popup["popups"]["crossfade"], true);
    app.quit();
}
