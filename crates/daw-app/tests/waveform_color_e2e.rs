//! Amplitude-aware waveform colors and explicit view toggles through the real app.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

const TRACK_COLOR: [u8; 4] = [48, 160, 224, 255];

struct Fixture {
    app: Harness,
    clips: [u64; 3],
    sources: Vec<(PathBuf, Vec<u8>)>,
}

fn fixture() -> Fixture {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_audio_track("Amplitude colors");
    project.track_mut(track).unwrap().color = [TRACK_COLOR[0], TRACK_COLOR[1], TRACK_COLOR[2]];
    let mut ids = Vec::new();
    let mut sources = Vec::new();
    for (index, (name, amplitude)) in [("Silent cue", 0.0), ("Quiet cue", 0.05), ("Loud cue", 1.0)]
        .into_iter()
        .enumerate()
    {
        let path = app.directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: (0..48_000)
                .map(|frame| {
                    if frame % 2 == 0 {
                        amplitude
                    } else {
                        -amplitude
                    }
                })
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        sources.push((path.clone(), std::fs::read(&path).unwrap()));
        let mut clip = Clip::audio(name, index as f64 * 3.0, 2.0, path.to_string_lossy());
        if index == 1
            && let ClipContent::Audio { gain, .. } = &mut clip.content
        {
            *gain = 0.5;
        }
        ids.push(clip.id);
        project.track_mut(track).unwrap().clips.push(clip);
    }
    let path = app.directory.path().join("Amplitude colors.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1400, "height":900}));
    app.click("Fit");
    wait_until(&mut app, |state| {
        state["waveform"]["ready_count"] == 3
            && state["waveform"]["pending_count"] == 0
            && state["spectrogram"]["ready_count"] == 3
            && state["spectrogram"]["pending_count"] == 0
    });
    Fixture {
        app,
        clips: ids.try_into().unwrap(),
        sources,
    }
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn toggle(app: &mut Harness, label: &str) -> Value {
    app.click("View");
    app.click(label);
    app.key("Escape")
}

fn intensity(app: &mut Harness, value: &str) {
    let state = app.click("View");
    let r = &state["spectrogram"]["controls"]["intensity"];
    app.command(json!({"command":"pointer", "phase":"double_click",
        "x":(r[0].as_f64().unwrap()+r[2].as_f64().unwrap())*0.5,
        "y":(r[1].as_f64().unwrap()+r[3].as_f64().unwrap())*0.5}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter");
    app.key("Escape");
}

fn palette(app: &mut Harness, label: &str) {
    app.click("View");
    app.click(label);
    app.key("Escape");
}

fn preview<'a>(state: &'a Value, clip: u64, view: &str) -> &'a Value {
    state["waveform"]["painted"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["clip"] == clip && p["view"] == view)
        .unwrap()
}

fn mesh<'a>(state: &'a Value, clip: u64, view: &str) -> &'a Value {
    let p = preview(state, clip, view);
    assert_eq!(p["stage"], "refined");
    state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mesh| {
            let b = &mesh["bounds"];
            let r = &p["rect"];
            mesh["vertices"] == p["vertices"]
                && mesh["indices"] == p["indices"]
                && mesh["transparent_vertices"].as_u64().unwrap() > 0
                && b[0].as_f64().unwrap() >= r[0].as_f64().unwrap() - 2.0
                && b[2].as_f64().unwrap() <= r[2].as_f64().unwrap() + 2.0
                && b[1].as_f64().unwrap() >= r[1].as_f64().unwrap() - 2.0
                && b[3].as_f64().unwrap() <= r[3].as_f64().unwrap() + 2.0
        })
        .unwrap_or_else(|| panic!("Missing submitted waveform mesh for {p}"))
}

fn solid_colors(state: &Value, clip: u64, view: &str) -> Vec<[u8; 4]> {
    let colors: Vec<[u8; 4]> = mesh(state, clip, view)["vertex_data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|vertex| std::array::from_fn(|i| vertex["color"][i].as_u64().unwrap() as u8))
        .filter(|color: &[u8; 4]| color[3] > 0)
        .collect();
    assert!(!colors.is_empty());
    colors
}

fn distance(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..3).map(|i| a[i].abs_diff(b[i]) as u32).sum()
}

fn luminance(color: [u8; 4]) -> f64 {
    let linear = |value: u8| {
        let srgb = f64::from(value) / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

fn contrast(a: [u8; 4], b: [u8; 4]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn chroma(color: [u8; 4]) -> u8 {
    color[..3].iter().max().unwrap() - color[..3].iter().min().unwrap()
}

fn theme_palette(dark: bool) -> daw_ui::theme::Palette {
    if dark {
        daw_ui::theme::DARK
    } else {
        daw_ui::theme::LIGHT
    }
}

fn peak_color(dark: bool) -> [u8; 4] {
    daw_ui::waveform_peak_color(
        eframe::egui::Color32::from_rgb(TRACK_COLOR[0], TRACK_COLOR[1], TRACK_COLOR[2]),
        theme_palette(dark),
    )
    .to_array()
}

fn readable_peak(colors: &[[u8; 4]], dark: bool) {
    let palette = theme_palette(dark);
    let expected = peak_color(dark);
    assert!(colors.iter().all(|color| *color == expected));
    for color in colors {
        assert!(
            contrast(*color, palette.bg.to_array()) >= 3.0,
            "Peak waveform {color:?} loses canvas contrast in dark={dark}"
        );
        assert!(
            contrast(*color, palette.raised.to_array()) >= 3.0,
            "Peak waveform {color:?} loses raised-surface contrast in dark={dark}"
        );
    }
}

fn colored(state: &Value, clips: [u64; 3], dark: bool) {
    let palette = theme_palette(dark);
    let peak = peak_color(dark);
    let endpoint = daw_ui::waveform_quiet_color(
        eframe::egui::Color32::from_rgb(TRACK_COLOR[0], TRACK_COLOR[1], TRACK_COLOR[2]),
        palette.muted,
    )
    .to_array();
    let silent = solid_colors(state, clips[0], "arranger");
    assert!(silent.iter().all(|color| *color == endpoint));
    let quiet = solid_colors(state, clips[1], "arranger");
    assert!(
        quiet
            .iter()
            .all(|color| distance(*color, endpoint) < distance(*color, peak))
    );
    assert!(
        quiet
            .iter()
            .any(|color| *color != endpoint && *color != peak)
    );
    // Check the submitted quiet trace independently of its exact RGB recipe:
    // it remains readable on both canvas and raised surfaces and clearly less
    // saturated than the full-amplitude channel hue in both native themes.
    for color in silent.iter().chain(&quiet) {
        assert!(
            contrast(*color, palette.bg.to_array()) >= 3.0,
            "Quiet waveform {color:?} loses canvas contrast in dark={dark}"
        );
        assert!(
            contrast(*color, palette.raised.to_array()) >= 3.0,
            "Quiet waveform {color:?} loses raised-surface contrast in dark={dark}"
        );
        assert!(
            u16::from(chroma(*color)) * 2 < u16::from(chroma(TRACK_COLOR)),
            "Quiet waveform {color:?} must be more desaturated than {TRACK_COLOR:?}"
        );
    }
    readable_peak(&solid_colors(state, clips[2], "arranger"), dark);
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(state[field], before[field], "View controls changed {field}");
    }
    for field in ["generation", "analyses_started", "completed", "cache_bytes"] {
        assert_eq!(
            state["spectrogram"][field], before["spectrogram"][field],
            "View controls changed analysis {field}"
        );
    }
    assert_eq!(
        state["spectrogram"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| &source["texture_ids"])
            .collect::<Vec<_>>(),
        before["spectrogram"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| &source["texture_ids"])
            .collect::<Vec<_>>()
    );
}

fn assert_no_spectrum(state: &Value, before: &Value) {
    assert_eq!(state["spectrogram"]["view"]["enabled"], false);
    assert!(
        state["spectrogram"]["painted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let texture_ids: Vec<_> = before["spectrogram"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|source| source["texture_ids"].as_array().unwrap())
        .collect();
    assert!(
        state["painted_images"]
            .as_array()
            .unwrap()
            .iter()
            .all(|image| !texture_ids.contains(&&image["texture_id"])),
        "Disabled spectrum must submit no texture quads"
    );
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn amplitude_colors_and_view_toggles_preserve_cached_analysis_geometry_and_audio() {
    let Fixture {
        mut app,
        clips,
        sources,
    } = fixture();
    let baseline = paint(&mut app);
    let original_audio = export(&mut app, "Original.wav");
    assert_eq!(baseline["spectrogram"]["view"]["enabled"], true);
    assert_eq!(baseline["spectrogram"]["view"]["waveform_coloring"], true);
    for dark in [false, true] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        let enabled = paint(&mut app);
        colored(&enabled, clips, dark);
        // Fitting changes the displayed height, not the amplitude represented
        // by its color, including the quiet clip's non-unity saved gain.
        app.click("View");
        app.click("Fit");
        app.key("Escape");
        let fitted = paint(&mut app);
        assert_eq!(fitted["timeline"]["waveform_amplitude"], "Fit");
        let reference = preview(&fitted, clips[1], "arranger")["amplitude_reference"]
            .as_f64()
            .unwrap();
        assert!(
            reference > 0.0 && reference < 0.05,
            "Fit must actually normalize the quiet clip"
        );
        colored(&fitted, clips, dark);
        for clip in clips {
            assert_eq!(
                solid_colors(&fitted, clip, "arranger"),
                solid_colors(&enabled, clip, "arranger"),
                "Fit changed actual-amplitude colors for clip {clip}"
            );
        }
        unchanged(&fitted, &baseline);
        app.click("View");
        app.click("Default");
        app.key("Escape");
        let default = paint(&mut app);
        assert_eq!(default["timeline"]["waveform_amplitude"], "Default");
        colored(&default, clips, dark);
        unchanged(&default, &baseline);
        palette(&mut app, "Blue–yellow");
        intensity(&mut app, "63");
        toggle(&mut app, "Spectrum");
        let hidden = paint(&mut app);
        assert_no_spectrum(&hidden, &baseline);
        assert_eq!(hidden["spectrogram"]["view"]["palette"], "Blue–yellow");
        assert!((hidden["spectrogram"]["view"]["intensity"].as_f64().unwrap() - 0.63).abs() < 1e-5);
        colored(&hidden, clips, dark);
        unchanged(&hidden, &baseline);
        toggle(&mut app, "Waveform coloring");
        let flat = paint(&mut app);
        assert_no_spectrum(&flat, &baseline);
        assert_eq!(flat["spectrogram"]["view"]["waveform_coloring"], false);
        for clip in clips {
            readable_peak(&solid_colors(&flat, clip, "arranger"), dark);
            for field in ["vertices", "indices", "columns", "rect"] {
                assert_eq!(
                    preview(&flat, clip, "arranger")[field],
                    preview(&enabled, clip, "arranger")[field],
                    "Color mode changed geometry {field}"
                );
            }
        }
        unchanged(&flat, &baseline);
        let warm = paint(&mut app);
        assert_eq!(
            warm["waveform"]["geometry_builds"],
            flat["waveform"]["geometry_builds"]
        );
        assert!(
            warm["waveform"]["painted"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["cached"] == true && p["sample_reads"] == 0)
        );
        toggle(&mut app, "Spectrum");
        let visible = paint(&mut app);
        assert_eq!(visible["spectrogram"]["view"]["enabled"], true);
        assert_eq!(visible["spectrogram"]["view"]["waveform_coloring"], false);
        assert!(
            !visible["spectrogram"]["painted"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            visible["spectrogram"]["painted"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["palette"] == "Blue–yellow")
        );
        unchanged(&visible, &baseline);
        toggle(&mut app, "Waveform coloring");
        let restored = paint(&mut app);
        colored(&restored, clips, dark);
        unchanged(&restored, &baseline);
    }
    // The same View controls are available in the editor and share their state.
    app.click("Quiet cue");
    app.click("Window");
    app.click("Clip Editor");
    toggle(&mut app, "Spectrum");
    toggle(&mut app, "Waveform coloring");
    let editor = paint(&mut app);
    assert_eq!(editor["view"], "Clip");
    assert_no_spectrum(&editor, &baseline);
    readable_peak(
        &solid_colors(&editor, clips[1], "audio_editor"),
        editor["effective_theme"] == "Dark",
    );
    unchanged(&editor, &baseline);
    toggle(&mut app, "Spectrum");
    toggle(&mut app, "Waveform coloring");
    let restored = paint(&mut app);
    assert!(
        !restored["spectrogram"]["painted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    unchanged(&restored, &baseline);
    assert_eq!(
        export(&mut app, "Viewed.wav").samples,
        original_audio.samples
    );
    for (source, bytes) in sources {
        assert_eq!(std::fs::read(source).unwrap(), bytes);
    }
    app.quit();
}
