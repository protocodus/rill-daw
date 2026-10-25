//! Spectral underlays use real background analysis and real UI paint commands.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const FRAMES: usize = RATE as usize * 2;

fn tone(path: &Path, first: f64, second: f64) -> AudioData {
    let audio = AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..FRAMES)
            .flat_map(|frame| {
                let hz = if frame < FRAMES / 2 { first } else { second };
                let sample = (std::f64::consts::TAU * hz * frame as f64 / f64::from(RATE)).sin()
                    as f32
                    * 0.3;
                // Opposing stereo channels must not cancel in spectral analysis.
                [sample, -sample]
            })
            .collect(),
    };
    audio.write_wav(path).unwrap();
    audio
}

fn open(app: &mut Harness, project: &Project, name: &str) -> Value {
    let path = app.directory.path().join(format!("{name}.daw.json"));
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1400, "height":900}));
    app.inspect()
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
            "Spectral preview did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn spectrum(state: &Value) -> &Value {
    &state["spectrogram"]
}

fn ready(app: &mut Harness, sources: usize) -> Value {
    wait(app, |state| {
        spectrum(state)["pending_count"] == 0
            && spectrum(state)["ready_count"].as_u64() == Some(sources as u64)
    })
}

fn source<'a>(state: &'a Value, path: &Path) -> &'a Value {
    spectrum(state)["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| {
            source["path"]
                .as_str()
                .is_some_and(|name| Path::new(name).file_name() == path.file_name())
        })
        .unwrap_or_else(|| {
            panic!(
                "Missing spectral source {}: {}",
                path.display(),
                spectrum(state)
            )
        })
}

fn assert_document_unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(
            state[field], before[field],
            "Spectral preview changed {field}"
        );
    }
}

fn assert_tones(source: &Value, first: f64, second: f64) {
    assert_eq!(source["status"], "ready");
    assert_eq!(source["sample_rate"], RATE);
    assert_eq!(source["source_frames"], FRAMES);
    assert_eq!(source["bands"], 48);
    let columns = source["columns"].as_u64().unwrap();
    assert!((8..=1024).contains(&columns));
    let peaks = source["peak_hz"].as_array().unwrap();
    for (index, expected) in [(0, first), (1, second)] {
        let actual = peaks[index].as_f64().unwrap();
        assert!(
            (actual / expected).ln().abs() < 0.18,
            "Time-frequency peak {index}: {actual} Hz, expected near {expected} Hz"
        );
    }
}

fn painted<'a>(state: &'a Value, view: &str) -> Vec<&'a Value> {
    spectrum(state)["painted"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|paint| paint["view"] == view)
        .collect()
}

fn assert_real_underlay(state: &Value, paint: &Value) {
    let alpha = paint["opacity"].as_f64().unwrap();
    assert!(
        alpha > 0.0 && alpha <= 0.3,
        "Spectrum must remain a faint underlay: {paint}"
    );
    assert!(
        state["painted_images"]
            .as_array()
            .unwrap()
            .iter()
            .any(|image| {
                if image["texture_id"] != paint["texture_id"] {
                    return false;
                }
                let actual = [
                    image["rect"]["min"][0].as_f64().unwrap(),
                    image["rect"]["min"][1].as_f64().unwrap(),
                    image["rect"]["max"][0].as_f64().unwrap(),
                    image["rect"]["max"][1].as_f64().unwrap(),
                ];
                actual.iter().enumerate().all(|(index, value)| {
                    (value - paint["rect"][index].as_f64().unwrap()).abs() < 1.0
                })
            }),
        "Spectral diagnostics must correspond to a real textured paint command: {paint}"
    );
}

fn paint_frame(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn assert_samples(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    if let Some((index, (actual, expected))) = actual
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (actual, expected))| (*actual - *expected).abs() > 1e-7)
    {
        panic!("Audio changed at interleaved sample {index}: {actual} != {expected}");
    }
}

fn surface_fixture() -> (Harness, PathBuf, PathBuf, u64) {
    let mut app = Harness::new();
    let primary = app.directory.path().join("low-then-high.wav");
    let alternate = app.directory.path().join("high-then-low.wav");
    tone(&primary, 512.0, 4096.0);
    tone(&alternate, 4096.0, 512.0);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Spectral sources");
    let a = Clip::audio("Spectrum source", 0.0, 4.0, primary.to_string_lossy());
    let b = Clip::audio("Alternate spectrum", 0.0, 4.0, alternate.to_string_lossy());
    let ids = [a.id, b.id];
    project.track_mut(track).unwrap().clips.extend([a, b]);
    let group = project.group_audio_takes(track, &ids).unwrap();
    let comp = project.audio_take_regions(track, group)[0].id;
    project.clone_clip(track, comp).unwrap();
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Neighbour",
        1.0,
        2.0,
        primary.to_string_lossy(),
    ));
    open(&mut app, &project, "spectral-surfaces");
    (app, primary, alternate, comp)
}

#[test]
fn async_spectral_underlays_cover_arranger_editor_neighbours_takes_and_both_themes() {
    let (mut app, primary, alternate, comp) = surface_fixture();
    let before = app.inspect();
    let state = ready(&mut app, 1);
    assert_tones(source(&state, &primary), 512.0, 4096.0);
    assert_document_unchanged(&state, &before);
    assert!(spectrum(&state)["jobs_started"].as_u64().unwrap() > 0);
    assert_eq!(spectrum(&state)["completed"], 1);
    assert!(spectrum(&state)["cache_hits"].as_u64().unwrap() > 0);
    assert!(
        spectrum(&state)["cache_bytes"].as_u64().unwrap()
            <= spectrum(&state)["limits"]["bytes"].as_u64().unwrap()
    );
    for dark in [true, false] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        let state = paint_frame(&mut app);
        let underlays = painted(&state, "arranger");
        assert!(underlays.len() >= 3);
        for paint in underlays {
            assert_real_underlay(&state, paint);
        }
        assert_document_unchanged(&state, &before);
        assert_eq!(spectrum(&state)["completed"], 1);
    }
    // The neighbouring clip overlaps the middle, and the ghost shares a name.
    // Select the original comp by clicking its uncovered leading edge.
    let state = app.inspect();
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == comp)
        .unwrap()["rect"];
    let state = app.command(json!({"command":"pointer", "phase":"click",
        "x":rect["min"][0].as_f64().unwrap() + 8.0,
        "y":(rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5}));
    assert_eq!(state["selected_clip"], comp);
    app.click("Window");
    app.click("Clip Editor");
    let state = paint_frame(&mut app);
    let selected = painted(&state, "audio_editor");
    assert!(selected.iter().any(|paint| paint["clip"] == comp));
    let neighbours = painted(&state, "audio_neighbour");
    assert!(!neighbours.is_empty());
    for paint in selected.into_iter().chain(neighbours) {
        assert_real_underlay(&state, paint);
    }
    assert_eq!(spectrum(&state)["completed"], 1);
    app.click("Clip");
    app.click("Takes…");
    let state = ready(&mut app, 2);
    assert_tones(source(&state, &alternate), 4096.0, 512.0);
    let state = paint_frame(&mut app);
    let rows = painted(&state, "takes");
    assert_eq!(rows.len(), 2);
    for paint in rows {
        assert_real_underlay(&state, paint);
    }
    assert_document_unchanged(&state, &before);
    app.quit();
}

#[test]
fn reverse_trim_and_stretch_map_one_cached_source_into_the_correct_timeline_spans() {
    let mut app = Harness::new();
    let path = app.directory.path().join("mapped-spectrum.wav");
    tone(&path, 512.0, 4096.0);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Mapped spectra");
    let normal = Clip::audio("Normal", 0.0, 4.0, path.to_string_lossy());
    let mut reverse = Clip::audio("Reverse", 4.0, 4.0, path.to_string_lossy());
    if let ClipContent::Audio { reverse, .. } = &mut reverse.content {
        *reverse = true;
    }
    let mut trimmed = Clip::audio("Trimmed", 8.0, 2.0, path.to_string_lossy());
    trimmed.offset = 1.0;
    let mut stretched = Clip::audio("Stretched", 10.0, 8.0, path.to_string_lossy());
    stretched.source_rate = 0.5;
    if let ClipContent::Audio { processing, .. } = &mut stretched.content {
        processing.transpose_semitones = 12.0;
    }
    let transposed = stretched.id;
    let expected = [
        (normal.id, 0.0, FRAMES as f64),
        (reverse.id, FRAMES as f64 - 1.0, -1.0),
        (trimmed.id, FRAMES as f64 * 0.25, FRAMES as f64 * 0.75),
        (stretched.id, 0.0, FRAMES as f64),
    ];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([normal, reverse, trimmed, stretched]);
    let before = open(&mut app, &project, "mapped-spectra");
    app.click("Fit");
    ready(&mut app, 1);
    let state = paint_frame(&mut app);
    assert_tones(source(&state, &path), 512.0, 4096.0);
    let paints = painted(&state, "arranger");
    for (id, left, right) in expected {
        let paint = paints.iter().find(|paint| paint["clip"] == id).unwrap();
        assert_real_underlay(&state, paint);
        let rect = &state["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == id)
            .unwrap()["rect"];
        let x = rect["min"][0].as_f64().unwrap();
        let width = rect["max"][0].as_f64().unwrap() - x;
        for (field, edge) in [("source_left", 0), ("source_right", 2)] {
            // The arranger clips the underlay inside its three-pixel border.
            // Verify the visible endpoints against the independent full-clip
            // source mapping instead of expecting hidden edge samples.
            let fraction = (paint["rect"][edge].as_f64().unwrap() - x) / width;
            let expected = left + (right - left) * fraction;
            assert!(
                (paint[field].as_f64().unwrap() - expected).abs() <= 2.0,
                "{field} mapped incorrectly: {paint}, expected {expected}"
            );
        }
        if id == transposed {
            assert_eq!(paint["transpose"], 12.0);
            assert!(paint["uv"][1].as_f64().unwrap() > 0.0);
            assert!((paint["uv"][3].as_f64().unwrap() - 1.0).abs() < 1e-6);
        }
    }
    assert_eq!(spectrum(&state)["completed"], 1);
    assert_document_unchanged(&state, &before);
    app.quit();
}

#[test]
fn background_previews_preserve_playback_export_history_and_drop_old_project_results() {
    let mut app = Harness::with_virtual_audio(None, None);
    let path = app.directory.path().join("playing-spectrum.wav");
    let audio = tone(&path, 512.0, 4096.0);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Playing spectrum");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Playing source",
        0.0,
        4.0,
        path.to_string_lossy(),
    ));
    let before = open(&mut app, &project, "playing-spectrum");
    // Exercise ordinary input immediately, without waiting for spectral work.
    app.click("Playing source");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Play");
    let started = wait(&mut app, |state| state["engine"]["playing"] == true);
    let mut output = Vec::new();
    for dark in [true, false] {
        app.command(json!({"command":"system_theme", "dark":dark}));
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
        assert_document_unchanged(&state, &before);
        for frame in state["audio_output"].as_array().unwrap() {
            output.extend(
                frame
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|sample| sample.as_f64().unwrap() as f32),
            );
        }
    }
    assert_samples(&output, &audio.samples[..output.len()]);
    let settled = ready(&mut app, 1);
    assert_document_unchanged(&settled, &before);
    assert!(spectrum(&settled)["jobs_started"].as_u64().unwrap() > 0);
    let generation = spectrum(&settled)["generation"].as_u64().unwrap();
    let exported = export(&mut app, "spectral-export.wav");
    assert!(exported.samples.len() >= audio.samples.len());
    assert_samples(&exported.samples[..audio.samples.len()], &audio.samples);
    // Export includes the normal effect-tail allowance even without inserts.
    assert!(
        exported.samples[audio.samples.len()..]
            .iter()
            .all(|sample| *sample == 0.0)
    );

    let replacement_path = app.directory.path().join("replacement-spectrum.wav");
    tone(&replacement_path, 2048.0, 256.0);
    let mut replacement = Project::new();
    replacement.tempo = 120.0;
    let track = replacement.add_audio_track("Replacement");
    replacement
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::audio(
            "Replacement source",
            0.0,
            4.0,
            replacement_path.to_string_lossy(),
        ));
    let replacement_before = open(&mut app, &replacement, "replacement-spectrum");
    let state = ready(&mut app, 1);
    assert!(spectrum(&state)["generation"].as_u64().unwrap() > generation);
    assert_eq!(spectrum(&state)["sources"].as_array().unwrap().len(), 1);
    assert_tones(source(&state, &replacement_path), 2048.0, 256.0);
    assert_document_unchanged(&state, &replacement_before);
    let painted_state = paint_frame(&mut app);
    for paint in painted(&painted_state, "arranger") {
        assert_eq!(
            Path::new(paint["path"].as_str().unwrap()).file_name(),
            replacement_path.file_name()
        );
    }
    app.quit();
}
