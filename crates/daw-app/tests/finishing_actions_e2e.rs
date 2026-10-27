//! Mouse-accessible phrase repetition, batch routing and repeat mixdown export.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn front(app: &mut Harness, label: &str) -> Value {
    // Default semantic clicks resolve the foreground menu and reveal clipped
    // commands with real wheel input. Explicit occurrence indexing can pick a
    // repeated background Mixer action instead of the active menu entry.
    app.click(label)
}

fn menu(app: &mut Harness, parent: &str, label: &str) -> Value {
    front(app, parent);
    front(app, label)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn source(app: &Harness) -> std::path::PathBuf {
    let path = app.directory.path().join("Original.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..96_000).map(|i| (i as f32 * 0.07).sin() * 0.1).collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}

fn export_mix(app: &mut Harness, name: &str) -> AudioData {
    let options = menu(app, "File", "Export WAV…");
    if options["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    let path = app.directory.path().join(name);
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    front(app, "Export…");
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn channel_action(app: &mut Harness, id: u64, action: &str) -> Value {
    let state = app.inspect();
    let channel = state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap();
    let (x, y) = center(&channel["name"]);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    front(app, action)
}

#[test]
fn mouse_only_channel_selection_routes_and_sends_as_one_undoable_batch() {
    let mut app = Harness::new();
    let original = source(&app);
    let original_bytes = std::fs::read(&original).unwrap();
    let mut project = Project::new();
    let a = project.add_audio_track("Dry A");
    let b = project.add_audio_track("Dry B");
    for (id, name) in [(a, "First cue"), (b, "Second cue")] {
        project.track_mut(id).unwrap().clips.push(Clip::audio(
            name,
            0.0,
            1.0,
            original.to_string_lossy(),
        ));
    }
    let music = project.add_bus("Music");
    let room = project.add_bus("Room");
    // An existing send keeps both its level and position when missing sends
    // are added to the rest of the selection.
    project.add_send(a, room, 0.125, true).unwrap();
    let path = app.directory.path().join("Batch.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Mixer");
    let before = app.inspect();
    channel_action(&mut app, a, "Select channel");
    let selected = channel_action(&mut app, b, "Add channel to selection");
    assert_eq!(selected["view"], "Mixer");
    assert_eq!(selected["finishing"]["selected_channels"], json!([a, b]));
    assert_eq!(selected["project"], before["project"]);
    assert_eq!(selected["history"], before["history"]);
    front(&mut app, "Route selected channels");
    let routed = front(&mut app, "Music");
    for id in [a, b] {
        assert_eq!(track(&routed, id)["output_bus"], music);
    }
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], routed["project"]);
    front(&mut app, "Route selected channels");
    front(&mut app, "Add post-fader send");
    let sent = front(&mut app, "Room");
    assert_eq!(
        track(&sent, a)["sends"],
        json!([{"target_bus":room,"gain":0.125,"pre_fader":true}])
    );
    assert_eq!(
        track(&sent, b)["sends"],
        json!([{"target_bus":room,"gain":0.25,"pre_fader":false}])
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], routed["project"]);
    let removed = channel_action(&mut app, b, "Remove channel from selection");
    assert_eq!(removed["finishing"]["selected_channels"], json!([a]));
    let all = channel_action(&mut app, a, "Select all channels");
    assert_eq!(
        all["finishing"]["selected_channels"],
        json!([a, b, music, room])
    );
    // Routing to a member of the selection would route that bus to itself;
    // filtering the intersection never offers the invalid choice.
    front(&mut app, "Route selected channels");
    let state = app.inspect();
    // Observe actual foreground menu text, excluding the Mixer strip names
    // retained underneath the popup. Selected buses must never offer themselves
    // as output or send destinations, even when their headers remain visible.
    let labels = state["menu_label_rects"].as_array().unwrap();
    assert!(labels.iter().any(|label| label["text"] == "Master output"));
    for forbidden in ["Music", "Room"] {
        assert!(
            !labels.iter().any(|label| label["text"] == forbidden),
            "Selected bus {forbidden} was offered as a destination"
        );
    }
    app.click("Mixer");
    let cleared = channel_action(&mut app, a, "Clear channel selection");
    assert_eq!(cleared["finishing"]["selected_channels"], json!([]));
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
}

#[test]
fn repeat_to_loop_end_keeps_phrase_rests_and_trims_the_last_source_window() {
    let mut app = Harness::new();
    let original = source(&app);
    let original_bytes = std::fs::read(&original).unwrap();
    let mut project = Project::new();
    project.loop_enabled = true;
    project.loop_start = 0.25;
    project.loop_end = 9.25;
    let audio = project.add_audio_track("Audio");
    let midi = project.add_midi_track("MIDI");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Hit",
        0.5,
        1.0,
        original.to_string_lossy(),
    ));
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Reply",
        3.0,
        1.0,
        vec![Note::new(60, 0.0, 0.5, 100)],
    ));
    let path = app.directory.path().join("Phrase.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    // The initial overview already includes the extra phrase rest. Fitting
    // tightly to the final clip would intentionally hide that trailing time.
    let before = app.inspect();
    let original_mix = export_mix(&mut app, "Before repeat.wav");
    // Select the Cursor tool with the mouse; the phrase includes leading and
    // trailing silence beyond both clips, across the two track rows.
    front(&mut app, "Cursor");
    let state = app.inspect();
    let timeline = &state["timeline"];
    let origin = timeline["canvas"]["min"][0].as_f64().unwrap()
        - timeline["start_beat"].as_f64().unwrap() * timeline["pixels_per_beat"].as_f64().unwrap();
    let pixels = timeline["pixels_per_beat"].as_f64().unwrap();
    let rows = timeline["tracks"].as_array().unwrap();
    let top = rows.iter().find(|row| row["track"] == audio).unwrap()["header"]["min"][1]
        .as_f64()
        .unwrap()
        + 20.0;
    let bottom = rows.iter().find(|row| row["track"] == midi).unwrap()["header"]["max"][1]
        .as_f64()
        .unwrap()
        - 20.0;
    let selected=app.command(json!({"command":"pointer","phase":"drag","x":origin+0.25*pixels,"y":top,"to_x":origin+4.25*pixels,"to_y":bottom}));
    assert_eq!(selected["timeline"]["selection_span"], json!([0.25, 4.25]));
    let repeated = menu(&mut app, "Edit", "Repeat to loop end");
    let audio_clips = track(&repeated, audio)["clips"].as_array().unwrap();
    let midi_clips = track(&repeated, midi)["clips"].as_array().unwrap();
    assert_eq!(
        audio_clips
            .iter()
            .map(|clip| clip["start"].as_f64().unwrap())
            .collect::<Vec<_>>(),
        [0.5, 4.5, 8.5]
    );
    assert_eq!(
        audio_clips
            .iter()
            .map(|clip| clip["length"].as_f64().unwrap())
            .collect::<Vec<_>>(),
        [1.0, 1.0, 0.75]
    );
    assert_eq!(
        midi_clips
            .iter()
            .map(|clip| clip["start"].as_f64().unwrap())
            .collect::<Vec<_>>(),
        [3.0, 7.0]
    );
    assert_eq!(audio_clips[0], before["project"]["tracks"][0]["clips"][0]);
    assert_eq!(midi_clips[0], before["project"]["tracks"][1]["clips"][0]);
    assert_ne!(
        midi_clips[0]["content"]["Midi"]["notes"][0]["id"],
        midi_clips[1]["content"]["Midi"]["notes"][0]["id"]
    );
    assert!(midi_clips[1]["content"]["Midi"]["notes"][0]["id"].is_u64());
    assert_eq!(
        repeated["timeline"]["selected_clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let mix = export_mix(&mut app, "Repeated.wav");
    assert_eq!(mix.frames(), 222_000);
    let original_pcm = &original_mix.samples[12_000 * 2..36_000 * 2];
    assert_eq!(&mix.samples[12_000 * 2..36_000 * 2], original_pcm);
    assert_eq!(&mix.samples[108_000 * 2..132_000 * 2], original_pcm);
    assert_eq!(
        &mix.samples[204_000 * 2..222_000 * 2],
        &original_pcm[..18_000 * 2]
    );
    assert!(
        mix.samples[132_000 * 2..204_000 * 2]
            .iter()
            .all(|sample| *sample == 0.0)
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], repeated["project"]);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
}

#[test]
fn reexport_reuses_only_successful_destination_and_tail_choice_then_resets_on_open() {
    let mut app = Harness::new();
    let original = source(&app);
    let original_bytes = std::fs::read(&original).unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Hit",
        0.0,
        1.0,
        original.to_string_lossy(),
    ));
    let path = app.directory.path().join("Export.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    menu(&mut app, "File", "Export WAV…");
    app.click("Include effect tails");
    let mix = app.directory.path().join("Mix.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":mix}));
    front(&mut app, "Export…");
    let first = app.command(json!({"command":"wait_export"}));
    assert_eq!(
        first["finishing"]["last_export"],
        json!({"path":mix,"include_effect_tails":false})
    );
    let expected = AudioData::load_wav(&mix).unwrap();
    assert_eq!(expected.frames(), 24_000);
    // Change the popup preference and cancel; quick export still remembers the
    // successful render's tail option, not an abandoned options edit.
    menu(&mut app, "File", "Export WAV…");
    app.click("Include effect tails");
    app.click("Cancel");
    // A render that reaches the worker but cannot create its destination must
    // not turn the last-success command into a retry of that failed location.
    menu(&mut app, "File", "Export WAV…");
    let failed = app
        .directory
        .path()
        .join("Missing directory")
        .join("Never.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":failed}));
    front(&mut app, "Export…");
    let failed = app.raw_command(json!({"command":"wait_export"}));
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("WAV export failed")
    );
    let failed = &failed["state"];
    assert_eq!(failed["error"], true);
    assert_eq!(
        failed["finishing"]["last_export"],
        json!({"path":mix,"include_effect_tails":false})
    );
    std::fs::write(&mix, b"replace this previous mix").unwrap();
    menu(&mut app, "File", "Re-export last WAV (replace)");
    app.command(json!({"command":"wait_export"}));
    assert_eq!(AudioData::load_wav(&mix).unwrap().samples, expected.samples);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    // Reopening another document must not leave a one-click replacement link
    // to an unrelated song's mixdown.
    app.command(json!({"command":"open","path":path}));
    let reopened = app.inspect();
    assert!(reopened["finishing"]["last_export"].is_null());
}

#[test]
fn mixer_arrows_select_and_extend_across_tracks_and_buses_without_editing() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let tracks: Vec<_> = (0..10)
        .map(|index| project.add_midi_track(format!("Track {index}")))
        .collect();
    let bus = project.add_bus("Last bus");
    let path = app.directory.path().join("Navigation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":800,"height":720}));
    let before = app.click("Mixer");
    let next = app.key("ArrowRight");
    assert_eq!(next["selected_track"], tracks[1]);
    let extended = app.command(json!({"command":"key","key":"ArrowDown","shift":true}));
    assert_eq!(extended["selected_track"], tracks[2]);
    assert_eq!(
        extended["finishing"]["selected_channels"],
        json!([tracks[1], tracks[2]])
    );
    let only = app.key("ArrowRight");
    assert_eq!(only["finishing"]["selected_channels"], json!([tracks[3]]));
    let previous = app.key("ArrowUp");
    assert_eq!(previous["selected_track"], tracks[2]);
    for _ in 0..10 {
        app.key("ArrowRight");
    }
    let state = app.inspect();
    assert_eq!(state["selected_track"], bus);
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    assert_eq!(state["view"], "Mixer");
    let selected = state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == bus)
        .unwrap();
    assert!(selected["name"]["min"][0].as_f64().unwrap() >= 0.0);
    assert!(selected["name"]["max"][0].as_f64().unwrap() <= 800.0);
}
