//! Mixer channels expose their inserts, outputs and sends without losing context.
mod common;

use common::Harness;
use daw_core::{Effect, EffectSlot, Project};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
struct Bounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Bounds {
    fn from(rect: &Value) -> Self {
        Self {
            left: rect["min"][0].as_f64().unwrap(),
            top: rect["min"][1].as_f64().unwrap(),
            right: rect["max"][0].as_f64().unwrap(),
            bottom: rect["max"][1].as_f64().unwrap(),
        }
    }

    fn width(self) -> f64 {
        self.right - self.left
    }

    fn height(self) -> f64 {
        self.bottom - self.top
    }

    fn contains(self, other: Self) -> bool {
        self.left <= other.left + 0.5
            && self.top <= other.top + 0.5
            && self.right >= other.right - 0.5
            && self.bottom >= other.bottom - 0.5
    }

    fn center(self) -> (f64, f64) {
        (
            (self.left + self.right) * 0.5,
            (self.top + self.bottom) * 0.5,
        )
    }
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn channel(state: &Value, track: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()
}

fn click_bounds(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = Bounds::from(rect).center();
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn close_window(app: &mut Harness, state: &Value, title: &str) -> Value {
    let title = Bounds::from(label(state, title));
    let window = state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(Bounds::from)
        .find(|window| window.contains(title))
        .unwrap();
    let (_, y) = title.center();
    let x = window.right - (y - window.top);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn unchanged(state: &Value, original: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(state[field], original[field], "Navigation changed {field}");
    }
}

#[test]
fn mixer_separates_channels_and_exposes_effects_outputs_and_sends_through_the_mouse() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let drums = project.add_audio_track("Drums");
        let bass = project.add_audio_track("Bass");
        let music = project.add_bus("Music");
        let room = project.add_bus("Room");
        for (track, color) in [
            (drums, [70, 145, 210]),
            (bass, [150, 90, 200]),
            (music, [230, 160, 70]),
            (room, [70, 180, 135]),
        ] {
            project.track_mut(track).unwrap().color = color;
        }
        project.set_output(drums, Some(music)).unwrap();
        project.set_output(bass, Some(music)).unwrap();
        project.add_send(drums, room, 0.25, true).unwrap();
        for name in ["Compressor", "Tape"] {
            project
                .track_mut(drums)
                .unwrap()
                .inserts
                .push(EffectSlot::new(
                    name,
                    Effect::Vst3 {
                        path: "/missing/mixer-readability.vst3".into(),
                        class_id: None,
                    },
                ));
        }
        let path = app.directory.path().join("mixer-readability.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":900,
            "pixels_per_point":density}));
        let original = app.click("Mixer");
        // Keep the mouse away from the channel actions while inspecting their
        // ordinary surfaces and colors, rather than their temporary hover state.
        app.command(json!({"command":"pointer", "phase":"move", "x":1090, "y":890}));
        let state = app.command(json!({"command":"inspect", "paint":true}));
        let channels = state["mixer"]["channels"].as_array().unwrap();
        assert_eq!(channels.len(), 4);

        let mut strips = Vec::new();
        for channel in channels {
            let column = Bounds::from(&channel["strip"]);
            let name = Bounds::from(&channel["name"]);
            let output = Bounds::from(&channel["target"]);
            let sends = Bounds::from(&channel["sends"]);
            assert!(
                sends.top >= output.bottom,
                "Send actions overlap main output"
            );
            assert!(sends.height() >= f64::from(daw_ui::tokens::CONTROL_HEIGHT) - 0.1);
            let strip = state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|paint| {
                    let rect = Bounds::from(&paint["rect"]);
                    (rect.left - column.left).abs() <= 0.1
                        && (column.right - rect.right).abs() <= 0.1
                        && rect.contains(name)
                        && rect.contains(sends)
                        && rect.height() > 250.0
                        && paint["stroke_width"].as_f64().unwrap() >= 1.0
                })
                .unwrap_or_else(|| panic!("No bounded strip for channel {channel}"));
            assert!(strip["stroke_width"].as_f64().unwrap() >= 1.0);
            let strip = Bounds::from(&strip["rect"]);
            for control in [name, output, sends] {
                assert!(control.left - strip.left >= f64::from(daw_ui::tokens::SPACE_MD) - 0.1);
                assert!(strip.right - control.right >= f64::from(daw_ui::tokens::SPACE_MD) - 0.1);
            }
            strips.push(strip);
        }
        assert!(
            strips
                .windows(2)
                .all(|pair| pair[1].left - pair[0].right
                    >= f64::from(daw_ui::tokens::SPACE_SM) - 0.1),
            "Each channel needs a visible gutter: {strips:?}"
        );
        assert!(
            strips[2].left - strips[1].right >= f64::from(daw_ui::tokens::SPACE_LG),
            "Bus group must have a wider separator than adjacent channels"
        );

        let drum_channel = channel(&state, drums);
        let summary = Bounds::from(&drum_channel["insert_summary"]);
        assert!(summary.contains(Bounds::from(label(&state, "Compressor"))));
        assert!(
            summary.contains(Bounds::from(label(&state, "+1"))),
            "Insert count outside summary: {summary:?}, {}",
            label(&state, "+1")
        );
        assert!(
            summary.top >= Bounds::from(&drum_channel["effects"]).bottom
                && summary.bottom <= Bounds::from(&drum_channel["gain"]).top,
            "Insert names must stay between the Effects action and gain controls"
        );
        assert_eq!(channel(&state, music)["incoming_count"], 2);
        assert_eq!(channel(&state, room)["incoming_count"], 1);
        assert!(
            Bounds::from(&channel(&state, music)["source"])
                .contains(Bounds::from(label(&state, "2 in")))
        );
        assert!(
            Bounds::from(&channel(&state, room)["source"])
                .contains(Bounds::from(label(&state, "1 in")))
        );
        let output = Bounds::from(&drum_channel["target"]);
        assert!(output.contains(Bounds::from(label(&state, "Ch 1 > Music"))));
        assert!(
            Bounds::from(&drum_channel["sends"])
                .contains(Bounds::from(label(&state, "Send · Room")))
        );
        assert!(
            state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|paint| {
                    let rail = Bounds::from(&paint["rect"]);
                    output.contains(rail)
                        && rail.width() <= 5.0
                        && rail.height() >= 12.0
                        && paint["fill"] == json!([230, 160, 70, 255])
                }),
            "Output must identify the destination bus with its track color"
        );
        unchanged(&state, &original);

        for control in ["target", "sends"] {
            let current = app.inspect();
            let routing = click_bounds(&mut app, &channel(&current, drums)[control]);
            assert_eq!(routing["popups"]["routing"], true);
            assert_eq!(routing["selected_track"], drums);
            label(&routing, "Routing of Drums");
            label(&routing, "Pre");
            unchanged(&routing, &original);
            let restored = close_window(&mut app, &routing, "Routing of Drums");
            assert_eq!(restored["view"], "Mixer");
            assert_eq!(restored["popups"]["routing"], false);
            unchanged(&restored, &original);
        }
        // An empty channel asks for its destination immediately, without
        // changing selection, opening another workspace, or creating a send.
        let current = app.inspect();
        let choices = click_bounds(&mut app, &channel(&current, bass)["sends"]);
        assert_eq!(choices["selected_track"], current["selected_track"]);
        assert_eq!(choices["popups"]["routing"], false);
        label(&choices, "Send from Bass");
        label(&choices, "New bus and send");
        unchanged(&choices, &original);
        // Cancel from empty mixer body beside the master, rather than from
        // an assumed header coordinate that may activate workspace navigation.
        let viewport = Bounds::from(&choices["mixer"]["reorder_viewport"]);
        let master = Bounds::from(&choices["mixer"]["master_strip"]);
        assert!(viewport.right - master.right > 8.0);
        let x = (master.right + viewport.right) * 0.5;
        let (_, y) = viewport.center();
        for area in choices["native_menu"]["foreground_areas"]
            .as_array()
            .unwrap()
        {
            assert!(
                x < area[0].as_f64().unwrap()
                    || x > area[2].as_f64().unwrap()
                    || y < area[1].as_f64().unwrap()
                    || y > area[3].as_f64().unwrap(),
                "Cancellation point ({x}, {y}) is inside the menu {area}"
            );
        }
        let restored = app.command(json!({"command":"pointer", "phase":"click",
            "x":x, "y":y}));
        assert_eq!(restored["view"], "Mixer");
        assert_eq!(restored["selected_track"], current["selected_track"]);
        assert!(
            !restored["menu_label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label["text"] == "New bus and send")
        );
        unchanged(&restored, &original);

        let effects = click_bounds(&mut app, &channel(&restored, drums)["effects"]);
        assert_eq!(effects["popups"]["effects"], true);
        label(&effects, "01  Compressor");
        label(&effects, "02  Tape");
        unchanged(&effects, &original);
        let restored = close_window(&mut app, &effects, "Effects of Drums");
        assert_eq!(restored["view"], "Mixer");
        unchanged(&restored, &original);
        app.quit();
    }
}
