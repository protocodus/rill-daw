//! New arranger tracks receive distinct colors without recoloring existing work.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

fn menu(app: &mut Harness, title: &str, item: &str) -> Value {
    app.click(title);
    app.click(item)
}

fn colors(state: &Value) -> BTreeMap<u64, [u8; 3]> {
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    project
        .tracks
        .iter()
        .map(|track| (track.id, track.color))
        .collect()
}

fn assert_new_color(previous: &BTreeMap<u64, [u8; 3]>, state: &Value) -> BTreeMap<u64, [u8; 3]> {
    let current = colors(state);
    assert_eq!(current.len(), previous.len() + 1);
    for (id, color) in previous {
        assert_eq!(
            current.get(id),
            Some(color),
            "Adding a track recolored existing track {id}"
        );
    }
    let added = current
        .iter()
        .find(|(id, _)| !previous.contains_key(id))
        .unwrap()
        .1;
    assert!(
        !previous.values().any(|color| color == added),
        "New track reused an occupied color before the palette was exhausted"
    );
    current
}

#[test]
fn arranger_additions_advance_colors_and_survive_undo_redo_and_project_reopen() {
    let mut app = Harness::new();
    let mut assigned = BTreeMap::new();
    let mut before_last = BTreeMap::new();
    for item in [
        "Add audio track",
        "Add audio track",
        "Add audio track",
        "Add MIDI track",
        "Add bus",
        "Add folder",
        "Add MIDI track",
        "Add audio track",
    ] {
        before_last = assigned.clone();
        let state = menu(&mut app, "Track", item);
        assigned = assert_new_color(&assigned, &state);
        if item == "Add MIDI track" {
            // Cancel assignment: color belongs to the new track, independently
            // of whether an instrument has been selected yet.
            app.key("Escape");
        }
    }
    assert_eq!(
        assigned.values().collect::<HashSet<_>>().len(),
        assigned.len()
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(colors(&undone), before_last);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(
        colors(&redone),
        assigned,
        "Redo must restore the original assignment"
    );

    let path = app.directory.path().join("track-colors.rill");
    let saved = app.command(json!({"command":"save", "path":path}));
    assert_eq!(colors(&saved), assigned);
    app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":path}));
    assert_eq!(colors(&reopened), assigned);
    let next = menu(&mut app, "Track", "Add audio track");
    assert_new_color(&assigned, &next);
    app.quit();
}

#[test]
fn audio_drops_and_instrument_creation_advance_colors_but_existing_tracks_keep_theirs() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    let existing = project.add_audio_track("Custom color");
    project.track_mut(existing).unwrap().color = [12, 34, 56];
    let path = app.directory.path().join("custom-color.rill");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open", "path":path}));
    let mut assigned = colors(&initial);
    let audio = app.directory.path().join("drum.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 4_800],
    }
    .write_wav(&audio)
    .unwrap();

    let filled = app.command(json!({"command":"drop", "paths":[audio], "track":0, "beat":0.0}));
    assert_eq!(
        colors(&filled),
        assigned,
        "Dropping into an existing track must preserve its custom color"
    );
    assert_eq!(
        filled["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for beat in [0.0, 1.0] {
        let dropped = app.command(json!({"command":"drop", "paths":[audio], "beat":beat}));
        assigned = assert_new_color(&assigned, &dropped);
    }

    menu(&mut app, "Track", "Add instrument…");
    app.click("Search instruments");
    app.command(json!({"command":"text", "text":"Rill Fixture A"}));
    // The query field itself already displays the plugin name while scanning;
    // wait for the actual creation action, not a matching text-field label.
    common::wait_visible(&mut app, "Add instrument track");
    let instrument = app.click("Add instrument track");
    assert_new_color(&assigned, &instrument);
    let created = instrument["project"]["tracks"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(created["kind"], "Midi");
    assert_eq!(created["name"], "Rill Fixture A");
    assert_eq!(colors(&instrument)[&existing], [12, 34, 56]);
    app.quit();
}
