//! Source-specific mixer output/send menus modify the running song atomically.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn settle(app: &mut Harness) -> Value {
    for _ in 0..12 {
        app.inspect();
    }
    app.inspect()
}

fn center(rect: &Value) -> [f64; 2] {
    [
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    ]
}

fn pointer(app: &mut Harness, position: [f64; 2]) -> Value {
    app.command(json!({"command":"pointer", "phase":"click", "x":position[0], "y":position[1]}))
}

fn open_menu(app: &mut Harness, source: u64, control: &str) -> Value {
    for _ in 0..15 {
        let state = settle(app);
        let channel = state["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == source)
            .unwrap();
        let rect = &channel[control];
        let viewport = &state["mixer"]["reorder_viewport"];
        if (0..2).all(|axis| {
            rect["min"][axis].as_f64().unwrap() >= viewport["min"][axis].as_f64().unwrap()
                && rect["max"][axis].as_f64().unwrap() <= viewport["max"][axis].as_f64().unwrap()
        }) {
            pointer(app, center(rect));
            return settle(app);
        }
        let [x, y] = center(viewport);
        let horizontal = rect["max"][0].as_f64().unwrap() > viewport["max"][0].as_f64().unwrap();
        let delta = if rect["max"][1].as_f64().unwrap() > viewport["max"][1].as_f64().unwrap()
            || horizontal
        {
            -100.0
        } else {
            100.0
        };
        app.command(json!({"command":"scroll", "x":x,"y":y,"delta":if horizontal { 0.0 } else { delta },"delta_x":if horizontal { delta } else { 0.0 }}));
    }
    panic!("Cannot reach {source} {control}");
}

fn menu_has(state: &Value, text: &str) -> bool {
    state["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label["text"] == text)
}

fn choose(app: &mut Harness, label: &str) -> Value {
    let state = settle(app);
    let rect = state["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["menu_label_rects"]));
    pointer(app, center(rect))
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}
fn redo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Redo")
}

fn open(app: &mut Harness, project: &Project, dark: bool) {
    let path = app.directory.path().join("inline-routing.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    app.command(json!({"command":"viewport", "width":800, "height":600}));
    app.click("Mixer");
}

#[test]
fn inline_output_choices_preserve_source_context_and_insert_one_undoable_bus() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let selected = project.add_audio_track("Selected source");
        let source = project.add_midi_track("Keys");
        let downstream = project.add_bus("Mix");
        let room = project.add_bus("Room");
        project.set_output(source, Some(downstream)).unwrap();
        project.add_send(source, room, 0.4, true).unwrap();
        open(&mut app, &project, dark);
        let before = settle(&mut app);
        let opened = open_menu(&mut app, source, "target");
        for label in ["Master", "Mix", "Room", "New bus and route", "Routing…"] {
            assert!(menu_has(&opened, label), "Missing {label}");
        }
        for key in ["project", "history", "selected_track"] {
            assert_eq!(opened[key], before[key]);
        }
        app.key("Escape");
        assert_eq!(app.inspect()["project"], before["project"]);

        open_menu(&mut app, source, "target");
        let routed = choose(&mut app, "Room");
        assert_eq!(track(&routed, source)["output_bus"], room);
        assert_eq!(routed["selected_track"], selected);
        assert_eq!(
            track(&routed, source)["sends"],
            track(&before, source)["sends"]
        );
        assert_eq!(undo(&mut app)["project"], before["project"]);
        assert_eq!(redo(&mut app)["project"], routed["project"]);

        open_menu(&mut app, source, "target");
        let master = choose(&mut app, "Master");
        assert!(track(&master, source)["output_bus"].is_null());
        assert_eq!(undo(&mut app)["project"], routed["project"]);
        open_menu(&mut app, source, "target");
        let grouped = choose(&mut app, "New bus and route");
        let bus = track(&grouped, source)["output_bus"].as_u64().unwrap();
        assert_eq!(track(&grouped, bus)["name"], "Bus 1");
        assert_eq!(
            track(&grouped, bus)["output_bus"],
            room,
            "Existing downstream processing must remain in the path"
        );
        assert_eq!(grouped["selected_track"], selected);
        assert_eq!(
            track(&grouped, source)["sends"],
            track(&before, source)["sends"]
        );
        assert_eq!(undo(&mut app)["project"], routed["project"]);
        assert_eq!(redo(&mut app)["project"], grouped["project"]);
        app.quit();
    }
}

#[test]
fn populated_send_menu_adds_another_send_and_retains_level_editing() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let selected = project.add_audio_track("Selected source");
    let source = project.add_audio_track("Voice");
    let room = project.add_bus("Room");
    let hall = project.add_bus("Hall");
    project.add_send(source, room, 0.4, true).unwrap();
    open(&mut app, &project, false);
    let before = settle(&mut app);
    let menu = open_menu(&mut app, source, "add_send");
    assert!(!menu_has(&menu, "Room"));
    for label in ["Hall", "New bus and send", "Edit sends…"] {
        assert!(menu_has(&menu, label));
    }
    assert_eq!(menu["project"], before["project"]);
    assert_eq!(menu["selected_track"], selected);
    let sent = choose(&mut app, "Hall");
    assert_eq!(track(&sent, source)["sends"].as_array().unwrap().len(), 2);
    assert_eq!(track(&sent, source)["sends"][1]["target_bus"], hall);
    assert_eq!(sent["selected_track"], selected);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    assert_eq!(redo(&mut app)["project"], sent["project"]);
    open_menu(&mut app, source, "add_send");
    let editing = choose(&mut app, "Edit sends…");
    assert_eq!(editing["popups"]["routing"], true);
    assert_eq!(editing["selected_track"], source);
    assert_eq!(editing["project"], sent["project"]);
    app.quit();
}

#[test]
fn output_menu_excludes_self_and_upstream_main_and_send_paths() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_bus("Source");
    let upstream = project.add_bus("Output upstream");
    let sent = project.add_bus("Send upstream");
    project.set_output(upstream, Some(source)).unwrap();
    project.add_send(sent, source, 0.0, true).unwrap();
    open(&mut app, &project, true);
    let before = settle(&mut app);
    let menu = open_menu(&mut app, source, "target");
    for label in ["Source", "Output upstream", "Send upstream"] {
        assert!(!menu_has(&menu, label), "Unsafe output {label}");
    }
    assert!(menu_has(&menu, "Master") && menu_has(&menu, "New bus and route"));
    app.key("Escape");
    assert_eq!(app.inspect()["project"], before["project"]);
    app.quit();
}

#[test]
fn long_destination_lists_keep_creation_and_details_reachable_at_800_pixels() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Voice");
    for index in 0..40 {
        project.add_bus(format!(
            "Return {index:02} — An intentionally long room and ambience destination name"
        ));
    }
    open(&mut app, &project, false);
    let before = settle(&mut app);
    let opened = open_menu(&mut app, source, "target");
    let footer = |state: &Value| {
        state["menu_label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "New bus and route")
            .unwrap()
            .clone()
    };
    let created_rect = footer(&opened);
    for label in opened["menu_label_rects"].as_array().unwrap() {
        assert!(label["min"][0].as_f64().unwrap() >= 0.0, "{label}");
        assert!(label["max"][0].as_f64().unwrap() <= 800.0, "{label}");
        assert!(label["max"][1].as_f64().unwrap() <= 600.0, "{label}");
    }
    assert!(menu_has(&opened, "Routing…"));
    let list_top = opened["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == "Master")
        .unwrap();
    let [x, y] = center(list_top);
    app.command(json!({"command":"scroll","x":x,"y":y + 45.0,"delta":-3000.0}));
    let scrolled = settle(&mut app);
    assert_eq!(
        footer(&scrolled),
        created_rect,
        "New bus action must stay below the independently scrolling choices"
    );
    assert!(menu_has(&scrolled, "Routing…"));
    let grouped = choose(&mut app, "New bus and route");
    assert_eq!(grouped["project"]["tracks"].as_array().unwrap().len(), 42);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}
