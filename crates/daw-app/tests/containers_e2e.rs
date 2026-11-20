//! R1 Split and R1 Chain through the running app: the OTT Style preset,
//! lanes edited from the panel and by tools, the drill-in to a lane's
//! effect and back, Undo, and a save round trip.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(state["tool_result"]["ok"], true, "{name}: {state}");
    state
}

fn session(state: &Value) -> (Value, Value) {
    (
        state["song_session"]["id"].clone(),
        state["song_session"]["revision"].clone(),
    )
}

fn rms_db(app: &mut Harness) -> f64 {
    let analysis = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0}),
    );
    analysis["tool_result"]["result"]["rms_dbfs"]
        .as_f64()
        .unwrap()
}

/// One audio track playing a second of quiet white noise.
fn noise_project(app: &mut Harness) -> u64 {
    let mut seed = 0x2545_f491_u32;
    let samples = (0..48_000)
        .flat_map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let value = (seed as f32 / u32::MAX as f32 - 0.5) * 0.2;
            [value, value]
        })
        .collect();
    let source = app.directory.path().join("noise.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples,
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Noise");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Noise burst",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("split.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    track
}

fn lanes(state: &Value) -> &Vec<Value> {
    state["project"]["tracks"][0]["inserts"][0]["effect"]["Container"]["lanes"]
        .as_array()
        .unwrap_or_else(|| panic!("No container: {}", state["project"]["tracks"][0]))
}

fn lane_devices(state: &Value) -> Vec<Vec<String>> {
    lanes(state)
        .iter()
        .map(|lane| {
            lane["inserts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|slot| match &slot["effect"] {
                    effect if effect["Builtin"].is_object() => {
                        effect["Builtin"]["device"].as_str().unwrap().to_owned()
                    }
                    effect => effect.to_string(),
                })
                .collect()
        })
        .collect()
}

fn click_occurrence(app: &mut Harness, label: &str, occurrence: usize) -> Value {
    app.command(json!({"command":"click","label":label,"occurrence":occurrence}))
}

#[test]
fn split_loads_ott_edits_lanes_and_survives_a_save() {
    let mut app = Harness::new();
    let track = noise_project(&mut app);
    let dry = rms_db(&mut app);

    // Both containers are in the catalog; a new Split holds three empty bands.
    let listed = song_tool(&mut app, "list_plugins", json!({"query":"R1 Split"}));
    let plugin = &listed["tool_result"]["result"]["plugins"][0];
    assert_eq!(plugin["id"], "rill:r1-split", "{listed}");
    assert_eq!(plugin["is_instrument"], false);
    let (id, revision) = session(&listed);
    let assigned = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "plugin_id":"rill:r1-split","insert":true}),
    );
    let split = assigned["tool_result"]["result"]["insert_id"]
        .as_u64()
        .unwrap();
    assert_eq!(lane_devices(&assigned), vec![Vec::<String>::new(); 3]);
    // Empty bands sum back to the input.
    let empty = rms_db(&mut app);
    assert!((empty - dry).abs() < 0.2, "dry {dry} dB, split {empty} dB");

    // OTT Style: one compressor per band, as one Undo step.
    let presets = song_tool(
        &mut app,
        "list_device_presets",
        json!({"track_id":track,"insert_id":split,"query":"OTT"}),
    );
    assert_eq!(
        presets["tool_result"]["result"]["presets"][0]["name"], "OTT Style",
        "{presets}"
    );
    let (id, revision) = session(&presets);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "insert_id":split,"name":"OTT Style"}),
    );
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    assert_eq!(
        lane_devices(&loaded),
        vec![vec!["r1-compressor".to_owned()]; 3]
    );
    let squashed = rms_db(&mut app);
    assert!(
        squashed.is_finite() && (squashed - dry).abs() > 0.2,
        "dry {dry} dB, OTT {squashed} dB"
    );

    // Tools: add a filter to the high band, then move it to the front of
    // the low band; Undo takes the move back.
    let (id, revision) = session(&loaded);
    let added = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "plugin_id":"rill:r1-filter","insert":true,"container_id":split,"lane":2}),
    );
    let filter = added["tool_result"]["result"]["insert_id"].clone();
    assert_eq!(lane_devices(&added)[2], ["r1-compressor", "r1-filter"]);
    let (id, revision) = session(&added);
    let moved = song_tool(
        &mut app,
        "edit_effect_lanes",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "container_id":split,"action":"move_effect","effect_id":filter,"lane":0,"index":0}),
    );
    assert_eq!(moved["history"]["undo"][0], "Move effect");
    assert_eq!(lane_devices(&moved)[0], ["r1-filter", "r1-compressor"]);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], added["project"]);
    // A container never moves into its own lane.
    let (id, revision) = session(&undone);
    let refused = app.command(json!({"command":"song_tool","name":"edit_effect_lanes",
        "arguments":{"session_id":id,"expected_revision":revision,"track_id":track,
            "container_id":split,"action":"move_effect","effect_id":split,"lane":1}}));
    assert_eq!(refused["tool_result"]["ok"], false, "{refused}");
    assert_eq!(refused["project"], added["project"]);

    // The panel: every band is a card with its effects and + Effect.
    app.command(json!({"command":"viewport","width":1400,"height":1000,"pixels_per_point":1.0}));
    app.click("Noise burst");
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(&mut app, "+ Effect");
    let state = app.inspect();
    let adds = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "+ Effect")
        .count();
    assert_eq!(adds, 3, "{}", state["visible_labels"]);

    // + Effect on the mid band chooses an effect for that lane.
    let chooser = click_occurrence(&mut app, "+ Effect", 1);
    assert_eq!(chooser["popups"]["library"], true);
    common::wait_library_device(&mut app, "R1 Saturator");
    let state = app.inspect();
    let card = state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["name"] == "R1 Saturator")
        .unwrap()
        .clone();
    let center = |axis: usize| {
        (card["rect"]["min"][axis].as_f64().unwrap() + card["rect"]["max"][axis].as_f64().unwrap())
            * 0.5
    };
    app.command(json!({"command":"pointer","phase":"click","x":center(0),"y":center(1)}));
    let inserted = app.click("Add to lane");
    assert_eq!(inserted["history"]["undo"][0], "Add effect to lane");
    assert_eq!(
        lane_devices(&inserted)[1],
        ["r1-compressor", "r1-saturator"]
    );
    // Back on the Split's panel, the saturator opens and links back.
    common::wait_visible(&mut app, "R1 Saturator");
    let state = app.inspect();
    let rects = state["label_rects"].as_array().unwrap();
    let center_y =
        |rect: &Value| (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let name = rects
        .iter()
        .rfind(|rect| rect["text"] == "R1 Saturator")
        .unwrap();
    let open = rects
        .iter()
        .filter(|rect| rect["text"] == "Open")
        // The row's actions sit on the line under the name, in its card.
        .find(|rect| {
            let below = center_y(rect) - center_y(name);
            (10.0..60.0).contains(&below)
                && rect["min"][0].as_f64() > name["min"][0].as_f64()
                && rect["min"][0].as_f64().unwrap() < name["min"][0].as_f64().unwrap() + 320.0
        })
        .unwrap_or_else(|| panic!("No Open beside the saturator: {rects:?}"));
    let x = (open["min"][0].as_f64().unwrap() + open["max"][0].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":center_y(open)}));
    // The lane's effect opens in the same window, linked to its container.
    common::wait_visible(&mut app, "‹ R1 Split");
    common::wait_visible(&mut app, "Lane 2");
    app.click("‹ R1 Split");
    common::wait_visible(&mut app, "+ Effect");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], added["project"]);

    // A save keeps the lanes, their effects and the Split's settings.
    let path = app.directory.path().join("saved.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        added["project"]["tracks"][0]["inserts"]
    );
    let again = rms_db(&mut app);
    assert!(again.is_finite(), "{again}");
    app.quit();
}
