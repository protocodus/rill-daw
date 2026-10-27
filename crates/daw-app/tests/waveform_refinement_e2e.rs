//! Progressive waveform geometry, exact transients and background spectra through the real app.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const FRAMES: usize = RATE as usize * 4;

fn signal(path: &Path, high_first: bool) -> AudioData {
    let mut audio = AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..FRAMES)
            .flat_map(|frame| {
                let high = (frame < FRAMES / 2) == high_first;
                let hz = if high { 4096.0 } else { 512.0 };
                let sample = (std::f64::consts::TAU * hz * frame as f64 / f64::from(RATE)).sin()
                    as f32
                    * 0.05;
                [sample, -sample]
            })
            .collect(),
    };
    // These one-sample transients are deliberately away from ordinary block
    // boundaries. Refined peaks must retain them regardless of sampling stride.
    audio.samples[66_613 * 2] = 0.93;
    audio.samples[66_613 * 2 + 1] = -0.87;
    audio.samples[128_321 * 2] = -0.79;
    audio.samples[128_321 * 2 + 1] = 0.83;
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

fn waveform(state: &Value) -> &Value {
    &state["waveform"]
}

fn wait(app: &mut Harness, condition: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        assert_eq!(state["error"], false, "{}", state["status"]);
        if condition(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Waveform preview did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ready(app: &mut Harness, count: usize) -> Value {
    wait(app, |state| {
        waveform(state)["pending_count"] == 0
            && waveform(state)["ready_count"].as_u64() == Some(count as u64)
    })
}

fn painted<'a>(state: &'a Value, view: &str) -> Vec<&'a Value> {
    waveform(state)["painted"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|paint| paint["view"] == view)
        .collect()
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn assert_unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(
            state[field], before[field],
            "Waveform refinement changed {field}"
        );
    }
}

fn assert_refined_mesh(state: &Value, preview: &Value, density: f64) {
    assert_eq!(preview["stage"], "refined");
    let columns = preview["columns"].as_u64().unwrap();
    let channels = preview["channels"].as_u64().unwrap();
    assert!(columns > 0 && (1..=2).contains(&channels));
    // Preparation and source duration cannot multiply visible drawing work.
    let visible_width =
        (preview["rect"][2].as_f64().unwrap() - preview["rect"][0].as_f64().unwrap()).min(1400.0);
    assert!(columns <= (visible_width * density).ceil() as u64 * channels + 4 * channels);
    let vertices = preview["vertices"].as_u64().unwrap();
    let indices = preview["indices"].as_u64().unwrap();
    assert!(vertices > 0 && vertices <= columns * 32 + 32);
    assert!(indices > 0 && indices <= columns * 96 + 96);
    assert!(preview["fringe_vertices"].as_u64().unwrap() > 0);
    let meshes: Vec<_> = state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mesh| {
            let bounds = &mesh["bounds"];
            let rect = &preview["rect"];
            mesh["valid"] == true
                && mesh["transparent_vertices"].as_u64().unwrap() > 0
                && mesh["max_alpha"].as_u64().unwrap() > 0
                && bounds[0].as_f64().unwrap() < rect[2].as_f64().unwrap()
                && bounds[2].as_f64().unwrap() > rect[0].as_f64().unwrap()
                && bounds[1].as_f64().unwrap() < rect[3].as_f64().unwrap()
                && bounds[3].as_f64().unwrap() > rect[1].as_f64().unwrap()
        })
        .collect();
    let submitted = meshes.iter().enumerate().any(|(index, mesh)| {
        (mesh["vertices"] == vertices && mesh["indices"] == indices)
            || meshes[index + 1..].iter().any(|other| {
                mesh["vertices"].as_u64().unwrap() + other["vertices"].as_u64().unwrap() == vertices
                    && mesh["indices"].as_u64().unwrap() + other["indices"].as_u64().unwrap()
                        == indices
            })
    });
    assert!(
        submitted,
        "Refinement must submit real anti-aliased geometry, not just report a stage: {preview}"
    );
}

fn assert_spectrum_behind(state: &Value, view: &str) {
    let layers: Vec<_> = state["spectrogram"]["painted"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|layer| layer["view"] == view)
        .collect();
    assert!(!layers.is_empty(), "Missing spectrum backdrop in {view}");
    for layer in layers {
        let opacity = layer["opacity"].as_f64().unwrap();
        assert!(opacity > 0.0 && opacity <= 0.3);
        assert!(
            state["painted_images"]
                .as_array()
                .unwrap()
                .iter()
                .any(|image| image["texture_id"] == layer["texture_id"])
        );
    }
}

fn surface_fixture() -> (Harness, PathBuf, u64) {
    let mut app = Harness::new();
    let primary = app.directory.path().join("transient-spectrum.wav");
    let alternate = app.directory.path().join("alternate-transients.wav");
    signal(&primary, false);
    signal(&alternate, true);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Refined waveforms");
    let first = Clip::audio("Transient source", 0.0, 8.0, primary.to_string_lossy());
    let second = Clip::audio("Alternate take", 0.0, 8.0, alternate.to_string_lossy());
    let ids = [first.id, second.id];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let group = project.group_audio_takes(track, &ids).unwrap();
    let selected = project.audio_take_regions(track, group)[0].id;
    project.clone_clip(track, selected).unwrap();
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Neighbour",
        2.0,
        2.0,
        primary.to_string_lossy(),
    ));
    open(&mut app, &project, "refinement-surfaces");
    app.click("Fit");
    (app, primary, selected)
}

#[test]
fn raw_previews_refine_to_real_aa_meshes_with_exact_transients_and_shared_source_caches() {
    let (mut app, primary, selected) = surface_fixture();
    let before = app.inspect();
    let settled = ready(&mut app, 1);
    assert!(waveform(&settled)["raw_paints"].as_u64().unwrap() > 0);
    assert!(waveform(&settled)["refined_paints"].as_u64().unwrap() > 0);
    assert_eq!(waveform(&settled)["completed"], 1);
    assert!(waveform(&settled)["cache_hits"].as_u64().unwrap() > 0);
    assert!(
        waveform(&settled)["cache_bytes"].as_u64().unwrap()
            <= waveform(&settled)["limits"]["bytes"].as_u64().unwrap()
    );
    let source = waveform(&settled)["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| Path::new(entry["path"].as_str().unwrap()).file_name() == primary.file_name())
        .unwrap();
    assert_eq!(source["frames"], FRAMES);
    assert_eq!(source["channels"], 2);
    assert!(source["levels"].as_u64().unwrap() > 1);
    assert_unchanged(&settled, &before);
    wait(&mut app, |state| state["spectrogram"]["ready_count"] == 1);

    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1400, "height":900,
            "pixels_per_point":density}));
        let state = paint(&mut app);
        let previews = painted(&state, "arranger");
        assert!(previews.len() >= 3);
        for preview in &previews {
            assert_refined_mesh(&state, preview, density);
        }
        let main = previews
            .iter()
            .find(|preview| preview["clip"] == selected)
            .unwrap();
        assert!(main["minimum"].as_f64().unwrap() <= -0.86);
        assert!(main["maximum"].as_f64().unwrap() >= 0.92);
        assert_spectrum_behind(&state, "arranger");
        assert_unchanged(&state, &before);
        assert_eq!(waveform(&state)["completed"], 1);
        let warm = paint(&mut app);
        assert_eq!(
            waveform(&warm)["geometry_builds"],
            waveform(&state)["geometry_builds"]
        );
        assert!(
            waveform(&warm)["geometry_cache_hits"].as_u64().unwrap()
                > waveform(&state)["geometry_cache_hits"].as_u64().unwrap()
        );
        for preview in painted(&warm, "arranger") {
            assert_eq!(preview["cached"], true);
            assert_eq!(
                preview["sample_reads"], 0,
                "Static repaint must reuse prepared geometry rather than rescan PCM"
            );
        }
    }

    let state = app.inspect();
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == selected)
        .unwrap()["rect"];
    app.command(json!({"command":"pointer", "phase":"click",
        "x":rect["min"][0].as_f64().unwrap()+12.0,
        "y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())*0.5}));
    app.click("Window");
    app.click("Clip Editor");
    let state = paint(&mut app);
    for view in ["audio_editor", "audio_neighbour"] {
        let previews = painted(&state, view);
        assert!(!previews.is_empty(), "Missing refined {view}");
        for preview in previews {
            assert_refined_mesh(&state, preview, 2.0);
        }
        assert_spectrum_behind(&state, view);
    }
    assert_unchanged(&state, &before);
    assert_eq!(waveform(&state)["completed"], 1);
    app.click("Clip");
    app.click("Takes…");
    ready(&mut app, 2);
    wait(&mut app, |state| state["spectrogram"]["ready_count"] == 2);
    let state = paint(&mut app);
    let rows = painted(&state, "takes");
    assert_eq!(rows.len(), 2);
    for preview in rows {
        assert_refined_mesh(&state, preview, 2.0);
    }
    assert_spectrum_behind(&state, "takes");
    assert_unchanged(&state, &before);
    app.quit();
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn trimmed_forward_and_reverse_previews_exclude_impulses_just_outside_the_source_window() {
    let mut app = Harness::new();
    let path = app.directory.path().join("outside-window-impulses.wav");
    let mut audio = AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: vec![0.0; RATE as usize * 3],
    };
    // Source beats 2..3 are exactly silent: [24000, 48000) frames at 120 BPM.
    // Both immediately adjacent samples are loud so rounding either boundary
    // outward creates a visible transient that the audible clip never contains.
    audio.samples[(24_000 - 1) * 2] = 0.95;
    audio.samples[(24_000 - 1) * 2 + 1] = -0.9;
    audio.samples[48_000 * 2] = -0.85;
    audio.samples[48_000 * 2 + 1] = 0.93;
    audio.write_wav(&path).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Source boundary");
    project.track_mut(track).unwrap().gain = 1.0;
    let mut forward = Clip::audio("Silent forward", 0.0, 1.0, path.to_string_lossy());
    forward.offset = 1.0;
    let mut reversed = Clip::audio("Silent reverse", 1.0, 1.0, path.to_string_lossy());
    reversed.offset = 1.0;
    if let ClipContent::Audio { reverse, .. } = &mut reversed.content {
        *reverse = true;
    }
    let ids = [forward.id, reversed.id];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([forward, reversed]);
    let before = open(&mut app, &project, "silent-boundary-window");
    app.click("Fit");
    ready(&mut app, 1);
    for (name, id) in [("Silent forward", ids[0]), ("Silent reverse", ids[1])] {
        app.click(name);
        app.click("Window");
        app.click("Clip Editor");
        app.click("Fit");
        let state = paint(&mut app);
        let previews = painted(&state, "audio_editor");
        let selected = previews
            .iter()
            .find(|preview| preview["clip"] == id)
            .unwrap();
        assert_refined_mesh(&state, selected, 1.0);
        assert_eq!(
            selected["minimum"], 0.0,
            "Outside source transient leaked into {name}"
        );
        assert_eq!(
            selected["maximum"], 0.0,
            "Outside source transient leaked into {name}"
        );
        assert_unchanged(&state, &before);
        app.click("Back");
    }
    let rendered = export(&mut app, "silent-boundaries.wav");
    assert!(
        rendered.samples.iter().all(|sample| *sample == 0.0),
        "Silent source windows must stay silent in the actual mixdown"
    );
    assert_unchanged(&app.inspect(), &before);
    app.quit();
}

#[test]
fn refined_columns_keep_gain_and_reverse_trim_stretch_source_mapping_when_zoomed() {
    let mut app = Harness::new();
    let path = app.directory.path().join("mapped-transients.wav");
    signal(&path, false);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Mapped waveforms");
    let mut normal = Clip::audio("Quiet source", 0.0, 8.0, path.to_string_lossy());
    if let ClipContent::Audio { gain, .. } = &mut normal.content {
        *gain = 0.5;
    }
    let mut reversed = Clip::audio("Reverse", 8.0, 8.0, path.to_string_lossy());
    if let ClipContent::Audio { reverse, .. } = &mut reversed.content {
        *reverse = true;
    }
    let mut trimmed = Clip::audio("Trim", 16.0, 4.0, path.to_string_lossy());
    trimmed.offset = 2.0;
    let mut stretched = Clip::audio("Stretched octave", 20.0, 16.0, path.to_string_lossy());
    stretched.source_rate = 0.5;
    if let ClipContent::Audio { processing, .. } = &mut stretched.content {
        processing.transpose_semitones = 12.0;
    }
    let quiet = normal.id;
    let expected = [
        (normal.id, 0.0, FRAMES as f64),
        (reversed.id, FRAMES as f64 - 1.0, -1.0),
        (trimmed.id, FRAMES as f64 * 0.25, FRAMES as f64 * 0.75),
        (stretched.id, 0.0, FRAMES as f64),
    ];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([normal, reversed, trimmed, stretched]);
    let before = open(&mut app, &project, "mapped-refinement");
    app.click("Fit");
    ready(&mut app, 1);
    let state = paint(&mut app);
    let previews = painted(&state, "arranger");
    for (id, left, right) in expected {
        let preview = previews
            .iter()
            .find(|preview| preview["clip"] == id)
            .unwrap();
        assert_refined_mesh(&state, preview, 1.0);
        let clip = &state["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == id)
            .unwrap()["rect"];
        let x = clip["min"][0].as_f64().unwrap();
        let width = clip["max"][0].as_f64().unwrap() - x;
        for (field, edge) in [("source_left", 0), ("source_right", 2)] {
            let fraction = (preview["rect"][edge].as_f64().unwrap() - x) / width;
            let expected = left + (right - left) * fraction;
            assert!(
                (preview[field].as_f64().unwrap() - expected).abs() <= 2.0,
                "{field} has incorrect source mapping: {preview}; expected {expected}"
            );
        }
    }
    let quiet = previews
        .iter()
        .find(|preview| preview["clip"] == quiet)
        .unwrap();
    assert!((quiet["minimum"].as_f64().unwrap() + 0.435).abs() < 0.003);
    assert!((quiet["maximum"].as_f64().unwrap() - 0.465).abs() < 0.003);
    assert_eq!(waveform(&state)["completed"], 1);
    assert_unchanged(&state, &before);

    // Zooming can leave most of a long occurrence outside the viewport. Its
    // geometry must still be bounded by visible pixels, not full clip duration.
    let row = &state["timeline"]["clips"][0]["rect"];
    let y = (row["min"][1].as_f64().unwrap() + row["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"scroll","x":900.0,"y":y,"delta":180.0,"ctrl":true}));
    let zoomed = paint(&mut app);
    let visible = painted(&zoomed, "arranger");
    assert!(
        !visible.is_empty(),
        "Zoom must retain visible refined geometry"
    );
    for preview in visible {
        assert_refined_mesh(&zoomed, preview, 1.0);
    }
    assert_unchanged(&zoomed, &before);
    app.quit();
}

#[test]
fn refinement_preserves_playing_samples_and_new_documents_discard_previous_source_geometry() {
    let mut app = Harness::with_virtual_audio(None, None);
    let path = app.directory.path().join("playing-transients.wav");
    let source = signal(&path, false);
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Playing waveform");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Playing source",
        0.0,
        8.0,
        path.to_string_lossy(),
    ));
    let before = open(&mut app, &project, "playing-refinement");
    app.click("Playing source");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Play");
    let started = wait(&mut app, |state| state["engine"]["playing"] == true);
    let mut audio = Vec::new();
    for dark in [false, true] {
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
        assert_unchanged(&state, &before);
        audio.extend(
            state["audio_output"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|frame| {
                    frame
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|sample| sample.as_f64().unwrap() as f32)
                }),
        );
    }
    assert_eq!(audio, source.samples[..audio.len()]);
    let settled = ready(&mut app, 1);
    assert_unchanged(&settled, &before);
    let generation = waveform(&settled)["generation"].as_u64().unwrap();
    app.click("Pause");
    let rendered = export(&mut app, "refinement.wav");
    assert_eq!(rendered.samples[..source.samples.len()], source.samples);
    assert!(
        rendered.samples[source.samples.len()..]
            .iter()
            .all(|sample| *sample == 0.0)
    );

    let next_path = app.directory.path().join("replacement-transients.wav");
    signal(&next_path, true);
    let mut next = Project::new();
    let track = next.add_audio_track("Replacement");
    next.track_mut(track).unwrap().clips.push(Clip::audio(
        "Replacement source",
        0.0,
        8.0,
        next_path.to_string_lossy(),
    ));
    let replaced = open(&mut app, &next, "replacement-refinement");
    let state = ready(&mut app, 1);
    assert!(waveform(&state)["generation"].as_u64().unwrap() > generation);
    assert_eq!(waveform(&state)["sources"].as_array().unwrap().len(), 1);
    assert_unchanged(&state, &replaced);
    let state = paint(&mut app);
    for preview in painted(&state, "arranger") {
        assert_eq!(
            Path::new(preview["path"].as_str().unwrap()).file_name(),
            next_path.file_name()
        );
        assert_refined_mesh(&state, preview, 1.0);
    }
    app.quit();
}
