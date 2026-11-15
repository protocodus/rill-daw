//! Composable sample players through the running application, including the
//! production renderer, editor gestures, Undo and portable project recall.
mod common;

use common::Harness;
use daw_core::{
    AudioData, Clip, DeviceSample, Instrument, Note, PlayerKind, PlayerNode, Project, VelocityChild,
};
use serde_json::{Value, json};

fn tool(app: &mut Harness, name: &str, mut arguments: Value) -> Value {
    let state = app.inspect();
    arguments["session_id"] = state["song_session"]["id"].clone();
    arguments["expected_revision"] = state["song_session"]["revision"].clone();
    let state = app.command(json!({"command":"song_tool", "name":name, "arguments":arguments}));
    assert_eq!(
        state["tool_result"]["ok"], true,
        "{name}: {}",
        state["tool_result"]
    );
    state
}

fn instrument(state: &Value, track: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == track)
        .unwrap()["instrument"]["Builtin"]
}

fn render(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn fundamental(audio: &AudioData, start: f64) -> f64 {
    let first = (start * audio.sample_rate as f64) as usize;
    let count = (0.08 * audio.sample_rate as f64) as usize;
    let samples: Vec<_> = audio
        .samples
        .chunks_exact(audio.channels)
        .skip(first)
        .take(count)
        .map(|frame| frame[0])
        .collect();
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    crossings as f64 / 0.08
}

fn tone(app: &Harness, name: &str, hz: f32, padded: bool) -> std::path::PathBuf {
    let path = app.directory.path().join(name);
    let samples = (0..24_000)
        .map(|frame| {
            if padded && !(4_800..14_400).contains(&frame) {
                0.0
            } else {
                0.4 * (std::f32::consts::TAU * hz * frame as f32 / 48_000.0).sin()
            }
        })
        .collect();
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples,
    }
    .write_wav(&path)
    .unwrap();
    path
}

#[test]
fn nested_velocity_round_robin_and_random_players_render_and_recall_in_both_instruments() {
    for device in ["r1-sampler", "r1-drums"] {
        let mut app = Harness::new();
        let mut project = Project::new();
        project.tempo = 120.0;
        let track = project.add_instrument_track("Nested players");
        project
            .set_instrument(track, Instrument::builtin(device))
            .unwrap();
        for slot in 0..12 {
            let path = tone(
                &app,
                &format!("tone-{slot}.wav"),
                200.0 + slot as f32 * 100.0,
                false,
            );
            project
                .load_device_sample(
                    track,
                    DeviceSample {
                        slot,
                        path: path.to_string_lossy().into_owned(),
                        name: format!("Tone {slot}"),
                    },
                )
                .unwrap();
        }
        let note = if device == "r1-drums" { 36 } else { 60 };
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Test hits",
            0.0,
            6.0,
            [30, 30, 85, 85, 127, 127]
                .into_iter()
                .enumerate()
                .map(|(index, velocity)| Note::new(note, index as f64, 0.35, velocity))
                .collect(),
        ));
        let path = app.directory.path().join("nested.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        let mut id = 1;
        let mut next = || {
            let value = id;
            id += 1;
            value
        };
        let root_id = next();
        let bands = (0..3)
            .map(|band| {
                let band_id = next();
                let rounds = (0..2)
                    .map(|round| {
                        let round_id = next();
                        let children = (0..2)
                            .map(|variation| {
                                PlayerNode::sample(next(), band * 4 + round * 2 + variation)
                            })
                            .collect();
                        PlayerNode {
                            id: round_id,
                            name: String::new(),
                            kind: PlayerKind::Random { children },
                        }
                    })
                    .collect();
                VelocityChild {
                    min: [1, 64, 106][band as usize],
                    max: [63, 105, 127][band as usize],
                    child: PlayerNode {
                        id: band_id,
                        name: format!("Band {}", band + 1),
                        kind: PlayerKind::RoundRobin { children: rounds },
                    },
                }
            })
            .collect();
        let tree = PlayerNode {
            id: root_id,
            name: "Expressive instrument".into(),
            kind: PlayerKind::Velocity { children: bands },
        };
        tool(
            &mut app,
            "set_sample_player",
            json!({"track_id":track,"slot":0,"player":tree}),
        );
        let audio = render(&mut app, "nested.wav");
        for hit in 0..6 {
            let hz = fundamental(&audio, hit as f64 * 0.5 + 0.04);
            let first_slot = (hit / 2) * 4 + (hit % 2) * 2;
            let first_hz = 200.0 + first_slot as f64 * 100.0;
            assert!(
                (hz - first_hz).abs() < 20.0 || (hz - first_hz - 100.0).abs() < 20.0,
                "{device}, hit {hit}: {hz} Hz is outside the selected velocity/round-robin branch"
            );
        }
        let portable = app.directory.path().join("portable.rill");
        app.command(json!({"command":"save", "path":portable}));
        app.command(json!({"command":"open", "path":portable}));
        let recalled = render(&mut app, "recalled.wav");
        assert_eq!(
            audio.samples, recalled.samples,
            "{device}: reset and portable recall preserve the entire nested random sequence"
        );
        assert_eq!(
            instrument(&app.inspect(), track)["sample_players"]["0"],
            json!(tree)
        );
        app.quit();
    }
}

#[test]
fn a_user_can_build_nested_players_and_trim_a_leaf_without_changing_its_source() {
    let mut app = Harness::new();
    let source = tone(&app, "padded.wav", 440.0, true);
    let original = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let track = project.add_instrument_track("Sample editor");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project
        .load_device_sample(
            track,
            DeviceSample {
                slot: 0,
                path: source.to_string_lossy().into_owned(),
                name: "Padded tone".into(),
            },
        )
        .unwrap();
    let path = app.directory.path().join("editor.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1500,"height":1050,"pixels_per_point":1.0}));
    app.click("Sample editor");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Edit player…");
    app.click("Edit player…");
    common::wait_visible(&mut app, "Wrap in");
    app.click("Wrap in");
    let wrapped = app.click("Velocity");
    assert!(instrument(&wrapped, track)["sample_players"]["0"]["kind"]["Velocity"].is_object());
    app.click("Padded tone  ·  Sample  ·  1–127");
    app.click("Wrap in");
    app.click("Round robin");
    app.click("Padded tone  ·  Sample");
    app.click("Wrap in");
    app.click("Random");
    app.click("Padded tone  ·  Sample");
    common::wait_until(&mut app, |state| {
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Sample actions")
    });
    app.click("Sample actions");
    let trimmed = app.click("Trim silence");
    let root = instrument(&trimmed, track)["sample_players"]["0"].clone();
    let leaf = &root["kind"]["Velocity"]["children"][0]["child"]["kind"]["RoundRobin"]["children"]
        [0]["kind"]["Random"]["children"][0]["kind"]["Sample"];
    assert!(
        (leaf["start"].as_f64().unwrap() - 0.2).abs() < 0.01,
        "{leaf}"
    );
    assert!((leaf["end"].as_f64().unwrap() - 0.6).abs() < 0.01, "{leaf}");
    app.click("Sample actions");
    let reversed = app.click("Reverse audible region");
    assert_ne!(instrument(&reversed, track)["sample_players"]["0"], root);
    app.key("Escape");
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        instrument(&app.inspect(), track)["sample_players"]["0"],
        root
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    app.click("Edit player…");
    app.click("Sample actions");
    app.click("Export sample…");
    let exported = app.directory.path().join("trimmed-source.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":exported}));
    app.click("Bounce…");
    app.command(json!({"command":"wait_export"}));
    let wav = AudioData::load_wav(&exported).unwrap();
    let seconds = wav.frames() as f64 / wav.sample_rate as f64;
    assert!(
        (seconds - 0.202).abs() < 0.002,
        "Leaf export must use its visible crop, got {seconds}s"
    );
    app.quit();
}

#[test]
fn named_composite_pad_accepts_a_new_child_atomically_and_rejects_bad_files() {
    let mut app = Harness::new();
    let source = tone(&app, "first.wav", 220.0, false);
    let additional = tone(&app, "additional.wav", 660.0, false);
    let broken = app.directory.path().join("broken.wav");
    std::fs::write(&broken, b"not an audio recording").unwrap();
    let mut project = Project::new();
    let track = project.add_instrument_track("Drum editor");
    project
        .set_instrument(track, Instrument::builtin("r1-drums"))
        .unwrap();
    project
        .load_device_sample(
            track,
            DeviceSample {
                slot: 0,
                path: source.to_string_lossy().into_owned(),
                name: "Source name".into(),
            },
        )
        .unwrap();
    project
        .set_sample_player(
            track,
            0,
            Some(PlayerNode {
                id: 1,
                name: String::new(),
                kind: PlayerKind::All {
                    children: vec![PlayerNode::sample(2, 0)],
                },
            }),
        )
        .unwrap();
    let path = app.directory.path().join("pad.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    app.click("Drum editor");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Rename…");
    app.click("Rename…");
    app.command(json!({"command":"text","text":"Layered kick"}));
    let renamed = app.click("Apply name");
    assert_eq!(
        instrument(&renamed, track)["sample_names"]["0"],
        "Layered kick"
    );
    assert_eq!(
        instrument(&renamed, track)["samples"][0]["name"],
        "Source name"
    );
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "Layered kick")
        .unwrap();
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let before = state["project"].clone();
    app.command(json!({"command":"drop_at","paths":[additional],"x":x,"y":y}));
    let loaded = app.command(json!({"command":"wait_audio_import"}));
    let tree = &instrument(&loaded, track)["sample_players"]["0"]["kind"]["All"]["children"];
    assert_eq!(tree.as_array().unwrap().len(), 2, "{tree}");
    assert_eq!(
        instrument(&loaded, track)["samples"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        app.inspect()["project"],
        before,
        "One Undo restores the entire import and tree"
    );
    let response = app.raw_command(json!({"command":"drop_at","paths":[broken],"x":x,"y":y}));
    let rejected = if response["ok"] == false {
        response["state"].clone()
    } else {
        app.raw_command(json!({"command":"wait_audio_import"}))["state"].clone()
    };
    assert_eq!(
        rejected["project"], before,
        "Failed decoding must not append an empty player"
    );
    assert_eq!(rejected["error"], true);
    app.quit();
}

#[test]
fn project_samples_drag_into_leaf_rows_in_both_player_editors() {
    for device in ["r1-sampler", "r1-drums"] {
        let mut app = Harness::new();
        let source = tone(&app, "drag-source.wav", 440.0, false);
        let mut project = Project::new();
        let track = project.add_instrument_track("Drag target");
        project
            .set_instrument(track, Instrument::builtin(device))
            .unwrap();
        project
            .add_project_sample("Reusable bell", source.to_string_lossy())
            .unwrap();
        let path = app.directory.path().join("drag.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"viewport","width":1500,"height":1050}));
        app.click("Drag target");
        app.click("Window");
        app.click("Instrument");
        common::wait_visible(&mut app, "Edit player…");
        app.click("Edit player…");
        let state = common::wait_until(&mut app, |state| {
            state["sample_player"]["palette"]["samples"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        });
        let from = &state["sample_player"]["palette"]["samples"][0];
        let to = &state["sample_player"]["rows"][0];
        let center = |rect: &Value| {
            (
                (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
                (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
            )
        };
        let (x, y) = center(from);
        let (tx, ty) = center(to);
        app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":tx,"to_y":ty}));
        let loaded = app.command(json!({"command":"wait_audio_import"}));
        assert_eq!(
            instrument(&loaded, track)["samples"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "{device}"
        );
        assert_eq!(loaded["project"]["samples"].as_array().unwrap().len(), 1);
        assert_eq!(
            instrument(&loaded, track)["samples"][0]["name"],
            "Reusable bell"
        );
        assert_eq!(
            instrument(&loaded, track)["samples"][0]["path"],
            loaded["project"]["samples"][0]["path"]
        );
        assert_eq!(loaded["popups"]["sample_player"], true);
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert!(instrument(&undone, track)["samples"].is_null());
        assert_eq!(undone["project"]["samples"], loaded["project"]["samples"]);
        app.quit();
    }
}
