//! Named track densities are persistent view preferences, not song edits.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn unchanged(before: &Value, after: &Value) {
    for key in ["project", "history", "revision", "modified"] {
        assert_eq!(
            before[key], after[key],
            "A density/view action changed {key}"
        );
    }
}

fn item<'a>(state: &'a Value, list: &str, track: u64) -> &'a Value {
    state["timeline"][list]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["track"] == track)
        .unwrap()
}

fn point(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value, secondary: bool) -> Value {
    let (x, y) = point(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,
        "button":if secondary { "secondary" } else { "primary" }}))
}

fn preset(app: &mut Harness, name: &str) -> Value {
    app.click("View");
    app.click("Track density");
    app.click(name)
}

fn full_track_count(state: &Value) -> usize {
    let canvas = &state["timeline"]["canvas"];
    state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| {
            track["folder"] != true
                && track["header"]["min"][1].as_f64().unwrap() >= canvas["min"][1].as_f64().unwrap()
                && track["header"]["max"][1].as_f64().unwrap() <= canvas["max"][1].as_f64().unwrap()
        })
        .count()
}

#[test]
fn laptop_density_presets_keep_channel_actions_and_remember_the_view_without_edits() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let folder = project.add_folder("Drums");
    let first = project.add_audio_track("Kick");
    let second = project.add_audio_track("Snare");
    project.set_parent_folder(first, Some(folder)).unwrap();
    project.set_parent_folder(second, Some(folder)).unwrap();
    project.track_mut(second).unwrap().collapsed = true;
    for index in 0..10 {
        project.add_audio_track(format!("Channel {}", index + 3));
    }
    let path = app.directory.path().join("density.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let initial = app.command(json!({"command":"viewport","width":1280,"height":800}));
    assert_eq!(initial["timeline"]["density"], "Compact");
    assert_eq!(initial["timeline"]["row_height"], 64.0);

    for (dark, scale) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":scale}),
        );
        let loop_start = app.inspect();
        preset(&mut app, "Compact");
        let compact = app.inspect();
        unchanged(&loop_start, &compact);
        let compact_count = full_track_count(&compact);
        assert!(
            compact_count >= 6,
            "A laptop must show at least six complete source tracks, saw {compact_count}"
        );
        for id in [first, second] {
            let track = item(&compact, "tracks", id);
            assert_eq!(track["depth"], 1);
            for section in ["mix", "meter", "record_arm", "details"] {
                assert!(
                    !track["sections"][section].is_null(),
                    "Compact {section} missing for {id}"
                );
            }
            for control in ["gain", "pan", "mute", "solo"] {
                let bounds = &item(&compact, "channels", id)[control];
                assert!(
                    bounds["max"][1].as_f64().unwrap() - bounds["min"][1].as_f64().unwrap() >= 20.0
                );
            }
            let grip = &track["reorder_grip"];
            assert!(grip["max"][0].as_f64().unwrap() - grip["min"][0].as_f64().unwrap() >= 13.0);
        }
        let gain = item(&compact, "channels", first)["gain"].clone();
        let menu = click_rect(&mut app, &gain, true);
        unchanged(&loop_start, &menu);
        assert!(
            menu["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == "Automate")
        );
        app.key("Escape");
        let compact = app.inspect();
        let details = item(&compact, "tracks", first)["sections"]["details"].clone();
        let channel = click_rect(&mut app, &details, false);
        unchanged(&loop_start, &channel);
        assert_eq!(channel["popups"]["properties"], true);
        assert_eq!(channel["selected_track"], first);
        app.click("Arranger");

        let state = app.inspect();
        let muted = click_rect(&mut app, &item(&state, "channels", first)["mute"], false);
        assert_eq!(
            muted["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|track| track["id"] == first)
                .unwrap()["mute"],
            true
        );
        app.click("Edit");
        let undone = app.click("Undo");
        assert_eq!(undone["project"], initial["project"]);
        // Undo creates a redo branch; density changes must preserve that branch.
        let baseline = app.inspect();
        preset(&mut app, "Detailed");
        let detailed = app.inspect();
        unchanged(&baseline, &detailed);
        assert_eq!(detailed["timeline"]["row_height"], 144.0);
        let detailed_count = full_track_count(&detailed);
        assert!(detailed_count < compact_count);
        eprintln!(
            "1280×800 @ {scale}×: {compact_count} complete source tracks in Compact, {detailed_count} in Detailed"
        );
        assert!(!item(&detailed, "tracks", first)["sections"]["routing"].is_null());
        preset(&mut app, "Standard");
        let standard = app.inspect();
        unchanged(&baseline, &standard);
        assert_eq!(standard["timeline"]["row_height"], 89.0);
        assert_eq!(standard["timeline"]["preferred_density"], "Standard");
    }
    let mut app = app.restart();
    app.command(json!({"command":"open","path":path}));
    let restored = app.inspect();
    assert_eq!(restored["timeline"]["row_height"], 89.0);
    assert_eq!(restored["timeline"]["preferred_density"], "Standard");
    assert_eq!(restored["project"], serde_json::to_value(project).unwrap());
    app.quit();
}
