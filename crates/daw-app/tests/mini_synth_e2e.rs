//! Rill's built-in R1 Mini through the running app: the default instrument of
//! a new MIDI track, tools in plain units, presets, and its panel.
mod common;

use common::Harness;
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

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

/// RMS of the 250 Hz–4 kHz band over the phrase, where the filter acts.
fn mid_band_db(app: &mut Harness) -> f64 {
    let analysis = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":4.0}),
    );
    let result = &analysis["tool_result"]["result"];
    assert_eq!(result["silent"], false, "{analysis}");
    result["band_rms_estimates_dbfs"]["250_to_4000_hz"]
        .as_f64()
        .unwrap()
}

fn cutoff(state: &Value) -> Option<f64> {
    state["project"]["tracks"][0]["instrument_parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["id"] == 31)
        .map(|parameter| parameter["value"].as_f64().unwrap())
}

#[test]
fn new_midi_tracks_play_r1_mini_shaped_by_tools_presets_and_its_panel() {
    let mut app = Harness::new();
    app.key("Escape");
    let added = menu(&mut app, "Track", "Add MIDI track");
    let track = added["project"]["tracks"][0]["id"].as_u64().unwrap();
    let clip = added["project"]["tracks"][0]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    assert_eq!(
        added["project"]["tracks"][0]["instrument"],
        json!({"Builtin":{"device":"r1-mini"}}),
        "{}",
        added["project"]
    );
    // The chooser still opens to pick something else; closing it keeps R1 Mini.
    app.key("Escape");
    let (id, revision) = session(&added);
    song_tool(
        &mut app,
        "apply_edits",
        json!({"session_id":id,"expected_revision":revision,"label":"Phrase","edits":[
            {"op":"add_notes","track":track,"clip":clip,
             "notes":[{"pitch":57,"start":0.0,"length":3.5,"velocity":100},
                      {"pitch":64,"start":0.0,"length":3.5,"velocity":100}]}]}),
    );
    // Sounds out of the box, with no instrument to choose first.
    let open = mid_band_db(&mut app);
    assert!(open > -40.0, "default R1 Mini at {open} dB");

    // Tools speak the synth's units, and the filter closes audibly.
    let described = song_tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"query":"filter.cutoff"}),
    );
    let parameter = &described["tool_result"]["result"]["parameters"][0];
    assert_eq!(parameter["key"], "filter.cutoff", "{described}");
    assert_eq!(parameter["unit"], "Hz");
    let (id, revision) = session(&described);
    let before = app.inspect();
    let set = song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "parameters":[{"parameter":"filter.cutoff","value":"100 Hz"},
                          {"parameter":"filter.env","value":0},
                          {"parameter":"osc1.model","value":"dco"}]}),
    );
    let stored = &set["tool_result"]["result"]["parameters"];
    assert_eq!(stored[2]["value"], "DCO", "{set}");
    let closed = mid_band_db(&mut app);
    assert!(closed < open - 12.0, "open {open} dB, closed {closed} dB");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);

    // Factory presets load in one Undo step.
    let presets = song_tool(
        &mut app,
        "list_device_presets",
        json!({"track_id":track,"query":"bass"}),
    );
    assert!(
        presets["tool_result"]["result"]["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["name"] == "Rubber Bass" && preset["source"] == "factory"),
        "{presets}"
    );
    let (id, revision) = session(&presets);
    let loaded = song_tool(
        &mut app,
        "load_device_preset",
        json!({"name":"rubber bass","track_id":track,"session_id":id,"expected_revision":revision}),
    );
    assert_eq!(loaded["tool_result"]["result"]["skipped"], json!([]));
    assert_eq!(loaded["history"]["undo"][0], "Device preset");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);

    // The panel: drag the hero Cutoff knob, then right-click it and Automate.
    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    app.click("New pattern");
    menu(&mut app, "Window", "Instrument");
    common::wait_visible(&mut app, "Cutoff");
    let state = app.inspect();
    for label in ["Osc 1", "Voice", "Amp envelope", "LFO", "Mod wheel"] {
        assert!(
            state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|visible| visible == label),
            "Missing {label}: {}",
            state["visible_labels"]
        );
    }
    // The filter's Cutoff comes first: left of the velocity's, above the LFO's.
    let position = |entry: &Value| {
        (
            entry["min"][1].as_f64().unwrap(),
            entry["min"][0].as_f64().unwrap(),
        )
    };
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["text"] == "Cutoff")
        .min_by(|a, b| position(a).partial_cmp(&position(b)).unwrap())
        .unwrap();
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let name = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let knob = name + 12.0 + f64::from(daw_devices_ui::KNOB_HERO) * 0.5;
    let dragged = app.command(
        json!({"command":"pointer","phase":"drag","x":x,"y":knob,"to_x":x,"to_y":knob - 40.0}),
    );
    assert!(
        cutoff(&dragged) > cutoff(&state),
        "{}",
        dragged["project"]["tracks"][0]
    );
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
                == json!({"track":track,"parameter":{"VstParameter":{"effect":null,"id":31}}})
        })
        .unwrap_or_else(|| panic!("No Cutoff lane: {}", state["project"]));
    assert_eq!(state["selected_track"], lane["id"]);
    app.quit();
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn pointer(app: &mut Harness, phase: &str, (x, y): (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":x,"y":y}))
}

#[test]
fn panel_keyboard_and_wheels_play_the_track_and_let_go_when_the_panel_closes() {
    let mut app = Harness::new();
    app.key("Escape");
    let added = menu(&mut app, "Track", "Add MIDI track");
    let track = added["project"]["tracks"][0]["id"].clone();
    app.key("Escape");
    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    menu(&mut app, "Window", "Instrument");
    common::wait_visible(&mut app, "Cutoff");
    // The keyboard is the panel's last row: scroll it into view.
    let window = app.inspect()["window_rects"][0].clone();
    let (x, y) = center(&window);
    let state = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-2000.0}));
    let performance = state["native_editor"]["performance"].clone();
    assert!(
        performance["keyboard"]["max"][1].as_f64() < window["max"][1].as_f64(),
        "{performance} {window}"
    );

    // Middle C: the 15th of 28 white keys from C2, pressed near its bottom.
    let keyboard = &performance["keyboard"];
    let left = keyboard["min"][0].as_f64().unwrap();
    let key_width = (keyboard["max"][0].as_f64().unwrap() - left) / 28.0;
    let middle_c = (
        left + 14.5 * key_width,
        keyboard["max"][1].as_f64().unwrap() - 5.0,
    );
    let pressed = pointer(&mut app, "down", middle_c);
    assert_eq!(
        pressed["audition_request"][0], track,
        "{}",
        pressed["audition_request"]
    );
    assert_eq!(pressed["audition_request"][1], 60);
    let released = pointer(&mut app, "up", middle_c);
    assert!(released["audition_request"].is_null());

    // The pitch wheel bends while held and springs back on release.
    let pitch = center(&performance["pitch_wheel"]);
    pointer(&mut app, "down", pitch);
    let bent = pointer(&mut app, "move", (pitch.0, pitch.1 - 20.0));
    let wheels = &bent["native_editor"]["performance"]["wheels"];
    assert_eq!(wheels[0], track, "{wheels}");
    assert!(wheels[1].as_f64().unwrap() > 0.2, "{wheels}");
    let back = pointer(&mut app, "up", (pitch.0, pitch.1 - 20.0));
    assert_eq!(back["native_editor"]["performance"]["wheels"][1], 0.0);

    // The mod wheel stays where it is left.
    let modulation = center(&performance["mod_wheel"]);
    let moved = app.command(
        json!({"command":"pointer","phase":"drag","x":modulation.0,"y":modulation.1,
        "to_x":modulation.0,"to_y":modulation.1 - 20.0}),
    );
    let wheels = &moved["native_editor"]["performance"]["wheels"];
    assert!(wheels[2].as_f64().unwrap() > 0.2, "{wheels}");

    // Closing the panel while a key is held lets the note go.
    pointer(&mut app, "down", middle_c);
    let closed = app.key("Escape");
    assert_eq!(closed["popups"]["instrument"], false);
    assert!(
        closed["audition_request"].is_null(),
        "{}",
        closed["audition_request"]
    );
    assert!(closed["native_editor"]["performance"]["wheels"].is_null());
    pointer(&mut app, "up", middle_c);

    // Reopening reattaches the mod wheel it still shows, with the bend centred.
    menu(&mut app, "Window", "Instrument");
    common::wait_visible(&mut app, "Cutoff");
    let reopened = app.inspect();
    let wheels = &reopened["native_editor"]["performance"]["wheels"];
    assert_eq!(wheels[0], track, "{wheels}");
    assert_eq!(wheels[1], 0.0, "{wheels}");
    assert!(wheels[2].as_f64().unwrap() > 0.2, "{wheels}");
    app.quit();
}
