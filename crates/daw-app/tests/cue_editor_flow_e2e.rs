//! Cue editing through the running editor: navigation, preview and group reuse.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> (Harness, u64, u64, u64, Vec<u8>) {
    let mut app = Harness::new();
    let source = app.directory.path().join("cue-original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..240_000)
            .map(|frame| (frame as f32 * 0.013).sin() * 0.2)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Cue palette");
    let first = Clip::audio("First cue", 0.0, 2.0, source.to_string_lossy());
    let second = Clip::audio("Second cue", 8.0, 1.0, source.to_string_lossy());
    let ids = (first.id, second.id);
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let lower = project.add_audio_track("Lower palette");
    let lower_clip = Clip::audio("Lower cue", 0.0, 2.0, source.to_string_lossy());
    let lower_id = lower_clip.id;
    project.track_mut(lower).unwrap().clips.push(lower_clip);
    let midi = project.add_midi_track("MIDI cues");
    project.track_mut(midi).unwrap().clips.extend([
        Clip::midi("Low MIDI cue", 0.0, 2.0, vec![Note::new(36, 0.0, 1.0, 100)]),
        Clip::midi(
            "High MIDI cue",
            4.0,
            2.0,
            vec![Note::new(108, 0.0, 1.0, 100)],
        ),
    ]);
    let path = app.directory.path().join("cues.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("First cue");
    app.click("Window");
    app.click("Clip Editor");
    (app, ids.0, ids.1, lower_id, bytes)
}

fn cmd(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key","key":key,"cmd":true}))
}

fn view_action(app: &mut Harness, label: &str) -> Value {
    app.click("View");
    app.click(label)
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

fn source_unchanged(app: &Harness, original: &[u8]) {
    assert!(
        std::fs::read(app.directory.path().join("cue-original.wav")).unwrap() == original,
        "original source WAV bytes must remain unchanged"
    );
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn next_previous_and_arrow_selection_keep_tool_scale_project_and_history() {
    let (mut app, first, second, lower, original) = fixture();
    let compact = app.command(json!({"command":"viewport","width":800.0,"height":760.0}));
    for (name, minimum_width) in [("Previous", 50.0), ("Next", 25.0)] {
        assert!(
            compact["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == name)
        );
        let label = compact["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == name)
            .unwrap();
        assert!(label["max"][0].as_f64().unwrap() <= 800.0);
        assert!(
            label["max"][0].as_f64().unwrap() - label["min"][0].as_f64().unwrap() >= minimum_width,
            "{name} must paint its full short label, rather than an ellipsis"
        );
    }
    app.key("I");
    let state = app.inspect();
    let rect = &state["audio_editor"]["rect"];
    let x = (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5;
    let y = (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"scroll","x":x,"y":y,"delta":100.0,"ctrl":true}));
    let before =
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":80.0,"ctrl":true,"shift":true}));
    let next = app.click("Next");
    assert_eq!(next["selected_clip"], second);
    for key in ["pixels_per_beat", "row_height", "tool"] {
        assert_eq!(
            next["audio_editor"][key], before["audio_editor"][key],
            "retained {key}"
        );
    }
    let width = next["audio_editor"]["rect"][2].as_f64().unwrap()
        - next["audio_editor"]["rect"][0].as_f64().unwrap();
    let start = next["audio_editor"]["start_beat"].as_f64().unwrap();
    let end = start + width / next["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
    assert!(
        start <= 8.0 && end >= 8.0,
        "the next cue is revealed at the existing scale"
    );
    assert_eq!(
        view_action(&mut app, "Previous clip")["selected_clip"],
        first
    );
    assert_eq!(app.key("Tab")["selected_clip"], second);
    assert_eq!(
        app.command(json!({"command":"key","key":"Tab","shift":true}))["selected_clip"],
        first
    );
    assert_eq!(app.key("ArrowDown")["selected_clip"], lower);
    assert_eq!(app.key("ArrowUp")["selected_clip"], first);
    let right = app.key("ArrowRight");
    assert_eq!(right["selected_clip"], second);
    assert_eq!(right["project"], before["project"]);
    assert_eq!(right["history"], before["history"]);
    assert_eq!(
        right["audio_editor"]["cursor"],
        before["audio_editor"]["cursor"]
    );
    let nudged = cmd(&mut app, "ArrowRight");
    assert_eq!(clip(&nudged, second)["start"], 8.25);
    let undone = cmd(&mut app, "Z");
    assert_eq!(undone["project"], before["project"]);
    assert_eq!(
        view_action(&mut app, "Previous clip")["selected_clip"],
        first
    );
    let extended = app.command(json!({"command":"key","key":"ArrowRight","shift":true}));
    assert_eq!(extended["selected_clip"], second);
    assert_eq!(extended["selected_clips"].as_array().unwrap().len(), 2);
    let gain = &extended["audio_editor"]["gain_rect"];
    let editing = app.command(json!({"command":"pointer","phase":"double_click",
        "x":(gain[0].as_f64().unwrap()+gain[2].as_f64().unwrap())*0.5,
        "y":(gain[1].as_f64().unwrap()+gain[3].as_f64().unwrap())*0.5}));
    let text_arrow = app.key("ArrowLeft");
    assert_eq!(
        text_arrow["selected_clip"], second,
        "focused text retains cursor keys"
    );
    assert_eq!(text_arrow["project"], editing["project"]);
    app.key("Escape");
    source_unchanged(&app, &original);
    app.quit();
}

#[test]
fn grid_slice_preview_is_read_only_then_all_cues_duplicate_as_one_phrase() {
    let (mut app, first, _, _, original) = fixture();
    let before = app.inspect();
    let audible = export(&mut app, "unsliced.wav");
    app.click("Clip");
    let preview = app.click("Slice on grid…");
    assert_eq!(preview["audio_editor"]["grid_slice_open"], true);
    assert_eq!(
        preview["audio_editor"]["grid_preview"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(preview["project"], before["project"]);
    assert_eq!(preview["history"], before["history"]);
    let split = app.click("Split grid");
    assert_eq!(
        split["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    assert_eq!(split["audio_editor"]["grid_slice_open"], false);
    assert_eq!(split["selected_clip"], first);
    assert_eq!(split["selected_clips"].as_array().unwrap().len(), 8);
    let sliced = export(&mut app, "sliced.wav");
    assert_eq!(sliced.samples.len(), audible.samples.len());
    assert!(
        sliced.samples == audible.samples,
        "source-reference cuts preserve rendered audio; first mismatch {:?}",
        sliced
            .samples
            .iter()
            .zip(&audible.samples)
            .enumerate()
            .find(|(_, (a, b))| a != b)
    );
    app.click("Arranger");
    let copies = cmd(&mut app, "D");
    assert_eq!(
        copies["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        17,
        "every produced cue remains selected for reuse"
    );
    assert_eq!(cmd(&mut app, "Z")["project"], split["project"]);
    assert_eq!(cmd(&mut app, "Z")["project"], before["project"]);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], split["project"]);
    source_unchanged(&app, &original);
    app.quit();
}

#[test]
fn cancelling_grid_preview_makes_no_edit_and_short_selection_limits_cuts() {
    let (mut app, _, _, _, original) = fixture();
    let before = app.inspect();
    app.click("Clip");
    app.click("Slice on grid…");
    let cancelled = app.click("Close grid slicing");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    app.key("P");
    let state = app.inspect();
    let v = &state["audio_editor"];
    let x = |beat: f64| {
        v["rect"][0].as_f64().unwrap()
            + (beat - v["start_beat"].as_f64().unwrap()) * v["pixels_per_beat"].as_f64().unwrap()
    };
    let y = (v["rect"][1].as_f64().unwrap() + v["rect"][3].as_f64().unwrap()) * 0.5;
    app.command(
        json!({"command":"pointer","phase":"drag","x":x(0.5),"y":y,"to_x":x(1.5),"to_y":y}),
    );
    let selected = app.inspect();
    app.click("View");
    let fitted = app.click("Fit selection");
    assert!(
        fitted["audio_editor"]["pixels_per_beat"].as_f64().unwrap()
            > selected["audio_editor"]["pixels_per_beat"]
                .as_f64()
                .unwrap()
    );
    assert_eq!(fitted["project"], before["project"]);
    assert_eq!(fitted["history"], before["history"]);
    app.click("View");
    let restored = app.click("Previous view");
    for key in ["start_beat", "pixels_per_beat", "row_offset", "row_height"] {
        assert_eq!(
            restored["audio_editor"][key], selected["audio_editor"][key],
            "Previous view restores {key}"
        );
    }
    app.click("Clip");
    let preview = app.click("Slice on grid…");
    assert_eq!(
        preview["audio_editor"]["grid_preview"],
        json!([0.75, 1.0, 1.25])
    );
    assert_eq!(preview["project"], before["project"]);
    let split = app.click("Split grid");
    assert_eq!(
        split["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(cmd(&mut app, "Z")["project"], before["project"]);
    source_unchanged(&app, &original);
    app.quit();
}

#[test]
fn audio_to_midi_and_distant_pitch_cues_reveal_content_without_resetting_scale() {
    let (mut app, _, _, _, original) = fixture();
    let before = app.inspect();
    view_action(&mut app, "Next clip");
    view_action(&mut app, "Next clip");
    let low = view_action(&mut app, "Next clip");
    let visible = |state: &Value, pitch: u64| {
        let editor = &state["midi_editor"];
        let plot = &editor["plot"];
        let note = editor["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["pitch"] == pitch)
            .expect("target cue note is visible");
        let rect = &note["rect"];
        assert!(rect["min"][0].as_f64().unwrap() >= plot["min"][0].as_f64().unwrap());
        assert!(rect["max"][0].as_f64().unwrap() <= plot["max"][0].as_f64().unwrap());
        assert!(rect["min"][1].as_f64().unwrap() >= plot["min"][1].as_f64().unwrap());
        assert!(rect["max"][1].as_f64().unwrap() <= plot["max"][1].as_f64().unwrap());
    };
    visible(&low, 36);
    app.key("W");
    let plot = &low["midi_editor"]["plot"];
    let x = (plot["min"][0].as_f64().unwrap() + plot["max"][0].as_f64().unwrap()) * 0.5;
    let y = (plot["min"][1].as_f64().unwrap() + plot["max"][1].as_f64().unwrap()) * 0.5;
    let zoomed = app.command(json!({"command":"scroll","x":x,"y":y,"delta":80.0,"ctrl":true}));
    let high = view_action(&mut app, "Next clip");
    visible(&high, 108);
    for field in ["pixels_per_beat", "row_height", "tool"] {
        assert_eq!(
            high["midi_editor"][field], zoomed["midi_editor"][field],
            "retain {field}"
        );
    }
    assert_eq!(high["project"], before["project"]);
    assert_eq!(high["history"], before["history"]);
    source_unchanged(&app, &original);
    app.quit();
}
