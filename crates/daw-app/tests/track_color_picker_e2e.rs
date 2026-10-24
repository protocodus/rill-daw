//! User color edits propagate through actual arranger and mixer paint output.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use eframe::egui::Color32;
use serde_json::{Value, json};

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn strip(state: &Value, id: u64) -> &Value {
    state["timeline"]["track_colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|strip| strip["track"] == id)
        .unwrap()
}

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn center(rect: &Value) -> (f64, f64) {
    let [left, top, right, bottom] = bounds(rect);
    ((left + right) * 0.5, (top + bottom) * 0.5)
}

fn pointer(app: &mut Harness, rect: &Value, phase: &str) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":phase, "x":x, "y":y}))
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn rgb(state: &Value, id: u64) -> [u8; 3] {
    serde_json::from_value(track(state, id)["color"].clone()).unwrap()
}

fn fixture(app: &mut Harness) -> (u64, u64, u64, u64) {
    let source = app.directory.path().join("color-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Color audio");
    for (name, start) in [("Color take A", 0.0), ("Color take B", 2.0)] {
        project.track_mut(audio).unwrap().clips.push(Clip::audio(
            name,
            start,
            2.0,
            source.to_string_lossy(),
        ));
    }
    let folder = project.add_folder("Color folder");
    let midi = project.add_midi_track("Color MIDI");
    project.track_mut(midi).unwrap().collapsed = true;
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Color phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    project.set_parent_folder(midi, Some(folder)).unwrap();
    let bus = project.add_bus("Color bus");
    let path = app.directory.path().join("color-picker.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Color take A");
    (audio, midi, folder, bus)
}

fn choose_red(app: &mut Harness, id: u64, red: u8) -> (Value, Value) {
    let before = app.inspect();
    let [left, top, _, bottom] = bounds(&strip(&before, id)["rect"]);
    // Hit the visible three-point stripe itself, not merely its wider hit area.
    let open = app.command(json!({"command":"pointer", "phase":"click",
        "x":left+1.5, "y":(top+bottom)*0.5}));
    assert!(
        !strip(&open, id)["picker"].is_null(),
        "strip must open its picker"
    );
    assert_eq!(open["project"], before["project"]);
    assert_eq!(open["history"], before["history"]);
    assert_eq!(open["selected_track"], before["selected_track"]);
    assert_eq!(open["selected_clip"], before["selected_clip"]);
    assert_eq!(open["timeline"]["cursor"], before["timeline"]["cursor"]);
    assert_eq!(open["view"], "Arrangement");
    let title = format!("Color of {}", track(&before, id)["name"].as_str().unwrap());
    assert!(
        open["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == &title)
    );
    let label = format!("R {}", rgb(&before, id)[0]);
    let rect = open["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| {
            panic!(
                "Missing native RGB field {label}: {}",
                open["visible_labels"]
            )
        });
    pointer(app, rect, "double_click");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":red.to_string()}));
    app.key("Enter");
    let after = app.key("Escape");
    assert!(strip(&after, id)["picker"].is_null());
    assert_eq!(rgb(&after, id)[0], red);
    assert_eq!(&rgb(&after, id)[1..], &rgb(&before, id)[1..]);
    let mut expected = before["project"].clone();
    expected["tracks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|track| track["id"] == id)
        .unwrap()["color"] = track(&after, id)["color"].clone();
    assert_eq!(
        after["project"], expected,
        "Only this track's identity color changes"
    );
    assert_eq!(after["selected_track"], before["selected_track"]);
    assert_eq!(after["selected_clip"], before["selected_clip"]);
    assert_eq!(after["timeline"]["cursor"], before["timeline"]["cursor"]);
    (before, after)
}

fn assert_arranger_paint(state: &Value, id: u64) {
    let color = rgb(state, id);
    let color = Color32::from_rgb(color[0], color[1], color[2]);
    let marker = bounds(&strip(state, id)["rect"]);
    let shapes = state["painted_rects"].as_array().unwrap();
    assert!(
        shapes.iter().any(|shape| {
            let rect = bounds(&shape["rect"]);
            shape["fill"] == json!(color.to_array())
                && rect[0] >= marker[0] - 0.1
                && rect[2] <= marker[2] + 0.1
                && rect[1] <= marker[1] + 0.1
                && rect[3] >= marker[3] - 0.1
        }),
        "arranger stripe must actually paint the chosen color"
    );
    for clip in state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|clip| clip["track"] == id)
    {
        let rect = bounds(&clip["rect"]);
        let selected = state["selected_clip"] == clip["id"];
        let stroke = color
            .gamma_multiply(if selected { 1.0 } else { 0.65 })
            .to_array();
        assert!(
            shapes.iter().any(|shape| {
                bounds(&shape["rect"])
                    .into_iter()
                    .zip(rect)
                    .all(|(a, b)| (a - b).abs() < 0.1)
                    && shape["stroke"] == json!(stroke)
                    && shape["stroke_width"].as_f64().unwrap() > 0.0
            }),
            "clip {} must repaint from its track color",
            clip["id"]
        );
    }
}

fn mixer_body<'a>(state: &'a Value, name: &str) -> &'a Value {
    let name = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing mixer channel {name}"));
    let (x, y) = center(name);
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| {
            let [left, top, right, bottom] = bounds(&shape["rect"]);
            left < x
                && right > x
                && top < y
                && bottom > y
                && (right - left - 180.0).abs() < 0.1
                && bottom - top > 250.0
        })
        .unwrap_or_else(|| panic!("Missing full-height mixer strip for {name:?}"))
}

fn assert_mixer_paint(app: &mut Harness, id: u64) -> Value {
    app.click("Mixer");
    let state = painted(app);
    assert_eq!(state["view"], "Mixer");
    let palette = if state["effective_theme"] == "Dark" {
        daw_ui::theme::DARK
    } else {
        daw_ui::theme::LIGHT
    };
    let shapes = state["painted_rects"].as_array().unwrap();
    for channel in state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| matches!(track["kind"].as_str(), Some("Audio" | "Midi" | "Bus")))
    {
        let name = channel["name"].as_str().unwrap();
        let label = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == name)
            .unwrap();
        let (x, y) = center(label);
        let color = rgb(&state, channel["id"].as_u64().unwrap());
        let color = json!([color[0], color[1], color[2], 255]);
        assert!(
            shapes.iter().any(|shape| {
                let [left, top, right, bottom] = bounds(&shape["rect"]);
                shape["fill"] == color
                    && left < x
                    && right > x
                    && top < y
                    && bottom > y
                    && bottom - top >= 33.0
                    && right - left > 140.0
            }),
            "{name}: the whole channel-name header must keep the arranger RGB color"
        );
        assert!(
            shapes.iter().any(|shape| {
                let [left, top, right, bottom] = bounds(&shape["rect"]);
                shape["fill"] == color
                    && left < x
                    && right > x
                    && top < y
                    && y - top < 65.0
                    && (bottom - top - 2.0).abs() < 0.1
                    && right - left > 140.0
            }),
            "{name}: the channel marker must keep the arranger RGB color"
        );
        let fill = &mixer_body(&state, name)["fill"];
        assert_ne!(fill, &color, "{name}: the full strip uses a subdued tint");
        assert_ne!(
            fill,
            &json!(palette.panel.to_array()),
            "{name}: tint is visible on unselected channels"
        );
        assert_ne!(
            fill,
            &json!(palette.track_selected.to_array()),
            "{name}: selected channels retain their own color"
        );
    }
    assert_eq!(
        mixer_body(&state, "Master")["fill"],
        json!(palette.panel.to_array()),
        "Master retains its neutral surface"
    );
    let fill = mixer_body(&state, track(&state, id)["name"].as_str().unwrap())["fill"].clone();
    app.click("Arranger");
    fill
}

#[test]
fn arranger_color_picker_updates_clip_and_mixer_paint_with_undo_and_recall() {
    for (dark, density) in [(false, 1.0), (true, 1.0), (false, 2.0), (true, 2.0)] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        let (audio, _, _, _) = fixture(&mut app);
        let original_tint = assert_mixer_paint(&mut app, audio);
        let (before, changed) = choose_red(&mut app, audio, 213);
        assert_arranger_paint(&painted(&mut app), audio);
        let changed_tint = assert_mixer_paint(&mut app, audio);
        assert_ne!(
            changed_tint, original_tint,
            "The strip tint follows color edits"
        );
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(
            undone["project"], before["project"],
            "One undo restores a complete color edit"
        );
        assert_arranger_paint(&painted(&mut app), audio);
        assert_eq!(assert_mixer_paint(&mut app, audio), original_tint);
        let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
        assert_eq!(redone["project"], changed["project"]);
        assert_eq!(assert_mixer_paint(&mut app, audio), changed_tint);
        let saved = app.directory.path().join("chosen-colors.rill");
        app.command(json!({"command":"save", "path":saved}));
        app = app.restart();
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        let loaded = app.command(json!({"command":"open", "path":saved}));
        assert_eq!(rgb(&loaded, audio), rgb(&changed, audio));
        assert_arranger_paint(&painted(&mut app), audio);
        assert_eq!(assert_mixer_paint(&mut app, audio), changed_tint);
        app.quit();
    }
}

#[test]
fn compact_and_folder_color_strips_do_not_select_tracks_or_collapse_folders() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1100, "height":720}));
    let (_, midi, folder, bus) = fixture(&mut app);
    for (id, red) in [(midi, 227), (folder, 71), (bus, 19)] {
        let (_, changed) = choose_red(&mut app, id, red);
        assert_eq!(track(&changed, midi)["collapsed"], true);
        assert_eq!(track(&changed, folder)["collapsed"], false);
        assert_arranger_paint(&painted(&mut app), id);
        if id != folder {
            assert_mixer_paint(&mut app, id);
        }
    }
    app.quit();
}
