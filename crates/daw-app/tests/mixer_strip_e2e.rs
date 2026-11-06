//! Channel strips read in signal order: bus identities and controls start below
//! their role heading, while the fader bottoms, outputs and sends remain aligned.
//! Each strip centers its calibrated fader/meter unit on its own channel axis.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn edge(rect: &Value, corner: &str, axis: usize) -> f64 {
    rect[corner][axis].as_f64().unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (edge(rect, "min", 0) + edge(rect, "max", 0)) * 0.5,
        (edge(rect, "min", 1) + edge(rect, "max", 1)) * 0.5,
    )
}

fn click(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn inside(state: &Value, text: &str, rect: &Value) -> bool {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| {
            label["text"] == text
                && edge(label, "min", 0) >= edge(rect, "min", 0) - 0.5
                && edge(label, "max", 0) <= edge(rect, "max", 0) + 0.5
                && edge(label, "min", 1) >= edge(rect, "min", 1) - 0.5
                && edge(label, "max", 1) <= edge(rect, "max", 1) + 0.5
        })
}

#[test]
fn strips_align_sections_center_the_fader_unit_and_list_sends_compactly() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Source");
        project.track_mut(source).unwrap().gain = 0.0;
        let quiet = project.add_audio_track("Quiet");
        let mut buses = Vec::new();
        for name in ["Room", "Plate", "Delay"] {
            buses.push(project.add_bus(name));
        }
        project.add_send(source, buses[0], 0.5, true).unwrap();
        project.add_send(source, buses[1], 0.25, false).unwrap();
        project.add_send(source, buses[2], 0.25, false).unwrap();
        project.add_send(quiet, buses[0], 0.5, true).unwrap();
        project.set_output(buses[1], Some(buses[0])).unwrap();
        let path = app.directory.path().join("mixer-strip.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1440, "height":1000,
            "pixels_per_point":density}));
        let original = app.click("Mixer");
        let state = app.command(json!({"command":"inspect", "paint":true}));
        let channels = state["mixer"]["channels"].as_array().unwrap();
        assert_eq!(channels.len(), 5);
        let master = &state["mixer"]["master"];

        // Buses reserve their role heading above identity and controls. Their
        // shorter faders retain the same bottom edge and output/send positions.
        for channel in channels {
            let offset = if channel["bus"] == true {
                f64::from(daw_ui::MIXER_ROLE_HEIGHT)
            } else {
                0.0
            };
            let sections = channel["sections"].as_array().unwrap();
            let first = channels[0]["sections"].as_array().unwrap();
            for (section, reference) in sections.iter().zip(first) {
                assert_eq!(section["name"], reference["name"]);
                for corner in ["min", "max"] {
                    let expected_offset = match (section["name"].as_str().unwrap(), corner) {
                        ("Source" | "Effects", _) | ("Volume & pan", "min") => offset,
                        _ => 0.0,
                    };
                    assert_eq!(
                        edge(&section["rect"], corner, 1),
                        edge(&reference["rect"], corner, 1) + expected_offset,
                        "{}",
                        section["name"]
                    );
                }
            }
            // The [scale][rail][gap][meter] unit is centered on the strip axis.
            let strip = &channel["strip"];
            // The fader response is its rail; the scale gutter lies left of it.
            let unit_left =
                edge(&channel["fader"], "min", 0) - f64::from(daw_ui::tokens::MIXER_SCALE_GUTTER);
            let unit_right = edge(&channel["meter"], "max", 0);
            let axis = (edge(strip, "min", 0) + edge(strip, "max", 0)) * 0.5;
            assert!(
                ((unit_left + unit_right) * 0.5 - axis).abs() <= 1.0,
                "fader unit off the strip axis: {channel}"
            );
            assert_eq!(
                edge(&channel["meter"], "min", 1),
                edge(&master["meter"], "min", 1) + offset
            );
            assert_eq!(channel["meter"]["max"][1], master["meter"]["max"][1]);
            assert_eq!(channel["target"]["min"][1], master["target"]["min"][1]);
        }

        // Silence reads as −∞ dB; the source names its real input.
        assert!(inside(&state, "−∞ dB", &channels[0]["gain"]));
        assert!(inside(&state, "Audio · In 1", &channels[0]["source"]));
        // Master sums the channels whose output is Master (all but Plate).
        assert!(inside(&state, "Sum · 4 in", &master["source"]));

        // Up to two send rows; more collapse into "+N more". Pre-fader sends
        // carry a PRE mark before their level in dB.
        let rows = channels[0]["send_rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(inside(&state, "Room", &rows[0]));
        assert!(inside(&state, "PRE −6.0", &rows[0]));
        assert!(inside(&state, "+2 more", &channels[0]["sends"]));
        let quiet_rows = channels[1]["send_rows"].as_array().unwrap();
        assert_eq!(quiet_rows.len(), 1);
        assert_eq!(
            channels[0]["add_send"]["min"][1], channels[1]["add_send"]["min"][1],
            "the send area is reserved on every strip"
        );
        // Outputs are named by destination only.
        assert!(inside(&state, "Master", &channels[0]["target"]));
        let plate = channels
            .iter()
            .find(|channel| channel["track"] == buses[1])
            .unwrap();
        assert!(inside(&state, "Room", &plate["target"]));
        assert!(
            !state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label.as_str().unwrap().starts_with("Ch "))
        );

        let more = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "+2 more")
            .unwrap()
            .clone();
        let routing = click(&mut app, &more);
        assert_eq!(routing["popups"]["routing"], true);
        assert_eq!(routing["selected_track"], source);
        assert_eq!(routing["project"], original["project"]);
        app.key("Escape");

        // Master's output opens the audio device settings.
        let mixer = app.inspect();
        let setup = click(&mut app, &mixer["mixer"]["master"]["target"]);
        assert_eq!(setup["popups"]["setup"], true);
        assert!(visible(&setup, "Audio"));
        assert_eq!(setup["project"], original["project"]);
        app.key("Escape");

        // An empty chain's "+ Insert" row opens the effect library for its channel.
        let mixer = app.inspect();
        let library = click(&mut app, &mixer["mixer"]["channels"][1]["insert_summary"]);
        assert_eq!(library["popups"]["library"], true);
        assert_eq!(library["project"], original["project"]);
        app.key("Escape");
        app.quit();
    }
}
