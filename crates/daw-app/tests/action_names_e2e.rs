//! Action names expose their existing results through the running application.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, MAX_IMPORTED_AUDIO_BYTES, Note, Project, decode_audio_to_wav};
use serde_json::{Value, json};

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn context(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let id = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["name"] == name)
        .unwrap()["id"]
        .clone();
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    app.command(json!({
        "command":"pointer", "phase":"click", "button":"secondary",
        "x":(rect["min"][0].as_f64().unwrap()+rect["max"][0].as_f64().unwrap())*0.5,
        "y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())*0.5,
    }))
}

fn loop_actions(state: &Value) -> Vec<&str> {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|label| label.starts_with("Loop playback over") || *label == "Loop this clip")
        .collect()
}

#[test]
fn context_names_distinguish_playback_looping_from_independent_phrase_copies() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.loop_start = 0.0;
    project.loop_end = 8.5;
    project.loop_enabled = false;
    let track = project.add_midi_track("Phrase instrument");
    project.track_mut(track).unwrap().clips.extend([
        Clip::midi("Phrase A", 0.0, 1.0, vec![Note::new(60, 0.0, 1.0, 90)]),
        Clip::midi("Phrase B", 2.0, 1.0, vec![Note::new(67, 0.0, 1.0, 90)]),
    ]);
    let path = app.directory.path().join("named-actions.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1300, "height":1000}));
    context(&mut app, "Phrase A");
    let before = app.click("Select only this clip");
    assert!(
        has(&before, "Duplicate"),
        "The familiar toolbar tool stays visible"
    );
    let options = context(&mut app, "Phrase A");
    assert_eq!(loop_actions(&options), ["Loop playback over clip"]);
    assert!(has(&options, "Duplicate after"));
    assert!(has(&options, "Repeat to loop end"));

    let looped = app.click("Loop playback over clip");
    assert_eq!(looped["project"]["loop_start"], 0.0);
    assert_eq!(looped["project"]["loop_end"], 1.0);
    assert_eq!(looped["project"]["loop_enabled"], true);
    assert_eq!(looped["project"]["tracks"], before["project"]["tracks"]);
    let disabled = context(&mut app, "Phrase A");
    assert!(has(&disabled, "Extend the loop beyond the phrase"));
    let refused = app.click("Repeat to loop end");
    assert_eq!(refused["project"], looped["project"]);
    assert_eq!(refused["history"], looped["history"]);
    app.key("Escape");
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);

    context(&mut app, "Phrase A");
    let duplicated = app.click("Duplicate after");
    let clips = duplicated["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap();
    assert_eq!(clips.len(), 3);
    let original = &before["project"]["tracks"][0]["clips"][0];
    let copy = clips.iter().find(|clip| clip["start"] == 1.0).unwrap();
    assert_eq!(copy["length"], original["length"]);
    assert_ne!(copy["id"], original["id"]);
    assert!(
        copy["link_group"].is_null(),
        "Duplicate must stay independent"
    );
    assert_eq!(copy["content"]["Midi"]["notes"][0]["pitch"], 60);
    assert_ne!(
        copy["content"]["Midi"]["notes"][0]["id"],
        original["content"]["Midi"]["notes"][0]["id"]
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);

    context(&mut app, "Phrase A");
    let only_a_selected = app.click("Select only this clip");
    context(&mut app, "Phrase B");
    let only_b_repeated = app.click("Repeat to loop end");
    let clips = only_b_repeated["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap();
    assert_eq!(clips.len(), 8);
    let a_clips: Vec<_> = clips
        .iter()
        .filter(|clip| clip["content"]["Midi"]["notes"][0]["pitch"] == 60)
        .collect();
    assert_eq!(
        a_clips,
        vec![original],
        "An unrelated selection is not repeated"
    );
    let mut b_bounds: Vec<_> = clips
        .iter()
        .filter(|clip| clip["content"]["Midi"]["notes"][0]["pitch"] == 67)
        .map(|clip| {
            (
                clip["start"].as_f64().unwrap(),
                clip["length"].as_f64().unwrap(),
            )
        })
        .collect();
    b_bounds.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        b_bounds,
        vec![
            (2.0, 1.0),
            (3.0, 1.0),
            (4.0, 1.0),
            (5.0, 1.0),
            (6.0, 1.0),
            (7.0, 1.0),
            (8.0, 0.5)
        ],
        "Repeating outside the selection targets the pointed clip"
    );
    assert_eq!(
        only_b_repeated["history"]["undo"],
        json!(["Repeat to loop end"])
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        only_a_selected["project"]
    );

    context(&mut app, "Phrase A");
    app.click("Select only this clip");
    context(&mut app, "Phrase B");
    let selected = app.click("Add to selection");
    let options = context(&mut app, "Phrase A");
    assert_eq!(loop_actions(&options), ["Loop playback over selection"]);
    let repeated = app.click("Repeat to loop end");
    let mut bounds: Vec<_> = repeated["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|clip| {
            (
                clip["start"].as_f64().unwrap(),
                clip["length"].as_f64().unwrap(),
            )
        })
        .collect();
    bounds.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        bounds,
        vec![
            (0.0, 1.0),
            (2.0, 1.0),
            (3.0, 1.0),
            (5.0, 1.0),
            (6.0, 1.0),
            (8.0, 0.5)
        ],
        "Repeat keeps the phrase rest and trims only the final partial copy"
    );
    assert_eq!(repeated["history"]["undo"], json!(["Repeat to loop end"]));
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        selected["project"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        repeated["project"]
    );
    app.quit();
}

#[test]
fn export_audio_names_file_types_and_shows_bit_depth_only_for_pcm_formats() {
    let mut app = Harness::new();
    let source = app.directory.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..24_000)
            .map(|index| (index as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin() * 0.2)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Export tone");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Export phrase",
        0.0,
        1.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("export-names.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let mut current = "WAV";
    for (kind, extension, depth) in [
        ("WAV", "wav", true),
        ("FLAC", "flac", true),
        ("Ogg Vorbis", "ogg", false),
        ("MP3", "mp3", false),
    ] {
        let file = app.click("File");
        assert!(has(&file, "Export audio…"));
        assert!(!has(&file, "Export WAV…"));
        let options = app.click("Export audio…");
        assert_eq!(options["popups"]["export"], true);
        if options["export_include_effect_tails"] == true {
            app.click("Include effect tails");
        }
        if current != kind {
            app.click(current);
            app.click(kind);
        }
        let selected = app.inspect();
        assert!(
            has(&selected, "File type"),
            "{}",
            selected["visible_labels"]
        );
        assert_eq!(has(&selected, "Bit depth"), depth, "{kind}");
        assert!(!has(&selected, "Type"));
        assert!(!has(&selected, "Format"));
        if kind == "FLAC" {
            assert!(has(&selected, "24-bit PCM"));
            app.click("24-bit PCM");
            let choices = app.inspect();
            assert!(
                !has(&choices, "32-bit float"),
                "FLAC cannot offer float depth"
            );
            app.key("Escape");
        }
        let base = app.directory.path().join(format!("delivery-{extension}"));
        let output = base.with_extension(extension);
        app.command(json!({"command":"file_dialog", "kind":"export", "path":base}));
        app.click("Export…");
        let exported = app.command(json!({"command":"wait_export"}));
        assert_eq!(exported["exported_file"], json!(output));
        assert!(has(
            &exported,
            &format!("Exported · delivery-{extension}.{extension}")
        ));
        let decoded = app
            .directory
            .path()
            .join(format!("decoded-{extension}.wav"));
        decode_audio_to_wav(&output, &decoded, MAX_IMPORTED_AUDIO_BYTES).unwrap();
        let audio = AudioData::load_wav(decoded).unwrap();
        assert_eq!(audio.sample_rate, 48_000);
        assert!(audio.frames().abs_diff(24_000) <= 2_304, "{kind}");
        assert!(audio.samples.iter().all(|sample| sample.is_finite()));
        assert!(
            audio.samples.iter().any(|sample| sample.abs() > 0.05),
            "{kind}"
        );
        current = kind;
    }
    app.quit();
}
