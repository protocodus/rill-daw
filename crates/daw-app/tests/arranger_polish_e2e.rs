//! Arrangement polish: toolbar order, the loop label beside the playhead,
//! Mute/Solo colors, compact folder rows and readable MIDI previews.
mod common;

use common::Harness;
use daw_ui::tokens;
use eframe::egui::{self, Color32};
use serde_json::{Value, json};

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn center_y(rect: &Value) -> f64 {
    let [_, top, _, bottom] = bounds(rect);
    (top + bottom) * 0.5
}

fn fixture(app: &mut Harness) -> (u64, u64, u64) {
    let mut project = daw_core::Project::new();
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 16.0;
    let folder = project.add_folder("Band");
    let midi = project.add_midi_track("Keys");
    project.set_parent_folder(midi, Some(folder)).unwrap();
    project
        .track_mut(midi)
        .unwrap()
        .clips
        .push(daw_core::Clip::midi(
            "Two notes",
            0.0,
            8.0,
            vec![
                daw_core::Note::new(60, 0.0, 1.0, 90),
                daw_core::Note::new(62, 2.0, 1.0, 90),
            ],
        ));
    let audio = project.add_audio_track("Voice");
    let path = app.directory.path().join("polish.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    (folder, midi, audio)
}

fn text_width(text: &str, font: egui::FontId) -> f32 {
    let ctx = egui::Context::default();
    daw_ui::theme::apply(&ctx);
    let mut width = 0.0;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        width = ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        });
    });
    width
}

#[test]
fn toolbar_follows_the_workflow_with_snap_and_nudge_beside_the_tools() {
    let mut app = Harness::new();
    fixture(&mut app);
    for width in [1280, 1480] {
        app.command(
            json!({"command":"viewport","width":width,"height":800,"pixels_per_point":1.0}),
        );
        let state = app.command(json!({"command":"inspect","paint":true}));
        let row = center_y(label(&state, "Select"));
        let x = |name: &str| bounds(label(&state, name))[0];
        for name in ["Slip", "Snap 1/16", "Nudge"] {
            assert!(
                (center_y(label(&state, name)) - row).abs() <= 2.0,
                "{name} shares the tools' row at {width}"
            );
        }
        assert!(x("Slip") < x("Snap 1/16") && x("Snap 1/16") < x("Nudge"));
        // Add pattern follows in reading order, possibly wrapped to the next row.
        let add_row = center_y(label(&state, "Add pattern"));
        assert!(
            (add_row - row).abs() <= 2.0 && x("Nudge") < x("Add pattern") || add_row > row + 2.0
        );
        // View controls form one right-aligned cluster: zoom, View, Tool help.
        let fit = bounds(label(&state, "Fit"));
        let view = bounds(label(&state, "View"));
        let help = bounds(label(&state, "Tool help"));
        assert!(fit[2] <= view[0] && view[2] <= help[0]);
        assert!(
            (center_y(label(&state, "Fit")) - center_y(label(&state, "Tool help"))).abs() <= 2.0
        );
        assert!(help[2] <= f64::from(width));
    }
    // Snap still changes through the visible chooser.
    app.click("Snap 1/16");
    let chosen = app.click("1/8");
    label(&chosen, "Snap 1/8");
    app.quit();
}

#[test]
fn loop_label_uses_the_caption_font_and_stays_clear_of_the_playhead() {
    let mut app = Harness::new();
    fixture(&mut app);
    app.command(json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":1.0}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    let timeline = &state["timeline"];
    let indicator = bounds(&timeline["loop_indicator"]);
    let x_at = |timeline: &Value, beat: f64| {
        timeline["canvas"]["min"][0].as_f64().unwrap()
            + (beat - timeline["start_beat"].as_f64().unwrap())
                * timeline["pixels_per_beat"].as_f64().unwrap()
    };
    let origin = x_at(timeline, 0.0);
    let header = timeline["header_width"].as_f64().unwrap();
    assert!(
        origin >= header + f64::from(tokens::SPACE_SM) - 0.5,
        "beat 0 keeps a gutter from the header edge"
    );
    let text = "1.1 – 5.1 · 0:08";
    let painted = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| {
            shape["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("1.1 – 5.1 · "))
        })
        .unwrap_or_else(|| panic!("loop label: {}", state["visible_labels"]));
    let rect = bounds(&painted["rect"]);
    let full = painted["text"].as_str().unwrap();
    assert!(full.starts_with(text));
    let expected = text_width(full, egui::FontId::proportional(tokens::FONT_CAPTION));
    assert!(
        (rect[2] - rect[0] - f64::from(expected)).abs() < 1.0,
        "loop label uses the proportional caption font"
    );
    assert!(rect[1] >= indicator[1] && rect[3] <= indicator[3]);
    // Playhead at beat 0: the label starts past it.
    assert!(rect[0] >= origin + f64::from(tokens::SPACE_SM) - 0.5);
    // Move the playhead into the label; the label steps past it.
    let ruler_y = center_y(&timeline["ruler"]);
    let target = (rect[0] + rect[2]) * 0.5;
    let moved = app.command(json!({"command":"pointer","phase":"click","x":target,"y":ruler_y}));
    let cursor = moved["timeline"]["cursor"].as_f64().unwrap();
    let playhead = x_at(&moved["timeline"], cursor);
    let state = app.command(json!({"command":"inspect","paint":true}));
    let label = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| {
            shape["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("1.1 – 5.1 · "))
        })
        .unwrap();
    let rect = bounds(&label["rect"]);
    assert!(
        rect[0] >= playhead + f64::from(tokens::SPACE_SM) - 0.5 || rect[2] <= playhead,
        "playhead {playhead} covers the loop label {rect:?}"
    );
    app.quit();
}

#[test]
fn mute_and_solo_use_their_own_colors_and_folders_paint_one_compact_row() {
    let mut app = Harness::new();
    let (folder, midi, _) = fixture(&mut app);
    app.command(json!({"command":"viewport","width":1280,"height":900,"pixels_per_point":1.0}));
    app.command(json!({"command":"system_theme","dark":true}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    let channel = state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == midi)
        .unwrap()
        .clone();
    let rail = center_y(&channel["gain"]);
    for (key, color) in [
        ("mute", daw_ui::theme::DARK.mute),
        ("solo", daw_ui::theme::DARK.solo),
    ] {
        let rect = bounds(&channel[key]);
        assert!((rect[2] - rect[0] - f64::from(tokens::COMPACT_HEIGHT)).abs() < 0.5);
        assert!((rect[3] - rect[1] - f64::from(tokens::COMPACT_HEIGHT)).abs() < 0.5);
        assert!(
            (center_y(&channel[key]) - rail).abs() <= 1.5,
            "{key} sits on the rail line"
        );
        let x = (rect[0] + rect[2]) * 0.5;
        let y = (rect[1] + rect[3]) * 0.5;
        let clicked = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        let track = clicked["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == midi)
            .unwrap();
        assert_eq!(track[key], true);
        let painted = app.command(json!({"command":"inspect","paint":true}));
        assert!(
            painted["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| {
                    shape["fill"] == json!(color.to_array()) && {
                        let b = bounds(&shape["rect"]);
                        (b[0] - rect[0]).abs() < 1.0 && (b[3] - rect[3]).abs() < 1.0
                    }
                }),
            "active {key} is filled with its palette color"
        );
        app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    }
    let tracks = state["timeline"]["tracks"].as_array().unwrap();
    let row = tracks
        .iter()
        .find(|track| track["track"] == folder)
        .unwrap();
    let [_, top, _, bottom] = bounds(&row["header"]);
    assert!(
        (bottom - top - f64::from(daw_ui::TRACK_HEADER_FOLDER_HEIGHT)).abs() < 1.0,
        "folder rows show only their identity line"
    );
    assert!(row["sections"]["mix"].is_null());
    // Folder Mute shares the channel control on the identity line.
    let folder_mute = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| (text["text"].clone(), text["rect"].clone()))
        .find(|(text, rect)| text == "M" && center_y(rect) > top && center_y(rect) < bottom)
        .expect("folder Mute on its identity line")
        .1;
    let [l, t, r, b] = bounds(&folder_mute);
    let muted =
        app.command(json!({"command":"pointer","phase":"click","x":(l+r)*0.5,"y":(t+b)*0.5}));
    let folder_track = muted["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == folder)
        .unwrap();
    assert_eq!(folder_track["mute"], true);
    app.quit();
}

#[test]
fn midi_previews_paint_readable_track_colored_notes() {
    let mut app = Harness::new();
    let (_, midi, _) = fixture(&mut app);
    app.command(json!({"command":"viewport","width":1280,"height":900,"pixels_per_point":1.0}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    let clip = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["track"] == midi)
        .unwrap();
    let content = bounds(&clip["content_rect"]);
    let track = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == midi)
        .unwrap();
    let rgb: Vec<u8> = track["color"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect();
    let color = json!(Color32::from_rgb(rgb[0], rgb[1], rgb[2]).to_array());
    let notes: Vec<_> = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| {
            let b = bounds(&shape["rect"]);
            shape["fill"] == color
                && b[0] >= content[0] - 0.5
                && b[2] <= content[2] + 0.5
                && b[1] >= content[1] - 0.5
                && b[3] <= content[3] + 0.5
        })
        .collect();
    assert_eq!(notes.len(), 2, "each note paints once in full track color");
    for note in notes {
        let b = bounds(&note["rect"]);
        let height = b[3] - b[1];
        assert!((2.0..=6.01).contains(&height), "note height {height}");
        assert!(
            note["stroke_width"].as_f64().unwrap() >= 1.0,
            "notes keep a darker outline"
        );
    }
    // Title text and preview share one left inset.
    assert!((bounds(&clip["title_rect"])[0] - content[0]).abs() < 0.5);
    app.quit();
}
