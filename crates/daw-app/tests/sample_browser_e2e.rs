//! On-demand sample indexing and manual loading through the running Library.
mod common;
use common::Harness;
use daw_core::{AudioData, Instrument, Project};
use serde_json::{Value, json};
use std::path::Path;

fn audio(path: &Path) {
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4_800)
            .map(|index| (index as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin() * 0.3)
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}
fn library(app: &mut Harness) -> Value {
    app.click("Window");
    app.click("Device browser");
    app.click("Samples")
}
fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}
fn instrument_sample(state: &Value, track: u64) -> Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == track)
        .unwrap()["instrument"]["Builtin"]["samples"]
        .as_array()
        .and_then(|samples| samples.first())
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn roots_are_indexed_only_on_request_and_cached_tree_is_searchable_after_restart() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let root = app.directory.path().join("Sample library");
    let drums = root.join("Drums");
    std::fs::create_dir_all(&drums).unwrap();
    audio(&drums.join("Kick.WAV"));
    audio(&root.join("Bell.wav"));
    std::fs::write(drums.join("Readme.txt"), "not audio").unwrap();
    let initial = library(&mut app);
    assert_eq!(initial["library"]["indexed"], 0);
    app.click("Folders…");
    app.click("Sample folder path");
    app.command(json!({"command":"text","text":root}));
    let added = app.click("Add folder");
    assert_eq!(
        added["library"]["indexed"], 0,
        "adding a root must not scan"
    );
    assert_eq!(added["library"]["scanning"], false);
    assert!(
        !app.directory
            .path()
            .join("config/sample-index.json")
            .exists()
    );
    app.click("Done");
    app.click("Scan folders");
    let scanned = common::wait_until(&mut app, |state| {
        state["library"]["indexed"] == 2 && state["library"]["scanning"] == false
    });
    assert!(visible(&scanned, "Bell.wav"));
    assert!(!visible(&scanned, "Readme.txt"));
    app.click("Search samples");
    let searched = app.command(json!({"command":"text","text":"dRuMs kIcK"}));
    assert_eq!(searched["library"]["total"], 1);
    assert!(
        visible(&searched, "Kick.WAV"),
        "search expands matching folder paths"
    );
    assert!(!visible(&searched, "Bell.wav"));
    audio(&root.join("New.wav"));
    app.click("Clear search");
    assert_eq!(
        app.inspect()["library"]["indexed"],
        2,
        "opening and editing search do not rescan"
    );
    let mut app = app.restart();
    let cached = library(&mut app);
    assert_eq!(
        cached["library"]["indexed"], 2,
        "startup reads saved index without scanning roots"
    );
    assert!(!visible(&cached, "New.wav"));
    app.click("Scan folders");
    common::wait_until(&mut app, |state| {
        state["library"]["indexed"] == 3 && state["library"]["scanning"] == false
    });
    app.click("Folders…");
    let removed = app.click("Remove folder");
    assert_eq!(removed["library"]["roots"].as_array().unwrap().len(), 0);
    assert_eq!(removed["library"]["total"], 0);
    app.quit();
}

#[test]
fn project_samples_load_manually_into_sampler_and_undo_without_changing_the_pool() {
    let mut app = Harness::new();
    let source = app.directory.path().join("source.wav");
    audio(&source);
    let mut project = Project::new();
    let track = project.add_instrument_track("Sample instrument");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project
        .add_project_sample("Pool kick", source.to_string_lossy())
        .unwrap();
    let alias = project
        .add_project_sample("Soft kick", source.to_string_lossy())
        .unwrap();
    let song = app.directory.path().join("pool.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    let before = library(&mut app);
    assert_eq!(before["library"]["total"], 2);
    assert!(visible(&before, "Pool kick"));
    assert!(visible(&before, "Soft kick"));
    assert_eq!(
        before["project"]["samples"][0]["path"], before["project"]["samples"][1]["path"],
        "archive deduplicates the aliases' audio"
    );
    app.click("Pool kick");
    let selected = app.click("Soft kick");
    assert_eq!(selected["library"]["selected"], format!("project:{alias}"));
    app.click("Use…");
    app.click("Load into instrument…");
    app.click("Load sample");
    let loaded = common::wait_until(&mut app, |state| {
        instrument_sample(state, track)["name"] == "Soft kick"
    });
    assert_eq!(instrument_sample(&loaded, track)["name"], "Soft kick");
    assert_eq!(
        loaded["popups"]["library"], true,
        "ordinary explicit load stays in the browser"
    );
    assert_eq!(loaded["project"]["samples"], before["project"]["samples"]);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert!(instrument_sample(&undone, track).is_null());
    assert_eq!(undone["project"]["samples"], before["project"]["samples"]);
    app.command(json!({"command":"file_dialog","kind":"import_audio","path":source}));
    app.click("Use…");
    app.click("Use file…");
    let imported = common::wait_until(&mut app, |state| {
        state["project"]["samples"].as_array().unwrap().len() == 3
    });
    let sample = &imported["project"]["samples"][2];
    assert_ne!(sample["path"].as_str().unwrap(), source.to_str().unwrap());
    assert!(Path::new(sample["path"].as_str().unwrap()).is_file());
    assert!(
        instrument_sample(&imported, track).is_null(),
        "ordinary file admission adds to the pool"
    );
    app.quit();
}

fn rightmost_label(state: &Value, label: &str) -> Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["text"] == label)
        .max_by(|a, b| {
            a["min"][0]
                .as_f64()
                .unwrap()
                .total_cmp(&b["min"][0].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]))
        .clone()
}
fn click_rightmost(app: &mut Harness, label: &str) -> Value {
    let rect = rightmost_label(&app.inspect(), label);
    app.command(json!({"command":"pointer","phase":"click","x":(rect["min"][0].as_f64().unwrap()+rect["max"][0].as_f64().unwrap())/2.0,"y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())/2.0}))
}
fn configure_root(app: &mut Harness, root: &Path) {
    app.click("Folders…");
    app.click("Sample folder path");
    app.command(json!({"command":"text","text":root}));
    app.click("Add folder");
    app.click("Done");
    app.click("Scan folders");
    common::wait_until(app, |state| {
        state["library"]["indexed"].as_u64().unwrap_or(0) > 0
            && state["library"]["scanning"] == false
    });
}

#[test]
fn global_external_favorites_edit_search_and_survive_root_removal_and_restart() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1500,"height":1050}));
    let root = app.directory.path().join("Library");
    let drums = root.join("DRUMS");
    std::fs::create_dir_all(&drums).unwrap();
    let kick = drums.join("Kick.wav");
    audio(&kick);
    audio(&root.join("Air.wav"));
    library(&mut app);
    configure_root(&mut app, &root);
    let before = app.inspect();
    app.click("Search samples");
    let searched = app.command(json!({"command":"text","text":"drums"}));
    assert_eq!(
        searched["library"]["total"], 1,
        "folder-only search retains descendants"
    );
    app.click("Kick.wav");
    let metadata = common::wait_until(&mut app, |state| {
        state["library"]["metadata"]["state"] == "ready"
    });
    assert!(
        (metadata["library"]["metadata"]["duration_seconds"]
            .as_f64()
            .unwrap()
            - 0.1)
            .abs()
            < 0.001
    );
    assert_eq!(metadata["library"]["metadata"]["sample_rate"], 48000);
    click_rightmost(&mut app, "Add favorite");
    common::wait_until(&mut app, |state| {
        state["library"]["favorites"].as_array().unwrap().len() == 1
            && state["library"]["favorite_busy"] == false
    });
    app.click("Edit tag…");
    click_rightmost(&mut app, "Keyboard");
    app.click("Caps");
    app.click("P");
    app.click("Caps");
    for key in ["u", "n", "c", "h", "y"] {
        app.click(key);
    }
    app.click("Done");
    app.command(json!({"command":"click","label":"Star","occurrence":1}));
    app.click("Drum");
    // Edit the actual native color picker beside its visible label.
    let color = rightmost_label(&app.inspect(), "Tag color");
    let picker=app.command(json!({"command":"pointer","phase":"click","x":color["max"][0].as_f64().unwrap()+30.0,"y":(color["min"][1].as_f64().unwrap()+color["max"][1].as_f64().unwrap())/2.0}));
    let red = picker["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|label| label.starts_with("R "))
        .expect("native RGB controls")
        .to_owned();
    let red = rightmost_label(&picker, &red);
    app.command(json!({"command":"pointer","phase":"double_click","x":(red["min"][0].as_f64().unwrap()+red["max"][0].as_f64().unwrap())/2.0,"y":(red["min"][1].as_f64().unwrap()+red["max"][1].as_f64().unwrap())/2.0}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"220"}));
    app.key("Enter");
    app.key("Escape");
    let tagged = app.click("Apply tag");
    let favorite = tagged["library"]["favorites"][0].clone();
    assert_eq!(favorite["favorite"]["tag"], "Punchy");
    assert_eq!(favorite["favorite"]["icon"], "drum");
    assert!(favorite["favorite"]["color"][0].as_u64().unwrap() > 200);
    assert_eq!(tagged["project"], before["project"]);
    assert_eq!(tagged["history"], before["history"]);
    app.click("Clear search");
    app.click("Search samples");
    let found = app.command(json!({"command":"text","text":"pUnChY drums"}));
    assert_eq!(
        found["library"]["total"], 1,
        "search combines tag and folder words"
    );
    app.click("Clear search");
    app.click("Folders…");
    app.click("Remove folder");
    app.click("Done");
    let only = app.click("Favorites");
    assert_eq!(only["library"]["total"], 1);
    assert_eq!(only["library"]["favorites"][0], favorite);
    let mut app = app.restart();
    library(&mut app);
    app.click("Favorites");
    assert_eq!(app.inspect()["library"]["favorites"][0], favorite);
    std::fs::remove_file(&kick).unwrap();
    app.click("Kick.wav");
    let missing = common::wait_until(&mut app, |state| {
        state["library"]["metadata"]["state"] == "failed"
    });
    assert!(
        missing["library"]["metadata"]["error"]
            .as_str()
            .unwrap()
            .len()
            > 5
    );
    click_rightmost(&mut app, "Remove favorite");
    assert!(
        app.inspect()["library"]["favorites"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    app.quit();
}

#[test]
fn favoriting_a_project_alias_creates_one_global_sound_that_survives_closing_the_project() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1500,"height":1000}));
    let path = app.directory.path().join("original.wav");
    audio(&path);
    let mut project = Project::new();
    project
        .add_project_sample("Original", path.to_string_lossy())
        .unwrap();
    project
        .add_project_sample("Alternate", path.to_string_lossy())
        .unwrap();
    let song = app.directory.path().join("source.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    library(&mut app);
    app.click("Alternate");
    let before = app.inspect();
    click_rightmost(&mut app, "Add favorite");
    let favorite = common::wait_until(&mut app, |state| {
        state["library"]["favorites"].as_array().unwrap().len() == 1
            && state["library"]["favorite_busy"] == false
    });
    assert_eq!(favorite["project"], before["project"]);
    assert_eq!(favorite["history"], before["history"]);
    let entry = favorite["library"]["favorites"][0].clone();
    assert_eq!(entry["name"], "Alternate");
    let retained = Path::new(entry["path"].as_str().unwrap());
    assert!(retained.is_file());
    assert_ne!(retained, path);
    let empty = app.directory.path().join("empty.rill");
    Project::new().save(&empty).unwrap();
    app.command(json!({"command":"open","path":empty}));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&song).unwrap();
    let mut app = app.restart();
    library(&mut app);
    let favorites = app.click("Favorites");
    assert_eq!(favorites["library"]["favorites"][0], entry);
    app.click("Alternate");
    let metadata = common::wait_until(&mut app, |state| {
        state["library"]["metadata"]["state"] == "ready"
    });
    assert!(
        metadata["library"]["metadata"]["duration_seconds"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    app.quit();
}

#[test]
fn autoplay_toggles_switches_and_stops_in_browser_and_player_palette_without_song_edits() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1500,"height":1050}));
    let mut project = Project::new();
    let track = project.add_instrument_track("Sample destination");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    for (name, hz) in [("Low tone", 220.0), ("High tone", 880.0)] {
        let path = app.directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: (0..48000)
                .map(|frame| (frame as f32 * std::f32::consts::TAU * hz / 48000.0).sin() * 0.25)
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        project
            .add_project_sample(name, path.to_string_lossy())
            .unwrap();
    }
    let song = app.directory.path().join("auto.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    library(&mut app);
    let before = app.inspect();
    assert_eq!(before["library"]["auto_play"], false);
    app.click("Low tone");
    assert_ne!(app.inspect()["engine"]["sample_audition_active"], true);
    app.click("Auto-play");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    app.click("High tone");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    let high = app.command(json!({"command":"pump_audio","frames":8192}));
    let values: Vec<_> = high["audio_output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame[0].as_f64().unwrap())
        .collect();
    let crossings = values
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!((crossings as f64 * 48000.0 / values.len() as f64 - 880.0).abs() < 20.0);
    app.click("Auto-play");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(stopped["engine"]["sample_audition_active"], false);
    assert_eq!(stopped["project"], before["project"]);
    assert_eq!(stopped["history"], before["history"]);
    app.click("Auto-play");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Edit player…");
    app.click("Edit player…");
    app.click("Low tone");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    app.click("Auto-play");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(stopped["engine"]["sample_audition_active"], false);
    assert_eq!(stopped["project"], before["project"]);
    assert_eq!(stopped["history"], before["history"]);
    let expanded = app.click("Details…");
    assert_eq!(
        expanded["library"]["captured"], true,
        "the editor's selected implicit Sample atom is a valid destination"
    );
    assert_eq!(expanded["library"]["target"], json!([track, 0]));
    app.quit();
}

#[test]
fn folder_scope_keeps_sources_stable_and_search_can_expand_without_rescanning() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    let root = app.directory.path().join("Sample roots");
    let drums = root.join("Drums");
    let hats = drums.join("Hats");
    std::fs::create_dir_all(&hats).unwrap();
    audio(&root.join("Bell.wav"));
    audio(&drums.join("Kick.wav"));
    audio(&hats.join("Closed hat.wav"));
    library(&mut app);
    configure_root(&mut app, &root);
    let expanded = app.click("Expand Sample roots");
    assert_eq!(
        expanded["library"]["scope"], "all",
        "expanding does not select a folder"
    );
    app.click("Expand Drums");
    let scoped = app.click("Drums");
    assert_eq!(
        scoped["library"]["scope"],
        format!("folder:{}", drums.display())
    );
    assert_eq!(
        scoped["library"]["total"], 2,
        "folder includes nested descendants"
    );
    let source_ids = |state: &Value| {
        state["library"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>()
    };
    let sources = source_ids(&scoped);
    app.click("Kick.wav");
    app.click("Search samples");
    let empty = app.command(json!({"command":"text","text":"bElL"}));
    assert_eq!(empty["library"]["total"], 0);
    assert!(empty["library"]["selected"].is_null());
    assert_eq!(
        source_ids(&empty),
        sources,
        "search never filters or expands sources"
    );
    assert!(
        visible(&empty, "Project samples"),
        "pool drop target remains mounted"
    );
    let all = app.click("Search all samples");
    assert_eq!(all["library"]["scope"], "all");
    assert_eq!(all["library"]["query"], "bElL");
    assert_eq!(all["library"]["total"], 1);
    assert!(visible(&all, "Bell.wav"));
    assert_eq!(source_ids(&all), sources);
    let cleared = app.click("Clear search");
    assert_eq!(cleared["library"]["total"], 3);
    assert_eq!(cleared["library"]["indexed"], 3);
    app.click("Favorites");
    let empty_favorites = app.inspect();
    assert_eq!(empty_favorites["library"]["total"], 0);
    assert_eq!(
        empty_favorites["library"]["drop_targets"][0]["id"],
        "project"
    );
    assert!(visible(&empty_favorites, "Browse all samples"));
    // A disconnected root is readable feedback in the fixed source footer,
    // including at the minimum viewport; it never hides the source navigation.
    std::fs::remove_dir_all(&root).unwrap();
    app.click("Scan folders");
    common::wait_visible(&mut app, "Some folders unavailable");
    app.click("Details");
    assert!(visible(&app.inspect(), "Sample scan details"));
    let closed = app.click("Done");
    assert_eq!(closed["library"]["scope"], "favorites");
    assert_eq!(closed["popups"]["library"], true);
    app.quit();
}

#[test]
fn palette_expansion_preserves_scope_query_selection_and_selected_leaf_context() {
    use daw_core::{PlayerKind, PlayerNode};
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    let sample = app.directory.path().join("palette.wav");
    audio(&sample);
    let mut project = Project::new();
    let track = project.add_instrument_track("Palette instrument");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project
        .set_sample_player(
            track,
            0,
            Some(PlayerNode {
                id: 1,
                name: "Layer stack".into(),
                kind: PlayerKind::All {
                    children: vec![
                        PlayerNode {
                            id: 2,
                            name: "Chosen leaf".into(),
                            ..PlayerNode::sample(2, 3)
                        },
                        PlayerNode::sample(3, 4),
                    ],
                },
            }),
        )
        .unwrap();
    let sample_id = project
        .add_project_sample("Palette bell", sample.to_string_lossy())
        .unwrap();
    let song = app.directory.path().join("palette.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Edit player…");
    app.click("Edit player…");
    let composite = app.click("Details…");
    assert_eq!(
        composite["library"]["captured"], false,
        "a composite selection does not invent a leaf destination"
    );
    assert!(composite["library"]["target"].is_null());
    app.click("Back");
    let state = app.inspect();
    let leaf = state["sample_player"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == 2)
        .unwrap();
    app.command(json!({"command":"pointer","phase":"click","x":(leaf["min"][0].as_f64().unwrap()+leaf["max"][0].as_f64().unwrap())/2.0,"y":(leaf["min"][1].as_f64().unwrap()+leaf["max"][1].as_f64().unwrap())/2.0}));
    app.click("Project samples");
    app.click("Search samples");
    app.command(json!({"command":"text","text":"bell"}));
    app.click("Palette bell");
    let palette = app.inspect();
    assert_eq!(palette["sample_player"]["palette"]["scope"], "project");
    assert!(visible(&palette, "Preview"));
    assert!(visible(&palette, "In Project Samples"));
    let bounds = &palette["sample_player"]["palette"]["bounds"];
    for label in ["Details…", "Keyboard", "1 sample"] {
        let rect = palette["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["text"] == label)
            .unwrap();
        assert!(rect["min"][0].as_f64().unwrap() >= bounds["min"][0].as_f64().unwrap());
        assert!(
            rect["max"][0].as_f64().unwrap() <= bounds["max"][0].as_f64().unwrap(),
            "{label} must fit the palette: {rect}, {bounds}"
        );
    }
    let full = app.click("Details…");
    assert_eq!(full["library"]["scope"], "project");
    assert_eq!(full["library"]["query"], "bell");
    assert_eq!(full["library"]["selected"], format!("project:{sample_id}"));
    assert_eq!(full["library"]["target"], json!([track, 3]));
    assert_eq!(full["library"]["captured"], true);
    assert!(visible(&full, "Load sample"));
    app.click("Load sample");
    let returned = common::wait_until(&mut app, |state| {
        state["popups"]["sample_player"] == true && state["popups"]["library"] == false
    });
    assert_eq!(instrument_sample(&returned, track)["slot"], 3);
    assert_eq!(
        returned["sample_player"]["palette"]["selected"],
        format!("project:{sample_id}")
    );
    let ordinary = library(&mut app);
    assert_eq!(
        ordinary["library"]["captured"], false,
        "a fresh global browser entry must not reuse a dismissed picker"
    );
    assert!(ordinary["library"]["target"].is_null());
    assert!(visible(&ordinary, "In Project Samples"));
    app.quit();
}
