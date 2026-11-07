//! Freeze and Return to originals use the real background renderer and document
//! history, including private media, channel automation and native multi-outs.
mod common;
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
#[allow(dead_code)]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Effect, EffectSlot,
    Instrument, Note, PluginInputRoute, Project, Send,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn track(state: &Value, id: u64) -> &Value {
    if id == 0 {
        return &state["project"]["master"];
    }
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}
fn source(app: &Harness) -> PathBuf {
    let path = app.directory.path().join("untouched-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..240_000).flat_map(|_| [0.2, -0.1]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}
fn open(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("freeze-source.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}
fn wait(app: &mut Harness) -> Value {
    app.command(json!({"command":"wait_freeze","timeout_ms":30_000}))
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn same_audio(expected: &AudioData, actual: &AudioData) {
    assert_eq!(actual.sample_rate, expected.sample_rate);
    assert_eq!(actual.channels, expected.channels);
    assert_eq!(actual.samples.len(), expected.samples.len());
    let difference = expected
        .samples
        .iter()
        .zip(&actual.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        difference < 2e-6,
        "Freeze changed rendered audio by {difference}"
    );
}
fn history(app: &mut Harness, redo: bool) -> Value {
    app.click("Edit");
    app.click(if redo { "Redo" } else { "Undo" })
}
fn edits(app: &mut Harness, edits: Value) -> Value {
    let before = app.inspect();
    let state=app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":before["song_session"]["id"],"expected_revision":before["song_session"]["revision"],
        "label":"Edit preserved original","edits":edits}}));
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    state
}
fn audio_project(app: &Harness) -> (Project, u64, u64, u64, PathBuf) {
    let path = source(app);
    let mut project = Project::new();
    project.tempo = 120.0;
    let source = project.add_audio_track("Source");
    let first = Clip::audio("First phrase", 2.0, 2.0, path.to_string_lossy());
    let second = Clip::audio("Second phrase", 6.0, 1.0, path.to_string_lossy());
    let (first_id, second_id) = (first.id, second.id);
    let channel = project.track_mut(source).unwrap();
    channel.gain = 0.6;
    channel.pan = 0.3;
    channel.input_gain = 0.5;
    channel.clips.extend([first, second]);
    (project, source, first_id, second_id, path)
}
fn rendered_track(state: &Value, source: u64) -> u64 {
    state["freeze"]["completed"]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|output| output["source_track"] == source)
        .unwrap()["track"]
        .as_u64()
        .unwrap()
}
fn media_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        if !path.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "wav") {
                files.insert(path);
            }
        }
    }
    files
}

#[test]
fn cancel_or_replace_during_native_snapshot_never_starts_the_delayed_freeze() {
    let worker = tempfile::tempdir().unwrap();
    let source = worker.path().join("freeze_snapshot_host.rs");
    let metadata =
        include_str!("common/parameter_host.rs").replace("fn main() {", "fn metadata_main() {");
    let editor = include_str!("common/native_parameter_editor.rs").replace(
        "let request = number(&command, \"request_id\")",
        "let gate = executable.with_file_name(\"hold-freeze-snapshot\"); while gate.exists() { std::thread::sleep(Duration::from_millis(2)); } let request = number(&command, \"request_id\")",
    );
    std::fs::write(&source, format!("{metadata}\n{editor}")).unwrap();
    let host = worker.path().join(if cfg!(windows) {
        "freeze-snapshot-host.exe"
    } else {
        "freeze-snapshot-host"
    });
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&host)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let plugins = common::plugin_fixture();
    for replace in [false, true] {
        let mut app = Harness::with_plugin_host(plugins.path(), &host);
        let mut project = Project::new();
        let track_id = project.add_midi_track("Native source");
        let channel = project.track_mut(track_id).unwrap();
        channel.instrument = Instrument::Vst3 {
            path: plugins
                .path()
                .join("RillFixture.vst3")
                .to_string_lossy()
                .into_owned(),
            class_id: Some(CLASS_ID.into()),
        };
        channel.instrument_state = vec![40];
        channel
            .clips
            .push(Clip::midi("Native phrase", 0.0, 4.0, vec![]));
        open(&mut app, &project);
        app.click("Native phrase");
        app.click("Window");
        app.click("Instrument");
        app.click("Open editor");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = app.inspect();
            if state["native_editor"]["open"] == true
                && state["native_editor"]["parameter_count"] == 3
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Native editor did not open: {state}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        app.click("Window");
        let before = app.click("Arranger");
        let gate = worker.path().join("hold-freeze-snapshot");
        std::fs::write(&gate, "hold").unwrap();
        let pending = app.command(json!({"command":"freeze","track":track_id,"wait":false}));
        assert_eq!(pending["freeze"]["busy"], true);
        assert!(pending["freeze"]["preparing"].is_number());
        assert_eq!(pending["project"], before["project"]);
        if replace {
            let empty = app.directory.path().join("replace-native.daw.json");
            Project::new().save(&empty).unwrap();
            app.command(json!({"command":"open","path":empty,"wait":false}));
        } else {
            app.click("Cancel Freeze");
        }
        std::fs::remove_file(gate).unwrap();
        let finished = if replace {
            app.command(json!({"command":"wait_project"}))
        } else {
            wait(&mut app)
        };
        assert_eq!(finished["freeze"]["busy"], false);
        assert!(finished["freeze"]["completed"].is_null());
        if replace {
            assert!(finished["project"]["tracks"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(finished["project"], before["project"]);
            assert_eq!(finished["history"], before["history"]);
        }
        assert!(
            media_files(&PathBuf::from(finished["scratch_dir"].as_str().unwrap()).join("freezes"))
                .is_empty()
        );
        app.quit();
    }
}

#[test]
fn clip_freeze_preserves_position_mix_automation_sources_and_one_undo() {
    let mut app = Harness::new();
    let (mut project, source, first, second, path) = audio_project(&app);
    let bytes = std::fs::read(&path).unwrap();
    let pre = project.add_bus("Pre send");
    let post = project.add_bus("Post send");
    project.track_mut(source).unwrap().sends = vec![
        Send {
            target_bus: pre,
            gain: 0.25,
            pre_fader: true,
        },
        Send {
            target_bus: post,
            gain: 0.5,
            pre_fader: false,
        },
    ];
    project.master.input_gain = 0.75;
    project.master.gain = 0.8;
    project.master.pan = -0.15;
    for parameter in [
        AutomationParameter::TrackGain,
        AutomationParameter::TrackPan,
        AutomationParameter::SendGain { bus: pre },
    ] {
        let id = project
            .add_automation_track(
                "Mix envelope",
                AutomationTarget {
                    track: source,
                    parameter,
                },
            )
            .unwrap();
        project.automation_lane_mut(id).unwrap().points = vec![
            AutomationPoint::new(
                0.0,
                parameter.normalize(if parameter == AutomationParameter::TrackPan {
                    -0.2
                } else {
                    0.4
                }),
            ),
            AutomationPoint::new(
                8.0,
                parameter.normalize(if parameter == AutomationParameter::TrackPan {
                    0.3
                } else {
                    0.8
                }),
            ),
        ];
    }
    let before = open(&mut app, &project);
    let baseline = export(&mut app, "before.wav");
    // The mouse context command exercises the same public workflow as the
    // driver command used for deterministic background/failure tests below.
    let region = before["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == first)
        .unwrap();
    let rect = &region["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = rect["min"][1].as_f64().unwrap() * 0.2 + rect["max"][1].as_f64().unwrap() * 0.8;
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    app.click("Freeze clip");
    let frozen = wait(&mut app);
    let printed = rendered_track(&frozen, source);
    assert_eq!(frozen["project"]["tracks"][1]["id"], printed);
    assert_eq!(track(&frozen, source)["clips"][0]["muted"], true);
    assert_eq!(track(&frozen, source)["clips"][1]["id"], second);
    assert_eq!(track(&frozen, source)["clips"][1]["muted"], false);
    assert_eq!(track(&frozen, source)["mute"], false);
    let channel = track(&frozen, printed);
    assert_eq!(channel["input_gain"], 1.0);
    assert_eq!(channel["gain"], track(&before, source)["gain"]);
    assert_eq!(channel["pan"], track(&before, source)["pan"]);
    assert!(channel["inserts"].as_array().unwrap().is_empty());
    assert_eq!(channel["clips"][0]["start"], 2.0);
    assert_eq!(channel["clips"][0]["length"], 2.0);
    assert_eq!(channel["clips"][0]["freeze"]["active"], true);
    let managed = PathBuf::from(
        channel["clips"][0]["content"]["Audio"]["path"]
            .as_str()
            .unwrap(),
    );
    assert!(managed.is_file());
    assert_eq!(frozen["history"]["undo"].as_array().unwrap().len(), 1);
    same_audio(&baseline, &export(&mut app, "after.wav"));
    assert_eq!(history(&mut app, false)["project"], before["project"]);
    assert!(managed.is_file(), "Undo must retain media needed by Redo");
    assert_eq!(history(&mut app, true)["project"], frozen["project"]);
    let returned = app.command(json!({"command":"unfreeze","track":printed}));
    assert_eq!(track(&returned, source)["clips"][0]["muted"], false);
    assert_eq!(track(&returned, printed)["clips"][0]["muted"], true);
    assert_eq!(
        track(&returned, printed)["clips"][0]["freeze"]["active"],
        false
    );
    same_audio(&baseline, &export(&mut app, "returned.wav"));
    assert_eq!(history(&mut app, false)["project"], frozen["project"]);
    let archive = app.directory.path().join("frozen.rill");
    app.command(json!({"command":"save","path":archive}));
    std::fs::remove_file(&path).unwrap();
    let reopened = app.command(json!({"command":"open","path":archive}));
    assert_eq!(
        track(&reopened, printed)["clips"][0]["freeze"]["active"],
        true
    );
    app.command(json!({"command":"unfreeze","track":source}));
    same_audio(&baseline, &export(&mut app, "portable-return.wav"));
    // The original was only removed deliberately after the portable save.
    let original_in_bundle = track(&reopened, source)["clips"][0]["content"]["Audio"]["path"]
        .as_str()
        .unwrap();
    assert_eq!(std::fs::read(original_in_bundle).unwrap(), bytes);
    app.quit();
}

#[test]
fn track_freeze_is_contiguous_and_return_restores_split_originals_without_deleting_print_edits() {
    let mut app = Harness::new();
    let (project, source, first, _, _) = audio_project(&app);
    let before = open(&mut app, &project);
    let baseline = export(&mut app, "track-before.wav");
    let header = before["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["track"] == source)
        .unwrap();
    let rect = &header["name"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    app.click("Freeze track");
    let frozen = wait(&mut app);
    let printed = rendered_track(&frozen, source);
    assert_eq!(track(&frozen, source)["mute"], true);
    assert!(
        track(&frozen, source)["clips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|clip| clip["muted"] == true)
    );
    assert_eq!(track(&frozen, printed)["mute"], false);
    assert_eq!(track(&frozen, printed)["clips"][0]["start"], 2.0);
    assert_eq!(track(&frozen, printed)["clips"][0]["length"], 5.0);
    same_audio(&baseline, &export(&mut app, "track-after.wav"));
    let printed_clip = track(&frozen, printed)["clips"][0]["id"].as_u64().unwrap();
    let edited = edits(
        &mut app,
        json!([
            {"op":"split_clip","track":source,"clip":first,"beat":3.0},
            {"op":"split_clip","track":printed,"clip":printed_clip,"beat":4.0}
        ]),
    );
    assert_eq!(track(&edited, source)["clips"].as_array().unwrap().len(), 3);
    assert_eq!(
        track(&edited, printed)["clips"].as_array().unwrap().len(),
        2
    );
    let returned = app.command(json!({"command":"unfreeze","track":printed}));
    assert_eq!(track(&returned, source)["mute"], false);
    assert!(
        track(&returned, source)["clips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|clip| clip["muted"] == false)
    );
    assert!(
        track(&returned, printed)["clips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|clip| clip["muted"] == true)
    );
    assert_eq!(
        track(&returned, printed)["clips"].as_array().unwrap().len(),
        2
    );
    same_audio(&baseline, &export(&mut app, "track-return.wav"));
    assert_eq!(history(&mut app, false)["project"], edited["project"]);
    assert_eq!(history(&mut app, true)["project"], returned["project"]);
    app.quit();
}

#[test]
fn overlapping_audio_requires_whole_track_freeze_to_preserve_the_overlap_winner() {
    let mut app = Harness::new();
    let (mut project, source, first, _, _) = audio_project(&app);
    project.track_mut(source).unwrap().clips[1].start = 3.0;
    let before = open(&mut app, &project);
    let baseline = export(&mut app, "overlap-before.wav");
    let rejected = app.raw_command(json!({"command":"freeze","track":source,"clips":[first]}));
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert_eq!(rejected["state"]["project"], before["project"]);
    assert_eq!(rejected["state"]["history"], before["history"]);
    assert!(rejected["error"].as_str().unwrap().contains("Freeze track"));
    let frozen = app.command(json!({"command":"freeze","track":source}));
    assert_eq!(
        frozen["freeze"]["completed"]["outputs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    same_audio(&baseline, &export(&mut app, "overlap-after.wav"));
    app.quit();
}

#[test]
fn native_multi_output_freeze_preserves_sidechain_consumers_and_master_processing() {
    let instrument = EditorProbeFixture::build_multi_bus();
    let effect = EditorProbeFixture::build_multi_bus_effect();
    let mut app = Harness::with_vst_path(&instrument.path);
    let path = source(&app);
    let mut project = Project::new();
    let input = project.add_audio_track("Sidechain source");
    project.track_mut(input).unwrap().gain = 1.0;
    project.track_mut(input).unwrap().clips.push(Clip::audio(
        "Input",
        2.0,
        2.0,
        path.to_string_lossy(),
    ));
    let owner = project.add_midi_track("Multi instrument");
    let channel = project.track_mut(owner).unwrap();
    channel.instrument = Instrument::Vst3 {
        path: instrument.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    channel.gain = 0.8;
    channel.pan = 0.1;
    channel.input_gain = 0.4;
    channel.instrument_inputs = vec![PluginInputRoute {
        source_track: input,
        bus: 1,
        first_channel: 2,
        gain: 1.0,
        pre_fader: true,
    }];
    channel.clips.push(Clip::midi(
        "Phrase",
        2.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let children = project
        .add_instrument_outputs(
            owner,
            vec![
                ("Pair 1".into(), 1, 0),
                ("Pair 2".into(), 1, 2),
                ("Odd mono".into(), 1, 4),
                ("Mono bus".into(), 2, 0),
            ],
        )
        .unwrap();
    for id in &children {
        project.track_mut(*id).unwrap().gain = 0.25;
    }
    let consumer = project.add_audio_track("Duck consumer");
    project.track_mut(consumer).unwrap().clips.push(Clip::audio(
        "Consumer",
        0.0,
        8.0,
        path.to_string_lossy(),
    ));
    let mut slot = EffectSlot::new(
        "Sidechain effect",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    slot.inputs = vec![PluginInputRoute {
        source_track: owner,
        bus: 1,
        first_channel: 0,
        gain: 0.5,
        pre_fader: false,
    }];
    let effect_id = slot.id;
    project.track_mut(consumer).unwrap().inserts.push(slot);
    let mut master = EffectSlot::new(
        "Master effect",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    master.inputs = vec![PluginInputRoute {
        source_track: children[3],
        bus: 1,
        first_channel: 0,
        gain: 0.2,
        pre_fader: true,
    }];
    project.master.inserts.push(master);
    open(&mut app, &project);
    let baseline = export(&mut app, "native-before.wav");
    let frozen = app.command(json!({"command":"freeze","track":owner}));
    assert_eq!(
        frozen["freeze"]["completed"]["outputs"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    for id in std::iter::once(owner).chain(children.iter().copied()) {
        assert_eq!(track(&frozen, id)["mute"], true);
        let printed = rendered_track(&frozen, id);
        assert_eq!(track(&frozen, printed)["clips"][0]["start"], 2.0);
        assert_eq!(track(&frozen, printed)["mute"], false);
        let path = track(&frozen, printed)["clips"][0]["content"]["Audio"]["path"]
            .as_str()
            .unwrap();
        assert!(
            AudioData::load_wav(path)
                .unwrap()
                .samples
                .iter()
                .any(|sample| sample.abs() > 0.0)
        );
    }
    assert_eq!(
        track(&frozen, consumer)["inserts"][0]["inputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        track(&frozen, 0)["inserts"][0]["inputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let unavailable = instrument.path.with_extension("temporarily-unavailable");
    std::fs::rename(&instrument.path, &unavailable).unwrap();
    // A frozen source is pruned before native loading; the audio and its
    // downstream sidechains remain usable without the original instrument.
    same_audio(&baseline, &export(&mut app, "native-after.wav"));
    let missing = app.raw_command(json!({"command":"unfreeze","track":owner}));
    assert_eq!(missing["ok"], false, "{missing}");
    assert_eq!(missing["state"]["project"], frozen["project"]);
    assert_eq!(missing["state"]["history"], frozen["history"]);
    std::fs::rename(unavailable, &instrument.path).unwrap();
    let retry = app.command(json!({"command":"unfreeze","track":owner}));
    assert_eq!(track(&retry, owner)["mute"], false);
    assert_eq!(history(&mut app, false)["project"], frozen["project"]);
    // Edited copied routes are retained as user work on Return. The untouched
    // Master copy is removed, leaving its original native sidechain route.
    let mut routes = track(&frozen, consumer)["inserts"][0]["inputs"].clone();
    routes[1]["gain"] = json!(0.3);
    edits(
        &mut app,
        json!([{"op":"set_plugin_inputs","track":consumer,"effect":effect_id,"inputs":routes}]),
    );
    let returned = app.command(json!({"command":"unfreeze","track":owner}));
    assert_eq!(
        track(&returned, consumer)["inserts"][0]["inputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        (track(&returned, consumer)["inserts"][0]["inputs"][1]["gain"]
            .as_f64()
            .unwrap()
            - 0.3)
            .abs()
            < 1e-6
    );
    assert_eq!(
        track(&returned, 0)["inserts"][0]["inputs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(track(&returned, owner)["mute"], false);
    same_audio(&baseline, &export(&mut app, "native-return.wav"));
    app.quit();
}

#[test]
fn freeze_cancel_stale_failure_and_removed_original_never_publish_partial_work() {
    let mut app = Harness::with_freeze_delay(300);
    let (project, source, first, _, _) = audio_project(&app);
    let before = open(&mut app, &project);
    let scratch = PathBuf::from(before["scratch_dir"].as_str().unwrap()).join("freezes");
    app.command(json!({"command":"freeze","track":source,"wait":false}));
    app.click("Cancel Freeze");
    let canceled = wait(&mut app);
    assert_eq!(canceled["project"], before["project"]);
    assert_eq!(canceled["history"], before["history"]);
    assert!(media_files(&scratch).is_empty());
    let empty = app.directory.path().join("empty.daw.json");
    Project::new().save(&empty).unwrap();
    app.command(json!({"command":"freeze","track":source,"wait":false}));
    let replaced = app.command(json!({"command":"open","path":empty}));
    assert!(replaced["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(replaced["freeze"]["busy"], false);
    assert!(media_files(&scratch).is_empty());
    open(&mut app, &project);
    app.command(json!({"command":"freeze","track":source,"wait":false}));
    let edited = edits(
        &mut app,
        json!([{"op":"set_mix","channels":[{"track":source,"gain":0.9}]}]),
    );
    let stale = app.raw_command(json!({"command":"wait_freeze"}));
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["state"]["project"], edited["project"]);
    assert_eq!(stale["state"]["history"], edited["history"]);
    assert!(media_files(&scratch).is_empty());
    let frozen = app.command(json!({"command":"freeze","track":source,"clips":[first]}));
    let printed = rendered_track(&frozen, source);
    let deleted = edits(
        &mut app,
        json!([{"op":"delete_clip","track":source,"clip":first}]),
    );
    let failed_return = app.raw_command(json!({"command":"unfreeze","track":printed}));
    assert_eq!(failed_return["ok"], false, "{failed_return}");
    assert_eq!(failed_return["state"]["project"], deleted["project"]);
    assert_eq!(failed_return["state"]["history"], deleted["history"]);
    assert_eq!(
        track(&failed_return["state"], printed)["clips"][0]["muted"],
        false
    );
    // An unreadable source also leaves every original and history step intact.
    app.command(json!({"command":"open","path":empty}));
    let mut broken = Project::new();
    let bad = broken.add_audio_track("Broken source");
    broken.track_mut(bad).unwrap().clips.push(Clip::audio(
        "Missing",
        0.0,
        1.0,
        "missing-freeze-source.wav",
    ));
    let initial = open(&mut app, &broken);
    let failure = app.raw_command(json!({"command":"freeze","track":bad}));
    assert_eq!(failure["ok"], false, "{failure}");
    assert_eq!(failure["state"]["project"], initial["project"]);
    assert_eq!(failure["state"]["history"], initial["history"]);
    app.quit();
}
