//! Signal order, visible destinations and safe parallel bus sends through real UI.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    if parent == "+ Send" {
        open_send_choices(app);
    } else {
        app.click(parent);
    }
    app.click(item)
}

fn routing_footer(app: &mut Harness) -> Value {
    let state = app.inspect();
    let window = &state["window_rects"][0];
    app.command(json!({"command":"scroll",
        "x":(window["min"][0].as_f64().unwrap() + window["max"][0].as_f64().unwrap()) * 0.5,
        "y":window["max"][1].as_f64().unwrap() - 50.0, "delta":-10000.0}));
    for _ in 0..20 {
        app.inspect();
    }
    let scrolled = app.inspect();
    assert_eq!(scrolled["project"], state["project"]);
    scrolled
}

fn open_send_choices(app: &mut Harness) -> Value {
    let state = routing_footer(app);
    let target = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == "+ Send")
        .expect("Send action reachable by scrolling");
    assert!(bottom(target) - top(target) >= 13.0);
    app.command(json!({"command":"pointer", "phase":"move",
        "x":(target["min"][0].as_f64().unwrap() + target["max"][0].as_f64().unwrap()) * 0.5,
        "y":(top(target) + bottom(target)) * 0.5}));
    app.click("+ Send")
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect","paint":true}))
}
fn top(rect: &Value) -> f64 {
    rect["min"][1].as_f64().unwrap()
}
fn bottom(rect: &Value) -> f64 {
    rect["max"][1].as_f64().unwrap()
}
fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == label)
}
fn count(state: &Value, label: &str) -> usize {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count()
}
fn open_routing(app: &mut Harness, route: &str) {
    app.click("Mixer");
    app.click(route);
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn mixer_orders_signal_stages_and_separates_primary_and_send_arrows() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Source");
        let original = app.directory.path().join("source.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: (0..48_000)
                .map(|frame| (frame as f32 * 0.04).sin() * 0.1)
                .collect(),
        }
        .write_wav(&original)
        .unwrap();
        let original_bytes = std::fs::read(&original).unwrap();
        project.track_mut(source).unwrap().clips.push(Clip::audio(
            "Source clip",
            0.0,
            2.0,
            original.to_string_lossy(),
        ));
        let bus_a = project.add_bus("Music");
        let bus_b = project.add_bus("Room");
        let remote = project.add_bus("Remote");
        let lead = project.add_midi_track("Lead");
        for track in &mut project.tracks {
            track.color = [140, 160, 180];
        }
        project.set_output(source, Some(bus_a)).unwrap();
        project.add_send(source, bus_b, 0.25, true).unwrap();
        let path = app.directory.path().join("flow.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":900,"pixels_per_point":density}),
        );
        let before_audio = export(&mut app, "Before group view.wav");
        let before = app.click("Mixer");
        // The pointer remains over the workspace selector; this shows the
        // selected channel rather than relying on a synthetic graph snapshot.
        let state = paint(&mut app);
        let visual = state["mixer"]["channels"].as_array().unwrap();
        assert_eq!(
            visual
                .iter()
                .map(|channel| channel["track"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [source, bus_a, bus_b, remote, lead]
        );
        assert_eq!(
            visual
                .iter()
                .map(|channel| channel["channel"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(
            state["mixer"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|group| group["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Tracks", "Buses", "Tracks", "Master"]
        );
        assert!(
            state["mixer"]["groups"]
                .as_array()
                .unwrap()
                .windows(2)
                .all(|pair| pair[1]["rect"]["min"][0].as_f64().unwrap()
                    - pair[0]["rect"]["max"][0].as_f64().unwrap()
                    >= f64::from(daw_ui::tokens::SPACE_LG))
        );
        for channel in state["mixer"]["channels"].as_array().unwrap() {
            let stages = [
                "name",
                "source",
                "effects",
                "gain",
                "fader",
                "pan",
                "mute_solo",
                "meter",
                "target",
            ];
            for pair in stages.windows(2) {
                assert!(
                    bottom(&channel[pair[0]]) <= top(&channel[pair[1]]) + 1.0,
                    "signal order {pair:?}: {channel}"
                );
            }
            let kind = channel["source_kind"].as_str().unwrap();
            let label = &channel["source_label"];
            assert!(
                bottom(label) - top(label)
                    <= f64::from(daw_ui::tokens::FONT_SMALL + 2.0 * daw_ui::tokens::SPACE_XS),
                "{kind} must stay on one line: {label}"
            );
            assert!(
                label["max"][0].as_f64().unwrap()
                    <= channel["source"]["max"][0].as_f64().unwrap() + 0.1
            );
            assert!(
                bottom(&channel["source_menu"]) - top(&channel["source_menu"])
                    >= f64::from(daw_ui::tokens::CONTROL_HEIGHT) - 0.1
            );
            assert!(
                state["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|paint| paint["text"] == kind
                        && bottom(paint) - top(paint) <= 22.0
                        && paint["min"][0].as_f64().unwrap()
                            >= label["min"][0].as_f64().unwrap() - 0.1
                        && paint["max"][0].as_f64().unwrap()
                            <= label["max"][0].as_f64().unwrap() + 0.1),
                "painted full {kind} label must fit its source slot"
            );
            let header = state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|paint| paint["rect"] == channel["name"])
                .unwrap();
            if channel["bus"] == true {
                assert_ne!(
                    header["fill"],
                    json!([140, 160, 180, 255]),
                    "bus headers have a distinct treatment even with the same track color"
                );
                assert!(
                    state["label_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|paint| paint["text"] == "BUS"
                            && paint["min"][0].as_f64().unwrap()
                                >= channel["name"]["min"][0].as_f64().unwrap()
                            && paint["max"][0].as_f64().unwrap()
                                <= channel["name"]["max"][0].as_f64().unwrap())
                );
            } else {
                assert_eq!(header["fill"], json!([140, 160, 180, 255]));
            }
        }
        // Send lanes extend below the stable channel controls. Reveal them
        // with the same mouse scroll used in a short mixer workspace.
        app.command(json!({"command":"scroll","x":100,"y":400,"delta":-180}));
        for _ in 0..10 {
            app.inspect();
        }
        let state = paint(&mut app);
        let routes = state["mixer"]["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0]["target"], bus_a);
        assert_eq!(routes[1]["target"], bus_b);
        assert_eq!(routes[1]["kind"], "Pre send");
        assert!(bottom(&routes[0]["lane"]) <= top(&routes[1]["lane"]));
        for route in routes {
            assert!(
                state["painted_meshes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|mesh| mesh["vertices"] == 3
                        && mesh["indices"] == 3
                        && mesh["rect"] == route["head"]),
                "route must have a painted arrowhead"
            );
        }
        assert_eq!(state["project"], before["project"]);
        assert_eq!(state["history"], before["history"]);
        assert!(visible(&state, "Send · Room"));
        app.command(json!({"command":"scroll","x":100,"y":400,"delta":1000}));
        for _ in 0..10 {
            app.inspect();
        }
        app.click("Effects");
        assert_eq!(app.inspect()["popups"]["effects"], true);
        app.key("Escape");
        // At narrow widths, an offscreen Master keeps its true geometry: the
        // connected line clips at the viewport and does not invent an endpoint.
        app.command(json!({"command":"viewport","width":800,"height":600}));
        app.command(json!({"command":"scroll","x":100,"y":400,"delta":-10000}));
        for _ in 0..10 {
            app.inspect();
        }
        let scrolled = app.inspect();
        let room = scrolled["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == bus_b)
            .unwrap();
        app.command(json!({"command":"pointer","phase":"move","x":(room["pan"]["min"][0].as_f64().unwrap()+room["pan"]["max"][0].as_f64().unwrap())*0.5,"y":(top(&room["pan"])+bottom(&room["pan"]))*0.5}));
        let small = paint(&mut app);
        let routes = small["mixer"]["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 1);
        assert!(routes[0]["target"].is_null());
        assert_eq!(routes[0]["target_clipped"], true);
        assert!(routes[0]["to"][0].as_f64().unwrap() > 800.0);
        assert!(
            small["painted_meshes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|mesh| { mesh["vertices"] != 3 || mesh["rect"] != routes[0]["head"] }),
            "An offscreen destination must not acquire a false on-screen arrowhead"
        );
        assert_eq!(small["project"], before["project"]);
        assert_eq!(small["history"], before["history"]);
        assert_eq!(
            small["selected_track"], before["selected_track"],
            "route hover is a view preview"
        );
        let lane_y = top(&routes[0]["lane"]) + 8.0;
        let reading = app.command(json!({"command":"pointer","phase":"move","x":100,"y":lane_y}));
        assert_eq!(
            reading["mixer"]["flow_source"], bus_b,
            "the preview must stay put while reading its routing lane"
        );
        assert_eq!(reading["selected_track"], before["selected_track"]);
        app.click("Arranger");
        let same_selection = app.click("Source clip");
        assert_eq!(same_selection["selected_track"], reading["selected_track"]);
        let returned = app.click("Mixer");
        assert_eq!(
            returned["mixer"]["flow_source"], source,
            "re-entering Mixer restores the selection even when its ID has not changed"
        );
        assert_eq!(returned["project"], before["project"]);
        assert_eq!(returned["history"], before["history"]);
        // Returning to a workspace can restore its top scroll position. Give
        // the following selection/reading check room to show the routing band.
        app.command(json!({"command":"viewport","width":1100,"height":900}));
        app.click("Ch 2 > Master");
        app.key("Escape");
        let current = app.inspect();
        let selected_channel = current["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == bus_a)
            .unwrap();
        let x = (selected_channel["strip"]["min"][0].as_f64().unwrap()
            + selected_channel["strip"]["max"][0].as_f64().unwrap())
            * 0.5;
        app.command(json!({"command":"scroll","x":x,"y":400,"delta":-180}));
        for _ in 0..10 {
            app.inspect();
        }
        let current = paint(&mut app);
        let lane_y = top(&current["mixer"]["routes"][0]["lane"]) + 8.0;
        let selected = app.command(json!({"command":"pointer","phase":"move","x":100,"y":lane_y}));
        assert_eq!(selected["selected_track"], bus_a);
        assert_eq!(
            selected["mixer"]["flow_source"], bus_a,
            "an explicit changed selection wins over the previous hover"
        );
        assert_eq!(
            export(&mut app, "After group view.wav").samples,
            before_audio.samples
        );
        assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
        app.quit();
    }
}

#[test]
fn mixer_creates_buses_and_routes_to_multiple_destinations_without_feedback() {
    let mut app = Harness::new();
    let source_path = app.directory.path().join("original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.2; 48_000],
    }
    .write_wav(&source_path)
    .unwrap();
    let original_bytes = std::fs::read(&source_path).unwrap();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    project.track_mut(source).unwrap().gain = 0.4;
    project.track_mut(source).unwrap().clips.push(Clip::audio(
        "Audio",
        0.,
        2.,
        source_path.to_string_lossy(),
    ));
    let bus_a = project.add_bus("Music");
    project.track_mut(bus_a).unwrap().gain = 0.5;
    let bus_b = project.add_bus("Room");
    project.track_mut(bus_b).unwrap().gain = 0.25;
    let bus_c = project.add_bus("Parallel");
    project.track_mut(bus_c).unwrap().gain = 0.75;
    let path = app.directory.path().join("sends.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Mixer");
    let initial = app.inspect();
    let added = app.click("Add bus");
    assert_eq!(added["project"]["tracks"].as_array().unwrap().len(), 5);
    let second = app.click("Add bus");
    assert_eq!(second["project"]["tracks"][4]["name"], "Bus 1");
    assert_eq!(second["project"]["tracks"][5]["name"], "Bus 2");
    undo(&mut app, false);
    assert_eq!(undo(&mut app, false)["project"], initial["project"]);
    open_routing(&mut app, "Ch 1 > Master");
    app.click("Master output");
    app.click("Music");
    menu(&mut app, "+ Send", "Room");
    menu(&mut app, "+ Send", "Parallel");
    let controls = routing_footer(&mut app);
    let occurrence = count(&controls, "Pre")
        .checked_sub(1)
        .expect("Last send's Pre control is visible");
    let pre = controls["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == "Pre")
        .unwrap();
    assert!(bottom(pre) - top(pre) >= 13.0);
    app.command(json!({"command":"pointer", "phase":"move",
        "x":(pre["min"][0].as_f64().unwrap() + pre["max"][0].as_f64().unwrap()) * 0.5,
        "y":(top(pre) + bottom(pre)) * 0.5}));
    let sent = app.command(json!({"command":"click","label":"Pre","occurrence":occurrence}));
    assert_eq!(sent["project"]["tracks"][0]["output_bus"], bus_a);
    assert_eq!(
        sent["project"]["tracks"][0]["sends"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(sent["project"]["tracks"][0]["sends"][1]["pre_fader"], true);
    assert_eq!(sent["project"]["tracks"][0]["sends"][0]["pre_fader"], false);
    let snapshot = sent["project"].clone();
    menu(&mut app, "+ Send", "+ Bus and send");
    let created = app.inspect();
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 5);
    assert_eq!(
        created["project"]["tracks"][0]["sends"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        undo(&mut app, false)["project"],
        snapshot,
        "bus plus send is one edit"
    );
    app.key("Escape");
    // A transitive Music→Room path prevents Room→Music and Room→Room.
    open_routing(&mut app, "Ch 2 > Master");
    app.click("Master output");
    app.click("Room");
    app.key("Escape");
    open_routing(&mut app, "Ch 3 > Master");
    let before_choices = routing_footer(&mut app);
    let choices = open_send_choices(&mut app);
    assert_eq!(count(&choices, "Music"), count(&before_choices, "Music"));
    assert_eq!(count(&choices, "Room"), count(&before_choices, "Room"));
    app.key("Escape");
    let before_choices = app.inspect();
    app.click("Master output");
    let choices = app.inspect();
    assert_eq!(count(&choices, "Music"), count(&before_choices, "Music"));
    assert_eq!(count(&choices, "Room"), count(&before_choices, "Room"));
    app.key("Escape");
    app.key("Escape");
    let state = undo(&mut app, false);
    assert_eq!(state["project"], snapshot);
    // The core still validates every write; filtering is only the UI choice.
    let mut graph: Project = serde_json::from_value(snapshot.clone()).unwrap();
    graph.set_output(bus_a, Some(bus_b)).unwrap();
    let before = graph.clone();
    assert!(graph.add_send(bus_b, bus_a, 0.25, false).is_err());
    assert_eq!(graph, before);
    assert!(graph.set_output(bus_b, Some(bus_a)).is_err());
    assert_eq!(graph, before);
    let saved = app.directory.path().join("parallel.rill");
    app.command(json!({"command":"save","path":saved}));
    let mixed = app.directory.path().join("parallel.wav");
    app.command(json!({"command":"export","path":mixed}));
    app.command(json!({"command":"wait_export"}));
    let audio = AudioData::load_wav(&mixed).unwrap();
    // Main Music + post Room + pre Parallel all reach Master.
    assert!(
        (audio.samples[1000] - 0.0825).abs() < 1e-5,
        "{}",
        audio.samples[1000]
    );
    assert_eq!(std::fs::read(&source_path).unwrap(), original_bytes);
    let mut app = app.restart();
    app.command(json!({"command":"open","path":saved}));
    assert_eq!(
        app.inspect()["project"]["tracks"][0]["sends"],
        snapshot["tracks"][0]["sends"]
    );
    app.quit();
}
