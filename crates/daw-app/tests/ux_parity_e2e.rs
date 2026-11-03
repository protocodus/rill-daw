//! Mouse-only workflows, context menus, and automation clip command parity.
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

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["text"] == text)
        .unwrap_or_else(|| panic!("Missing label {text:?}: {}", state["visible_labels"]))
}

fn menu(app: &mut Harness, top: &str, item: &str) -> Value {
    app.click(top);
    app.click(item)
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

#[test]
fn clear_clip_selection_and_context_menus_rename_tracks_and_channels() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Initial keys");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("empty-and-rename.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));

    // Select clip in Arrangement, then clear clip selection from Edit menu.
    let selected = app.click("Phrase");
    assert!(!selected["selected_clip"].is_null());
    let back = menu(&mut app, "Edit", "Clear clip selection");
    assert!(back["selected_clip"].is_null());

    // Right-click track header in Arrangement -> Rename…
    let (hx, hy) = center(label(&back, "Initial keys"));
    app.command(
        json!({"command":"pointer", "phase":"click", "x":hx, "y":hy, "button":"secondary"}),
    );
    app.click("Rename…");
    app.command(json!({"command":"text", "text":"Warm Rhodes"}));
    let renamed_track = app.key("Enter");
    assert_eq!(renamed_track["project"]["tracks"][0]["name"], "Warm Rhodes");
    assert_eq!(
        undo(&mut app, false)["project"]["tracks"][0]["name"],
        "Initial keys"
    );
    assert_eq!(
        undo(&mut app, true)["project"]["tracks"][0]["name"],
        "Warm Rhodes"
    );

    // Right-click channel strip button in Mixer -> Rename…
    let mixer = menu(&mut app, "Window", "Mixer");
    let (mx, my) = center(label(&mixer, "Warm Rhodes"));
    app.command(
        json!({"command":"pointer", "phase":"click", "x":mx, "y":my, "button":"secondary"}),
    );
    app.click("Rename…");
    app.command(json!({"command":"text", "text":"Stage EP"}));
    let renamed_mixer = app.key("Enter");
    assert_eq!(renamed_mixer["project"]["tracks"][0]["name"], "Stage EP");
    assert_eq!(
        undo(&mut app, false)["project"]["tracks"][0]["name"],
        "Warm Rhodes"
    );
    app.quit();
}

#[test]
fn selected_automation_clips_support_edit_menu_and_keyboard_duplicate_ghost_independent_delete_and_undo()
 {
    let mut app = Harness::new();
    let mut project = Project::new();
    let audio = project.add_audio_track("Synth bus");
    let lane = project
        .add_automation_track(
            "Synth gain",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let first = project.create_automation_clip(lane, 0.0, 4.0).unwrap();
    let path = app.directory.path().join("auto-clip-commands.rill");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open", "path":path}));

    let header_pos = |state: &Value, index: usize| -> (f64, f64) {
        let rect = &state["timeline"]["automation"][0]["clips"][index]["rect"];
        (
            (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
            rect["min"][1].as_f64().unwrap() + 10.0,
        )
    };
    fn clips(state: &Value) -> &Vec<Value> {
        state["project"]["tracks"][1]["automation"]["clips"]
            .as_array()
            .unwrap()
    }

    // Click the automation clip header to select it.
    let (x, y) = header_pos(&initial, 0);
    let selected = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(selected["selected_track"], lane);
    assert_eq!(selected["selected_clip"], first);

    // Edit -> Duplicate duplicates the automation clip as an independent copy at beat 4.0.
    let duplicated = menu(&mut app, "Edit", "Duplicate");
    assert_eq!(clips(&duplicated).len(), 2);
    let second = clips(&duplicated)[1]["id"].as_u64().unwrap();
    assert_eq!(clips(&duplicated)[1]["start"], 4.0);
    assert!(clips(&duplicated)[1]["link_group"].is_null());
    assert_eq!(duplicated["selected_clip"], second);

    // Edit -> Ghost copy creates a linked automation clip at beat 8.0, and Make independent detaches it.
    let ghosted = menu(&mut app, "Edit", "Ghost copy");
    assert_eq!(clips(&ghosted).len(), 3);
    let third = clips(&ghosted)[2]["id"].as_u64().unwrap();
    assert!(!clips(&ghosted)[2]["link_group"].is_null());
    assert_eq!(ghosted["selected_clip"], third);

    let detached = menu(&mut app, "Edit", "Make independent");
    assert!(clips(&detached)[2]["link_group"].is_null());
    assert_eq!(detached["selected_clip"], third);

    // Delete key deletes the selected automation clip, and Undo restores both the clip and its selection.
    let deleted = app.key("Backspace");
    assert_eq!(clips(&deleted).len(), 2);
    assert!(deleted["selected_clip"].is_null());

    let restored = undo(&mut app, false);
    assert_eq!(clips(&restored).len(), 3);
    assert_eq!(restored["selected_clip"], third);

    // Clear clip selection clears selected_clip for automation clips too.
    let cleared = menu(&mut app, "Edit", "Clear clip selection");
    assert!(cleared["selected_clip"].is_null());
    app.quit();
}

#[test]
fn midi_editor_mouse_nudge_scale_transpose_and_note_context_menu_and_audio_editor_context_menu_work()
 {
    let mut app = Harness::new();
    let wav_path = app.directory.path().join("tone.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: (0..96000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect(),
    }
    .write_wav(&wav_path)
    .unwrap();

    let mut project = Project::new();
    let midi_track = project.add_midi_track("Lead");
    project
        .track_mut(midi_track)
        .unwrap()
        .clips
        .push(Clip::midi(
            "Melody",
            0.0,
            8.0,
            vec![Note::new(60, 1.0, 1.0, 100), Note::new(63, 2.0, 1.0, 100)],
        ));
    let audio_track = project.add_audio_track("Vox");
    project
        .track_mut(audio_track)
        .unwrap()
        .clips
        .push(Clip::audio("Take 1", 0.0, 4.0, wav_path.to_string_lossy()));
    let path = app.directory.path().join("editors-ux.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));

    // 1. MIDI Editor mouse-only scale degree transpose, nudge, and note context menu.
    app.click("Melody");
    menu(&mut app, "Window", "Clip Editor");
    menu(&mut app, "Edit", "Select all notes");

    let up_scale = menu(&mut app, "Notes", "+1 scale degree");
    let midi_notes = |state: &Value| -> Vec<(u64, f64)> {
        state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| (n["pitch"].as_u64().unwrap(), n["start"].as_f64().unwrap()))
            .collect()
    };
    // C4 (60) -> D4 (62), Eb4 (63) -> F4 (65) in C Minor.
    assert_eq!(midi_notes(&up_scale), vec![(62, 1.0), (65, 2.0)]);

    let down_scale = menu(&mut app, "Notes", "-1 scale degree");
    assert_eq!(midi_notes(&down_scale), vec![(60, 1.0), (63, 2.0)]);

    let nudged_right = menu(&mut app, "Notes", "Nudge right");
    assert!(midi_notes(&nudged_right)[0].1 > 1.0);
    let nudged_left = menu(&mut app, "Notes", "Nudge left");
    assert!((midi_notes(&nudged_left)[0].1 - 1.0).abs() < 1e-9);

    let nudged_bar = menu(&mut app, "Notes", "Nudge right 1 bar");
    assert!((midi_notes(&nudged_bar)[0].1 - 5.0).abs() < 1e-9);
    let back_bar = menu(&mut app, "Notes", "Nudge left 1 bar");
    assert!((midi_notes(&back_bar)[0].1 - 1.0).abs() < 1e-9);

    // Right-click the first note on the piano roll canvas -> Select only this note, then Duplicate, then Delete.
    let note_rect = &back_bar["midi_editor"]["notes"][0]["rect"];
    let (nx, ny) = center(note_rect);
    app.command(
        json!({"command":"pointer", "phase":"click", "x":nx, "y":ny, "button":"secondary"}),
    );
    let single = app.click("Select only this note");
    let selected_count = single["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["selected"] == true)
        .count();
    assert_eq!(selected_count, 1);

    app.command(
        json!({"command":"pointer", "phase":"click", "x":nx, "y":ny, "button":"secondary"}),
    );
    let deleted_note = app.click("Delete");
    assert_eq!(midi_notes(&deleted_note).len(), 1);
    undo(&mut app, false);

    // 2. Clip Properties exact Clip gain entry via right-click context menu.
    menu(&mut app, "Window", "Arranger");
    app.click("Take 1");
    let props = menu(&mut app, "Window", "Properties");
    let (gx, gy) = center(label(&props, "0.0 dB"));
    app.command(
        json!({"command":"pointer", "phase":"click", "x":gx, "y":gy, "button":"secondary"}),
    );
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":"-6"}));
    let gain_applied = app.click("Apply value");
    let audio_gain = gain_applied["project"]["tracks"][1]["clips"][0]["content"]["Audio"]["gain"]
        .as_f64()
        .unwrap();
    assert!(
        (20.0 * audio_gain.log10() - (-6.0)).abs() < 0.05,
        "gain={audio_gain}"
    );
    undo(&mut app, false);

    // 3. Audio Editor Edit menu time selection and waveform canvas context menu.
    let audio_ed = menu(&mut app, "Window", "Clip Editor");
    assert!(audio_ed["audio_editor"]["range"].is_null());
    let selected_time = menu(&mut app, "Edit", "Select clip time");
    assert_eq!(selected_time["audio_editor"]["range"], json!([0.0, 4.0]));
    let cleared_time = menu(&mut app, "Edit", "Clear time selection");
    assert!(cleared_time["audio_editor"]["range"].is_null());

    let canvas = &cleared_time["audio_editor"]["rect"];
    let cx = (canvas[0].as_f64().unwrap() + canvas[2].as_f64().unwrap()) * 0.5;
    let cy = (canvas[1].as_f64().unwrap() + canvas[3].as_f64().unwrap()) * 0.5;
    app.command(
        json!({"command":"pointer", "phase":"click", "x":cx, "y":cy, "button":"secondary"}),
    );
    let reversed = app.click("Reverse");
    assert_eq!(
        reversed["project"]["tracks"][1]["clips"][0]["content"]["Audio"]["reverse"],
        true
    );
    app.quit();
}
