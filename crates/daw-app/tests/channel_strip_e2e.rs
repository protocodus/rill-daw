//! Every mixer channel runs an R1 Strip before its inserts: the filter
//! knobs and stage buttons on the strip, its window, and the tool.
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

/// A band's level; silence reads as a floor instead of no number.
fn band(app: &mut Harness, band: &str) -> f64 {
    let state = song_tool(
        app,
        "audio_analyze",
        json!({"start_beat":0.0,"end_beat":2.0}),
    );
    state["tool_result"]["result"]["band_rms_estimates_dbfs"][band]
        .as_f64()
        .unwrap_or(-200.0)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn strip_value(state: &Value, id: u32) -> Option<f64> {
    state["project"]["tracks"][0]["strip"]
        .as_array()?
        .iter()
        .find(|value| value["id"] == id)
        .and_then(|value| value["value"].as_f64())
}

/// One audio track playing a second of white noise.
fn noise_project(app: &mut Harness) -> u64 {
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
    let path = app.directory.path().join("strip.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    track
}

#[test]
fn a_mixer_channel_filters_through_its_strip_and_opens_it() {
    use daw_devices::devices::strip::{COMP_ON, HP_FREQUENCY, HP_ON, LP_ON};
    let mut app = Harness::new();
    let track = noise_project(&mut app);
    let dry_low = band(&mut app, "below_250_hz");
    let dry_high = band(&mut app, "above_4000_hz");

    // The tool sets the channel's strip in the parameters' own units.
    let before = app.inspect();
    let (id, revision) = session(&before);
    let set = song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":track,
            "channel_strip":true,"parameters":[{"parameter":"hp.on","value":true},
            {"parameter":"hp.frequency","value":"1000 Hz"},{"parameter":"hp.slope","value":"24 dB"}]}),
    );
    assert_eq!(set["history"]["undo"][0], "Device parameters");
    assert_eq!(strip_value(&set, HP_ON), Some(1.0));
    let cut = band(&mut app, "below_250_hz");
    assert!(
        cut < dry_low - 8.0,
        "dry {dry_low}, cut {cut}: {}",
        set["project"]["tracks"][0]
    );
    // Width and the rest of the channel's own controls stay its own.
    let (id, revision) = session(&set);
    let refused = app.command(json!({"command":"song_tool","name":"set_device_parameters",
        "arguments":{"session_id":id,"expected_revision":revision,"track_id":track,
        "channel_strip":true,"parameters":[{"parameter":"width","value":0}]}}));
    assert_eq!(refused["tool_result"]["ok"], false, "{refused}");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);

    // On the mixer the low-pass knob, turned down, cuts the highs; its
    // double-click switches it off again.
    let mixer = app.click("Mixer");
    let controls = &mixer["mixer_strips"][0]["strip"];
    assert_eq!(mixer["mixer_strips"][0]["track"], track, "{mixer}");
    // Both knobs stay within the channel, which the stage buttons span.
    for knob in ["high_pass", "low_pass"] {
        assert!(
            controls[knob]["min"][0].as_f64().unwrap()
                >= controls["eq"]["min"][0].as_f64().unwrap()
        );
        assert!(
            controls[knob]["max"][0].as_f64().unwrap()
                <= controls["comp"]["max"][0].as_f64().unwrap() + 0.5
        );
    }
    let (x, y) = center(&controls["low_pass"]);
    let knob = x + 12.0;
    let turned = app.command(
        json!({"command":"pointer","phase":"drag","x":knob,"y":y,"to_x":knob,"to_y":y + 60.0}),
    );
    assert_eq!(
        strip_value(&turned, LP_ON),
        Some(1.0),
        "{}",
        turned["project"]
    );
    assert!(band(&mut app, "above_4000_hz") < dry_high - 8.0);
    let reset = app.command(json!({"command":"pointer","phase":"double_click","x":knob,"y":y}));
    assert_eq!(strip_value(&reset, LP_ON), Some(0.0));
    assert_eq!(strip_value(&reset, HP_FREQUENCY), None);

    // Comp opens the channel's strip window; its lamp switches the stage,
    // and the button lights.
    let (x, y) = center(&controls["comp"]);
    let opened = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(opened["popups"]["channel_strip"], true, "{opened}");
    assert!(
        opened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "COMPRESSOR"),
        "{}",
        opened["visible_labels"]
    );
    let lamp = opened["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|entry| entry["text"] == "COMPRESSOR")
        .unwrap();
    let (x, y) = center(lamp);
    let switched = app.command(json!({"command":"pointer","phase":"click","x":x - 60.0,"y":y}));
    assert_eq!(
        strip_value(&switched, COMP_ON),
        Some(1.0),
        "{}",
        switched["project"]
    );
    app.quit();
}
