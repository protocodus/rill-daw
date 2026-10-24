//! Subprocess/UI tests with independent analytic overlap and spline oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, CrossfadeCurve, Project};
use serde_json::{Value, json};

fn open(app: &mut Harness, project: &Project) {
    let path = app.directory.path().join("overlap.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn near(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 3e-6,
        "{actual} != {expected}"
    );
}
fn config(state: &Value) -> Value {
    state["project"]["tracks"][0]["clips"][1]["content"]["Audio"]["crossfade"].clone()
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn check_blend(audio: &AudioData, control1: [f64; 2], control2: [f64; 2]) {
    // Both x handles remain 1/3 and 2/3, so Bezier parameter t == progress.
    for progress in [0.125_f64, 0.25, 0.5, 0.75, 0.875] {
        let frame = ((2.0 + progress) * 24000.0).round() as usize;
        let incoming_frame = frame - 48000;
        let a = 0.08 + frame as f64 / 1000000.0;
        let b = 0.4 - incoming_frame as f64 / 2000000.0;
        let p = 3.0 * (1.0 - progress).powi(2) * progress * control1[1]
            + 3.0 * (1.0 - progress) * progress.powi(2) * control2[1]
            + progress.powi(3);
        let expected = a * (1.0 - p) + b * p;
        near(audio.samples[frame * 2], expected);
        near(audio.samples[frame * 2 + 1], -expected * 0.5);
    }
}

#[test]
fn latest_start_and_later_storage_tie_win_with_older_clips_resuming() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Priority");
    project.track_mut(track).unwrap().gain = 1.0;
    for (name, start, length, amplitude) in [
        ("A", 0.0, 8.0, 0.08),
        ("B", 2.0, 4.0, 0.19),
        ("C", 3.0, 1.0, 0.31),
        ("D", 2.0, 3.0, 0.43),
    ] {
        let source = app.directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: (0..192000)
                .flat_map(|_| [amplitude, -amplitude * 0.5])
                .collect(),
        }
        .write_wav(&source)
        .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            name,
            start,
            length,
            source.to_string_lossy(),
        ));
    }
    open(&mut app, &project);
    let rendered = export(&mut app, "priority.wav");
    assert!(rendered.samples.len() >= 192000 * 2);
    for frame in 0..192000 {
        let expected = match frame {
            0..48000 => 0.08,
            48000..72000 => 0.43,
            72000..96000 => 0.31,
            96000..120000 => 0.43,
            120000..144000 => 0.19,
            _ => 0.08,
        };
        near(rendered.samples[frame * 2], expected);
        near(rendered.samples[frame * 2 + 1], -expected * 0.5);
    }
    assert!(
        rendered
            .samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 0.430001)
    );
    let archive = app.directory.path().join("priority.rill");
    app.command(json!({"command":"save", "path":archive}));
    app.command(json!({"command":"open", "path":archive}));
    assert_eq!(rendered.samples, export(&mut app, "recalled.wav").samples);
    // Consolidation must bake the same ownership timeline, including the
    // serialized tie and the older regions that resume underneath newer takes.
    app.click("A");
    app.click("Edit");
    app.click("Join clips…");
    app.click("Join");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["join_pending"] == false {
            assert_eq!(state["error"], false, "{state}");
            assert_eq!(
                state["project"]["tracks"][0]["clips"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{state}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        rendered.samples,
        export(&mut app, "joined-priority.wav").samples
    );
    app.quit();
}

#[test]
fn crossfade_popup_duration_presets_spline_drag_undo_and_archive_render() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Crossfade");
    project.track_mut(track).unwrap().gain = 1.0;
    for (name, start, length, second) in [
        ("Outgoing take", 0.0, 6.0, false),
        ("Incoming take", 2.0, 4.0, true),
    ] {
        let source = app.directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: (0..192000)
                .flat_map(|frame| {
                    let value = if second {
                        0.4 - frame as f32 / 2000000.0
                    } else {
                        0.08 + frame as f32 / 1000000.0
                    };
                    [value, -value * 0.5]
                })
                .collect(),
        }
        .write_wav(&source)
        .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            name,
            start,
            length,
            source.to_string_lossy(),
        ));
    }
    open(&mut app, &project);
    app.click("Incoming take");
    app.click("Edit");
    let popup = app.click("Crossfade…");
    assert_eq!(popup["popups"]["crossfade"], true);
    assert!(config(&popup).is_null());
    let enabled = app.click("Enable crossfade");
    let (x, y) = center(&enabled["crossfade"]["duration"]);
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":"1"}));
    let duration = app.key("Enter");
    assert_eq!(config(&duration)["duration"], 1.0);
    let linear = app.click("Linear");
    assert_eq!(
        config(&linear)["curve"],
        serde_json::to_value(CrossfadeCurve::LINEAR).unwrap()
    );
    check_blend(
        &export(&mut app, "linear.wav"),
        [1.0 / 3.0, 1.0 / 3.0],
        [2.0 / 3.0, 2.0 / 3.0],
    );
    let smooth = app.click("Smooth");
    assert_eq!(
        config(&smooth)["curve"],
        serde_json::to_value(CrossfadeCurve::SMOOTH).unwrap()
    );
    check_blend(
        &export(&mut app, "smooth.wav"),
        [1.0 / 3.0, 0.0],
        [2.0 / 3.0, 1.0],
    );
    let state = app.inspect();
    let (x, y) = center(&state["crossfade"]["handles"][0]);
    let plot = &state["crossfade"]["plot"];
    let target_y = plot["max"][1].as_f64().unwrap()
        - 0.8 * (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap());
    let dragged = app.command(
        json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":x, "to_y":target_y}),
    );
    let curve = config(&dragged)["curve"].clone();
    let y1 = curve["control1"][1].as_f64().unwrap();
    assert!((y1 - 0.8).abs() < 0.02, "{curve}");
    let undo = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(config(&undo), config(&smooth));
    let redo = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    assert_eq!(config(&redo), config(&dragged));
    check_blend(
        &export(&mut app, "custom.wav"),
        [1.0 / 3.0, y1],
        [2.0 / 3.0, 1.0],
    );
    let archive = app.directory.path().join("crossfade.rill");
    app.command(json!({"command":"save", "path":archive}));
    let reopened = app.command(json!({"command":"open", "path":archive}));
    assert_eq!(config(&reopened), config(&dragged));
    check_blend(
        &export(&mut app, "custom-recalled.wav"),
        [1.0 / 3.0, y1],
        [2.0 / 3.0, 1.0],
    );
    let model: Project = serde_json::from_value(reopened["project"].clone()).unwrap();
    assert!(matches!(
        model.tracks[0].clips[1].content,
        ClipContent::Audio {
            crossfade: Some(_),
            ..
        }
    ));
    app.quit();
}
