//! Visible zoom/selection framing uses real mouse controls in all music editors.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let midi = project.add_midi_track("Zoom synth");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Zoom phrase",
        16.25,
        8.5,
        vec![
            Note::new(60, 0.375, 0.625, 90),
            Note::new(72, 6.125, 0.75, 100),
        ],
    ));
    let source = app.directory.path().join("zoom.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Zoom audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Zoom recording",
        0.375,
        8.25,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("zoom.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn zoom_button(state: &Value, symbol: &str) -> Value {
    let fit = label(state, "Fit");
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| {
            rect["text"] == symbol
                && (center(rect).1 - center(fit).1).abs() < 4.0
                && center(rect).0 < center(fit).0
        })
        .max_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .unwrap_or_else(|| panic!("Missing zoom {symbol}: {}", state["visible_labels"]))
        .clone()
}

fn zoom(app: &mut Harness, symbol: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(&zoom_button(&state, symbol));
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn framing(state: &Value, key: &str) -> Value {
    let view = &state[key];
    json!([
        view["start_beat"],
        view["pixels_per_beat"],
        view["row_offset"],
        view["row_height"]
    ])
}

fn canvas(state: &Value, key: &str) -> [f64; 4] {
    if key == "audio_editor" {
        return std::array::from_fn(|i| state[key]["rect"][i].as_f64().unwrap());
    }
    let rect = &state[key][if key == "timeline" { "canvas" } else { "plot" }];
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn unchanged(state: &Value, before: &Value, key: &str) {
    for field in [
        "project",
        "history",
        "modified",
        "revision",
        "selected_track",
        "selected_clip",
        "selected_notes",
    ] {
        assert_eq!(state[field], before[field], "View action changed {field}");
    }
    if key == "audio_editor" {
        assert_eq!(state[key]["range"], before[key]["range"]);
    }
    if key == "midi_editor" {
        let selected = |s: &Value| {
            s[key]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|note| note["selected"] == true)
                .map(|note| note["id"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(selected(state), selected(before));
    }
}

fn previous(app: &mut Harness) -> Value {
    app.click("View");
    app.click("Previous view")
}

fn open_editor(app: &mut Harness, clip: &str) {
    app.click("Arranger");
    app.click("Fit");
    app.click(clip);
    app.click("Window");
    app.click("Clip Editor");
}

fn controls(app: &mut Harness, key: &str, width: u32) {
    let before = app.click("Fit");
    let fit = label(&before, "Fit");
    let selection = label(&before, "Fit selection");
    let plus = zoom_button(&before, "+");
    let minus = zoom_button(&before, "−");
    for rect in [fit, selection, &plus, &minus] {
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
        assert!(rect["max"][0].as_f64().unwrap() <= f64::from(width));
        assert!(
            (center(rect).1 - center(fit).1).abs() < 4.0,
            "Zoom group must wrap together"
        );
    }
    assert!(minus["max"][0].as_f64().unwrap() < plus["min"][0].as_f64().unwrap());
    assert!(plus["max"][0].as_f64().unwrap() < fit["min"][0].as_f64().unwrap());
    assert!(fit["max"][0].as_f64().unwrap() < selection["min"][0].as_f64().unwrap());
    let rect = canvas(&before, key);
    let scale = before[key]["pixels_per_beat"].as_f64().unwrap();
    let middle = before[key]["start_beat"].as_f64().unwrap() + (rect[2] - rect[0]) * 0.5 / scale;
    let enlarged = zoom(app, "+");
    let next_scale = enlarged[key]["pixels_per_beat"].as_f64().unwrap();
    assert!(next_scale > scale);
    let next_rect = canvas(&enlarged, key);
    let next_middle = enlarged[key]["start_beat"].as_f64().unwrap()
        + (next_rect[2] - next_rect[0]) * 0.5 / next_scale;
    assert!(
        (middle - next_middle).abs() < 1e-4,
        "Center must stay anchored"
    );
    assert_eq!(before[key]["row_height"], enlarged[key]["row_height"]);
    assert_eq!(center(label(&enlarged, "Fit")), center(fit));
    unchanged(&enlarged, &before, key);
    let restored = previous(app);
    assert_eq!(framing(&restored, key), framing(&before, key));
    unchanged(&restored, &before, key);
    zoom(app, "+");
    let reduced = zoom(app, "−");
    assert!((reduced[key]["pixels_per_beat"].as_f64().unwrap() - scale).abs() < 1e-3);
    unchanged(&reduced, &before, key);
    assert_eq!(framing(&app.click("Fit"), key), framing(&before, key));
}

#[test]
fn visible_zoom_controls_wrap_together_and_preserve_music_in_every_editor() {
    let mut app = fixture();
    for width in [800, 1100, 1280, 1440] {
        for (dark, density) in [(false, 1.0), (true, 2.0)] {
            app.command(json!({"command":"viewport","width":width,"height":1000,"pixels_per_point":density}));
            app.command(json!({"command":"system_theme","dark":dark}));
            app.click("Arranger");
            controls(&mut app, "timeline", width);
            open_editor(&mut app, "Zoom phrase");
            controls(&mut app, "midi_editor", width);
            open_editor(&mut app, "Zoom recording");
            controls(&mut app, "audio_editor", width);
        }
    }
    app.quit();
}

fn framed_range(state: &Value, key: &str, start: f64, end: f64) {
    let rect = canvas(state, key);
    let left = state[key]["start_beat"].as_f64().unwrap();
    let right = left + (rect[2] - rect[0]) / state[key]["pixels_per_beat"].as_f64().unwrap();
    assert!(
        left <= start + 1e-5 && right >= end - 1e-5,
        "Selection must be completely visible"
    );
    assert!(
        right - left < (end - start).max(0.25) * 1.2,
        "Selection must be closely framed"
    );
}

#[test]
fn visible_fit_selection_frames_fractional_clips_notes_and_audio_ranges_without_editing() {
    let mut app = fixture();
    app.click("Fit");
    let before = app.click("Zoom phrase");
    let fitted = app.click("Fit selection");
    framed_range(&fitted, "timeline", 16.25, 24.75);
    unchanged(&fitted, &before, "timeline");
    assert_eq!(
        framing(&previous(&mut app), "timeline"),
        framing(&before, "timeline")
    );
    open_editor(&mut app, "Zoom phrase");
    let empty_selection = app.click("Fit");
    let disabled = app.click("Fit selection");
    assert_eq!(
        framing(&disabled, "midi_editor"),
        framing(&empty_selection, "midi_editor")
    );
    unchanged(&disabled, &empty_selection, "midi_editor");
    let note = &disabled["midi_editor"]["notes"][0]["rect"];
    let (x, y) = center(note);
    let before = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let fitted = app.click("Fit selection");
    framed_range(&fitted, "midi_editor", 0.375, 1.0);
    unchanged(&fitted, &before, "midi_editor");
    assert_eq!(
        framing(&previous(&mut app), "midi_editor"),
        framing(&before, "midi_editor")
    );
    open_editor(&mut app, "Zoom recording");
    let state = app.click("Fit");
    let rect = canvas(&state, "audio_editor");
    let at = |beat: f64| {
        rect[0]
            + (beat - state["audio_editor"]["start_beat"].as_f64().unwrap())
                * state["audio_editor"]["pixels_per_beat"].as_f64().unwrap()
    };
    let y = (rect[1] + rect[3]) * 0.5;
    let before = app.command(
        json!({"command":"pointer","phase":"drag","x":at(1.25),"y":y,"to_x":at(3.5),"to_y":y}),
    );
    assert_eq!(before["audio_editor"]["range"], json!([1.25, 3.5]));
    let fitted = app.click("Fit selection");
    framed_range(&fitted, "audio_editor", 1.25, 3.5);
    unchanged(&fitted, &before, "audio_editor");
    assert_eq!(
        framing(&previous(&mut app), "audio_editor"),
        framing(&before, "audio_editor")
    );
    app.quit();
}

#[test]
fn zoom_buttons_respect_viewport_limits_and_origin_without_touching_selection() {
    let mut app = fixture();
    for key in ["timeline", "midi_editor", "audio_editor"] {
        if key == "midi_editor" {
            open_editor(&mut app, "Zoom phrase");
        }
        if key == "audio_editor" {
            open_editor(&mut app, "Zoom recording");
        }
        let before = app.click("Fit");
        for _ in 0..35 {
            zoom(&mut app, "+");
        }
        let bounded = app.inspect();
        assert_eq!(bounded[key]["pixels_per_beat"], 4096.0);
        assert_eq!(framing(&zoom(&mut app, "+"), key), framing(&bounded, key));
        for _ in 0..35 {
            zoom(&mut app, "−");
        }
        let wide = app.inspect();
        assert_eq!(wide[key]["start_beat"], 0.0);
        assert!(wide[key]["pixels_per_beat"].as_f64().unwrap() > 0.0);
        unchanged(&wide, &before, key);
    }
    app.quit();
}

#[test]
fn one_bar_snap_uses_the_current_meter_and_keeps_old_beat_steps_honestly_labelled() {
    let mut app = fixture();
    app.click("4/4");
    app.click("3/4");
    let before = app.inspect();
    app.click("Snap 1/16");
    let chosen = app.click("1 bar");
    label(&chosen, "Snap 1 bar");
    unchanged(&chosen, &before, "timeline");
    let state = app.click("Fit");
    app.click("Zoom phrase");
    let clip = &state["timeline"]["clips"][0]["rect"];
    let (x, y) = center(clip);
    let scale = state["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let moved = app
        .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+scale,"to_y":y}));
    assert_eq!(moved["project"]["tracks"][0]["clips"][0]["start"], 18.0);
    app.click("Edit");
    assert_eq!(app.click("Undo")["project"], before["project"]);
    app.click("3/4");
    let changed = app.click("4/4");
    label(&changed, "Snap 3 beats");
    app.click("Snap 3 beats");
    label(&app.click("1 bar"), "Snap 1 bar");
    app.quit();
}
