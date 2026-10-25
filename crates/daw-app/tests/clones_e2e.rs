//! Ghost clips through the running app: editing, independence, recall and audio.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn clips(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"].as_array().unwrap()
}
fn clip(state: &Value, id: u64) -> &Value {
    clips(state).iter().find(|clip| clip["id"] == id).unwrap()
}
fn selected(state: &Value) -> u64 {
    state["selected_clip"].as_u64().unwrap()
}
fn notes(state: &Value, id: u64) -> &Value {
    &clip(state, id)["content"]["Midi"]["notes"]
}
fn audio(state: &Value, id: u64) -> &Value {
    &clip(state, id)["content"]["Audio"]
}
fn menu(app: &mut Harness, menu: &str, action: &str) -> Value {
    let state = app.click(menu);
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == action)
        .count()
        - 1;
    app.command(json!({"command":"click","label":action,"occurrence":occurrence}))
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn pointer(app: &mut Harness, phase: &str, from: (f64, f64), to: Option<(f64, f64)>) -> Value {
    let mut command = json!({"command":"pointer","phase":phase,"x":from.0,"y":from.1});
    if let Some((x, y)) = to {
        command["to_x"] = json!(x);
        command["to_y"] = json!(y);
    }
    app.command(command)
}
fn arrangement(app: &mut Harness) -> Value {
    menu(app, "Window", "Arrangement");
    app.key("Q");
    app.click("Fit")
}
fn clip_rect(state: &Value, id: u64) -> &Value {
    &state["timeline"]["clips"]
        .as_array()
        .expect("Arranger exposes the actual painted clip rectangles")
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"]
}
fn select(app: &mut Harness, id: u64) -> Value {
    let state = arrangement(app);
    let state = pointer(app, "click", center(clip_rect(&state, id)), None);
    assert_eq!(selected(&state), id);
    state
}
fn assert_linked_badges(state: &Value, linked: &[u64], independent: &[u64]) {
    for (ids, expected) in [(linked, true), (independent, false)] {
        for id in ids {
            let painted = state["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|clip| clip["id"] == *id)
                .unwrap();
            assert_eq!(painted["linked"], expected, "Clip {id}");
            assert_eq!(
                !painted["link_icon"].is_null(),
                expected,
                "Chain icon on clip {id}"
            );
        }
    }
}
fn midi_point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let view = &state["midi_editor"];
    (
        view["plot"]["min"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap(),
        view["plot"]["min"][1].as_f64().unwrap()
            + (127.5 - f64::from(pitch) - view["row_offset"].as_f64().unwrap())
                * view["row_height"].as_f64().unwrap(),
    )
}
fn draw_note(app: &mut Harness, beat: f64, pitch: u8) -> Value {
    let state = app.inspect();
    pointer(app, "double_click", midi_point(&state, beat, pitch), None)
}
fn rename(app: &mut Harness, name: &str) -> Value {
    let state = menu(app, "Window", "Properties");
    let rect = &state["clip_name_rect"];
    let position = (
        (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5,
        (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5,
    );
    pointer(app, "click", position, None);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":name}));
    let state = app.key("Enter");
    assert_eq!(
        clip(&state, selected(&state))["name"],
        name,
        "Rename state: {}",
        json!({"rect":state["clip_name_rect"],"labels":state["visible_labels"],"error":state["error"],"status":state["status"],"clips":clips(&state)})
    );
    app.key("Escape")
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-5, "{actual} != {expected}");
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn wait_audio(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.inspect();
        if state["audio_editor"]["pending"] == false {
            assert_eq!(state["audio_editor"]["error"], "", "{state}");
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Audio operation did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn midi_linked_clones_share_notes_and_name_but_duplicate_and_unlink_are_independent() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Linked MIDI");
    let original = Clip::midi("Phrase", 0.0, 4.0, vec![Note::new(60, 1.0, 1.0, 90)]);
    let original_id = original.id;
    project.track_mut(track).unwrap().clips.push(original);
    let path = app.directory.path().join("linked-midi.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Phrase");
    let first = menu(&mut app, "Edit", "Ghost copy");
    assert_eq!(first["history"]["undo"][0], "Create ghost copy");
    let first_id = selected(&first);
    assert_eq!(clips(&first).len(), 2);
    assert!(!clip(&first, original_id)["link_group"].is_null());
    assert_eq!(
        clip(&first, original_id)["link_group"],
        clip(&first, first_id)["link_group"]
    );
    assert_eq!(clip(&first, first_id)["start"], 4.0);
    assert_eq!(
        undo(&mut app, false)["project"],
        serde_json::to_value(&project).unwrap()
    );
    assert_eq!(undo(&mut app, true)["project"], first["project"]);
    select(&mut app, first_id);
    let second_id = selected(&menu(&mut app, "Edit", "Ghost copy"));
    let copied = menu(&mut app, "Edit", "Duplicate");
    let independent_id = selected(&copied);
    assert_eq!(clips(&copied).len(), 4);
    assert!(clip(&copied, independent_id)["link_group"].is_null());
    let independent_notes = notes(&copied, independent_id).clone();
    assert_ne!(
        notes(&copied, original_id)[0]["id"],
        independent_notes[0]["id"]
    );
    let state = arrangement(&mut app);
    assert_linked_badges(
        &state,
        &[original_id, first_id, second_id],
        &[independent_id],
    );
    select(&mut app, first_id);
    let editor = menu(&mut app, "Window", "Clip Editor");
    assert!(
        editor["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!("Ghost"))
    );
    let before = app.inspect()["project"].clone();
    let edited = draw_note(&mut app, 2.0, 64);
    assert_eq!(notes(&edited, original_id).as_array().unwrap().len(), 2);
    assert_eq!(notes(&edited, original_id), notes(&edited, first_id));
    assert_eq!(notes(&edited, original_id), notes(&edited, second_id));
    assert_eq!(notes(&edited, independent_id), &independent_notes);
    assert_eq!(undo(&mut app, false)["project"], before);
    assert_eq!(undo(&mut app, true)["project"], edited["project"]);
    let renamed = rename(&mut app, "Chorus 🎹");
    for id in [original_id, first_id, second_id] {
        assert_eq!(clip(&renamed, id)["name"], "Chorus 🎹");
    }
    assert_eq!(clip(&renamed, independent_id)["name"], "Phrase");
    select(&mut app, first_id);
    let detached = menu(&mut app, "Edit", "Make independent");
    assert!(clip(&detached, first_id)["link_group"].is_null());
    assert_ne!(
        notes(&detached, first_id)[0]["id"],
        notes(&detached, original_id)[0]["id"]
    );
    assert_eq!(undo(&mut app, false)["project"], renamed["project"]);
    assert_eq!(undo(&mut app, true)["project"], detached["project"]);
    let renamed = rename(&mut app, "Solo phrase");
    assert_eq!(clip(&renamed, first_id)["name"], "Solo phrase");
    assert_eq!(clip(&renamed, original_id)["name"], "Chorus 🎹");
    select(&mut app, original_id);
    let removed = menu(&mut app, "Edit", "Delete");
    assert_eq!(clips(&removed).len(), 3);
    assert_eq!(clip(&removed, second_id)["name"], "Chorus 🎹");
    select(&mut app, second_id);
    menu(&mut app, "Window", "Clip Editor");
    let edited = draw_note(&mut app, 3.0, 67);
    assert_eq!(notes(&edited, second_id).as_array().unwrap().len(), 3);
    assert_eq!(notes(&edited, first_id).as_array().unwrap().len(), 2);
    assert_eq!(notes(&edited, independent_id), &independent_notes);
    let path = app.directory.path().join("linked-midi.rill");
    app.command(json!({"command":"save","path":path}));
    let restored = app.command(json!({"command":"open","path":path}));
    assert_eq!(restored["project"], edited["project"]);
    let project: Project = serde_json::from_value(restored["project"].clone()).unwrap();
    project.validate().unwrap();
    app.quit();
}

#[test]
fn linked_clip_move_trim_and_split_are_local_while_note_edits_reach_every_instance() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Instance placement");
    let original = Clip::midi("Pattern", 0.0, 4.0, vec![Note::new(60, 1.0, 1.0, 100)]);
    let original_id = original.id;
    project.track_mut(track).unwrap().clips.push(original);
    let path = app.directory.path().join("placement.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Pattern");
    let clone_id = selected(&menu(&mut app, "Edit", "Ghost copy"));
    let before = select(&mut app, clone_id);
    let from = center(clip_rect(&before, clone_id));
    let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let moved = pointer(&mut app, "drag", from, Some((from.0 + scale, from.1)));
    near(clip(&moved, clone_id)["start"].as_f64().unwrap(), 5.0);
    assert_eq!(clip(&moved, original_id), clip(&before, original_id));
    let state = arrangement(&mut app);
    let scale = state["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let rect = clip_rect(&state, clone_id);
    let from = (rect["max"][0].as_f64().unwrap() - 1.0, center(rect).1);
    let trimmed = pointer(
        &mut app,
        "drag",
        from,
        Some((from.0 - scale * 0.25, from.1)),
    );
    near(clip(&trimmed, clone_id)["start"].as_f64().unwrap(), 5.0);
    near(clip(&trimmed, clone_id)["length"].as_f64().unwrap(), 3.75);
    near(clip(&trimmed, clone_id)["offset"].as_f64().unwrap(), 0.0);
    assert_eq!(clip(&trimmed, original_id), clip(&before, original_id));
    assert_eq!(notes(&trimmed, original_id), notes(&trimmed, clone_id));
    assert_eq!(undo(&mut app, false)["project"], moved["project"]);
    assert_eq!(undo(&mut app, true)["project"], trimmed["project"]);
    app.key("E");
    let state = app.inspect();
    let rect = clip_rect(&state, clone_id);
    let split = pointer(
        &mut app,
        "click",
        (rect["min"][0].as_f64().unwrap() + scale, center(rect).1),
        None,
    );
    assert_eq!(clips(&split).len(), 3);
    assert_eq!(clip(&split, original_id), clip(&before, original_id));
    assert!(
        clips(&split)
            .iter()
            .all(|instance| instance["link_group"] == clip(&split, original_id)["link_group"])
    );
    select(&mut app, original_id);
    menu(&mut app, "Window", "Clip Editor");
    let changed = draw_note(&mut app, 2.5, 64);
    for instance in clips(&changed) {
        assert_eq!(
            &instance["content"]["Midi"]["notes"],
            notes(&changed, original_id)
        );
        assert_eq!(
            instance["length"],
            clip(&split, instance["id"].as_u64().unwrap())["length"]
        );
        assert_eq!(
            instance["offset"],
            clip(&split, instance["id"].as_u64().unwrap())["offset"]
        );
    }
    app.quit();
}

#[test]
fn audio_clones_share_processing_and_render_identically_after_portable_recall() {
    let mut app = Harness::new();
    let source = app.directory.path().join("clone-source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: (0..96000)
            .flat_map(|frame| {
                let sample = 0.1 + frame as f32 / 400000.0;
                [sample, -sample * 0.5]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Linked audio");
    let original = Clip::audio("Take", 0.0, 4.0, source.to_string_lossy());
    let original_id = original.id;
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(original);
    let path = app.directory.path().join("linked-audio.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Take");
    let state = select(&mut app, original_id);
    let (x, y) = center(clip_rect(&state, original_id));
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let clone_id = selected(&app.click("Ghost copy"));
    let copied = menu(&mut app, "Edit", "Duplicate");
    let independent_id = selected(&copied);
    let independent_content = audio(&copied, independent_id).clone();
    select(&mut app, clone_id);
    menu(&mut app, "Window", "Clip Editor");
    app.key("I");
    audio_at(&mut app, 0.5);
    app.click("Linear");
    app.click("Equal power");
    app.key("O");
    audio_at(&mut app, 3.0);
    app.click("Linear");
    app.click("S curve");
    menu(&mut app, "Process", "Reverse");
    let state = app.inspect();
    let rect = &state["audio_editor"]["gain_rect"];
    pointer(
        &mut app,
        "double_click",
        (
            (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5,
            (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5,
        ),
        None,
    );
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"-6"}));
    let edited = app.key("Enter");
    assert_eq!(audio(&edited, original_id), audio(&edited, clone_id));
    assert_eq!(audio(&edited, independent_id), &independent_content);
    near(
        audio(&edited, original_id)["gain"].as_f64().unwrap(),
        10.0_f64.powf(-0.3),
    );
    assert_eq!(audio(&edited, original_id)["reverse"], true);
    assert_eq!(audio(&edited, original_id)["fade_in"], 0.5);
    assert_eq!(audio(&edited, original_id)["fade_out"], 1.0);
    let rendered = export(&mut app, "linked.wav");
    let gain = audio(&edited, original_id)["gain"].as_f64().unwrap();
    for frame in [1200, 6000, 24000, 48000, 84000, 95000] {
        let beat = frame as f64 / 24000.0;
        let expected = (0.1 + (95999 - frame) as f64 / 400000.0)
            * gain
            * f64::from(daw_core::FadeCurve::EqualPower.value(beat / 0.5))
            * f64::from(daw_core::FadeCurve::SCurve.value(4.0 - beat));
        for start in [0, 96000] {
            near(f64::from(rendered.samples[(start + frame) * 2]), expected);
            near(
                f64::from(rendered.samples[(start + frame) * 2 + 1]),
                -expected * 0.5,
            );
        }
        near(
            f64::from(rendered.samples[(192000 + frame) * 2]),
            0.1 + frame as f64 / 400000.0,
        );
    }
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    menu(&mut app, "Process", "Normalize");
    let normalized = wait_audio(&mut app);
    assert_eq!(
        audio(&normalized, original_id),
        audio(&normalized, clone_id)
    );
    near(
        audio(&normalized, original_id)["gain"].as_f64().unwrap() * (0.1 + 95999.0 / 400000.0),
        10.0_f64.powf(-0.05),
    );
    assert_eq!(audio(&normalized, independent_id), &independent_content);
    assert_eq!(undo(&mut app, false)["project"], edited["project"]);
    let archive = app.directory.path().join("audio-clones.rill");
    app.command(json!({"command":"save","path":archive}));
    std::fs::remove_file(source).unwrap();
    let restored = app.command(json!({"command":"open","path":archive}));
    assert_eq!(restored["error"], false);
    assert_eq!(
        clip(&restored, original_id)["link_group"],
        clip(&restored, clone_id)["link_group"]
    );
    assert_eq!(audio(&restored, original_id), audio(&restored, clone_id));
    assert_eq!(export(&mut app, "recalled.wav").samples, rendered.samples);
    select(&mut app, clone_id);
    menu(&mut app, "Edit", "Make independent");
    menu(&mut app, "Window", "Clip Editor");
    let independent = menu(&mut app, "Process", "Reverse");
    assert_eq!(audio(&independent, original_id)["reverse"], true);
    assert_eq!(audio(&independent, clone_id)["reverse"], false);
    assert_eq!(audio(&independent, independent_id)["reverse"], false);
    app.quit();
}

fn audio_at(app: &mut Harness, beat: f64) -> Value {
    let state = app.inspect();
    let view = &state["audio_editor"];
    pointer(
        app,
        "click",
        (
            view["rect"][0].as_f64().unwrap()
                + (view["clip_start"].as_f64().unwrap() + beat
                    - view["start_beat"].as_f64().unwrap())
                    * view["pixels_per_beat"].as_f64().unwrap(),
            (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) * 0.5,
        ),
        None,
    )
}
