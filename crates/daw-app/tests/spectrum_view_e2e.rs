//! Spectrum controls change cached visual textures and tint, never source/audio.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;

fn fixture(virtual_audio: bool, takes: bool) -> (Harness, PathBuf, AudioData, u64) {
    let mut app = if virtual_audio {
        Harness::with_virtual_audio(None, None)
    } else {
        Harness::new()
    };
    let source = app.directory.path().join("Spectrum source.wav");
    let audio = AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..RATE as usize * 2)
            .flat_map(|i| {
                let hz = if i < RATE as usize { 440.0 } else { 3520.0 };
                let sample = (i as f32 * std::f32::consts::TAU * hz / RATE as f32).sin() * 0.3
                    + (i as f32 * std::f32::consts::TAU * 9000.0 / RATE as f32).sin() * 0.002;
                [sample, -sample]
            })
            .collect(),
    };
    audio.write_wav(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Spectrum channel");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().color = [100, 150, 200];
    let first = Clip::audio("Spectrum cue", 0.0, 4.0, source.to_string_lossy());
    let id = first.id;
    project.track_mut(track).unwrap().clips.push(first);
    let id = if takes {
        let second = Clip::audio("Retained take", 0.0, 4.0, source.to_string_lossy());
        let other = second.id;
        project.track_mut(track).unwrap().clips.push(second);
        let group = project.group_audio_takes(track, &[id, other]).unwrap();
        let comp = project.audio_take_regions(track, group)[0].id;
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Neighbour cue",
            2.0,
            4.0,
            source.to_string_lossy(),
        ));
        comp
    } else {
        id
    };
    let path = app.directory.path().join("Spectrum view.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1400, "height":900}));
    wait(&mut app, |state| {
        state["spectrogram"]["ready_count"] == 1 && state["spectrogram"]["pending_count"] == 0
    });
    (app, source, audio, id)
}

fn wait(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        assert_eq!(state["error"], false, "{}", state["status"]);
        if ready(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Spectrum did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(state[field], before[field], "Spectrum view changed {field}");
    }
    for field in ["analyses_started", "completed", "generation", "cache_bytes"] {
        assert_eq!(
            state["spectrogram"][field], before["spectrogram"][field],
            "Presentation changed analysis {field}"
        );
    }
}

fn intensity(app: &mut Harness, value: &str) -> Value {
    let state = app.click("View");
    let r = &state["spectrogram"]["controls"]["intensity"];
    app.command(json!({"command":"pointer", "phase":"double_click", "x":(r[0].as_f64().unwrap()+r[2].as_f64().unwrap())*0.5, "y":(r[1].as_f64().unwrap()+r[3].as_f64().unwrap())*0.5}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter");
    app.key("Escape")
}

fn palette(app: &mut Harness, value: &str) -> Value {
    app.click("View");
    app.click(value);
    app.key("Escape")
}

fn assert_texture_paint<'a>(state: &'a Value, palette_index: usize, view: &str) -> Vec<&'a Value> {
    let spectrum = &state["spectrogram"];
    let texture = &spectrum["sources"][0]["texture_ids"][palette_index];
    let paints: Vec<_> = spectrum["painted"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["view"] == view)
        .collect();
    assert!(
        !paints.is_empty(),
        "Missing {view} spectrum paint: {spectrum}"
    );
    for p in &paints {
        assert_eq!(&p["texture_id"], texture);
        let matching = state["painted_images"]
            .as_array()
            .unwrap()
            .iter()
            .find(|image| {
                image["texture_id"] == p["texture_id"]
                    && (image["rect"]["min"][0].as_f64().unwrap() - p["rect"][0].as_f64().unwrap())
                        .abs()
                        < 1.0
                    && (image["rect"]["min"][1].as_f64().unwrap() - p["rect"][1].as_f64().unwrap())
                        .abs()
                        < 1.0
            })
            .unwrap_or_else(|| panic!("No actual textured quad for {p}"));
        let vertices = matching["vertex_colors"].as_array().unwrap();
        assert_eq!(vertices.len(), 4);
        for vertex in vertices {
            assert_eq!(vertex[3], p["tint_alpha"]);
            if palette_index == 1 {
                assert_eq!(vertex[0], vertex[1], "Color palette must use neutral tint");
                assert_eq!(vertex[1], vertex[2]);
            }
        }
    }
    paints
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn opacity_and_palette_reuse_cached_textures_across_arranger_editor_and_takes() {
    let (mut app, source, _, comp) = fixture(false, true);
    let source_bytes = std::fs::read(&source).unwrap();
    let before = paint(&mut app);
    let audio = export(&mut app, "Original.wav");
    let original_texture_ids = before["spectrogram"]["sources"][0]["texture_ids"].clone();
    assert_eq!(original_texture_ids.as_array().unwrap().len(), 2);
    assert_eq!(before["spectrogram"]["view"]["palette"], "Monochrome");
    assert_texture_paint(&before, 0, "arranger");
    let colors = &before["spectrogram"]["sources"][0]["texture_colors"];
    assert!(
        colors[0]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c[0] == c[1] && c[1] == c[2])
    );
    assert!(colors[1][0][2].as_u64().unwrap() > colors[1][0][0].as_u64().unwrap());
    assert!(colors[1][2][0].as_u64().unwrap() > colors[1][2][2].as_u64().unwrap());
    palette(&mut app, "Blue–yellow");
    intensity(&mut app, "65");
    for dark in [true, false] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        let state = paint(&mut app);
        let paints = assert_texture_paint(&state, 1, "arranger");
        assert!(paints.iter().all(|p| {
            (p["opacity"].as_f64().unwrap() - 0.65 * if dark { 0.80 } else { 0.65 }).abs() < 1e-5
        }));
        unchanged(&state, &before);
    }
    intensity(&mut app, "0");
    let hidden = paint(&mut app);
    assert!(
        hidden["spectrogram"]["painted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!hidden["waveform"]["painted"].as_array().unwrap().is_empty());
    unchanged(&hidden, &before);
    intensity(&mut app, "100");
    let state = paint(&mut app);
    assert_texture_paint(&state, 1, "arranger");
    // The comp spans beats 0–4 and its neighbour starts at beat 2. Click the
    // uncovered body before the overlap, outside the ten-point trim handle.
    let clips = state["timeline"]["clips"].as_array().unwrap();
    let clip = clips.iter().find(|c| c["id"] == comp).unwrap();
    let rect = &clip["rect"];
    let left = rect["min"][0].as_f64().unwrap();
    let uncovered_right = clips
        .iter()
        .filter(|other| other["id"] != comp && other["track"] == clip["track"])
        .map(|other| other["rect"]["min"][0].as_f64().unwrap())
        .filter(|other_left| *other_left > left)
        .fold(rect["max"][0].as_f64().unwrap(), f64::min);
    let x = (left + uncovered_right) * 0.5;
    assert!(x > left + 10.0 && x < uncovered_right);
    let content = &clip["content_rect"];
    let selected = app.command(json!({"command":"pointer", "phase":"click", "x":x,
        "y":(content["min"][1].as_f64().unwrap()+content["max"][1].as_f64().unwrap())*0.5}));
    assert_eq!(selected["selected_clip"], comp);
    unchanged(&selected, &before);
    app.click("Window");
    app.click("Clip Editor");
    let state = paint(&mut app);
    unchanged(&state, &before);
    assert_texture_paint(&state, 1, "audio_editor");
    assert_texture_paint(&state, 1, "audio_neighbour");
    palette(&mut app, "Monochrome");
    app.click("Clip");
    app.click("Takes…");
    let state = paint(&mut app);
    assert_eq!(assert_texture_paint(&state, 0, "takes").len(), 2);
    intensity(&mut app, "37");
    palette(&mut app, "Blue–yellow");
    let state = paint(&mut app);
    assert_eq!(assert_texture_paint(&state, 1, "takes").len(), 2);
    assert_eq!(
        state["spectrogram"]["sources"][0]["texture_ids"],
        original_texture_ids
    );
    unchanged(&state, &before);
    assert!(
        state["spectrogram"]["cache_bytes"].as_u64().unwrap()
            <= state["spectrogram"]["limits"]["bytes"].as_u64().unwrap()
    );
    app.key("Escape");
    assert_eq!(export(&mut app, "Viewed.wav").samples, audio.samples);
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    app.quit();
}

#[test]
fn precise_intensity_is_clamped_and_view_controls_remain_inside_narrow_windows() {
    let (mut app, _, _, _) = fixture(false, false);
    let before = app.inspect();
    for (width, dark, density) in [(800.0, false, 1.0), (1200.0, true, 2.0)] {
        app.command(
            json!({"command":"viewport", "width":width, "height":760, "pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme", "dark":dark}));
        for (value, expected) in [("-20", 0.0), ("150", 1.0)] {
            intensity(&mut app, value);
            let state = app.click("View");
            assert_eq!(state["spectrogram"]["view"]["intensity"], expected);
            for field in ["intensity", "palette"] {
                let r = &state["spectrogram"]["controls"][field];
                assert!(r[0].as_f64().unwrap() >= 0.0 && r[2].as_f64().unwrap() <= width);
                assert!(r[1].as_f64().unwrap() >= 0.0 && r[3].as_f64().unwrap() <= 760.0);
            }
            app.key("Escape");
            unchanged(&state, &before);
        }
    }
    app.quit();
}

#[test]
fn spectrum_settings_leave_the_active_playback_generation_and_samples_unchanged() {
    let (mut app, source, audio, _) = fixture(true, false);
    let original_bytes = std::fs::read(&source).unwrap();
    let before = app.inspect();
    app.click("Play");
    let started = wait(&mut app, |state| state["engine"]["playing"] == true);
    let mut rendered = Vec::new();
    for (palette_name, percent) in [
        ("Blue–yellow", "100"),
        ("Monochrome", "0"),
        ("Blue–yellow", "28"),
    ] {
        palette(&mut app, palette_name);
        intensity(&mut app, percent);
        let state = app.command(json!({"command":"pump_audio", "frames":4096}));
        for field in [
            "engine_id",
            "generation",
            "underruns",
            "render_errors",
            "edit_errors",
        ] {
            assert_eq!(state["engine"][field], started["engine"][field]);
        }
        assert_eq!(state["engine"]["playing"], true);
        unchanged(&state, &before);
        for frame in state["audio_output"].as_array().unwrap() {
            rendered.extend(
                frame
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap() as f32),
            );
        }
    }
    assert_eq!(rendered, audio.samples[..rendered.len()]);
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
    app.quit();
}
