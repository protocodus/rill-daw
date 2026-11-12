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

fn analysis(app: &mut Harness) -> Value {
    let state = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0}),
    );
    state["tool_result"]["result"].clone()
}

/// A band's level; silence reads as a floor instead of no number.
fn band(analysis: &Value, band: &str) -> f64 {
    analysis["band_rms_estimates_dbfs"][band]
        .as_f64()
        .unwrap_or(-200.0)
}

fn apply_edits(app: &mut Harness, edits: Value) -> Value {
    let before = app.inspect();
    app.command(
        json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":before["song_session"]["id"],
        "expected_revision":before["song_session"]["revision"],
        "label":"Route sidechain","edits":edits}}),
    )
}

fn set_parameters(app: &mut Harness, track: u64, insert: &Value, parameters: Value) -> Value {
    let before = app.inspect();
    let (id, revision) = session(&before);
    song_tool(
        app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "insert_id":insert,"parameters":parameters}),
    )
}

/// Opens a project whose first track plays a second of a sine at half
/// scale and, with `trigger`, whose second plays a second of noise.
fn tone_project(app: &mut Harness, file: &str, frequency: f32, trigger: bool) -> (u64, u64) {
    let write = |name: &str, sample: &mut dyn FnMut(usize) -> f32| {
        let path = app.directory.path().join(name);
        AudioData {
            sample_rate: 48_000,
            channels: 2,
            samples: (0..48_000)
                .flat_map(|index| {
                    let value = sample(index);
                    [value, value]
                })
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        path
    };
    let tone = write("tone.wav", &mut |index| {
        0.5 * (std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin()
    });
    let mut seed = 0x2545_f491_u32;
    let noise = write("trigger.wav", &mut |_| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32 - 0.5) * 0.5
    });
    let mut project = Project::new();
    project.tempo = 120.0;
    let pad = project.add_audio_track("Pad");
    project.track_mut(pad).unwrap().gain = 1.0;
    project.track_mut(pad).unwrap().clips.push(Clip::audio(
        "Pad tone",
        0.0,
        2.0,
        tone.to_string_lossy(),
    ));
    let mut source = 0;
    if trigger {
        source = project.add_audio_track("Trigger");
        project.track_mut(source).unwrap().gain = 1.0;
        project.track_mut(source).unwrap().clips.push(Clip::audio(
            "Noise burst",
            0.0,
            2.0,
            noise.to_string_lossy(),
        ));
    }
    let path = app.directory.path().join(file);
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    (pad, source)
}

fn insert_device(app: &mut Harness, track: u64, name: &str, device: &str) -> (Value, Value) {
    let listed = song_tool(app, "list_plugins", json!({"query":name}));
    let plugin = &listed["tool_result"]["result"]["plugins"][0];
    assert_eq!(plugin["id"], format!("rill:{device}"), "{listed}");
    assert_eq!(plugin["format"], "Built-in");
    assert_eq!(plugin["is_instrument"], false);
    let (id, revision) = session(&listed);
    let assigned = song_tool(
        app,
        "assign_plugin",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "plugin_id":format!("rill:{device}"),"insert":true}),
    );
    assert_eq!(
        assigned["project"]["tracks"][0]["inserts"][0]["effect"],
        json!({"Builtin":{"device":device}})
    );
    let insert = assigned["tool_result"]["result"]["insert_id"].clone();
    (assigned, insert)
}

fn open_effect_editor(app: &mut Harness, clip: &str, label: &str) {
    app.command(json!({"command":"viewport","width":1480,"height":960,"pixels_per_point":1.0}));
    app.click(clip);
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(app, label);
}

/// Clicks the last drawn label with this text: the one on top.
fn click_top(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|entry| entry["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    let center = |axis: usize| {
        (rect["min"][axis].as_f64().unwrap() + rect["max"][axis].as_f64().unwrap()) * 0.5
    };
    app.command(json!({"command":"pointer","phase":"click","x":center(0),"y":center(1)}))
}

#[test]
fn r1_saturator_adds_harmonics_holds_level_and_drives_from_its_panel() {
    let mut app = Harness::new();
    let (track, _) = tone_project(&mut app, "saturator.rill", 55.0, false);
    let dry = analysis(&mut app);
    let rms = |analysis: &Value| analysis["rms_dbfs"].as_f64().unwrap();
    // The top band against the whole: the analysis bands overlap, so some
    // of a pure tone is there too.
    let harmonics = |analysis: &Value| band(analysis, "above_4000_hz") - rms(analysis);
    let (assigned, insert) = insert_device(&mut app, track, "R1 Saturator", "r1-saturator");

    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"drive"}),
    );
    let drive = &described["tool_result"]["result"]["parameters"][0];
    assert_eq!(drive["key"], "drive", "{described}");
    assert_eq!(
        (&drive["unit"], &drive["min"], &drive["max"]),
        (&json!("dB"), &json!(0.0), &json!(36.0))
    );
    assert!(described["tool_result"]["result"]["sidechain_input"].is_null());

    // Hard drive into the transistor curve: a sine grows harmonics far above
    // itself.
    let set = set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"model","value":"transistor"},{"parameter":"drive","value":"30 dB"},
            {"parameter":"auto_gain","value":false},{"parameter":"mix","value":100}]),
    );
    assert_eq!(
        set["tool_result"]["result"]["parameters"][0]["value"],
        "Transistor"
    );
    let driven = analysis(&mut app);
    assert!(
        harmonics(&driven) > harmonics(&dry) + 6.0,
        "dry {dry}, driven {driven}"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));

    // Auto gain takes the level of the drive back; the harmonics stay.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"auto_gain","value":true}]),
    );
    let held = analysis(&mut app);
    assert!(
        rms(&held) < rms(&driven) - 6.0,
        "driven {driven}, held {held}"
    );
    assert!(harmonics(&held) > harmonics(&dry) + 6.0, "{held}");
    // Mix at 0 % is the dry signal.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"mix","value":"0 %"}]),
    );
    let bypassed = analysis(&mut app);
    assert!(
        (harmonics(&bypassed) - harmonics(&dry)).abs() < 1.0,
        "{bypassed}"
    );
    assert!(
        (rms(&bypassed) - rms(&dry)).abs() < 0.5,
        "dry {dry}, mix 0 {bypassed}"
    );

    // A factory preset loads in one Undo step.
    let before = app.inspect();
    let (id, revision) = session(&before);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"fuzz box","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    let fuzz = analysis(&mut app);
    // Its Focus keeps the bass tone below the fuzz; the level still moves.
    assert!((rms(&fuzz) - rms(&dry)).abs() > 0.3, "{fuzz}");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));

    // The device has no sidechain to route to.
    let rejected = apply_edits(
        &mut app,
        json!([{"op":"set_plugin_inputs","track":track,"effect":insert,"inputs":[
            {"source_track":track,"bus":1,"first_channel":0,"gain":1.0,"pre_fader":false}]}]),
    );
    assert_eq!(rejected["tool_result"]["ok"], false, "{rejected}");
    assert_eq!(rejected["project"], loaded["project"]);

    // The panel: Drive is the large knob, drawn right under its name.
    open_effect_editor(&mut app, "Pad tone", "Drive");
    let state = app.inspect();
    let (x, name) = label_center(&state, "Drive");
    let knob = name + 12.0 + f64::from(daw_devices_ui::KNOB_HERO) * 0.5;
    let drive = |state: &Value| {
        state["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == 2)
            .map(|parameter| parameter["value"].as_f64().unwrap())
    };
    let dragged = app.command(
        json!({"command":"pointer","phase":"drag","x":x,"y":knob,"to_x":x,"to_y":knob + 40.0}),
    );
    assert!(drive(&dragged) < drive(&state), "{}", dragged["project"]);
    assert_eq!(dragged["history"]["undo"][0], "Effect parameter");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], state["project"]);
    // Model is a row of buttons; Character is named for what it does there.
    assert!(visible(&undone, "Hardness"), "{}", undone["visible_labels"]);
    let tube = app.click("Tube");
    assert!(visible(&tube, "Bias"), "{}", tube["visible_labels"]);
    assert_eq!(tube["history"]["undo"][0], "Effect parameter");

    // Save and reopen: the same device, the same sound.
    let saved = analysis(&mut app);
    let path = app.directory.path().join("saturated.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        tube["project"]["tracks"][0]["inserts"]
    );
    let again = analysis(&mut app);
    assert!((rms(&again) - rms(&saved)).abs() < 0.1, "{saved} {again}");
    app.quit();
}

#[test]
fn r1_compressor_reduces_limits_and_ducks_under_a_routed_sidechain() {
    let mut app = Harness::new();
    let (track, trigger) = tone_project(&mut app, "compressor.rill", 110.0, true);
    // The tone is all of the song below 250 Hz but for a trace of the noise.
    let low = |app: &mut Harness| band(&analysis(app), "below_250_hz");
    let dry = low(&mut app);
    let (assigned, insert) = insert_device(&mut app, track, "R1 Compressor", "r1-compressor");
    // The default: 4:1 above −18 dB, no makeup.
    let squeezed = low(&mut app);
    assert!(squeezed < dry - 4.0, "dry {dry} dB, default {squeezed} dB");

    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"threshold"}),
    );
    let result = &described["tool_result"]["result"];
    let threshold = &result["parameters"][0];
    assert_eq!(threshold["key"], "threshold", "{described}");
    assert_eq!(
        (&threshold["unit"], &threshold["min"], &threshold["max"]),
        (&json!("dB"), &json!(-60.0), &json!(0.0))
    );
    assert_eq!(result["sidechain_input"]["bus"], 1, "{described}");

    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"threshold","value":"-40 dB"},{"parameter":"ratio","value":10}]),
    );
    let crushed = low(&mut app);
    assert!(
        crushed < squeezed - 6.0,
        "default {squeezed} dB, -40 dB at 10:1 {crushed} dB"
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);

    // Limiter: nothing of the tone passes the ceiling, and the lookahead's
    // latency is compensated, so it still starts on the beat.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"mode","value":"limiter"},{"parameter":"ceiling","value":"-12 dB"}]),
    );
    let limited = song_tool(
        &mut app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0,"track_id":track}),
    );
    let peak = limited["tool_result"]["result"]["sample_peak_dbfs"]
        .as_f64()
        .unwrap();
    assert!((-14.0..=-11.9).contains(&peak), "{limited}");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);

    // Listening to its sidechain with nothing routed there, it hears silence.
    let (id, revision) = session(&undone);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"sidechain duck","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    let unrouted = low(&mut app);
    assert!(
        (unrouted - dry).abs() < 0.5,
        "dry {dry} dB, unrouted {unrouted} dB"
    );

    // Only bus 1 takes a source; routed there, the noise ducks the tone.
    let route = |bus: u32| {
        json!([{"op":"set_plugin_inputs","track":track,"effect":insert,"inputs":[
            {"source_track":trigger,"bus":bus,"first_channel":0,"gain":1.0,"pre_fader":false}]}])
    };
    let rejected = apply_edits(&mut app, route(2));
    assert_eq!(rejected["tool_result"]["ok"], false, "{rejected}");
    assert_eq!(
        rejected["tool_result"]["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(rejected["project"], loaded["project"]);
    let routed = apply_edits(&mut app, route(1));
    assert_eq!(routed["tool_result"]["ok"], true, "{routed}");
    let inputs = |state: &Value| state["project"]["tracks"][0]["inserts"][0]["inputs"].clone();
    assert_eq!(inputs(&routed)[0]["source_track"], trigger);
    let ducked = low(&mut app);
    assert!(ducked < dry - 6.0, "dry {dry} dB, ducked {ducked} dB");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], loaded["project"]);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], routed["project"]);

    // Save and reopen: the route and the ducking come back.
    let path = app.directory.path().join("ducked.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        routed["project"]["tracks"][0]["inserts"]
    );
    assert!((low(&mut app) - ducked).abs() < 0.2);

    // The panel names the source under Sidechain; clicking it opens Routing
    // on this device, where the mouse removes and adds sources.
    open_effect_editor(&mut app, "Pad tone", "Threshold");
    let state = app.inspect();
    assert!(visible(&state, "External"), "{}", state["visible_labels"]);

    // The curve's handle is the Threshold: its "T" stands up and to the
    // left of it. Dragged to the left it lowers, in one Undo step.
    let mark = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == "T")
        .unwrap_or_else(|| panic!("No handle: {}", state["visible_labels"]));
    let (x, y) = (
        mark["max"][0].as_f64().unwrap() + 8.0,
        mark["max"][1].as_f64().unwrap() + 8.0,
    );
    let threshold = |state: &Value| {
        state["project"]["tracks"][0]["inserts"][0]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == 3)
            .map(|parameter| parameter["value"].as_f64().unwrap())
            .unwrap_or_else(|| panic!("No threshold: {}", state["project"]))
    };
    let dragged = app
        .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x - 40.0,"to_y":y}));
    assert!(
        threshold(&dragged) < threshold(&state) - 0.1,
        "{}",
        dragged["project"]
    );
    assert_eq!(dragged["history"]["undo"][0], "Effect parameter");
    // A lower threshold under the same trigger ducks deeper.
    assert!(low(&mut app) < ducked - 1.0);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], state["project"]);
    let routing = click_top(&mut app, "Trigger");
    assert_eq!(routing["popups"]["routing"], true, "{}", routing["popups"]);
    assert!(
        visible(&routing, "Sidechain"),
        "{}",
        routing["visible_labels"]
    );
    let removed = app.click("Remove");
    assert_eq!(inputs(&removed), json!([]));
    assert_eq!(removed["history"]["undo"][0], "Route plug-in inputs");
    assert!((low(&mut app) - dry).abs() < 0.5);
    app.click("Add source…");
    let added = click_top(&mut app, "Trigger");
    assert_eq!(inputs(&added), inputs(&routed), "{}", added["project"]);
    assert!(low(&mut app) < dry - 6.0);
    app.quit();
}

/// The level between two beats of the song.
fn level_between(app: &mut Harness, start: f64, end: f64) -> f64 {
    let state = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":start,"end_beat":end}),
    );
    state["tool_result"]["result"]["rms_dbfs"]
        .as_f64()
        .unwrap_or(-200.0)
}

/// The stored value of a parameter of the first track's first insert.
fn stored(state: &Value, id: u32) -> Option<f64> {
    state["project"]["tracks"][0]["inserts"][0]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["id"] == id)
        .map(|parameter| parameter["value"].as_f64().unwrap())
}

#[test]
fn r1_modulator_combs_sweeps_and_takes_mode_defaults_from_its_panel() {
    let mut app = Harness::new();
    let (track, _) = tone_project(&mut app, "modulator.rill", 500.0, false);
    let dry = level_between(&mut app, 0.5, 2.0);
    let (assigned, insert) = insert_device(&mut app, track, "R1 Modulator", "r1-modulator");

    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"center.flanger"}),
    );
    let center = &described["tool_result"]["result"]["parameters"][0];
    assert_eq!(center["key"], "center.flanger", "{described}");
    assert_eq!(center["unit"], "ms");
    assert!(described["tool_result"]["result"]["sidechain_input"].is_null());

    // A still flanger is a comb: with 1 ms of delay its first notch sits on
    // the 500 Hz tone. A tool sets only what it names: Feedback stays at 0.
    let set = set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"mode","value":"flanger"},{"parameter":"depth","value":"0 %"},
            {"parameter":"center.flanger","value":"1 ms"},{"parameter":"mix","value":"50 %"}]),
    );
    assert_eq!(
        set["tool_result"]["result"]["parameters"][0]["value"],
        "Flanger"
    );
    assert_eq!(
        stored(&set, daw_devices::devices::modulator::FEEDBACK),
        None
    );
    let notch = level_between(&mut app, 0.5, 2.0);
    assert!(notch < dry - 12.0, "dry {dry}, notch {notch}");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    // At 2 ms the tone sits on a tooth instead.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"center.flanger","value":"2 ms"}]),
    );
    let tooth = level_between(&mut app, 0.5, 2.0);
    assert!(tooth > notch + 12.0, "notch {notch}, tooth {tooth}");
    assert!(tooth > dry - 1.0, "dry {dry}, tooth {tooth}");

    // Depth sweeps the comb across the tone: the level moves in time.
    let spread = |app: &mut Harness| {
        let levels: Vec<f64> = (1..4)
            .map(|quarter| {
                level_between(app, f64::from(quarter) * 0.5, f64::from(quarter + 1) * 0.5)
            })
            .collect();
        levels.iter().copied().fold(f64::MIN, f64::max)
            - levels.iter().copied().fold(f64::MAX, f64::min)
    };
    let still = spread(&mut app);
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"depth","value":"25 %"},{"parameter":"rate","value":"0.5 Hz"},
            {"parameter":"center.flanger","value":"1.5 ms"}]),
    );
    let swept = spread(&mut app);
    assert!(
        still < 1.0 && swept > still + 3.0,
        "still {still}, swept {swept}"
    );
    // Mix at 0 % is the dry signal.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"mix","value":"0 %"}]),
    );
    let bypassed = level_between(&mut app, 0.5, 2.0);
    assert!((bypassed - dry).abs() < 0.5, "dry {dry}, mix 0 {bypassed}");

    // A factory preset loads in one Undo step.
    let before = app.inspect();
    let (id, revision) = session(&before);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"wide chorus","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));

    // The panel: choosing a mode there also sets that mode's Feedback and
    // Center, as one step.
    open_effect_editor(&mut app, "Pad tone", "Character");
    let state = app.inspect();
    assert!(visible(&state, "Voices"), "{}", state["visible_labels"]);
    let phaser = app.click("Phaser");
    assert!(visible(&phaser, "Stages"), "{}", phaser["visible_labels"]);
    use daw_devices::devices::modulator::{DESCRIPTOR, FEEDBACK, MODE};
    assert_eq!(stored(&phaser, MODE), Some(1.0));
    let feedback = DESCRIPTOR.parameter(FEEDBACK).unwrap();
    let plain = feedback.to_plain(stored(&phaser, FEEDBACK).unwrap() as f32);
    assert!((plain - 50.0).abs() < 0.01, "{}", phaser["project"]);
    assert_eq!(phaser["history"]["undo"][0], "Effect parameter");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], state["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));

    // Save and reopen: the same device, the same sound.
    let saved = level_between(&mut app, 0.5, 2.0);
    let path = app.directory.path().join("modulated.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        phaser["project"]["tracks"][0]["inserts"]
    );
    let again = level_between(&mut app, 0.5, 2.0);
    assert!((again - saved).abs() < 0.1, "{saved} {again}");
    app.quit();
}

#[test]
fn r1_spacer_rings_longer_with_decay_and_feedback_and_switches_from_its_panel() {
    let mut app = Harness::new();
    let (track, _) = tone_project(&mut app, "spacer.rill", 440.0, false);
    let dry = level_between(&mut app, 0.0, 2.0);
    // The tone ends at beat 2; what sounds a second later is the effect.
    let tail = |app: &mut Harness| level_between(app, 4.0, 5.0);
    assert!(tail(&mut app) < -90.0);
    let (assigned, insert) = insert_device(&mut app, track, "R1 Spacer", "r1-spacer");

    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"reverb.decay"}),
    );
    let decay = &described["tool_result"]["result"]["parameters"][0];
    assert_eq!(decay["key"], "reverb.decay", "{described}");
    assert_eq!(decay["unit"], "s");

    // Reverb alone: a longer Decay leaves more a second after the sound.
    let set = set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"echo.on","value":false},{"parameter":"reverb.decay","value":"0.4 s"},
            {"parameter":"mix","value":"50 %"}]),
    );
    assert_eq!(
        set["tool_result"]["result"]["parameters"][0]["value"],
        false
    );
    let short = tail(&mut app);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], assigned["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"reverb.decay","value":"6 s"}]),
    );
    let long = tail(&mut app);
    assert!(long > short + 20.0, "short {short}, long {long}");
    assert!(long > -60.0, "{long}");

    // Echo alone: one repeat is over by then; Feedback keeps them coming.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"reverb.on","value":false},{"parameter":"echo.on","value":true},
            {"parameter":"echo.sync","value":false},{"parameter":"echo.time","value":"250 ms"},
            {"parameter":"echo.feedback","value":"0 %"}]),
    );
    let once = tail(&mut app);
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"echo.feedback","value":"80 %"}]),
    );
    let repeating = tail(&mut app);
    assert!(
        repeating > once + 20.0,
        "once {once}, repeating {repeating}"
    );
    assert!(repeating > -50.0, "{repeating}");
    // Mix at 0 % is the dry signal.
    set_parameters(
        &mut app,
        track,
        &insert,
        json!([{"parameter":"mix","value":"0 %"}]),
    );
    let bypassed = level_between(&mut app, 0.0, 2.0);
    assert!((bypassed - dry).abs() < 0.5, "dry {dry}, mix 0 {bypassed}");
    assert!(tail(&mut app) < -90.0);

    // A factory preset loads in one Undo step.
    let before = app.inspect();
    let (id, revision) = session(&before);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"large hall","track_id":track,"insert_id":insert,
            "session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    assert!(tail(&mut app) > -60.0);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));

    // The panel: each section's title is its switch.
    open_effect_editor(&mut app, "Pad tone", "Routing");
    let state = app.inspect();
    use daw_devices::devices::spacer::REVERB_ON;
    assert_eq!(stored(&state, REVERB_ON), Some(1.0), "{}", state["project"]);
    let off = click_top(&mut app, "REVERB");
    assert_eq!(stored(&off, REVERB_ON), Some(0.0), "{}", off["project"]);
    assert_eq!(off["history"]["undo"][0], "Effect parameter");
    assert!(tail(&mut app) < -90.0);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], state["project"]);

    // Save and reopen: the same device, the same sound.
    let saved = tail(&mut app);
    let path = app.directory.path().join("spaced.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["inserts"],
        state["project"]["tracks"][0]["inserts"]
    );
    let again = tail(&mut app);
    assert!((again - saved).abs() < 0.1, "{saved} {again}");
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
        json!([
            "R1 Compressor",
            "R1 Equalizer",
            "R1 Filter",
            "R1 Modulator",
            "R1 Saturator",
            "R1 Spacer"
        ])
    );
    builtin_details(&mut app, "R1 Filter", "rill:r1-filter", false);
    app.quit();
}
