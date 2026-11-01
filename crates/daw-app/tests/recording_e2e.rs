//! Input recording through the running UI and the production callback/writer.
//! Only the source and output device are virtual: projects are never manufactured
//! by the input driver. Exact WAV samples, routing and recall are the oracles.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;

fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}

fn menu(app: &mut Harness, menu: &str, action: &str) -> Value {
    app.click(menu);
    app.click(action)
}

fn fixture(looping: bool, backing: bool) -> Harness {
    let mut app = Harness::with_virtual_audio(None, None);
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = looping;
    project.loop_start = 0.0;
    project.loop_end = 1.0;
    let track = project.add_audio_track("Capture");
    project.track_mut(track).unwrap().gain = 1.0;
    if backing {
        let path = app.directory.path().join("Backing.wav");
        AudioData {
            sample_rate: RATE,
            channels: 1,
            samples: vec![0.1; 96_000],
        }
        .write_wav(&path)
        .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Backing",
            0.0,
            4.0,
            path.to_string_lossy(),
        ));
    }
    load(&mut app, &project, "recording-fixture.json");
    app.click("Capture");
    app
}

fn load(app: &mut Harness, project: &Project, name: &str) {
    let path = app.directory.path().join(name);
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
}

fn recording(app: &mut Harness) -> Value {
    menu(app, "Window", "Recording")
}

fn select_input(app: &mut Harness, input: &str) {
    let state = recording(app);
    let p = project(&state);
    let selected = state["selected_track"].as_u64().unwrap();
    let current = match p.track(selected).unwrap().audio_input {
        Some(daw_core::AudioInputChannels::Mono { channel }) => format!("Mono {}", channel + 1),
        Some(daw_core::AudioInputChannels::Stereo { left, right }) => {
            format!("Stereo {}–{}", left + 1, right + 1)
        }
        None => "No input".to_owned(),
    };
    app.click(&current);
    app.click(input);
}

fn arm(app: &mut Harness, input: &str) {
    select_input(app, input);
    app.click("Arm recording");
    app.key("Escape");
}

fn begin(app: &mut Harness) -> Value {
    app.click("Record");
    let state = wait(app, |state| state["recording"]["phase"] == "recording");
    assert_eq!(state["error"], false, "{}", state["status"]);
    state
}

fn stop(app: &mut Harness) -> Value {
    app.click("Stop");
    let state = wait(app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(state["error"], false, "{}", state["status"]);
    state
}

fn pump(app: &mut Harness, channels: u16, samples: &[f32]) -> Value {
    app.command(json!({"command":"pump_input", "channels":channels, "samples":samples}))
}

fn recording_audio(clip: &Clip) -> AudioData {
    match &clip.content {
        ClipContent::Audio { path, .. } => AudioData::load_wav(path).unwrap(),
        _ => panic!("Recording produced a nonaudio clip"),
    }
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

fn constant_output(state: &Value, expected: [f32; 2]) {
    let frames = state["audio_output"]
        .as_array()
        .expect("Virtual callback output");
    assert!(!frames.is_empty());
    // Any monitor gate smoothing may occupy the first small block.
    for frame in &frames[frames.len().saturating_sub(256)..] {
        near(frame[0].as_f64().unwrap() as f32, expected[0]);
        near(frame[1].as_f64().unwrap() as f32, expected[1]);
    }
}

fn set_number(app: &mut Harness, matches: impl Fn(&str) -> bool, value: &str) {
    let state = app.inspect();
    let field = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["text"].as_str().is_some_and(&matches))
        .unwrap_or_else(|| panic!("Numeric field missing: {}", state["visible_labels"]));
    let x = (field["min"][0].as_f64().unwrap() + field["max"][0].as_f64().unwrap()) * 0.5;
    let y = (field["min"][1].as_f64().unwrap() + field["max"][1].as_f64().unwrap()) * 0.5;
    // DragValue enters typing on a single click. A second click can hit the
    // adjacent field after its prefix disappears and the edit box narrows.
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter");
}

fn comp_selection(app: &mut Harness, source: u64, from: u32, to: u32, fade_ms: u32) {
    let state = menu(app, "Clip", "Takes…");
    let row = state["audio_takes"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == source)
        .unwrap();
    app.command(json!({"command":"pointer", "phase":"click",
        "x":(row["min"][0].as_f64().unwrap()+row["max"][0].as_f64().unwrap())*0.5,
        "y":(row["min"][1].as_f64().unwrap()+row["max"][1].as_f64().unwrap())*0.5}));
    if state["audio_takes"]["range"].is_null() {
        app.click("Time selection");
    }
    // Expand the left side first when moving to an earlier range; otherwise
    // expand the right side first. Both fields enforce a nonempty selection.
    let current = app.inspect();
    if f64::from(from) < current["audio_takes"]["range"][0].as_f64().unwrap() {
        set_number(
            app,
            |text| text.starts_with("From beat "),
            &(from + 1).to_string(),
        );
    }
    set_number(
        app,
        |text| text.starts_with("To beat "),
        &(to + 1).to_string(),
    );
    set_number(
        app,
        |text| text.starts_with("From beat "),
        &(from + 1).to_string(),
    );
    set_number(app, |text| text.ends_with(" ms"), &fade_ms.to_string());
    let state = app.inspect();
    assert_eq!(
        state["audio_takes"]["range"],
        json!([from as f64, to as f64])
    );
    app.click("Use selection");
}

#[test]
fn recorded_c3_d3_takes_comp_in_four_sections_with_exact_export_and_spline_crossfades() {
    let mut app = fixture(true, false);
    let mut p = project(&app.inspect());
    p.loop_end = 4.0;
    load(&mut app, &p, "four-beat-loop.json");
    app.click("Capture");
    arm(&mut app, "Stereo 1–2");
    let frequencies = [48.0_f64, 50.0].map(|note| 440.0 * 2.0_f64.powf((note - 69.0) / 12.0));
    let amplitudes = [0.2_f64, 0.3];
    let sources: [Vec<f32>; 2] = std::array::from_fn(|take| {
        (0..96_000)
            .map(|frame| {
                (amplitudes[take]
                    * (std::f64::consts::TAU * frequencies[take] * frame as f64 / RATE as f64)
                        .sin()) as f32
            })
            .collect()
    });
    begin(&mut app);
    for take in &sources {
        let stereo: Vec<f32> = take.iter().flat_map(|left| [*left, -*left * 0.5]).collect();
        pump(&mut app, 2, &stereo);
    }
    let state = stop(&mut app);
    let p = project(&state);
    let takes: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter(|clip| clip.is_take_source())
        .collect();
    assert_eq!(takes.len(), 2);
    for (index, take) in takes.iter().enumerate() {
        let audio = recording_audio(take);
        assert_eq!(audio.frames(), 96_000);
        for (frame, left) in sources[index].iter().enumerate() {
            assert_eq!(audio.samples[frame * 2], *left);
            assert_eq!(audio.samples[frame * 2 + 1], -*left * 0.5);
        }
    }
    let c3 = takes[0].id;
    menu(&mut app, "Window", "Clip Editor");
    comp_selection(&mut app, c3, 0, 1, 0);
    comp_selection(&mut app, c3, 2, 3, 0);
    let hard = export(&mut app, "c3-d3-hard-comp.wav");
    assert_eq!(hard.sample_rate, RATE);
    assert_eq!(hard.channels, 2);
    for frame in 0..96_000 {
        let expected = sources[(frame / 24_000) % 2][frame];
        near(hard.samples[frame * 2], expected);
        near(hard.samples[frame * 2 + 1], -expected * 0.5);
    }
    for quarter in 0..4 {
        let samples: Vec<f64> = hard.samples[quarter * 48_000..(quarter + 1) * 48_000]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0] as f64)
            .collect();
        let crossings: Vec<f64> = samples
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0] <= 0.0 && pair[1] > 0.0)
            .map(|(index, pair)| index as f64 - pair[0] / (pair[1] - pair[0]))
            .collect();
        let measured =
            (crossings.len() - 1) as f64 * RATE as f64 / (crossings.last().unwrap() - crossings[0]);
        assert!(
            (measured - frequencies[quarter % 2]).abs() < 0.01,
            "Quarter {quarter}: {measured} Hz"
        );
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f64>()
            / samples.len() as f64)
            .sqrt();
        assert!((rms - amplitudes[quarter % 2] / 2.0_f64.sqrt()).abs() < 0.003);
    }
    comp_selection(&mut app, c3, 0, 1, 10);
    comp_selection(&mut app, c3, 2, 3, 10);
    let smooth = export(&mut app, "c3-d3-smooth-comp.wav");
    let p = project(&app.inspect());
    let fades: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter_map(|clip| {
            if let ClipContent::Audio {
                crossfade: Some(fade),
                ..
            } = clip.content
            {
                Some((clip.start, fade))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(fades.len(), 3);
    for (_, fade) in &fades {
        assert!((fade.duration - 0.02).abs() < 1e-9);
        assert_eq!(fade.curve, daw_core::CrossfadeCurve::SMOOTH);
    }
    // Independent analytic cubic oracle: smooth handles at x=1/3,2/3 make
    // the incoming gain 3t²−2t³. Source position must not restart at a cut.
    for (frame, rendered) in smooth.samples[..192_000]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let quarter = frame / 24_000;
        let within = frame % 24_000;
        let expected = if quarter > 0 && within < 480 {
            let t = within as f64 / 480.0;
            let gain = (3.0 * t * t - 2.0 * t * t * t) as f32;
            sources[(quarter - 1) % 2][frame] * (1.0 - gain) + sources[quarter % 2][frame] * gain
        } else {
            assert_eq!(smooth.samples[frame * 2], hard.samples[frame * 2]);
            hard.samples[frame * 2]
        };
        near(rendered[0], expected);
        near(rendered[1], -expected * 0.5);
    }
    // The chosen C3 region begins at beat 2. Adjust only its incoming spline
    // with a real handle drag, then verify that all other samples stay intact.
    let popup = menu(&mut app, "Edit", "Crossfade…");
    let handle = &popup["crossfade"]["handles"][0];
    let plot = &popup["crossfade"]["plot"];
    let x = (handle["min"][0].as_f64().unwrap() + handle["max"][0].as_f64().unwrap()) * 0.5;
    let y = (handle["min"][1].as_f64().unwrap() + handle["max"][1].as_f64().unwrap()) * 0.5;
    let to_y = plot["max"][1].as_f64().unwrap()
        - 0.8 * (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap());
    let dragged = app
        .command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":x, "to_y":to_y}));
    let changed = project(&dragged);
    let clip = changed
        .clip(
            dragged["selected_track"].as_u64().unwrap(),
            dragged["selected_clip"].as_u64().unwrap(),
        )
        .unwrap();
    assert_eq!(clip.start, 2.0);
    let ClipContent::Audio {
        crossfade: Some(config),
        ..
    } = clip.content
    else {
        panic!("Missing comp crossfade")
    };
    let y1 = config.curve.control1[1] as f64;
    assert!((y1 - 0.8).abs() < 0.02);
    assert!((config.curve.control1[0] - 1.0 / 3.0).abs() < 1e-6);
    let custom = export(&mut app, "c3-d3-custom-spline.wav");
    for (frame, rendered) in custom.samples[..192_000]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let expected = if (48_000..48_480).contains(&frame) {
            let t = (frame - 48_000) as f64 / 480.0;
            let gain =
                (3.0 * (1.0 - t).powi(2) * t * y1 + 3.0 * (1.0 - t) * t * t + t.powi(3)) as f32;
            sources[1][frame] * (1.0 - gain) + sources[0][frame] * gain
        } else {
            assert_eq!(custom.samples[frame * 2], smooth.samples[frame * 2]);
            smooth.samples[frame * 2]
        };
        near(rendered[0], expected);
        near(rendered[1], -expected * 0.5);
    }
    app.quit();
}

#[test]
fn mono_input_recording_keeps_exact_raw_samples_beyond_empty_project_end_and_is_undoable() {
    let mut app = fixture(false, false);
    arm(&mut app, "Mono 2");
    let before = app.inspect();
    begin(&mut app);
    let expected: Vec<f32> = (0..48_000)
        .map(|frame| -0.3 + frame as f32 / 160_000.0)
        .collect();
    let stereo: Vec<f32> = expected.iter().flat_map(|right| [0.75, *right]).collect();
    let state = pump(&mut app, 2, &stereo);
    assert_eq!(state["recording"]["phase"], "recording");
    assert_eq!(
        state["engine"]["playing"], true,
        "Empty project stopped while recording"
    );
    let state = stop(&mut app);
    let p = project(&state);
    p.validate().unwrap();
    let clip = &p.tracks[0].clips[0];
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert!(!clip.is_take_source());
    assert_eq!(clip.start, 0.0);
    assert_eq!(clip.length, 2.0);
    let audio = recording_audio(clip);
    assert_eq!(audio.sample_rate, RATE);
    assert_eq!(audio.channels, 1);
    assert_eq!(audio.samples, expected);
    let undo = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(undo["project"], before["project"]);
    let redo = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    assert_eq!(redo["project"], state["project"]);
    app.quit();
}

#[test]
fn loop_recording_creates_distinct_complete_and_partial_takes_in_the_take_editor() {
    let mut app = fixture(true, false);
    arm(&mut app, "Mono 1");
    begin(&mut app);
    let expected: Vec<f32> = [vec![0.1; 24_000], vec![0.2; 24_000], vec![0.3; 12_000]].concat();
    pump(
        &mut app,
        2,
        &expected.iter().flat_map(|v| [*v, -*v]).collect::<Vec<_>>(),
    );
    let state = stop(&mut app);
    let p = project(&state);
    p.validate().unwrap();
    let sources: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter(|clip| clip.is_take_source())
        .collect();
    let regions: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter(|clip| !clip.is_take_source())
        .collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(regions.len(), 1);
    let group = sources[0].take_group().unwrap();
    for (index, source) in sources.iter().enumerate() {
        assert_eq!(source.take_group(), Some(group));
        assert_eq!(source.start, 0.0);
        let audio = recording_audio(source);
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.frames(), if index == 2 { 12_000 } else { 24_000 });
        for sample in audio.samples {
            near(sample, [0.1, 0.2, 0.3][index]);
        }
    }
    assert_eq!(
        regions[0].length, 1.0,
        "A partial final take must not truncate the complete pass"
    );
    let mixed = export(&mut app, "last-complete.wav");
    near(mixed.samples[2_000], 0.2);
    near(mixed.samples[40_000], 0.2);
    app.click("Window");
    app.click("Clip Editor");
    app.click("Clip");
    let chooser = app.click("Takes…");
    assert_eq!(chooser["audio_takes"]["rows"].as_array().unwrap().len(), 3);
    // Select the partial pass by the actual waveform row and use it explicitly.
    let row = &chooser["audio_takes"]["rows"][2];
    app.command(json!({"command":"pointer", "phase":"click", "x":(row["min"][0].as_f64().unwrap()+row["max"][0].as_f64().unwrap())/2.0, "y":(row["min"][1].as_f64().unwrap()+row["max"][1].as_f64().unwrap())/2.0}));
    app.click("Use whole take");
    let mixed = export(&mut app, "partial.wav");
    near(mixed.samples[2_000], 0.3);
    app.quit();
}

#[test]
fn stereo_recording_survives_archive_restart_without_arming_or_monitoring() {
    let mut app = fixture(false, false);
    arm(&mut app, "Stereo 1–2");
    begin(&mut app);
    let samples: Vec<f32> = (0..12_000)
        .flat_map(|frame| {
            let left = 0.2 * (std::f32::consts::TAU * frame as f32 / 100.0).sin();
            [left, -left * 0.5]
        })
        .collect();
    pump(&mut app, 2, &samples);
    let state = stop(&mut app);
    let p = project(&state);
    let audio = recording_audio(&p.tracks[0].clips[0]);
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.samples, samples);
    let archive = app.directory.path().join("recorded-session.rill");
    app.command(json!({"command":"save", "path":archive}));
    let ClipContent::Audio { path, .. } = &p.tracks[0].clips[0].content else {
        unreachable!()
    };
    std::fs::remove_file(path).unwrap();
    app = app.restart();
    let restored = app.command(json!({"command":"open", "path":archive}));
    assert_eq!(
        recording_audio(&project(&restored).tracks[0].clips[0]).samples,
        samples
    );
    assert_eq!(restored["recording"]["armed"], json!([]));
    assert_eq!(
        restored["device_open"], false,
        "Opening a session must not activate the microphone"
    );
    let popup = recording(&mut app);
    assert!(
        popup["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Stereo 1–2")
    );
    assert!(
        popup["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Off")
    );
    app.quit();
}

#[test]
fn overdub_layer_preserves_existing_playback_and_routes_new_audio_independently() {
    let mut app = fixture(false, true);
    select_input(&mut app, "Mono 1");
    app.click("Arm recording");
    app.click("New take");
    app.click("Overdub layer");
    app.key("Escape");
    let baseline = begin(&mut app);
    let during = pump(&mut app, 2, &vec![0.4; 24_000]);
    constant_output(&during, [0.1, 0.1]);
    assert_eq!(
        during["engine"]["engine_id"],
        baseline["engine"]["engine_id"]
    );
    assert_eq!(
        during["engine"]["generation"],
        baseline["engine"]["generation"]
    );
    let state = stop(&mut app);
    let p = project(&state);
    p.validate().unwrap();
    assert_eq!(p.tracks.len(), 2);
    assert_eq!(p.tracks[0].clips[0].name, "Backing");
    assert_eq!(p.tracks[1].clips.len(), 1);
    assert_ne!(p.tracks[0].color, p.tracks[1].color);
    assert_eq!(p.tracks[0].output_bus, p.tracks[1].output_bus);
    assert!(p.tracks[1].clips[0].audio_take.is_none());
    assert_eq!(
        recording_audio(&p.tracks[1].clips[0]).samples,
        vec![0.4; 12_000]
    );
    let audio = export(&mut app, "overdub.wav");
    near(audio.samples[2_000], 0.1 + 0.4 * p.tracks[1].gain);
    app.quit();
}

#[test]
fn recording_pause_finalizes_once_and_input_failure_recovers_the_captured_audio() {
    let mut app = fixture(false, false);
    arm(&mut app, "Mono 1");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.15; 8_000]);
    app.click("Pause");
    let paused = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(project(&paused).tracks[0].clips.len(), 1);
    assert_eq!(
        recording_audio(&project(&paused).tracks[0].clips[0]).frames(),
        4_000
    );
    for _ in 0..4 {
        app.inspect();
    }
    assert_eq!(project(&app.inspect()).tracks[0].clips.len(), 1);
    app.click("Start");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.35; 10_000]);
    app.command(json!({"command":"input_error"}));
    let failed = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle" && state["error"] == true
    });
    let p = project(&failed);
    p.validate().unwrap();
    assert!(
        p.tracks[0].clips.iter().any(|clip| {
            let audio = recording_audio(clip);
            audio.frames() == 5_000
                && audio
                    .samples
                    .iter()
                    .all(|sample| (*sample - 0.35).abs() < 1e-6)
        }),
        "Captured audio lost after input disconnect"
    );
    let details = app.click("Details");
    assert!(
        details["error_details"]
            .to_string()
            .to_lowercase()
            .contains("input")
    );
    app.click("Close");
    app.click("Dismiss");
    app.quit();
}

#[test]
fn compact_recording_failure_keeps_a_complete_input_target_and_all_take_actions_visible() {
    let mut app = fixture(false, true);
    let mut p = project(&app.inspect());
    let track = p.tracks[0].id;
    let first = p.tracks[0].clips[0].id;
    let alternate = Clip::audio(
        "Alternative",
        0.0,
        4.0,
        app.directory.path().join("Backing.wav").to_string_lossy(),
    );
    let second = alternate.id;
    p.tracks[0].clips.push(alternate);
    p.group_audio_takes(track, &[first, second]).unwrap();
    load(&mut app, &p, "recording-spacing-takes.rill");
    app.click("Backing");
    arm(&mut app, "Mono 1");
    recording(&mut app);
    app.click("Off");
    app.click("On");
    wait(&mut app, |state| state["recording"]["input_ready"] == true);
    app.command(json!({"command":"input_error"}));
    wait(&mut app, |state| state["error"] == true);
    app.click("Dismiss");
    for density in [1.0, 1.25, 2.0] {
        let mut state = app.command(
            json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":density}),
        );
        // Returning from Setup may restore the previously focused monitor
        // control lower in this scroll body. Reveal its first row with actual
        // wheel input; a collapsed viewport cannot pass the full-label check.
        let title = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "Recording · Capture")
            .unwrap();
        let x = (title["min"][0].as_f64().unwrap() + title["max"][0].as_f64().unwrap()) * 0.5;
        let y = title["max"][1].as_f64().unwrap()
            + f64::from(daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_MD);
        for _ in 0..3 {
            state = app.command(json!({"command":"scroll", "x":x, "y":y, "delta":1000.0}));
        }
        for text in [
            "Audio setup…",
            "Pre-count",
            "Click level",
            "Record",
            "Retry input",
            "Edit recording",
            "Takes…",
        ] {
            let bounds = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .find(|label| label["text"] == text)
                .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]));
            assert!(
                bounds["min"][0].as_f64().unwrap() >= 0.0,
                "{text}: {bounds}"
            );
            assert!(
                bounds["max"][0].as_f64().unwrap() <= 800.0,
                "{text}: {bounds}"
            );
            assert!(
                bounds["max"][1].as_f64().unwrap() <= 600.0,
                "{text}: {bounds}"
            );
            assert!(
                bounds["max"][1].as_f64().unwrap() - bounds["min"][1].as_f64().unwrap()
                    >= f64::from(daw_ui::tokens::FONT_BODY) - 1.0,
                "The complete {text} label must fit, not just a clipped sliver: {bounds}"
            );
        }
        let setup = app.click("Audio setup…");
        assert_eq!(setup["popups"]["setup"], true);
        assert_eq!(app.key("Escape")["popups"]["recording"], true);
    }
    app.quit();
}

#[test]
fn input_selection_can_be_cleared_without_mutating_existing_recordings() {
    let mut app = fixture(false, true);
    let state = app.inspect();
    let track = project(&state).tracks[0].id;
    let arm = state["timeline"]["recording_controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|control| control["track"] == track)
        .unwrap()["arm"]
        .clone();
    let click_arm = json!({"command":"pointer", "phase":"click",
        "x":(arm["min"][0].as_f64().unwrap()+arm["max"][0].as_f64().unwrap())*0.5,
        "y":(arm["min"][1].as_f64().unwrap()+arm["max"][1].as_f64().unwrap())*0.5});
    let armed = app.command(click_arm.clone());
    assert_eq!(armed["recording"]["armed"], json!([track]));
    assert_eq!(
        armed["popups"]["routing"], false,
        "Header arm overlapped the output-bus button"
    );
    let disarmed = app.command(click_arm);
    assert_eq!(disarmed["recording"]["armed"], json!([]));
    assert_eq!(disarmed["popups"]["routing"], false);
    let device = &disarmed["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|header| header["track"] == track)
        .unwrap()["sections"]["device"];
    let popup = app.command(json!({"command":"pointer", "phase":"click",
        "x":(device["min"][0].as_f64().unwrap()+device["max"][0].as_f64().unwrap())*0.5,
        "y":(device["min"][1].as_f64().unwrap()+device["max"][1].as_f64().unwrap())*0.5}));
    assert_eq!(popup["popups"]["recording"], true);
    assert!(
        popup["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!("Recording · Capture"))
    );
    app.click("Mono 1");
    app.click("Mono 2");
    app.click("Arm recording");
    let before = project(&app.inspect()).tracks[0].clips.clone();
    app.click("Mono 2");
    app.click("No input");
    let state = app.inspect();
    assert_eq!(project(&state).tracks[0].clips, before);
    app.key("Escape");
    app.click("Record");
    let state = app.inspect();
    assert_ne!(state["recording"]["phase"], "recording");
    assert_eq!(project(&state).tracks[0].clips, before);
    if state["error"] == true {
        app.click("Dismiss");
    }
    app.quit();
}

#[test]
fn monitoring_follows_channel_and_bus_controls_but_recorded_audio_stays_raw() {
    let mut app = Harness::with_virtual_audio(None, None);
    let mut p = Project::new();
    p.tempo = 120.0;
    let capture = p.add_audio_track("Capture");
    let bus = p.add_bus("Monitor bus");
    p.track_mut(capture).unwrap().gain = 0.5;
    p.track_mut(capture).unwrap().pan = 1.0;
    p.track_mut(bus).unwrap().gain = 0.5;
    p.set_output(capture, Some(bus)).unwrap();
    load(&mut app, &p, "monitor.json");
    app.click("Capture");
    select_input(&mut app, "Mono 2");
    app.click("Off");
    app.click("On");
    app.key("Escape");
    wait(&mut app, |state| state["recording"]["input_ready"] == true);
    let input = [0.7, 0.4].repeat(4096);
    let monitored = pump(&mut app, 2, &input);
    constant_output(&monitored, [0.0, 0.1]);
    assert_eq!(monitored["engine"]["playing"], false);
    recording(&mut app);
    app.click("On");
    app.click("Off");
    app.key("Escape");
    let muted = app.command(json!({"command":"pump_audio", "frames":2048}));
    constant_output(&muted, [0.0, 0.0]);
    recording(&mut app);
    app.click("Off");
    app.click("Auto");
    app.key("Escape");
    let unarmed = app.command(json!({"command":"pump_audio", "frames":2048}));
    constant_output(&unarmed, [0.0, 0.0]);
    recording(&mut app);
    app.click("Arm recording");
    app.key("Escape");
    wait(&mut app, |state| state["recording"]["input_ready"] == true);
    let armed = pump(&mut app, 2, &input);
    constant_output(&armed, [0.0, 0.1]);
    recording(&mut app);
    app.click("Auto");
    app.click("Off");
    app.key("Escape");
    begin(&mut app);
    let recorded = pump(&mut app, 2, &input);
    constant_output(&recorded, [0.0, 0.0]);
    let state = stop(&mut app);
    let p = project(&state);
    let clip = &p.track(capture).unwrap().clips[0];
    assert_eq!(recording_audio(clip).samples, vec![0.4; 4096]);
    app.quit();
}

#[test]
fn two_armed_inputs_record_in_sync_with_independent_channel_selection() {
    let mut app = fixture(false, false);
    menu(&mut app, "Track", "Add audio track");
    select_input(&mut app, "Mono 2");
    app.click("Arm recording");
    app.key("Escape");
    app.click("Capture");
    arm(&mut app, "Mono 1");
    let before = begin(&mut app);
    assert_eq!(before["recording"]["armed"].as_array().unwrap().len(), 2);
    let expected: Vec<[f32; 2]> = (0..6000)
        .map(|frame| [frame as f32 / 30000.0, -frame as f32 / 24000.0])
        .collect();
    pump(
        &mut app,
        2,
        &expected.iter().flatten().copied().collect::<Vec<_>>(),
    );
    let state = stop(&mut app);
    let p = project(&state);
    assert_eq!(p.tracks.len(), 2);
    for (index, track) in p.tracks.iter().enumerate() {
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].start, 0.0);
        assert_eq!(track.clips[0].length, 0.25);
        assert_eq!(
            recording_audio(&track.clips[0]).samples,
            expected
                .iter()
                .map(|frame| frame[index])
                .collect::<Vec<_>>()
        );
    }
    app.quit();
}

#[test]
fn auto_monitor_switches_between_stopped_playback_and_recording_without_recording_the_mix() {
    let mut app = fixture(false, true);
    select_input(&mut app, "Mono 1");
    app.click("Arm recording");
    app.click("Off");
    app.click("Auto");
    app.key("Escape");
    wait(&mut app, |state| state["recording"]["input_ready"] == true);
    let input = [0.4, -0.3].repeat(2048);
    let stopped = pump(&mut app, 2, &input);
    constant_output(&stopped, [0.4, 0.4]);
    app.click("Play");
    wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let playback = app.command(json!({"command":"pump_audio", "frames":2048}));
    constant_output(&playback, [0.1, 0.1]);
    assert_eq!(playback["recording"]["input_ready"], false);
    begin(&mut app);
    let recording = pump(&mut app, 2, &input);
    constant_output(&recording, [0.5, 0.5]);
    app.click("Stop recording");
    let committed = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    let p = project(&committed);
    let recorded = p.tracks[0]
        .clips
        .iter()
        .find(|clip| clip.name != "Backing")
        .unwrap();
    assert_eq!(recording_audio(recorded).samples, vec![0.4; 2048]);
    let playback = app.command(json!({"command":"pump_audio", "frames":2048}));
    constant_output(&playback, [0.1, 0.1]);
    app.click("Stop");
    app.quit();
}

#[test]
fn dummy_input_and_output_clocks_are_independent_and_four_channel_selection_is_exact() {
    let mut app = fixture(false, false);
    let device = app.command(json!({"command":"dummy_audio_device", "input_channels":4}));
    assert_eq!(device["recording"]["dummy_device"]["input_channels"], 4);
    arm(&mut app, "Mono 4");
    begin(&mut app);
    let input: Vec<f32> = (0..850)
        .flat_map(|frame| [0.1, 0.2, 0.3, -0.4 + frame as f32 / 10_000.0])
        .collect();
    app.command(json!({"command":"feed_input", "channels":4, "samples":input[..333*4]}));
    let fed = app.command(json!({"command":"feed_input", "channels":4, "samples":input[333*4..]}));
    assert_eq!(
        fed["recording"]["frames"], 0,
        "Input delivery advanced the output clock"
    );
    assert!(project(&fed).tracks[0].clips.is_empty());
    let consumed = app.command(json!({"command":"pump_audio", "frames":850}));
    assert_eq!(consumed["recording"]["frames"], 850);
    let state = stop(&mut app);
    let p = project(&state);
    let audio = recording_audio(&p.tracks[0].clips[0]);
    assert_eq!(audio.channels, 1);
    assert_eq!(
        audio.samples,
        input
            .as_chunks::<4>()
            .0
            .iter()
            .map(|frame| frame[3])
            .collect::<Vec<_>>()
    );
    app.quit();
}

#[test]
fn dummy_device_latency_is_compensated_before_loop_splitting_and_final_tail_is_retained() {
    let mut app = fixture(true, false);
    app.command(json!({"command":"dummy_audio_device", "input_latency_frames":128, "output_latency_frames":256}));
    arm(&mut app, "Mono 1");
    begin(&mut app);
    const DELAY: usize = 384;
    let expected: Vec<f32> = (0..60_000)
        .map(|frame| -0.2 + frame as f32 / 200_000.0)
        .collect();
    let delayed: Vec<f32> = std::iter::repeat_n(-0.9, DELAY)
        .chain(expected.iter().copied())
        .flat_map(|left| [left, -left])
        .collect();
    let captured = pump(&mut app, 2, &delayed[..60_000 * 2]);
    assert_eq!(captured["recording"]["frames"], 60_000 - DELAY);
    let stopping = app.click("Stop recording");
    assert_ne!(
        stopping["recording"]["phase"], "idle",
        "Capture lost the pending device tail"
    );
    pump(&mut app, 2, &delayed[60_000 * 2..]);
    let state = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(state["error"], false, "{}", state["status"]);
    let p = project(&state);
    let sources: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter(|clip| clip.is_take_source())
        .collect();
    assert_eq!(sources.len(), 3);
    for (index, source) in sources.iter().enumerate() {
        let end = ((index + 1) * 24_000).min(60_000);
        let audio = recording_audio(source);
        assert_eq!(source.start, 0.0);
        assert_eq!(audio.samples, expected[index * 24_000..end]);
    }
    assert_eq!(
        p.tracks[0]
            .clips
            .iter()
            .find(|clip| !clip.is_take_source())
            .unwrap()
            .length,
        1.0
    );
    app.click("Stop");
    app.quit();
}

fn recover_dummy_device_fault(output_failure: bool) {
    let mut app = fixture(false, false);
    arm(&mut app, "Mono 1");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.25; 1024]);
    if output_failure {
        app.command(json!({"command":"output_error"}));
    } else {
        app.command(json!({"command":"feed_input", "channels":2, "samples":vec![0.4; 20_000]}));
    }
    let state = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle" && state["error"] == true
    });
    let p = project(&state);
    p.validate().unwrap();
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert_eq!(
        recording_audio(&p.tracks[0].clips[0]).samples,
        vec![0.25; 512]
    );
    assert!(
        state["recording"]["error"]
            .as_str()
            .unwrap()
            .contains(if output_failure { "output" } else { "overflow" })
    );
    app.click("Dismiss");
    app.click("Stop");
    // A fresh capture must recover without reopening the document or losing
    // the playable prefix from the failed device.
    begin(&mut app);
    pump(&mut app, 2, &vec![0.15; 512]);
    let recovered = stop(&mut app);
    let p = project(&recovered);
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(
        p.tracks[0]
            .clips
            .iter()
            .any(|clip| recording_audio(clip).samples == vec![0.15; 256])
    );
    app.quit();
}

#[test]
fn dummy_input_overflow_reports_details_preserves_audio_and_allows_recording_again() {
    recover_dummy_device_fault(false);
}

#[test]
fn dummy_output_disconnect_preserves_audio_and_allows_recording_again() {
    recover_dummy_device_fault(true);
}

#[test]
fn new_project_finalizes_recording_before_the_unsaved_choice_and_cancel_keeps_it() {
    let mut app = fixture(false, false);
    arm(&mut app, "Mono 1");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.25; 8000]);
    menu(&mut app, "File", "New");
    let prompt = wait(&mut app, |state| {
        state["document_guard"]["phase"] == "confirm"
    });
    assert_eq!(prompt["recording"]["phase"], "idle");
    let p = project(&prompt);
    assert_eq!(p.tracks.len(), 1);
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert_eq!(
        recording_audio(&p.tracks[0].clips[0]).samples,
        vec![0.25; 4000]
    );
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], prompt["project"]);
    assert!(cancelled["document_guard"].is_null());
    let mixed = export(&mut app, "kept-after-cancel.wav");
    near(mixed.samples[1000], 0.25);
    menu(&mut app, "File", "New");
    wait(&mut app, |state| {
        state["document_guard"]["phase"] == "confirm"
    });
    app.click("Don't save");
    let new = wait(&mut app, |state| {
        state["project"]["tracks"] == json!([]) && state["document_guard"].is_null()
    });
    assert_eq!(new["recording"]["armed"], json!([]));
    assert_eq!(new["recording"]["input_ready"], false);
    app.quit();
}

#[test]
fn export_and_save_requests_include_capture_finalized_by_the_document_guard() {
    let mut app = fixture(false, false);
    arm(&mut app, "Mono 1");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.3; 4096]);
    let mixed = export(&mut app, "export-while-recording.wav");
    assert!(mixed.frames() >= 2048);
    for sample in &mixed.samples[..2048 * 2] {
        near(*sample, 0.3);
    }
    assert_eq!(app.inspect()["recording"]["phase"], "idle");
    app.click("Stop");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.2; 1000]);
    let archive = app.directory.path().join("saved-while-recording.rill");
    let saved = app.command(json!({"command":"save", "path":archive}));
    assert_eq!(saved["recording"]["phase"], "idle");
    assert_eq!(saved["modified"], false);
    assert_eq!(project(&saved).tracks[0].clips.len(), 2);
    app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":archive}));
    let p = project(&reopened);
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(
        p.tracks[0]
            .clips
            .iter()
            .any(|clip| recording_audio(clip).samples == vec![0.2; 500])
    );
    app.quit();
}

#[test]
fn recording_output_prepare_failure_returns_to_idle_and_recovers_after_restoring_the_source() {
    let mut app = fixture(false, true);
    arm(&mut app, "Mono 1");
    let before = app.inspect()["project"].clone();
    let missing = app.directory.path().join("Backing.wav");
    // The project opened successfully. Removing its fixture source now makes
    // the real output renderer fail when Record prepares playback.
    std::fs::remove_file(&missing).unwrap();
    app.click("Record");
    let failed = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle"
            && state["playback_loading"] == false
            && state["error"] == true
    });
    assert_eq!(failed["project"], before);
    assert!(
        failed["recording"]["error"]
            .as_str()
            .unwrap()
            .contains("Backing.wav")
    );
    app.click("Dismiss");
    recording(&mut app);
    app.click("Off");
    app.click("On");
    wait(&mut app, |state| {
        state["error"] == true && state["playback_loading"] == false
    });
    for _ in 0..16 {
        let state = app.inspect();
        assert_eq!(state["recording"]["phase"], "idle");
        assert_eq!(
            state["playback_loading"], false,
            "Failed monitoring retried without user action"
        );
        assert_eq!(state["project"], before);
    }
    app.click("Dismiss");
    AudioData {
        sample_rate: RATE,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&missing)
    .unwrap();
    app.click("Retry input");
    wait(&mut app, |state| state["recording"]["input_ready"] == true);
    app.key("Escape");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.25; 1024]);
    let recovered = stop(&mut app);
    let p = project(&recovered);
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(
        p.tracks[0]
            .clips
            .iter()
            .any(|clip| recording_audio(clip).samples == vec![0.25; 512])
    );
    app.quit();
}

#[test]
fn device_selection_persists_and_rate_change_finalizes_before_reinitializing() {
    let mut app = fixture(false, false);
    menu(&mut app, "Window", "Setup");
    app.click("Audio");
    app.click("System default");
    app.click("Rill virtual output");
    app.click("System default input");
    app.click("Rill virtual input");
    app.click("Apply");
    wait(&mut app, |state| state["project_pending"] == false);
    let settings: Value = serde_json::from_slice(
        &std::fs::read(app.directory.path().join("config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["input_device_name"], "Rill virtual input");
    assert_eq!(settings["device_name"], "Rill virtual output");
    arm(&mut app, "Mono 1");
    begin(&mut app);
    pump(&mut app, 2, &vec![0.3; 24_000]);
    menu(&mut app, "Window", "Setup");
    app.click("Audio");
    app.click("48000 Hz");
    app.click("44100 Hz");
    app.click("Apply");
    let state = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle"
            && state["settings"]["sample_rate"] == 44100
            && state["project_pending"] == false
    });
    let p = project(&state);
    assert_eq!(p.tracks[0].clips.len(), 1);
    let audio = recording_audio(&p.tracks[0].clips[0]);
    assert_eq!(audio.sample_rate, RATE);
    assert_eq!(audio.samples, vec![0.3; 12_000]);
    let mixed = export(&mut app, "new-rate.wav");
    assert_eq!(mixed.sample_rate, 44_100);
    near(mixed.samples[2000], 0.3);
    let path = app.directory.path().join("input-settings.rill");
    app.command(json!({"command":"save", "path":path}));
    app = app.restart();
    let state = app.command(json!({"command":"open", "path":path}));
    assert_eq!(state["settings"]["input_device_name"], "Rill virtual input");
    assert_eq!(state["settings"]["sample_rate"], 44_100);
    assert_eq!(state["recording"]["armed"], json!([]));
    app.quit();
}

#[test]
fn stopping_capture_keeps_backing_instrument_effect_and_transport_running() {
    let workers = tempfile::tempdir().unwrap();
    let source = workers.path().join("recording_host.rs");
    std::fs::write(&source, include_str!("common/live_audio_host.rs")).unwrap();
    let host = workers.path().join(if cfg!(windows) {
        "recording-host.exe"
    } else {
        "recording-host"
    });
    let build =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .args(["-C", "opt-level=2"])
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let plugins = common::plugin_fixture();
    let mut app = Harness::with_virtual_audio(Some(plugins.path()), Some(&host));
    let mut p = Project::new();
    p.tempo = 120.0;
    let capture = p.add_audio_track("Capture");
    p.track_mut(capture).unwrap().gain = 1.0;
    let backing = p.add_midi_track("Backing instrument");
    let path = plugins
        .path()
        .join("RillFixture.vst3")
        .to_string_lossy()
        .into_owned();
    p.set_instrument(
        backing,
        Instrument::Vst3 {
            path: path.clone(),
            class_id: Some("11111111111111111111111111111111".into()),
        },
    )
    .unwrap();
    let track = p.track_mut(backing).unwrap();
    track.gain = 1.0;
    track.clips.push(Clip::midi(
        "Sustained note",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 8.0, 100)],
    ));
    track.inserts.push(EffectSlot::new(
        "Backing delay",
        Effect::Vst3 {
            path,
            class_id: Some("33333333333333333333333333333333".into()),
        },
    ));
    load(&mut app, &p, "recording-sandboxes.json");
    app.click("Capture");
    arm(&mut app, "Mono 1");
    let before = begin(&mut app);
    let output = pump(&mut app, 2, &vec![0.3; 4096]);
    constant_output(&output, [0.1, 0.1]);
    let logs = || {
        let mut values: Vec<_> = std::fs::read_dir(workers.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
            .map(|path| {
                let entries: Vec<Value> = std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                (path, entries)
            })
            .collect();
        values.sort_by(|a, b| a.0.cmp(&b.0));
        values
    };
    let baseline = logs();
    assert_eq!(baseline.len(), 2);
    app.click("Stop recording");
    let stopped = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(stopped["engine"]["playing"], true);
    assert_eq!(
        stopped["engine"]["engine_id"],
        before["engine"]["engine_id"]
    );
    assert_eq!(
        stopped["engine"]["generation"],
        before["engine"]["generation"]
    );
    let continued = app.command(json!({"command":"pump_audio", "frames":4096}));
    constant_output(&continued, [0.1, 0.1]);
    let after = logs();
    assert_eq!(after.len(), baseline.len());
    for ((path, current), (old_path, old)) in after.iter().zip(&baseline) {
        assert_eq!(
            path, old_path,
            "Recording restarted an isolated plugin process"
        );
        for command in [
            "LoadPlugin",
            "StartProcessing",
            "StopProcessing",
            "MidiPanic",
            "SetTransportPosition",
            "LoadState",
            "Reconfigure",
        ] {
            let count = |lines: &[Value]| {
                lines
                    .iter()
                    .filter(|entry| **entry == json!(command) || !entry[command].is_null())
                    .count()
            };
            assert_eq!(
                count(current),
                count(old),
                "{command} changed while finalizing recording"
            );
        }
    }
    let p = project(&continued);
    assert_eq!(
        recording_audio(&p.track(capture).unwrap().clips[0]).samples,
        vec![0.3; 2048]
    );
    app.click("Stop");
    app.quit();
}
