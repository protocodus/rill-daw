//! Long channel/device names keep editing and routing actions within the window.
mod common;
use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Project};
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
    assert!(rect["max"][0].as_f64().unwrap() <= 1100.0);
    let top = rect["min"][1].as_f64().unwrap();
    let bottom = rect["max"][1].as_f64().unwrap();
    assert!(
        top >= 120.0 && bottom <= 696.0 && bottom - top >= 13.0,
        "{label}: {rect}"
    );
}
fn window(app: &mut Harness, name: &str) -> Value {
    app.click("Window");
    app.click(name)
}

fn scroll_routing(app: &mut Harness, down: bool) -> Value {
    let state = app.inspect();
    let window = &state["window_rects"][0];
    let x = (window["min"][0].as_f64().unwrap() + window["max"][0].as_f64().unwrap()) * 0.5;
    let y = window["max"][1].as_f64().unwrap() - 50.0;
    app.command(json!({"command":"scroll", "x":x, "y":y,
        "delta":if down { -10000.0 } else { 10000.0 }}));
    for _ in 0..20 {
        app.inspect();
    }
    app.inspect()
}

fn click_revealed(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let target = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == name)
        .unwrap();
    let x = (target["min"][0].as_f64().unwrap() + target["max"][0].as_f64().unwrap()) * 0.5;
    let y = (target["min"][1].as_f64().unwrap() + target["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    app.click(name)
}

#[test]
fn long_effect_and_bus_names_keep_reorder_remove_and_send_controls_reachable() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let track = project.add_midi_track("Synthesizer 🎹 with a long descriptive channel name and additional arrangement notes for this song");
        let effect_name = "Warm stereo tape delay with a long manufacturer and preset name — echoes of the evening skyline";
        let effect = Effect::Vst3 {
            path: app
                .directory
                .path()
                .join("Unavailable.vst3")
                .to_string_lossy()
                .into_owned(),
            class_id: None,
        };
        project.track_mut(track).unwrap().inserts = vec![
            EffectSlot::new(effect_name, effect.clone()),
            EffectSlot::new("Second insert", effect),
        ];
        let second = project.track(track).unwrap().inserts[1].id;
        project
            .track_mut(track)
            .unwrap()
            .clips
            .push(Clip::midi("Layout phrase", 0.0, 4.0, vec![]));
        let bus = project.add_bus("A shared reverb return with a very long descriptive name for the atmospheric synth instruments in this arrangement");
        project.add_send(track, bus, 2.5, false).unwrap();
        let path = app.directory.path().join("channels.daw.json");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":if dark {2.0} else {1.0}}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("Layout phrase");
        let state = window(&mut app, "Effects");
        let effects_title = format!("Effects of {}", project.track(track).unwrap().name);
        for label in [effects_title.as_str(), "More", "Open editor", "+ Insert"] {
            visible(&state, label);
        }
        let state = app.click(&format!("01  {effect_name}"));
        assert_eq!(
            state["project"]["tracks"][0]["inserts"][0]["enabled"],
            false
        );
        app.command(json!({"command":"click","label":"More","occurrence":1}));
        let state = app.click("Up");
        assert_eq!(state["project"]["tracks"][0]["inserts"][0]["id"], second);
        app.click("More");
        let state = app.click("Remove insert");
        assert_eq!(
            state["project"]["tracks"][0]["inserts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let state = window(&mut app, "Routing");
        assert_eq!(
            state["project"]["tracks"][0]["sends"][0]["gain"], 2.5,
            "opening Routing must preserve a valid send above the slider's edit range"
        );
        let routing_title = format!("Routing of {}", project.track(track).unwrap().name);
        for label in [
            routing_title.as_str(),
            "Level",
            "Pre",
            "Post",
            "×",
            "+ Send",
            "Create send track",
        ] {
            visible(&state, label);
        }
        // Generous send controls can put the output below the first viewport.
        // Reveal it with the wheel and exercise the actual output picker.
        let output = scroll_routing(&mut app, true);
        visible(&output, "Master output");
        assert_eq!(output["project"], state["project"]);
        let choices = click_revealed(&mut app, "Master output");
        assert!(
            choices["menu_label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label["text"] == project.track(bus).unwrap().name)
        );
        let unchanged = app.click("Master output");
        assert_eq!(unchanged["project"], state["project"]);
        let top = scroll_routing(&mut app, false);
        visible(&top, "Pre");
        let state = click_revealed(&mut app, "Pre");
        assert_eq!(state["project"]["tracks"][0]["sends"][0]["pre_fader"], true);
        assert_eq!(state["project"]["tracks"][0]["sends"][0]["gain"], 2.5);
        let state = app.click("×");
        assert!(
            state["project"]["tracks"][0]["sends"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(state["error"], false);
        app.quit();
    }
}

#[test]
fn long_insert_chain_scrolls_to_actions_and_shrinks_after_removal() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_audio_track("Compact effects");
    let effect = Effect::Vst3 {
        path: app
            .directory
            .path()
            .join("Unavailable.vst3")
            .to_string_lossy()
            .into_owned(),
        class_id: None,
    };
    project.track_mut(track).unwrap().inserts = (1..=20)
        .map(|index| EffectSlot::new(format!("Insert {index:02}"), effect.clone()))
        .collect();
    let path = app.directory.path().join("long-chain.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    window(&mut app, "Effects");
    app.command(json!({"command":"scroll","x":550.0,"y":500.0,"delta":-10000.0}));
    let mut state = app.inspect();
    for _ in 0..20 {
        state = app.inspect();
    }
    visible(&state, "+ Insert");
    visible(&state, "20  Insert 20");
    let expanded_height = {
        let labels = state["label_rects"].as_array().unwrap();
        let title = labels
            .iter()
            .find(|row| row["text"] == "Effects of Compact effects")
            .unwrap();
        let action = labels.iter().find(|row| row["text"] == "+ Insert").unwrap();
        action["max"][1].as_f64().unwrap() - title["min"][1].as_f64().unwrap()
    };
    let library = app.click("+ Insert");
    assert_eq!(library["popups"]["library"], true);
    let returned = app.key("Escape");
    assert_eq!(returned["popups"]["effects"], true);
    for remaining in (0..20).rev() {
        app.click("More");
        state = app.click("Remove insert");
        assert_eq!(
            state["project"]["tracks"][0]["inserts"]
                .as_array()
                .unwrap()
                .len(),
            remaining
        );
        assert_eq!(state["error"], false);
    }
    visible(&state, "+ Insert");
    let labels = state["label_rects"].as_array().unwrap();
    let title = labels
        .iter()
        .find(|row| row["text"] == "Effects of Compact effects")
        .unwrap();
    let action = labels.iter().find(|row| row["text"] == "+ Insert").unwrap();
    let empty_height = action["max"][1].as_f64().unwrap() - title["min"][1].as_f64().unwrap();
    // The empty card and its insertion action hug their measured content,
    // rather than retaining the long chain's scrolling height.
    assert!(
        empty_height + f64::from(daw_ui::tokens::SPACE_LG) < expanded_height,
        "an empty chain should hug its context links and remaining controls: {empty_height}"
    );
    let hint = labels
        .iter()
        .find(|row| row["text"] == "Add an insert to shape this channel.")
        .unwrap();
    assert!(
        action["min"][1].as_f64().unwrap() - hint["max"][1].as_f64().unwrap()
            <= f64::from(daw_ui::tokens::PARAMETER_ROW_HEIGHT),
        "Empty-state padding and the action gap must not retain an empty canvas"
    );
    app.click("Edit");
    let restored = app.click("Undo");
    assert_eq!(
        restored["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    visible(&restored, "More");
    visible(&restored, "+ Insert");
    app.quit();
}
