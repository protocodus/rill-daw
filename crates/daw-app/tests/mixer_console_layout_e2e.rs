//! A shared console grid keeps every channel role usable on laptop displays.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationTarget, Project};
use serde_json::{Value, json};

fn edge(rect: &Value, corner: &str, axis: usize) -> f64 {
    rect[corner][axis].as_f64().unwrap()
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        edge(inner, "min", axis) >= edge(outer, "min", axis) - 0.6
            && edge(inner, "max", axis) <= edge(outer, "max", axis) + 0.6
    })
}

fn click(app: &mut Harness, rect: &Value) -> Value {
    app.command(json!({"command":"pointer","phase":"click",
        "x":(edge(rect,"min",0)+edge(rect,"max",0))*0.5,
        "y":(edge(rect,"min",1)+edge(rect,"max",1))*0.5}))
}

fn painted_name(state: &Value, text: &str, region: &Value) -> bool {
    state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label["text"] == text && contains(region, &label["rect"]))
}

#[test]
fn laptop_console_keeps_channel_roles_aligned_and_all_primary_actions_visible() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Lead vocal");
        let keys = project.add_midi_track("Keys");
        let music = project.add_bus("Music bus");
        let reverb = project.add_bus("Reverb return");
        project.set_output(source, Some(music)).unwrap();
        project.set_output(keys, Some(music)).unwrap();
        project.add_send(source, reverb, 0.25, false).unwrap();
        for id in [source, keys, music, reverb] {
            project.track_mut(id).unwrap().gain = 1.0;
        }
        let pan = project
            .add_automation_track(
                "Pan",
                AutomationTarget {
                    track: keys,
                    parameter: AutomationParameter::TrackPan,
                },
            )
            .unwrap();
        project.automation_lane_mut(pan).unwrap().override_value = Some(0.75);
        let path = app.directory.path().join("console-layout.rill");
        project.save(&path).unwrap();
        let original = app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        for (width, height, compact) in [(1280.0, 800.0, true), (1440.0, 1000.0, false)] {
            let before_view = app.inspect();
            app.command(json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}));
            app.click("Mixer");
            let state = app.command(json!({"command":"inspect","paint":true}));
            assert_eq!(state["project"], original["project"]);
            assert_eq!(state["history"], before_view["history"]);
            assert_eq!(state["mixer"]["order"], "Song order");
            assert_eq!(state["mixer"]["compact"], compact);
            let pane = &state["mixer"]["reorder_viewport"];
            let channels = state["mixer"]["channels"].as_array().unwrap();
            assert_eq!(
                channels
                    .iter()
                    .map(|channel| channel["track"].as_u64().unwrap())
                    .collect::<Vec<_>>(),
                [source, keys, music, reverb]
            );
            let first = &channels[0];
            for channel in channels {
                for key in [
                    "name",
                    "source",
                    "effects",
                    "analyze",
                    "insert_summary",
                    "pan",
                    "mute",
                    "solo",
                    "gain",
                    "fader",
                    "meter",
                    "target",
                    "add_send",
                ] {
                    assert!(
                        contains(pane, &channel[key]),
                        "{key} must be mouse-reachable in {width}×{height}: {} outside {pane}",
                        channel[key]
                    );
                }
                for key in [
                    "effects",
                    "insert_summary",
                    "pan",
                    "mute",
                    "solo",
                    "gain",
                    "fader",
                    "meter",
                    "target",
                    "add_send",
                ] {
                    for corner in ["min", "max"] {
                        assert!(
                            (edge(&channel[key], corner, 1) - edge(&first[key], corner, 1)).abs()
                                * density
                                < 0.01,
                            "Every role shares the same {key} {corner} row"
                        );
                    }
                }
                let role_height = state["mixer"]["role_height"].as_f64().unwrap();
                let offset = if channel["bus"] == true {
                    role_height
                } else {
                    0.0
                };
                assert!(
                    (edge(&channel["name"], "min", 1) - edge(&first["name"], "min", 1) - offset)
                        .abs()
                        < 0.1
                );
                assert!(
                    edge(&channel["fader"], "max", 1) - edge(&channel["fader"], "min", 1)
                        >= f64::from(daw_ui::tokens::MIXER_FADER_MIN)
                );
                for key in ["mute", "solo", "source_menu"] {
                    assert!(
                        edge(&channel[key], "max", 1) - edge(&channel[key], "min", 1)
                            >= f64::from(daw_ui::tokens::COMPACT_HEIGHT),
                        "Compact typography must retain the shared mouse target: {key}"
                    );
                }
                for landmark in ["min_position", "max_position", "handle_center"] {
                    assert!(
                        (channel["fader_scale"][landmark][1].as_f64().unwrap()
                            - first["fader_scale"][landmark][1].as_f64().unwrap())
                        .abs()
                            * density
                            < 0.01
                    );
                }
                assert!(edge(&channel["effects"], "max", 0) < edge(&channel["analyze"], "min", 0));
                assert!(
                    edge(&channel["source_label"], "max", 0)
                        < edge(&channel["source_menu"], "min", 0)
                );
            }
            for channel in channels {
                let name = project
                    .track(channel["track"].as_u64().unwrap())
                    .unwrap()
                    .name
                    .as_str();
                assert!(
                    painted_name(&state, name, &channel["name"]),
                    "Normal channel names should not be needlessly truncated: {name}"
                );
            }
            assert!(painted_name(&state, "Audio · In 1", &first["source_label"]));
            let master = &state["mixer"]["master"];
            assert_eq!(first["meter"]["min"][1], master["meter"]["min"][1]);
            assert_eq!(first["meter"]["max"][1], master["meter"]["max"][1]);

            let effects = click(&mut app, &first["effects"]);
            assert_eq!(effects["popups"]["effects"], true);
            app.key("Escape");
            let analysis = click(&mut app, &first["analyze"]);
            assert!(analysis["signal_analysis"].is_object());
            app.key("Escape");
            let muted = click(&mut app, &first["mute"]);
            assert_eq!(muted["project"]["tracks"][0]["mute"], true);
            app.click("Edit");
            let undone = app.click("Undo");
            assert_eq!(undone["project"], original["project"]);

            // Automation retains its own arranger identity while mixer focus
            // and keyboard adjacency use the channel that owns the parameter.
            app.click("Arranger");
            let selected = app.click("Pan");
            assert_eq!(selected["selected_track"], pan);
            let owner = app.click("Mixer");
            assert_eq!(owner["selected_track"], pan);
            assert_eq!(owner["mixer"]["focused_channel"], keys);
            assert_eq!(owner["mixer"]["flow_source"], keys);
            let next = app.key("ArrowRight");
            assert_eq!(next["selected_track"], music);
            assert_eq!(next["project"], owner["project"]);
            assert_eq!(next["history"], owner["history"]);
        }
        app.quit();
    }
}
