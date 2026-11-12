//! Rill's built-in effects through the running app: the Library catalog,
//! tools in plain units, presets, the device panel and right-click automation.
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

fn high_band_db(app: &mut Harness) -> f64 {
    let analysis = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0}),
    );
    analysis["tool_result"]["result"]["band_rms_estimates_dbfs"]["above_4000_hz"]
        .as_f64()
        .unwrap()
}

fn label_center(state: &Value, label: &str) -> (f64, f64) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

/// Opens a project with one audio track playing a second of white noise.
fn noise_project(app: &mut Harness, file: &str) -> u64 {
    // Deterministic white noise: energy in every band.
    let mut seed = 0x2545_f491_u32;
    let samples = (0..48_000)
        .flat_map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let value = (seed as f32 / u32::MAX as f32 - 0.5) * 0.5;
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
    let path = app.directory.path().join(file);
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    track
}

#[test]
fn r1_filter_shapes_audio_loads_presets_and_automates_from_its_panel() {
    let mut app = Harness::new();
    let track = noise_project(&mut app, "filter.rill");
    let dry = high_band_db(&mut app);

    // The Library catalog lists the device beside hosted plug-ins.
    let listed = song_tool(&mut app, "list_plugins", json!({"query":"R1 Filter"}));
    let plugin = &listed["tool_result"]["result"]["plugins"][0];
    assert_eq!(plugin["id"], "rill:r1-filter", "{listed}");
    assert_eq!(plugin["format"], "Built-in");
    assert_eq!(plugin["is_instrument"], false);
    let (id, revision) = session(&listed);
    let assigned = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "plugin_id":"rill:r1-filter","insert":true}),
    );
    let insert = assigned["tool_result"]["result"]["insert_id"].clone();
    assert_eq!(
        assigned["project"]["tracks"][0]["inserts"][0]["effect"],
        json!({"Builtin":{"device":"r1-filter"}})
    );
    // Default settings: a 1 kHz low-pass, already darker than the source.
    let darker = high_band_db(&mut app);
    assert!(darker < dry - 6.0, "dry {dry} dB, default {darker} dB");

    // Tools speak the device's units; nothing normalized to guess.
    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"cutoff"}),
    );
    let cutoff = &described["tool_result"]["result"]["parameters"][0];
    assert_eq!(cutoff["key"], "cutoff", "{described}");
    assert_eq!(cutoff["unit"], "Hz");
    assert_eq!(cutoff["min"], 20.0);
    assert_eq!(cutoff["max"], 20_000.0);
    assert!((cutoff["value"].as_f64().unwrap() - 1_000.0).abs() < 0.5);
    let (id, revision) = session(&described);
    let rejected = app.command(json!({"command":"song_tool","name":"set_device_parameters",
        "arguments":{"session_id":id,"expected_revision":revision,"track_id":track,
            "insert_id":insert,"parameters":[{"parameter":"cutoff","value":50_000}]}}));
    assert_eq!(
        rejected["tool_result"]["error"]["code"],
        "invalid_arguments"
    );
    let set = song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,"insert_id":insert,
            "parameters":[{"parameter":"cutoff","value":"200 Hz"},{"parameter":"model","value":"diode pair"}]}),
    );
    let stored = &set["tool_result"]["result"]["parameters"];
    assert!(
        (stored[0]["value"].as_f64().unwrap() - 200.0).abs() < 0.5,
        "{set}"
    );
    assert_eq!(stored[1]["value"], "Diode Pair");
    let filtered = high_band_db(&mut app);
    assert!(
        filtered < darker - 12.0,
        "1 kHz {darker} dB, 200 Hz {filtered} dB"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);

    // Factory presets load in one Undo step; user presets save beside them.
    let presets = song_tool(
        &mut app,
        "list_device_presets",
        json!({"track_id":track,"insert_id":insert,"query":"acid"}),
    );
    assert!(
        presets["tool_result"]["result"]["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["name"] == "Acid Ladder" && preset["source"] == "factory"),
        "{presets}"
    );
    let (id, revision) = session(&presets);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"acid ladder","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_ne!(loaded["project"], assigned["project"]);
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    song_tool(
        &mut app,
        "store_device_preset",
        json!({"name":"My Acid","track_id":track,"insert_id":insert}),
    );
    let saved = app
        .directory
        .path()
        .join("config/presets/r1-filter/My Acid.rillpreset");
    assert!(
        std::fs::read_to_string(&saved)
            .unwrap()
            .contains("\"Transistor Ladder\"")
    );

    // The panel: right-click the Cutoff knob and choose Automate.
    app.command(json!({"command":"viewport","width":1280,"height":900,"pixels_per_point":1.0}));
    app.click("Noise burst");
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(&mut app, "Cutoff");
    let state = app.inspect();
    // Cutoff is the hero knob, drawn right under its name.
    let (x, name) = label_center(&state, "Cutoff");
    let knob = name + 12.0 + f64::from(daw_devices_ui::KNOB_HERO) * 0.5;
    let before = app.inspect();
    let dragged = app.command(
        json!({"command":"pointer","phase":"drag","x":x,"y":knob,"to_x":x,"to_y":knob - 40.0}),
    );
    let cutoff = |state: &Value| {
        state["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == 4)
            .map(|parameter| parameter["value"].as_f64().unwrap())
    };
    assert!(cutoff(&dragged) > cutoff(&before), "{}", dragged["project"]);
    assert_eq!(dragged["history"]["undo"][0], "Effect parameter");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);

    // The response plot's cutoff node moves the same parameter.
    let window = &undone["window_rects"][0];
    let inside = |rect: &Value| {
        (0..2).all(|axis| {
            rect["min"][axis].as_f64() >= window["min"][axis].as_f64()
                && rect["max"][axis].as_f64() <= window["max"][axis].as_f64()
        })
    };
    let marker = undone["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == "1" && inside(entry))
        .unwrap_or_else(|| panic!("No cutoff node: {}", undone["label_rects"]));
    // The node's number sits 10 points above its center.
    let node_x = (marker["min"][0].as_f64().unwrap() + marker["max"][0].as_f64().unwrap()) * 0.5;
    let node_y = marker["max"][1].as_f64().unwrap() + 10.0;
    let moved = app.command(json!({"command":"pointer","phase":"drag",
        "x":node_x,"y":node_y,"to_x":node_x + 60.0,"to_y":node_y}));
    assert!(cutoff(&moved) > cutoff(&before), "{}", moved["project"]);
    assert_eq!(moved["history"]["undo"][0], "Effect parameter");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);

    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":knob}));
    let state = app.click("Automate");
    let lane = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|lane| {
            lane["automation"]["target"]
                == json!({"track":track,"parameter":{"VstParameter":{"effect":insert,"id":4}}})
        })
        .unwrap_or_else(|| panic!("No Cutoff lane: {}", state["project"]));
    assert_eq!(state["selected_track"], lane["id"]);
    // Automate lands on the new lane in Arrangement, with a clip to draw in.
    assert_eq!(state["popups"]["automation"], false);
    assert_eq!(state["view"], "Arrangement");
    assert_eq!(state["selected_clip"], lane["automation"]["clips"][0]["id"]);
    assert!(
        lane["name"]
            .as_str()
            .unwrap()
            .ends_with("R1 Filter · Filter Cutoff"),
        "{lane}"
    );
    app.quit();
}

fn band_db(app: &mut Harness, band: &str) -> f64 {
    let analysis = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0}),
    );
    analysis["tool_result"]["result"]["band_rms_estimates_dbfs"][band]
        .as_f64()
        .unwrap()
}

#[test]
fn r1_equalizer_shapes_audio_loads_presets_and_edits_from_its_plot() {
    let mut app = Harness::new();
    let track = noise_project(&mut app, "equalizer.rill");
    let dry = (band_db(&mut app, "below_250_hz"), high_band_db(&mut app));

    let listed = song_tool(&mut app, "list_plugins", json!({"query":"R1 Equalizer"}));
    let plugin = &listed["tool_result"]["result"]["plugins"][0];
    assert_eq!(plugin["id"], "rill:r1-equalizer", "{listed}");
    assert_eq!(plugin["format"], "Built-in");
    assert_eq!(plugin["is_instrument"], false);
    let (id, revision) = session(&listed);
    let assigned = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "plugin_id":"rill:r1-equalizer","insert":true}),
    );
    let insert = assigned["tool_result"]["result"]["insert_id"].clone();
    assert_eq!(
        assigned["project"]["tracks"][0]["inserts"][0]["effect"],
        json!({"Builtin":{"device":"r1-equalizer"}})
    );
    // The default is flat: inserting it changes nothing audible.
    let flat = (band_db(&mut app, "below_250_hz"), high_band_db(&mut app));
    assert!(
        (flat.0 - dry.0).abs() < 0.2 && (flat.1 - dry.1).abs() < 0.2,
        "dry {dry:?}, flat {flat:?}"
    );

    // Tools speak decibels, hertz and option names.
    let (id, revision) = session(&assigned);
    let set = song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,"insert_id":insert,
            "parameters":[
                {"parameter":"band1.type","value":"low shelf"},
                {"parameter":"band1.frequency","value":"200 Hz"},
                {"parameter":"band1.gain","value":12},
                {"parameter":"lp.on","value":true},
                {"parameter":"lp.frequency","value":"1 kHz"},
                {"parameter":"lp.slope","value":"48 dB"}]}),
    );
    let stored = &set["tool_result"]["result"]["parameters"];
    assert_eq!(stored[0]["value"], "Low shelf");
    assert!((stored[2]["value"].as_f64().unwrap() - 12.0).abs() < 0.01);
    let shaped = (band_db(&mut app, "below_250_hz"), high_band_db(&mut app));
    assert!(
        shaped.0 > flat.0 + 6.0,
        "lows {} dB to {} dB",
        flat.0,
        shaped.0
    );
    assert!(
        shaped.1 < flat.1 - 20.0,
        "highs {} dB to {} dB",
        flat.1,
        shaped.1
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);

    let (id, revision) = session(&undone);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"telephone","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    let telephone = (band_db(&mut app, "below_250_hz"), high_band_db(&mut app));
    assert!(
        telephone.0 < flat.0 - 1.0 && telephone.1 < flat.1 - 8.0,
        "flat {flat:?}, telephone {telephone:?}"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);

    // The panel: dragging band 3's node in the plot sets its frequency and
    // gain together, in one Undo step.
    app.command(json!({"command":"viewport","width":1280,"height":1000,"pixels_per_point":1.0}));
    app.click("Noise burst");
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(&mut app, "Freq");
    let before = app.inspect();
    let window = &before["window_rects"][0];
    let inside = |rect: &Value| {
        (0..2).all(|axis| {
            rect["min"][axis].as_f64() >= window["min"][axis].as_f64()
                && rect["max"][axis].as_f64() <= window["max"][axis].as_f64()
        })
    };
    // The plot sits above the table, whose columns carry the same numbers.
    let marker = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["text"] == "3" && inside(entry))
        .min_by(|a, b| {
            a["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&b["min"][1].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("No band 3 node: {}", before["label_rects"]));
    // The node's number sits 10 points above its center.
    let node_x = (marker["min"][0].as_f64().unwrap() + marker["max"][0].as_f64().unwrap()) * 0.5;
    let node_y = marker["max"][1].as_f64().unwrap() + 10.0;
    let moved = app.command(json!({"command":"pointer","phase":"drag",
        "x":node_x,"y":node_y,"to_x":node_x + 65.0,"to_y":node_y - 40.0}));
    let value = |state: &Value, id: u64| {
        state["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == id)
            .map_or(0.5, |parameter| parameter["value"].as_f64().unwrap())
    };
    assert!(
        value(&moved, 32) > value(&before, 32),
        "{}",
        moved["project"]
    );
    assert!(value(&moved, 33) > 0.5, "{}", moved["project"]);
    assert_eq!(moved["history"]["undo"][0], "Effect parameter");
    // One Undo takes back both values of the drag, one Redo returns them.
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], moved["project"]);
    assert!(high_band_db(&mut app) > flat.1 + 1.0);

    // Save and reopen: the boost comes back as stored.
    let path = app.directory.path().join("equalized.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        moved["project"]["tracks"][0]["inserts"]
    );
    assert!(high_band_db(&mut app) > flat.1 + 1.0);
    app.quit();
}

fn select_card(app: &mut Harness, name: &str) -> Value {
    common::wait_library_device(app, name);
    let state = app.inspect();
    let card = state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["name"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["library"]))
        .clone();
    assert_eq!(
        card["preview"], true,
        "{name} shows its panel image: {card}"
    );
    let (x, y) = (
        (card["rect"]["min"][0].as_f64().unwrap() + card["rect"]["max"][0].as_f64().unwrap()) * 0.5,
        (card["rect"]["min"][1].as_f64().unwrap() + card["rect"]["max"][1].as_f64().unwrap()) * 0.5,
    );
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

/// Built-ins read ports and parameters from the registry and ship a panel
/// image, so the browser shows both without scanning or capturing.
fn builtin_details(app: &mut Harness, name: &str, id: &str, instrument: bool) {
    let selected = select_card(app, name);
    assert_eq!(selected["library"]["selected"], id);
    let pane = common::wait_until(app, |state| visible(state, "Browse parameters"));
    assert!(visible(&pane, "Included"), "{}", pane["visible_labels"]);
    assert!(!visible(&pane, "Capture preview"));
    let details = app.click("Device details");
    assert_eq!(
        details["plugin_details"]["status"], "ready",
        "{}",
        details["plugin_details"]
    );
    let metadata = &details["plugin_details"]["metadata"];
    assert_eq!(metadata["midi_input"], instrument);
    assert_eq!(metadata["outputs"][0]["channels"], 2);
    assert_eq!(
        metadata["inputs"].as_array().unwrap().is_empty(),
        instrument
    );
    let parameters = metadata["parameters"].as_array().unwrap();
    assert!(parameters.len() > 10, "{metadata}");
    assert!(
        parameters
            .iter()
            .all(|parameter| parameter["can_automate"] == true)
    );
    assert!(
        parameters
            .iter()
            .any(|parameter| parameter["name"].as_str().unwrap().contains("Cutoff")),
        "{metadata}"
    );
    let report = app.click("Copy details")["clipboard_text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(report.contains("Built into Rill"), "{report}");
    assert!(!report.contains("moduleinfo"), "{report}");
    let closed = app.click("Close");
    assert!(closed["plugin_details"].is_null());
}

#[test]
fn builtin_devices_show_metadata_and_preview_in_the_device_browser() {
    let mut app = Harness::new();
    app.click("Track");
    let chooser = app.click("Add MIDI track");
    assert_eq!(chooser["popups"]["library"], true);
    app.click("Built-in (R1)");
    let before = app.inspect();
    builtin_details(&mut app, "R1 Mini", "rill:r1-mini", true);
    assert_eq!(app.inspect()["project"], before["project"]);
    app.key("Escape");

    app.click("Window");
    app.click("Effects");
    let inserts = app.click("+ Insert");
    assert_eq!(
        inserts["library"]["order"],
        json!(["R1 Equalizer", "R1 Filter"])
    );
    builtin_details(&mut app, "R1 Filter", "rill:r1-filter", false);
    app.quit();
}
