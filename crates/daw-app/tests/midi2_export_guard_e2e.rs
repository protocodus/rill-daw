//! Rich MIDI projects require an explicit MIDI 1.0 export-copy decision.
mod common;
use common::Harness;
use daw_core::{Clip, Midi2ChannelVoice, Note, Project};
use serde_json::json;

#[test]
fn midi1_export_copy_warns_before_the_picker_and_preserves_the_rich_project() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Expressive phrase");
    let mut note = Note::new(60, 0.25, 1.0, 90);
    note.set_velocity16(40_001);
    note.set_release_velocity16(12_345);
    let note_id = note.id;
    let clip = Clip::midi("Rich notes", 2.0, 4.0, vec![note]);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let pressure = Midi2ChannelVoice {
        group: 0,
        channel: 0,
        status: 10,
        index: 60,
        detail: 0,
        data: 0x8123_4567,
    }
    .packet()
    .unwrap();
    project
        .upsert_note_expression(track, clip_id, note_id, 0.125, pressure)
        .unwrap();
    let path = app.directory.path().join("expressive.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open","path":path}));
    let midi = app.directory.path().join("midi1-copy.mid");
    std::fs::write(&midi, b"existing destination").unwrap();
    app.command(
        json!({"command":"file_dialog","kind":"export_midi","path":midi,"replace_existing":true}),
    );
    app.click("File");
    let warning = app.click("Export MIDI…");
    assert_eq!(warning["popups"]["midi_export"], true);
    assert!(
        warning["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label
                .as_str()
                .is_some_and(|label| label.contains("per-note expression are omitted")))
    );
    app.click("Cancel");
    assert_eq!(std::fs::read(&midi).unwrap(), b"existing destination");
    assert_eq!(app.inspect()["project"], before["project"]);
    app.click("File");
    app.click("Export MIDI…");
    let copied = app.click("Export MIDI 1.0 copy…");
    assert_eq!(copied["project"], before["project"]);
    assert_eq!(copied["history"], before["history"]);
    assert!(std::fs::read(&midi).unwrap().starts_with(b"MThd"));
    let saved = app.directory.path().join("retained.rill");
    app.command(json!({"command":"save","path":saved}));
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(reopened["project"], before["project"]);
    assert_eq!(
        reopened["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"][0]["midi2"]["velocity"],
        40_001
    );
    assert_eq!(
        reopened["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"][0]["midi2"]["expressions"]
            [0]["packet"]["words"][1],
        0x8123_4567_u32
    );
    app.quit();
}
