//! Whole-clip and selected-note musical operations through production menus.
mod common;

use common::Harness;
use daw_core::{AudioData, AutomationParameter, AutomationTarget, Clip, Note, Project};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn notes(state: &Value, id: u64) -> Vec<Value> {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .clone()
}
fn history(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}
fn unchanged(state: &Value, original: &Value) {
    assert_eq!(state["project"], original["project"]);
    assert_eq!(state["history"], original["history"]);
    assert_eq!(state["song_session"], original["song_session"]);
}
fn menu(app: &mut Harness, menu: &str, action: &str) -> Value {
    app.click(menu);
    app.click(action)
}
fn undo(app: &mut Harness) -> Value {
    menu(app, "Edit", "Undo")
}
fn context(app: &mut Harness, id: u64) -> Value {
    let before = app.inspect();
    let area = &before["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    let (x, y) = center(area);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}))
}
fn context_action(app: &mut Harness, id: u64, action: &str) -> Value {
    context(app, id);
    app.click("MIDI timing and pitch");
    app.click(action)
}
fn edit_clip(app: &mut Harness, id: u64) -> Value {
    app.click("Arranger");
    context(app, id);
    app.click("Edit notes")
}
fn select_note(app: &mut Harness, id: u64, add: bool) -> Value {
    let state = app.inspect();
    let rect = &state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == id)
        .unwrap_or_else(|| panic!("Note{id} not visible: {}", state["midi_editor"]))["rect"];
    let (x, y) = center(rect);
    let state=app.command(json!({"command":"pointer","phase":"click","button":if add {"secondary"} else {"primary"},"x":x,"y":y}));
    if add {
        app.click("Add to selection")
    } else {
        state
    }
}
fn near(actual: &Value, expected: f64) {
    let actual = actual.as_f64().unwrap();
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}
fn fixture(source: Vec<Note>) -> (Harness, u64, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Musical clip actions");
    let mut source = Clip::midi("Mapped source phrase", 1.25, 3.0, source);
    source.offset = 1.0;
    source.source_rate = 0.5;
    let id = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let ghost = project.clone_clip(track, id).unwrap();
    let linked = project.clip_mut(track, ghost).unwrap();
    linked.start = 8.0;
    linked.offset = 0.5;
    linked.length = 4.0;
    linked.source_rate = 1.0;
    let path = app.directory.path().join("clip-actions.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    (app, track, id, ghost)
}
fn phrase() -> Vec<Note> {
    let mut source = vec![
        Note::new(40, 0.125, 0.125, 50),
        Note::new(60, 1.11, 0.2, 71),
        Note::new(64, 1.63, 0.3, 93),
        Note::new(67, 2.31, 0.15, 110),
        Note::new(90, 3.5, 0.5, 100),
    ];
    for (index, note) in source.iter_mut().enumerate() {
        note.id = 70_000 + index as u64;
    }
    source
}
fn bounds_unchanged(after: &Value, before: &Value, id: u64) {
    for key in [
        "id",
        "start",
        "offset",
        "length",
        "source_rate",
        "link_group",
    ] {
        assert_eq!(clip(after, id)[key], clip(before, id)[key], "{id} {key}");
    }
}

#[test]
fn clip_timing_and_pitch_actions_keep_sources_ghosts_and_exact_undo_recall() {
    let (mut app, _, id, ghost) = fixture(phrase());
    let before = app.inspect();
    let half = context_action(&mut app, id, "Half time");
    near(&clip(&half, id)["length"], 6.0);
    near(&clip(&half, id)["source_rate"], 0.25);
    for field in ["start", "offset", "link_group", "content"] {
        assert_eq!(clip(&half, id)[field], clip(&before, id)[field]);
    }
    assert_eq!(clip(&half, ghost), clip(&before, ghost));
    assert_eq!(history(&half), history(&before) + 1);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], half["project"]);
    edit_clip(&mut app, id);
    let restored = menu(&mut app, "Clip", "Double time");
    assert_eq!(restored["project"], before["project"]);
    for (up, down, delta) in [
        ("Semitone up", "Semitone down", 1),
        ("Octave up", "Octave down", 12),
    ] {
        app.click("Arranger");
        let original = app.inspect();
        let moved = context_action(&mut app, id, up);
        for (after, before) in notes(&moved, id).iter().zip(notes(&original, id)) {
            assert_eq!(
                after["pitch"].as_u64().unwrap(),
                before["pitch"].as_u64().unwrap() + delta
            );
            for field in ["id", "start", "length", "velocity"] {
                assert_eq!(after[field], before[field]);
            }
        }
        assert_eq!(notes(&moved, id), notes(&moved, ghost));
        bounds_unchanged(&moved, &original, id);
        bounds_unchanged(&moved, &original, ghost);
        assert_eq!(history(&moved), history(&original) + 1);
        assert_eq!(undo(&mut app)["project"], original["project"]);
        menu(&mut app, "Edit", "Redo");
        let returned = context_action(&mut app, id, down);
        assert_eq!(returned["project"], original["project"]);
    }
    let delivered = context_action(&mut app, id, "Semitone up");
    let path = app.directory.path().join("retained-actions.rill");
    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(reopened["project"], delivered["project"]);
    assert_eq!(notes(&reopened, id), notes(&reopened, ghost));
    app.quit();
}

#[test]
fn pitch_bounds_reject_the_entire_clip_action_including_trimmed_source_notes() {
    for (pitch, actions) in [
        (127, ["Semitone up", "Octave up"]),
        (0, ["Semitone down", "Octave down"]),
    ] {
        let mut source = phrase();
        source[4].pitch = pitch;
        let (mut app, _, id, _) = fixture(source);
        let original = app.inspect();
        for action in actions {
            let rejected = context_action(&mut app, id, action);
            unchanged(&rejected, &original);
            assert_eq!(rejected["native_menu"]["open"], true);
            app.key("Escape");
            app.key("Escape");
        }
        app.quit();
    }
}

#[test]
fn selected_note_flips_are_involutions_keep_unselected_notes_and_ghost_identity() {
    let mut source = vec![
        Note::new(40, 0.125, 0.125, 44),
        Note::new(60, 1.0, 0.25, 80),
        Note::new(67, 2.0, 0.5, 96),
        Note::new(67, 1.0, 0.5, 72), // A mirror lands on this old note; do not auto-merge it.
        Note::new(90, 3.5, 0.5, 100),
    ];
    for (index, note) in source.iter_mut().enumerate() {
        note.id = 80_000 + index as u64;
    }
    source[1].set_velocity16(43_001);
    source[1].set_release_velocity16(11_113);
    source[2].set_velocity16(51_019);
    let (mut app, _, id, ghost) = fixture(source);
    edit_clip(&mut app, id);
    select_note(&mut app, 80_001, false);
    select_note(&mut app, 80_002, true);
    let original = app.inspect();
    for action in ["Flip horizontally", "Flip vertically"] {
        let changed = menu(&mut app, "Notes", action);
        assert_eq!(notes(&changed, id).len(), notes(&original, id).len());
        for before in notes(&original, id) {
            let after = notes(&changed, id)
                .into_iter()
                .find(|note| note["id"] == before["id"])
                .unwrap();
            if ![80_001, 80_002].contains(&before["id"].as_u64().unwrap()) {
                assert_eq!(after, before);
            } else {
                for field in ["id", "length", "velocity"] {
                    assert_eq!(after[field], before[field]);
                }
                for field in [
                    "velocity",
                    "release_velocity",
                    "attribute_type",
                    "attribute_data",
                ] {
                    assert_eq!(
                        after["midi2"][field], before["midi2"][field],
                        "A mirror preserves high-resolution performance data"
                    );
                }
                if action == "Flip horizontally" {
                    near(
                        &after["start"],
                        3.5 - (before["start"].as_f64().unwrap()
                            + before["length"].as_f64().unwrap()),
                    );
                    assert_eq!(after["pitch"], before["pitch"]);
                } else {
                    assert_eq!(after["start"], before["start"]);
                    assert_eq!(
                        after["pitch"].as_u64().unwrap(),
                        127 - before["pitch"].as_u64().unwrap()
                    );
                }
            }
        }
        assert_eq!(notes(&changed, id), notes(&changed, ghost));
        bounds_unchanged(&changed, &original, id);
        bounds_unchanged(&changed, &original, ghost);
        let twice = menu(&mut app, "Notes", action);
        assert_eq!(twice["project"], original["project"]);
        assert_eq!(history(&twice), history(&changed) + 1);
        assert_eq!(undo(&mut app)["project"], changed["project"]);
        assert_eq!(undo(&mut app)["project"], original["project"]);
    }
    app.quit();
}

fn preview_source(state: &Value, id: u64) -> Vec<Value> {
    let rate = clip(state, id)["source_rate"].as_f64().unwrap();
    state["midi_editor"]["preview_notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| {
            let mut note = note.clone();
            note["start"] = json!(note["start"].as_f64().unwrap() * rate);
            note["length"] = json!(note["length"].as_f64().unwrap() * rate);
            note
        })
        .collect()
}

#[test]
fn whole_clip_quantize_and_humanize_preview_all_audible_notes_but_keep_hidden_material() {
    let (mut app, _, id, ghost) = fixture(phrase());
    edit_clip(&mut app, id);
    select_note(&mut app, 70_001, false);
    menu(&mut app, "View", "Only scale keys");
    app.key("Escape");
    for (action, cancel, apply) in [
        ("Quantize…", "Cancel quantize", "Apply quantize"),
        ("Humanize…", "Cancel humanize", "Apply humanize"),
    ] {
        let folded = app.inspect();
        assert_eq!(folded["midi_editor"]["only_scale_keys"], true);
        app.click("Arranger");
        let original = app.inspect();
        let preview = context_action(&mut app, id, action);
        assert_eq!(preview["view"], "Clip");
        assert_eq!(
            preview["midi_editor"]["only_scale_keys"], true,
            "A preview must not change the folding preference"
        );
        assert_eq!(
            preview["midi_editor"]["visible_pitches"]
                .as_array()
                .unwrap()
                .len(),
            128,
            "Whole-clip preview must expose all affected pitches"
        );
        unchanged(&preview, &original);
        assert!(
            preview["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label
                    .as_str()
                    .is_some_and(|label| label.starts_with("Clip window")))
        );
        let expected = preview_source(&preview, id);
        let old = notes(&original, id);
        for hidden in [70_000, 70_004] {
            assert_eq!(
                expected.iter().find(|note| note["id"] == hidden).unwrap(),
                old.iter().find(|note| note["id"] == hidden).unwrap()
            );
        }
        assert!(
            expected
                .iter()
                .filter(|note| [70_002, 70_003].contains(&note["id"].as_u64().unwrap()))
                .any(|note| old
                    .iter()
                    .find(|before| before["id"] == note["id"])
                    .unwrap()
                    != note),
            "Whole clip transform must include notes outside the old note selection"
        );
        let cancelled = app.click(cancel);
        unchanged(&cancelled, &original);
        for field in [
            "only_scale_keys",
            "row_offset",
            "row_height",
            "start_beat",
            "pixels_per_beat",
            "visible_pitches",
        ] {
            assert_eq!(
                cancelled["midi_editor"][field], folded["midi_editor"][field],
                "Cancel restores {field}"
            );
        }
        let second = menu(&mut app, "Clip", action);
        assert_eq!(preview_source(&second, id), expected);
        let applied = app.click(apply);
        assert_eq!(notes(&applied, id), expected);
        assert_eq!(notes(&applied, id), notes(&applied, ghost));
        bounds_unchanged(&applied, &original, id);
        bounds_unchanged(&applied, &original, ghost);
        assert_eq!(history(&applied), history(&original) + 1);
        assert_eq!(undo(&mut app)["project"], original["project"]);
    }
    app.quit();
}

#[test]
fn loop_this_clip_context_uses_exact_audio_midi_and_automation_pattern_bounds() {
    let mut app = Harness::new();
    let path = app.directory.path().join("loop-audio.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Loop audio");
    let region = Clip::audio("Audio region", 1.375, 1.25, path.to_string_lossy());
    let audio_id = region.id;
    project.track_mut(audio).unwrap().clips.push(region);
    let midi = project.add_midi_track("Loop notes");
    let region = Clip::midi(
        "MIDI region",
        4.125,
        1.75,
        vec![Note::new(60, 0.0, 0.5, 90)],
    );
    let midi_id = region.id;
    project.track_mut(midi).unwrap().clips.push(region);
    let lane = project
        .add_automation_track(
            "Gain pattern",
            AutomationTarget {
                track: midi,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let pattern = project.create_automation_clip(lane, 7.375, 1.625).unwrap();
    let path = app.directory.path().join("loop-actions.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1280,"height":800}));
    let original = app.inspect();
    for (id, start, end) in [
        (audio_id, 1.375, 2.625),
        (midi_id, 4.125, 5.875),
        (pattern, 7.375, 9.0),
    ] {
        let invoked = if id == pattern {
            let state = app.inspect();
            let rect = &state["timeline"]["automation"]
                .as_array()
                .unwrap()
                .iter()
                .find(|track| track["track"] == lane)
                .unwrap()["clips"][0]["rect"];
            let x = center(rect).0;
            let y = rect["min"][1].as_f64().unwrap() + 10.0;
            app.command(
                json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}),
            )
        } else {
            context(&mut app, id)
        };
        let looped = app.click("Loop this clip");
        assert_eq!(looped["project"]["loop_enabled"], true);
        near(&looped["project"]["loop_start"], start);
        near(&looped["project"]["loop_end"], end);
        assert_eq!(history(&looped), history(&original) + 1);
        for key in ["selected_track", "selected_clip", "selected_notes"] {
            assert_eq!(
                looped[key], invoked[key],
                "Looping retains the menu's editing context"
            );
        }
        assert_eq!(undo(&mut app)["project"], original["project"]);
    }
    app.quit();
}
