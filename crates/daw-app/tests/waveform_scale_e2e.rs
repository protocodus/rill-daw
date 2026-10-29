//! Arranger waveform fitting changes pixels, never clip gain or original audio.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture {
    app: Harness,
    ids: [u64; 5],
    sources: Vec<(PathBuf, Vec<u8>)>,
}

fn fixture() -> Fixture {
    let mut app = Harness::new();
    let quiet_path = app.directory.path().join("Quiet source.wav");
    let loud_path = app.directory.path().join("Loud source.wav");
    let silent_path = app.directory.path().join("Silent source.wav");
    let mut quiet = AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..192_000)
            .flat_map(|frame| {
                let amplitude = if frame < 96_000 { 0.02 } else { 0.0002 };
                let sample = (std::f32::consts::TAU * frame as f32 / 48.0).sin() * amplitude;
                [sample, sample * 0.5]
            })
            .collect(),
    };
    // Both spikes are outside the trimmed windows. Fitting must use each clip,
    // rather than its whole source file or only the currently visible columns.
    quiet.samples[0] = 0.95;
    quiet.samples[191_999 * 2] = -0.95;
    quiet.write_wav(&quiet_path).unwrap();
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..192_000)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * frame as f32 / 48.0).sin() * 0.6;
                [sample, sample * 0.5]
            })
            .collect(),
    }
    .write_wav(&loud_path)
    .unwrap();
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.0; 192_000 * 2],
    }
    .write_wav(&silent_path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Quiet clips");
    let mut first = Clip::audio("Quiet", 0.0, 3.0, quiet_path.to_string_lossy());
    first.offset = 1.0;
    if let ClipContent::Audio { gain, .. } = &mut first.content {
        *gain = 0.5;
    }
    let mut second = Clip::audio("Very quiet", 4.0, 3.0, quiet_path.to_string_lossy());
    second.offset = 4.0;
    if let ClipContent::Audio { gain, reverse, .. } = &mut second.content {
        *gain = 0.5;
        *reverse = true;
    }
    let first_id = first.id;
    let second_id = second.id;
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let track = project.add_audio_track("Loud clips");
    let loud = Clip::audio("Loud", 0.0, 8.0, loud_path.to_string_lossy());
    let loud_id = loud.id;
    project.track_mut(track).unwrap().clips.push(loud);
    let track = project.add_audio_track("Silence and zero gain");
    let silent = Clip::audio("Silent", 0.0, 3.0, silent_path.to_string_lossy());
    let mut zero = Clip::audio("Zero gain", 4.0, 3.0, quiet_path.to_string_lossy());
    if let ClipContent::Audio { gain, .. } = &mut zero.content {
        *gain = 0.0;
    }
    let ids = [first_id, second_id, loud_id, silent.id, zero.id];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([silent, zero]);
    let path = app.directory.path().join("Waveform view.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1400, "height":1000}));
    app.click("Fit");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["waveform"]["ready_count"] == 3 && state["waveform"]["pending_count"] == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Waveform preparation timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let sources = [quiet_path, loud_path, silent_path]
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    Fixture { app, ids, sources }
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn preview(state: &Value, id: u64) -> &Value {
    state["waveform"]["painted"]
        .as_array()
        .unwrap()
        .iter()
        .find(|paint| paint["view"] == "arranger" && paint["clip"] == id)
        .unwrap()
}

fn envelope_bounds(state: &Value, id: u64) -> (f64, f64) {
    let preview = preview(state, id);
    let rect = &preview["rect"];
    let mesh = state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mesh| {
            let bounds = &mesh["bounds"];
            mesh["valid"] == true
                && mesh["transparent_vertices"].as_u64().unwrap() > 0
                && mesh["vertices"] == preview["vertices"]
                && mesh["indices"] == preview["indices"]
                && bounds[0].as_f64().unwrap() >= rect[0].as_f64().unwrap() - 2.0
                && bounds[2].as_f64().unwrap() <= rect[2].as_f64().unwrap() + 2.0
                && bounds[1].as_f64().unwrap() >= rect[1].as_f64().unwrap() - 2.0
                && bounds[3].as_f64().unwrap() <= rect[3].as_f64().unwrap() + 2.0
        })
        .unwrap_or_else(|| panic!("Missing actual waveform mesh: {preview}"));
    (
        mesh["bounds"][1].as_f64().unwrap(),
        mesh["bounds"][3].as_f64().unwrap(),
    )
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "revision",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            state[field], before[field],
            "Waveform presentation changed {field}"
        );
    }
}

fn scale(app: &mut Harness, label: &str) -> Value {
    app.click("View");
    app.click(label);
    app.key("Escape")
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn fit_fills_each_clip_independently_preserves_stereo_and_changes_only_presentation() {
    let Fixture {
        mut app,
        ids,
        sources,
    } = fixture();
    let original_audio = export(&mut app, "Original mix.wav");
    let before = paint(&mut app);
    assert_eq!(before["timeline"]["waveform_amplitude"], "Default");
    let (top, _) = envelope_bounds(&before, ids[0]);
    let default = preview(&before, ids[0]);
    assert_eq!(default["amplitude_reference"], 1.0);
    assert!((default["maximum"].as_f64().unwrap() - 0.01).abs() < 1e-6);
    let rect = &default["rect"];
    let row = (rect[3].as_f64().unwrap() - rect[1].as_f64().unwrap()) / 4.0;
    // Mesh bounds include a one-physical-pixel antialias coverage fringe.
    // At the fixture's 1x density this must be an absolute tolerance: after
    // Fit caps sparse tracks at 96 points it exceeds 5% of this small row.
    let coverage_fringe = 1.0;
    assert!(
        top + coverage_fringe > rect[1].as_f64().unwrap() + row * 0.95,
        "Default quiet waveform must remain near its channel center: top={top}, rect={rect}"
    );
    scale(&mut app, "Fit");
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1400, "height":1000, "pixels_per_point":density}),
        );
        let fitted = paint(&mut app);
        assert_eq!(fitted["timeline"]["waveform_amplitude"], "Fit");
        unchanged(&fitted, &before);
        for (id, expected) in ids[..3].iter().zip([0.01, 0.0001, 0.6]) {
            let preview = preview(&fitted, *id);
            assert_eq!(preview["stage"], "refined");
            assert!(
                (preview["amplitude_reference"].as_f64().unwrap() - expected).abs()
                    < expected * 0.001
            );
            let (top, bottom) = envelope_bounds(&fitted, *id);
            let rect = &preview["rect"];
            // Channel mapping uses the shared surface's preview region, which
            // reserves title and border space independently of track height.
            let content_rect = &fitted["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|clip| clip["id"] == *id)
                .unwrap()["content_rect"];
            let height = content_rect["max"][1].as_f64().unwrap() - rect[1].as_f64().unwrap();
            assert!(
                (top - rect[1].as_f64().unwrap()).abs() < 2.0,
                "Quiet left channel must fill its half"
            );
            assert!(
                (bottom - rect[1].as_f64().unwrap() - height * 0.875).abs() < 2.0,
                "One common stereo reference must keep the right channel half as loud: id={id}, density={density}, bounds={top}..{bottom}, rect={rect}, preview={preview}"
            );
        }
        for id in ids[3..].iter() {
            let preview = preview(&fitted, *id);
            assert_eq!(preview["amplitude_reference"], 1.0);
            assert_eq!(preview["minimum"], 0.0);
            assert_eq!(preview["maximum"], 0.0);
        }
        let warm = paint(&mut app);
        assert_eq!(
            warm["waveform"]["geometry_builds"],
            fitted["waveform"]["geometry_builds"]
        );
        for id in ids {
            assert_eq!(preview(&warm, id)["sample_reads"], 0);
        }
    }
    assert_eq!(
        export(&mut app, "Fitted mix.wav").samples,
        original_audio.samples
    );
    scale(&mut app, "Default");
    let restored = paint(&mut app);
    unchanged(&restored, &before);
    for id in ids {
        assert_eq!(preview(&restored, id)["amplitude_reference"], 1.0);
    }
    for (path, bytes) in sources {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    app.quit();
}

#[test]
fn fit_reference_stays_fixed_while_navigating_and_view_controls_fit_compact_windows() {
    let Fixture { mut app, ids, .. } = fixture();
    scale(&mut app, "Fit");
    let before = paint(&mut app);
    app.key("Plus");
    let zoomed = paint(&mut app);
    unchanged(&zoomed, &before);
    for id in ids[..3].iter() {
        assert_eq!(
            preview(&zoomed, *id)["amplitude_reference"],
            preview(&before, *id)["amplitude_reference"]
        );
    }
    app.key("F");
    for width in [800, 1100, 1480] {
        let state = app.command(json!({"command":"viewport", "width":width, "height":900}));
        assert_eq!(state["timeline"]["waveform_amplitude"], "Fit");
        app.click("View");
        let menu = app.inspect();
        for label in ["Waveform amplitude", "Default", "Fit"] {
            assert!(
                menu["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == label)
            );
        }
        app.key("Escape");
        let state = app.inspect();
        unchanged(&state, &before);
    }
    app.quit();
}

#[test]
fn a_cut_heavy_arrangement_keeps_hundreds_of_small_waveforms_warm() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Individual hits.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * frame as f32 / 48.0).sin() * 0.02;
                [sample, sample * 0.5]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    for row in 0..4 {
        let track = project.add_audio_track(format!("Cut track {row}"));
        for index in 0..48 {
            let mut clip = Clip::audio("Hit", index as f64 * 0.25, 0.125, source.to_string_lossy());
            clip.offset = (index % 16) as f64 * 0.1;
            project.track_mut(track).unwrap().clips.push(clip);
        }
    }
    let path = app.directory.path().join("Cut session.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1480, "height":1000}));
    app.click("Fit");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["waveform"]["ready_count"] == 1 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    scale(&mut app, "Fit");
    let prepared = paint(&mut app);
    let warm = paint(&mut app);
    let previews = warm["waveform"]["painted"].as_array().unwrap();
    assert_eq!(
        previews.len(),
        192,
        "All short clips must actually be visible"
    );
    assert_eq!(
        warm["waveform"]["geometry_builds"],
        prepared["waveform"]["geometry_builds"]
    );
    for preview in previews {
        assert_eq!(preview["stage"], "refined");
        assert_eq!(preview["cached"], true);
        assert_eq!(preview["sample_reads"], 0);
    }
    unchanged(&warm, &prepared);
    app.quit();
}

#[test]
fn fit_includes_overlapping_fades_and_preserves_extremely_quiet_gains() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Fade reference.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.2; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let mut ids = Vec::new();
    for (name, gain, faded) in [
        ("Faded", 1.0, true),
        ("Subnormal gain", f32::from_bits(1), false),
    ] {
        let track = project.add_audio_track(name);
        let mut clip = Clip::audio(name, 0.0, 4.0, source.to_string_lossy());
        if let ClipContent::Audio {
            gain: clip_gain,
            fade_in,
            fade_out,
            ..
        } = &mut clip.content
        {
            *clip_gain = gain;
            if faded {
                *fade_in = 4.0;
                *fade_out = 4.0;
            }
        }
        ids.push(clip.id);
        project.track_mut(track).unwrap().clips.push(clip);
    }
    let path = app.directory.path().join("Faded view.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Fit");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["waveform"]["ready_count"] == 1 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = app.inspect();
    scale(&mut app, "Fit");
    let fitted = paint(&mut app);
    unchanged(&fitted, &before);
    let reference = preview(&fitted, ids[0])["amplitude_reference"]
        .as_f64()
        .unwrap();
    assert!(
        (reference - 0.05).abs() < 0.0001,
        "Both fades attenuate the peak by four"
    );
    for id in ids {
        let preview = preview(&fitted, id);
        let rect = &preview["rect"];
        let (top, _) = envelope_bounds(&fitted, id);
        assert!(
            (top - rect[1].as_f64().unwrap()).abs() < 2.0,
            "Post-envelope fitting must reach full height even with a tiny gain"
        );
    }
    app.quit();
}
