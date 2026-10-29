//! Contextual AI uses the production assistant, tool dispatcher and editor UI.
//! Scripted provider responses deliberately request both valid and foreign edits.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationTarget, Clip, Effect, EffectSlot, Instrument, Note,
    PluginParameterValue, Project,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn menu(app: &mut Harness, item: &str) -> Value {
    app.click("Window");
    app.click(item)
}

fn local_ai(app: &mut Harness, kind: &str) -> Value {
    let before = app.inspect();
    let occurrence = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == "AI for this context")
        .count()
        .checked_sub(1)
        .expect("contextual AI control visible");
    let state = app
        .command(json!({"command":"click","label":"AI for this context","occurrence":occurrence}));
    assert_eq!(
        state["assistant"]["scope"]["kind"], kind,
        "{}",
        state["assistant"]
    );
    let scope = state["assistant"]["scope_label"]
        .as_str()
        .expect("visible scope");
    assert!(
        visible(&state, &format!("Scope · {scope}")),
        "{}",
        state["visible_labels"]
    );
    state
}

fn prompt(app: &mut Harness, text: &str) {
    let state = app.inspect();
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}));
}

fn run(app: &mut Harness, calls: Vec<Value>) -> Value {
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":calls}, {"text":"Finished the scoped request."}
    ]}));
    prompt(app, "Refine only the open context.");
    app.click("Send");
    let state = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(state["assistant"]["busy"], false);
    state
}

fn call(name: &str, arguments: Value) -> Value {
    json!({"id":name,"name":name,"arguments":arguments})
}

fn edits(state: &Value, changes: Value) -> Value {
    call(
        "apply_edits",
        json!({
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "label":"AI: scoped fixture", "edits":changes
        }),
    )
}

fn unchanged(actual: &Value, expected: &Value) {
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(
            actual[field], expected[field],
            "unexpected {field} mutation"
        );
    }
}

fn tool_names(state: &Value) -> Vec<&str> {
    state["assistant"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect()
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn clip(state: &Value, track_id: u64, clip_id: u64) -> &Value {
    track(state, track_id)["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == clip_id)
        .unwrap()
}

fn midi_fixture(ghost: bool) -> (Harness, u64, u64, u64, u64) {
    let mut app = Harness::new();
    let mut project = Project::new();
    let a = project.add_midi_track("Scope keys");
    let b = project.add_midi_track("Untouched bass");
    let phrase = Clip::midi("Scope phrase", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let first = phrase.id;
    project.track_mut(a).unwrap().clips.push(phrase);
    let second = if ghost {
        let id = project.clone_clip(a, first).unwrap();
        project.move_clip(a, id, 8.0).unwrap();
        id
    } else {
        let other = Clip::midi(
            "Neighbor phrase",
            8.0,
            4.0,
            vec![Note::new(67, 0.0, 1.0, 70)],
        );
        let id = other.id;
        project.track_mut(a).unwrap().clips.push(other);
        id
    };
    project.track_mut(b).unwrap().clips.push(Clip::midi(
        "Bass phrase",
        0.0,
        4.0,
        vec![Note::new(36, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("scope-midi.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Scope phrase");
    menu(&mut app, "Clip Editor");
    (app, a, first, second, b)
}

#[test]
fn piano_roll_scope_filters_tools_edits_one_clip_and_global_ai_resets_the_boundary() {
    let (mut app, channel, selected, neighbor, bass) = midi_fixture(false);
    let initial = app.inspect();
    let opened = local_ai(&mut app, "midi_clip");
    assert_eq!(opened["assistant"]["scope"]["track_id"], channel);
    assert_eq!(opened["assistant"]["scope"]["clip_id"], selected);
    assert!(
        opened["assistant"]["scope_label"]
            .as_str()
            .unwrap()
            .contains("Scope phrase")
    );
    let names = tool_names(&opened);
    assert!(names.contains(&"apply_edits"));
    assert!(!names.contains(&"set_mix"));
    assert!(!names.contains(&"compose_progression"));
    let examples = app.click("Examples");
    assert!(visible(&examples, "Humanize this phrase"));
    assert!(!visible(&examples, "Start a song"));
    unchanged(&examples, &initial);
    let browser = app.click("Tools");
    assert_eq!(browser["assistant"]["page"], "Tools");
    assert!(visible(&browser, "Find a tool"));
    app.click("Search tools…");
    let found = app.command(json!({"command":"text","text":"composer_scale"}));
    assert!(visible(&found, "composer_scale"));
    assert!(visible(&found, "1 tool"));
    let inserted = app.click("Use tool");
    assert_eq!(inserted["assistant"]["page"], "Conversation");
    assert!(!inserted["assistant"]["prompt"].as_str().unwrap().is_empty());
    assert!(
        inserted["assistant"]["prompt"]
            .as_str()
            .unwrap()
            .contains("composer_scale")
    );
    assert_eq!(inserted["assistant"]["busy"], false);
    unchanged(&inserted, &initial);

    let changed = run(
        &mut app,
        vec![edits(
            &initial,
            json!([
                {"op":"transform_notes","track":channel,"clip":selected,"beats":0.0,"semitones":2}
            ]),
        )],
    );
    assert_eq!(
        clip(&changed, channel, selected)["content"]["Midi"]["notes"][0]["pitch"],
        62
    );
    assert_eq!(
        clip(&changed, channel, neighbor),
        clip(&initial, channel, neighbor)
    );
    assert_eq!(track(&changed, bass), track(&initial, bass));
    assert_eq!(changed["history"]["undo"].as_array().unwrap().len(), 1);
    let outbound = changed["assistant"]["request_prompt"].as_str().unwrap();
    assert!(outbound.to_lowercase().contains("scope"), "{outbound}");
    assert!(outbound.contains(&selected.to_string()), "{outbound}");

    let selection = app.command(
        json!({"command":"song_tool","name":"set_selection","arguments":{"track_id":bass}}),
    );
    assert_eq!(selection["tool_result"]["ok"], true);
    assert_eq!(
        selection["assistant"]["scope"],
        opened["assistant"]["scope"]
    );
    let changed = run(
        &mut app,
        vec![edits(
            &selection,
            json!([
                {"op":"transform_notes","track":channel,"clip":selected,"beats":0.0,"semitones":1}
            ]),
        )],
    );
    assert_eq!(
        clip(&changed, channel, selected)["content"]["Midi"]["notes"][0]["pitch"],
        63
    );
    assert_eq!(track(&changed, bass), track(&initial, bass));

    // A single batch containing one valid operation and one foreign operation
    // must reject everything before touching the real document or undo stack.
    let denied = run(
        &mut app,
        vec![edits(
            &changed,
            json!([
                {"op":"transform_notes","track":channel,"clip":selected,"beats":0.0,"semitones":3},
                {"op":"transform_notes","track":channel,"clip":neighbor,"beats":0.0,"semitones":3}
            ]),
        )],
    );
    unchanged(&denied, &changed);
    assert_eq!(
        denied["assistant"]["last_tool_error"]["code"],
        "scope_violation"
    );
    let denied = run(
        &mut app,
        vec![edits(
            &denied,
            json!([
                {"op":"set_mix","channels":[{"track":bass,"gain":0.2}]}
            ]),
        )],
    );
    unchanged(&denied, &changed);
    let local_undo = app.click("Undo");
    unchanged(&local_undo, &changed);

    // The global toolbar always opens the whole song, even from a piano roll.
    app.key("Escape");
    let global = app.click("AI");
    assert_eq!(global["assistant"]["scope"]["kind"], "song");
    assert_eq!(global["assistant"]["conversation"], json!([]));
    assert_eq!(global["assistant"]["messages"], json!([]));
    assert!(tool_names(&global).contains(&"set_mix"));
    assert!(tool_names(&global).contains(&"compose_progression"));
    let mixed = run(
        &mut app,
        vec![edits(
            &global,
            json!([
                {"op":"set_mix","channels":[{"track":channel,"gain":0.5},{"track":bass,"gain":0.25}]}
            ]),
        )],
    );
    assert_eq!(track(&mixed, channel)["gain"], 0.5);
    assert_eq!(track(&mixed, bass)["gain"], 0.25);
    app.quit();
}

#[test]
fn linked_midi_edits_cannot_change_other_occurrences_without_explicit_independence() {
    let (mut app, channel, selected, linked, _) = midi_fixture(true);
    let initial = local_ai(&mut app, "midi_clip");
    let denied = run(
        &mut app,
        vec![edits(
            &initial,
            json!([
                {"op":"transform_notes","track":channel,"clip":selected,"beats":0.0,"semitones":12}
            ]),
        )],
    );
    unchanged(&denied, &initial);
    let independent = run(
        &mut app,
        vec![edits(
            &denied,
            json!([
                {"op":"make_independent","track":channel,"clip":selected}
            ]),
        )],
    );
    assert!(clip(&independent, channel, selected)["link_group"].is_null());
    assert_eq!(
        clip(&independent, channel, linked),
        clip(&initial, channel, linked)
    );
    let changed = run(
        &mut app,
        vec![edits(
            &independent,
            json!([
                {"op":"transform_notes","track":channel,"clip":selected,"beats":0.0,"semitones":12}
            ]),
        )],
    );
    assert_eq!(
        clip(&changed, channel, selected)["content"]["Midi"]["notes"][0]["pitch"],
        72
    );
    assert_eq!(
        clip(&changed, channel, linked),
        clip(&initial, channel, linked)
    );
    app.quit();
}

#[test]
fn composition_dialog_ai_uses_the_same_clip_boundary_in_both_appearances() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let (mut app, channel, selected, neighbor, _) = midi_fixture(false);
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        let baseline = app.inspect();
        app.click("Generate…");
        let scoped = local_ai(&mut app, "midi_clip");
        assert_eq!(scoped["assistant"]["scope"]["track_id"], channel);
        assert_eq!(scoped["assistant"]["scope"]["clip_id"], selected);
        let tools = app.click("Tools");
        for text in ["Tools", "Find a tool", "Use tool", "Send"] {
            assert!(
                visible(&tools, text),
                "Missing {text}: {}",
                tools["visible_labels"]
            );
            for rect in tools["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|rect| rect["text"] == text)
            {
                assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
                assert!(rect["max"][0].as_f64().unwrap() <= 800.0);
                assert!(rect["max"][1].as_f64().unwrap() <= 600.0);
            }
        }
        unchanged(&tools, &baseline);
        let changed = run(
            &mut app,
            vec![edits(
                &scoped,
                json!([
                    {"op":"add_notes","track":channel,"clip":selected,"notes":[{"pitch":64,"start":1.0,"length":1.0,"velocity":90}]}
                ]),
            )],
        );
        assert_eq!(
            clip(&changed, channel, selected)["content"]["Midi"]["notes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            clip(&changed, channel, neighbor),
            clip(&baseline, channel, neighbor)
        );
        app.quit();
    }
}

#[test]
fn mixer_routing_and_automation_headers_pin_their_own_edit_boundaries() {
    for kind in ["channel", "routing", "automation"] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let channel = project.add_midi_track("Local channel");
        let other = project.add_midi_track("Foreign channel");
        let bus = project.add_bus("Scope bus");
        project.track_mut(channel).unwrap().clips.push(Clip::midi(
            "Local phrase",
            0.0,
            4.0,
            vec![],
        ));
        let lane = project
            .add_automation_track(
                "Local pan lane",
                AutomationTarget {
                    track: channel,
                    parameter: AutomationParameter::TrackPan,
                },
            )
            .unwrap();
        let path = app.directory.path().join(format!("{kind}-scope.rill"));
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.click(if kind == "automation" {
            "Local pan lane"
        } else {
            "Local phrase"
        });
        menu(
            &mut app,
            match kind {
                "channel" => "Mixer",
                "routing" => "Routing",
                _ => "Automation",
            },
        );
        let initial = local_ai(&mut app, kind);
        assert_eq!(initial["assistant"]["scope"]["track_id"], channel);
        let (valid, foreign) = match kind {
            "channel" => (
                json!({"op":"set_mix","channels":[{"track":channel,"gain":0.5}]}),
                json!({"op":"set_mix","channels":[{"track":other,"gain":0.25}]}),
            ),
            "routing" => (
                json!({"op":"set_output","track":channel,"bus":bus}),
                json!({"op":"set_output","track":other,"bus":bus}),
            ),
            _ => {
                assert_eq!(initial["assistant"]["scope"]["lane_id"], lane);
                (
                    json!({"op":"upsert_automation_points","track":lane,"points":[[2.0,0.2],[4.0,0.8]]}),
                    json!({"op":"set_mix","channels":[{"track":other,"gain":0.25}]}),
                )
            }
        };
        let denied = run(
            &mut app,
            vec![edits(&initial, json!([valid.clone(), foreign]))],
        );
        unchanged(&denied, &initial);
        let changed = run(&mut app, vec![edits(&denied, json!([valid]))]);
        assert_ne!(
            changed["project"], initial["project"],
            "valid {kind} operation must commit"
        );
        assert_eq!(track(&changed, other), track(&initial, other));
        assert_eq!(changed["history"]["undo"].as_array().unwrap().len(), 1);
        app.quit();
    }
}

#[test]
fn audio_clip_scope_normalizes_only_the_open_occurrence_and_rejects_foreign_edits() {
    let mut app = Harness::new();
    let source = app.directory.path().join("shared-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.25; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let channel = project.add_audio_track("Scope audio");
    let first = Clip::audio("Scope recording", 0.0, 2.0, source.to_string_lossy());
    let second = Clip::audio("Same source elsewhere", 4.0, 2.0, source.to_string_lossy());
    let selected = first.id;
    let neighbor = second.id;
    project
        .track_mut(channel)
        .unwrap()
        .clips
        .extend([first, second]);
    let path = app.directory.path().join("scope-audio.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Scope recording");
    menu(&mut app, "Clip Editor");
    let initial = local_ai(&mut app, "audio_clip");
    assert_eq!(initial["assistant"]["scope"]["clip_id"], selected);
    assert!(tool_names(&initial).contains(&"audio_process_clip"));
    assert!(!tool_names(&initial).contains(&"compose_progression"));
    let normalized = run(
        &mut app,
        vec![call(
            "audio_process_clip",
            json!({
                "session_id":initial["song_session"]["id"],"expected_revision":initial["song_session"]["revision"],
                "track_id":channel,"clip_id":selected,"operation":"normalize","normalize_target_db":-6.0
            }),
        )],
    );
    let gain = clip(&normalized, channel, selected)["content"]["Audio"]["gain"]
        .as_f64()
        .unwrap();
    assert!(
        (gain - 10_f64.powf(-6.0 / 20.0) / 0.25).abs() < 1e-6,
        "{normalized}"
    );
    assert_eq!(
        clip(&normalized, channel, neighbor),
        clip(&initial, channel, neighbor)
    );
    assert_eq!(normalized["history"]["undo"].as_array().unwrap().len(), 1);
    let denied = run(
        &mut app,
        vec![edits(
            &normalized,
            json!([
                {"op":"set_audio_gain","track":channel,"clip":selected,"gain":0.1},
                {"op":"set_audio_gain","track":channel,"clip":neighbor,"gain":0.1}
            ]),
        )],
    );
    unchanged(&denied, &normalized);
    let denied = run(
        &mut app,
        vec![call(
            "audio_process_clip",
            json!({
                "session_id":denied["song_session"]["id"],"expected_revision":denied["song_session"]["revision"],
                "track_id":channel,"clip_id":neighbor,"operation":"normalize"
            }),
        )],
    );
    unchanged(&denied, &normalized);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    app.quit();
}

struct PluginFixture {
    worker: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}

impl PluginFixture {
    fn new(gated: bool) -> Self {
        let worker = tempfile::tempdir().unwrap();
        let mut code = include_str!("common/parameter_host.rs").to_owned();
        if gated {
            code = code.replace(
                "let mut definitions = vec![",
                r#"
                let ready = std::env::current_exe().unwrap().with_file_name("metadata-ready");
                let release = ready.with_file_name("metadata-release");
                std::fs::write(&ready, b"ready").unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !release.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let mut definitions = vec!["#,
            );
        }
        let source = worker.path().join("scope_plugin_host.rs");
        std::fs::write(&source, code).unwrap();
        let host = worker.path().join(if cfg!(windows) {
            "scope-host.exe"
        } else {
            "scope-host"
        });
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            worker,
            plugins: common::plugin_fixture(),
            host,
        }
    }

    fn app(&self) -> (Harness, u64, [u64; 2]) {
        let mut app = Harness::with_plugin_host(self.plugins.path(), &self.host);
        let mut project = Project::new();
        let channel = project.add_midi_track("Scope synth");
        let path = self
            .plugins
            .path()
            .join("RillFixture.vst3")
            .to_string_lossy()
            .into_owned();
        project
            .set_instrument(
                channel,
                Instrument::Vst3 {
                    path: path.clone(),
                    class_id: Some("11111111111111111111111111111111".into()),
                },
            )
            .unwrap();
        let track = project.track_mut(channel).unwrap();
        track
            .instrument_parameters
            .push(PluginParameterValue { id: 7, value: 0.4 });
        track.clips.push(Clip::midi(
            "Synth phrase",
            0.0,
            4.0,
            vec![Note::new(60, 0.0, 1.0, 80)],
        ));
        for (name, value) in [("Scope echo", 0.3), ("Other echo", 0.8)] {
            let mut effect = EffectSlot::new(
                name,
                Effect::Vst3 {
                    path: path.clone(),
                    class_id: Some("33333333333333333333333333333333".into()),
                },
            );
            effect
                .parameters
                .push(PluginParameterValue { id: 7, value });
            track.inserts.push(effect);
        }
        let effects = [track.inserts[0].id, track.inserts[1].id];
        let path = app.directory.path().join("scope-plugins.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.click("Synth phrase");
        (app, channel, effects)
    }
}

fn parameter(track_id: u64, effect: Option<u64>, value: f64) -> Value {
    json!({"op":"set_plugin_parameters","track":track_id,"effect":effect,"parameters":[{"id":7,"value":value}]})
}

#[test]
fn instrument_effect_and_chain_tools_are_limited_to_the_open_processor_context() {
    let fixture = PluginFixture::new(false);
    let (mut app, channel, effects) = fixture.app();
    menu(&mut app, "Instrument");
    let instrument = local_ai(&mut app, "instrument");
    assert_eq!(instrument["assistant"]["scope"]["track_id"], channel);
    assert!(tool_names(&instrument).contains(&"plugin_describe_parameters"));
    assert!(!tool_names(&instrument).contains(&"audio_process_clip"));
    let changed = run(
        &mut app,
        vec![
            call(
                "plugin_describe_parameters",
                json!({"track_id":channel,"query":"Filter Cutoff"}),
            ),
            edits(&instrument, json!([parameter(channel, None, 0.6)])),
        ],
    );
    assert_eq!(
        track(&changed, channel)["instrument_parameters"][0]["value"],
        0.6
    );
    assert_eq!(
        track(&changed, channel)["inserts"],
        track(&instrument, channel)["inserts"]
    );
    let denied = run(
        &mut app,
        vec![edits(
            &changed,
            json!([
                {"op":"set_insert_enabled","track":channel,"effect":effects[0],"enabled":false}
            ]),
        )],
    );
    unchanged(&denied, &changed);

    app.key("Escape");
    menu(&mut app, "Effects");
    // The chain overview and one effect editor expose separate edit boundaries.
    let chain = local_ai(&mut app, "effect_chain");
    assert_eq!(chain["assistant"]["conversation"], json!([]));
    let reordered = run(
        &mut app,
        vec![edits(
            &chain,
            json!([
                {"op":"reorder_insert","track":channel,"effect":effects[1],"index":0}
            ]),
        )],
    );
    assert_eq!(track(&reordered, channel)["inserts"][0]["id"], effects[1]);
    assert_eq!(
        track(&reordered, channel)["instrument_parameters"],
        track(&changed, channel)["instrument_parameters"]
    );
    let denied = run(
        &mut app,
        vec![edits(&reordered, json!([parameter(channel, None, 0.1)]))],
    );
    unchanged(&denied, &reordered);

    app.key("Escape");
    app.click("Open editor");
    let effect = local_ai(&mut app, "effect");
    let selected = effect["assistant"]["scope"]["effect_id"].as_u64().unwrap();
    assert_eq!(selected, effects[1]);
    let other = effects[0];
    let changed = run(
        &mut app,
        vec![
            call(
                "plugin_describe_parameters",
                json!({"track_id":channel,"insert_id":selected,"query":"Filter Cutoff"}),
            ),
            edits(&effect, json!([parameter(channel, Some(selected), 0.55)])),
        ],
    );
    let inserts = track(&changed, channel)["inserts"].as_array().unwrap();
    assert_eq!(
        inserts.iter().find(|slot| slot["id"] == selected).unwrap()["parameters"][0]["value"],
        0.55
    );
    let denied = run(
        &mut app,
        vec![edits(
            &changed,
            json!([
                {"op":"set_insert_enabled","track":channel,"effect":selected,"enabled":false},
                {"op":"set_insert_enabled","track":channel,"effect":other,"enabled":false}
            ]),
        )],
    );
    unchanged(&denied, &changed);
    app.quit();
}

#[test]
fn changing_documents_cancels_pending_contextual_plugin_work() {
    let fixture = PluginFixture::new(true);
    let (mut app, channel, _) = fixture.app();
    menu(&mut app, "Instrument");
    let initial = local_ai(&mut app, "instrument");
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[call("plugin_describe_parameters",json!({"track_id":channel,"query":"Filter Cutoff"}))]},
        {"calls":[edits(&initial,json!([parameter(channel,None,0.99)]))]},
        {"text":"This stale edit must never finish."}
    ]}));
    prompt(&mut app, "Adjust this instrument only.");
    app.click("Send");
    let ready = fixture.worker.path().join("metadata-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "Metadata worker never started: {}",
            app.inspect()["assistant"]
        );
        app.inspect();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.inspect()["assistant"]["busy"], true);
    let mut replacement = Project::new();
    replacement.name = "Replacement song".into();
    replacement.add_midi_track("Fresh channel");
    let path = app.directory.path().join("replacement.rill");
    replacement.save(&path).unwrap();
    let opened = app.command(json!({"command":"open","path":path}));
    std::fs::write(fixture.worker.path().join("metadata-release"), b"release").unwrap();
    let finished = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    unchanged(&finished, &opened);
    assert_eq!(finished["assistant"]["busy"], false);
    assert_eq!(finished["assistant"]["conversation"], json!([]));
    assert_eq!(finished["assistant"]["messages"], json!([]));
    assert_eq!(finished["assistant"]["prompt"], "");
    assert_ne!(
        finished["song_session"]["id"],
        initial["song_session"]["id"]
    );
    app.quit();
}

#[test]
fn replacing_an_instrument_requires_reopening_its_context_and_clears_old_conversation() {
    let fixture = PluginFixture::new(false);
    let (mut app, channel, _) = fixture.app();
    menu(&mut app, "Instrument");
    local_ai(&mut app, "instrument");
    let discussed = run(&mut app, vec![]);
    assert!(
        !discussed["assistant"]["conversation"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let replacement = loop {
        let catalog = app.command(json!({"command":"song_tool","name":"list_plugins","arguments":{"query":"Rill Fixture B"}}));
        if let Some(plugin) = catalog["tool_result"]["result"]["plugins"]
            .as_array()
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin["name"] == "Rill Fixture B")
            })
        {
            break plugin["id"].clone();
        }
        assert!(Instant::now() < deadline, "Fixture instrument did not scan");
        std::thread::sleep(Duration::from_millis(5));
    };
    let replaced = app.command(json!({"command":"song_tool","name":"assign_plugin","arguments":{
        "session_id":discussed["song_session"]["id"],"expected_revision":discussed["song_session"]["revision"],
        "track_id":channel,"plugin_id":replacement
    }}));
    assert_eq!(replaced["tool_result"]["ok"], true);
    assert_eq!(
        track(&replaced, channel)["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    let denied = run(
        &mut app,
        vec![edits(&replaced, json!([parameter(channel, None, 0.99)]))],
    );
    unchanged(&denied, &replaced);
    assert_eq!(
        denied["assistant"]["conversation"],
        discussed["assistant"]["conversation"]
    );
    app.key("Escape");
    menu(&mut app, "Instrument");
    let reopened = local_ai(&mut app, "instrument");
    assert_eq!(reopened["assistant"]["conversation"], json!([]));
    assert_eq!(reopened["assistant"]["prompt"], "");
    let changed = run(
        &mut app,
        vec![
            call(
                "plugin_describe_parameters",
                json!({"track_id":channel,"query":"Filter Cutoff"}),
            ),
            edits(&reopened, json!([parameter(channel, None, 0.2)])),
        ],
    );
    assert_eq!(
        track(&changed, channel)["instrument_parameters"][0]["value"],
        0.2
    );
    app.quit();
}

#[test]
fn deleting_the_pinned_target_cancels_pending_parameter_work_without_widening_scope() {
    let fixture = PluginFixture::new(true);
    let (mut app, channel, _) = fixture.app();
    menu(&mut app, "Instrument");
    let initial = local_ai(&mut app, "instrument");
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[call("plugin_describe_parameters",json!({"track_id":channel,"query":"Filter Cutoff"}))]},
        {"calls":[edits(&initial,json!([parameter(channel,None,0.99)]))]},
        {"text":"This deleted target must never be edited."}
    ]}));
    prompt(&mut app, "Adjust this instrument only.");
    app.click("Send");
    let ready = fixture.worker.path().join("metadata-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "Metadata worker never started");
        app.inspect();
        std::thread::sleep(Duration::from_millis(5));
    }
    let deleted = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":initial["song_session"]["id"],"expected_revision":initial["song_session"]["revision"],
        "label":"Manual target deletion","edits":[{"op":"delete_track","track":channel}]
    }}));
    assert_eq!(deleted["tool_result"]["ok"], true);
    std::fs::write(fixture.worker.path().join("metadata-release"), b"release").unwrap();
    let finished = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    unchanged(&finished, &deleted);
    assert_eq!(
        finished["assistant"]["scope"],
        initial["assistant"]["scope"]
    );
    assert_eq!(finished["assistant"]["busy"], false);
    assert!(!finished["assistant"]["last_tool_error"].is_null());
    app.quit();
}
