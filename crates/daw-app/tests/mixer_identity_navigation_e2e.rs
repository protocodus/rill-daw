//! Distinct downstream strips and bidirectional track navigation in the running app.
mod common;

use common::{Harness, wait_until};
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn context(app: &mut Harness, rect: &Value, action: &str) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "button":"secondary", "x":x, "y":y}));
    app.click(action)
}

fn unchanged(state: &Value, baseline: &Value) {
    for field in ["project", "history", "revision", "modified"] {
        assert_eq!(state[field], baseline[field], "Viewing changed {field}");
    }
}

fn inside_x(rect: &Value, viewport: &Value) -> bool {
    rect["min"][0].as_f64().unwrap() >= viewport["min"][0].as_f64().unwrap() - 1.0
        && rect["max"][0].as_f64().unwrap() <= viewport["max"][0].as_f64().unwrap() + 1.0
}

fn inside(rect: &Value, viewport: &Value) -> bool {
    (0..2).all(|axis| {
        rect["min"][axis].as_f64().unwrap() >= viewport["min"][axis].as_f64().unwrap() - 1.0
            && rect["max"][axis].as_f64().unwrap() <= viewport["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn identity_visible(state: &Value, track: u64) -> bool {
    let channel = channel(state, track);
    let viewport = &state["mixer"]["reorder_viewport"];
    inside(&channel["name"], viewport)
        && inside(&channel["source"], viewport)
        && (channel["role_heading"].is_null() || inside(&channel["role_heading"], viewport))
}

fn revealed_identity(app: &mut Harness, track: u64) -> Value {
    wait_until(app, |state| identity_visible(state, track));
    // Let the ordinary reveal animation settle before clicking its moving name.
    let mut state = app.inspect();
    for _ in 0..24 {
        state = app.inspect();
    }
    assert!(identity_visible(&state, track));
    state
}

#[test]
fn buses_and_send_returns_have_large_headings_and_lowered_usable_strips() {
    for (dark, width, height) in [(false, 1440, 1100), (true, 800, 600)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Dry source");
        let bus = project.add_bus("Music bus");
        let send = project.add_bus("Room return");
        project.set_output(source, Some(bus)).unwrap();
        project.add_send(source, send, 0.3, false).unwrap();
        // Mixed main-output and send destinations remain explicitly returns.
        project.set_output(bus, Some(send)).unwrap();
        let path = app.directory.path().join("roles.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":width, "height":height}));
        let before = app.click("Mixer");
        assert_eq!(channel(&before, source)["role"], "Track");
        assert!(channel(&before, source)["role_heading"].is_null());
        for (id, role) in [(bus, "Bus"), (send, "Send return")] {
            let downstream = channel(&before, id);
            assert_eq!(downstream["role"], role);
            let heading = &downstream["role_heading"];
            assert!(
                heading["max"][1].as_f64().unwrap()
                    <= downstream["strip"]["min"][1].as_f64().unwrap() + 0.1
            );
            assert!(
                (downstream["strip"]["min"][1].as_f64().unwrap()
                    - channel(&before, source)["strip"]["min"][1]
                        .as_f64()
                        .unwrap()
                    - before["mixer"]["role_height"].as_f64().unwrap())
                .abs()
                    < 1.0
            );
            assert!(
                (downstream["strip"]["max"][1].as_f64().unwrap()
                    - channel(&before, source)["strip"]["max"][1]
                        .as_f64()
                        .unwrap())
                .abs()
                    < 1.0
            );
            let fader = &downstream["fader"];
            assert!(
                fader["max"][1].as_f64().unwrap() - fader["min"][1].as_f64().unwrap()
                    >= f64::from(daw_ui::tokens::MIXER_FADER_MIN) - 1.0
            );
        }
        unchanged(&app.inspect(), &before);
        // A lowered bus heading and its clickable identity remain reachable
        // when selected through the persistent workspace choices too.
        let name = channel(&before, send)["name"].clone();
        context(&mut app, &name, "Select channel");
        app.click("Arranger");
        app.click("Mixer");
        let revealed = revealed_identity(&mut app, send);
        let (x, y) = center(&channel(&revealed, send)["name"]);
        let properties = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
        assert_eq!(properties["popups"]["properties"], true);
        unchanged(&properties, &before);
        app.key("Escape");
        app.quit();
    }
}

#[test]
fn mixer_arranger_navigation_reveals_distant_tracks_inside_collapsed_folders_without_edits() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        for index in 0..12 {
            project.add_audio_track(format!("Source {index}"));
        }
        let folder = project.add_folder("Collapsed group");
        let nested = project.add_folder("Nested group");
        project.set_parent_folder(nested, Some(folder)).unwrap();
        let target = project.add_midi_track("Hidden target");
        project.set_parent_folder(target, Some(nested)).unwrap();
        let first = Clip::midi(
            "Hidden phrase one",
            0.0,
            2.0,
            vec![Note::new(60, 0.0, 1.0, 90)],
        );
        let second = Clip::midi(
            "Hidden phrase two",
            4.0,
            2.0,
            vec![Note::new(64, 0.0, 1.0, 90)],
        );
        let second_id = second.id;
        project
            .track_mut(target)
            .unwrap()
            .clips
            .extend([first, second]);
        project.track_mut(folder).unwrap().collapsed = true;
        project.track_mut(nested).unwrap().collapsed = true;
        let path = app.directory.path().join("navigate.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":2400, "height":900}));
        let initial = app.click("Mixer");
        let name = channel(&initial, target)["name"].clone();
        let selected = context(&mut app, &name, "Select channel");
        assert_eq!(selected["selected_track"], target);
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        let arranged = app.click("Show in Arranger");
        assert_eq!(arranged["view"], "Arrangement");
        assert_eq!(arranged["selected_track"], target);
        let header = arranged["timeline"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["track"] == target)
            .expect("The hidden track is actually rendered")["header"]
            .clone();
        assert!(
            header["min"][1].as_f64().unwrap()
                >= arranged["timeline"]["canvas"]["min"][1].as_f64().unwrap()
        );
        assert!(
            header["max"][1].as_f64().unwrap()
                <= arranged["timeline"]["canvas"]["max"][1].as_f64().unwrap() + 1.0
        );
        unchanged(&arranged, &initial);
        // Selection navigation uses the revealed layout as well as the painter.
        app.click("Hidden phrase one");
        let next = app.key("ArrowRight");
        assert_eq!(next["selected_clip"], second_id);
        unchanged(&next, &initial);
        let name = arranged["timeline"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["track"] == target)
            .unwrap()["name"]
            .clone();
        let returned = context(&mut app, &name, "Show in Mixer");
        assert_eq!(returned["view"], "Mixer");
        let revealed = revealed_identity(&mut app, target);
        assert_eq!(revealed["selected_track"], target);
        unchanged(&revealed, &initial);
        // At 800×600 the revealed name must also accept an actual mouse click.
        let (x, y) = center(&channel(&revealed, target)["name"]);
        let properties = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
        assert_eq!(properties["popups"]["properties"], true);
        app.key("Escape");
        // The persistent workspace choices use the same reveal behavior.
        let arranged = app.click("Arranger");
        assert!(
            arranged["timeline"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["track"] == target)
        );
        app.click("Mixer");
        let revealed = revealed_identity(&mut app, target);
        unchanged(&revealed, &initial);
        // Returning from a utility restores a deliberately panned mixer;
        // only explicit workspace navigation follows the selected channel.
        let pane = &revealed["mixer"]["reorder_viewport"];
        app.command(
            json!({"command":"scroll", "x":pane["min"][0].as_f64().unwrap()+20.0,
            "y":pane["min"][1].as_f64().unwrap()+20.0, "delta":0.0, "delta_x":10000.0}),
        );
        for _ in 0..20 {
            app.inspect();
        }
        let panned = app.inspect();
        assert!(!inside_x(
            &channel(&panned, target)["name"],
            &panned["mixer"]["reorder_viewport"]
        ));
        app.click("Window");
        app.click("Setup");
        app.key("Escape");
        let restored = app.inspect();
        assert_eq!(
            channel(&restored, target)["name"],
            channel(&panned, target)["name"]
        );
        unchanged(&restored, &initial);
        app.quit();
    }
}

#[test]
fn switching_between_arranger_and_mixer_retains_the_selected_clip() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    let clip = Clip::midi(
        "Selected phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 90)],
    );
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("clip-selection.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let baseline = app.click("Selected phrase");
    assert_eq!(baseline["selected_clip"], id);
    assert_eq!(app.click("Mixer")["selected_clip"], id);
    let arranged = app.click("Show in Arranger");
    assert_eq!(arranged["selected_track"], track);
    assert_eq!(arranged["selected_clip"], id);
    unchanged(&arranged, &baseline);
    app.quit();
}
