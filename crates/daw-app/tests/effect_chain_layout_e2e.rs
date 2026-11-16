//! Focused insert chains keep names, actions and the add control usable.
mod common;

use common::Harness;
use daw_core::{Effect, EffectSlot, Project};
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn fixture(app: &mut Harness, count: usize) -> Project {
    let mut project = Project::new();
    let id = project.add_audio_track("Drums");
    for index in 0..count {
        project.track_mut(id).unwrap().inserts.push(EffectSlot::new(
            if index == 0 { "First insert".to_owned() }
            else { format!("Processor {index} with a deliberately long descriptive name to check narrow windows") },
            Effect::Vst3 { path: "/missing/layout-fixture.vst3".into(), class_id: None },
        ));
    }
    let path = app.directory.path().join("effects-layout.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    project
}

#[test]
fn long_chains_keep_insert_footer_and_mouse_actions_visible_in_both_themes() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let project = fixture(&mut app, 12);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":density}),
        );
        let mixer = app.click("Mixer");
        let count = label(&mixer, "12/12");
        let action = label(&mixer, "Effects");
        let summary = label(&mixer, "First insert");
        assert_eq!(mixer["mixer_devices"].as_array().unwrap().len(), 12);
        assert!(summary["min"][1].as_f64().unwrap() >= action["max"][1].as_f64().unwrap());
        assert!(
            summary["max"][0].as_f64().unwrap()
                <= mixer["mixer"]["channels"][0]["insert_summary"]["max"][0]
                    .as_f64()
                    .unwrap()
        );
        assert!(
            count["min"][0].as_f64().unwrap() > action["max"][0].as_f64().unwrap(),
            "insert count must not overlap the Effects label"
        );
        let chain = app.click("Effects");
        label(&chain, "Inserts");
        assert!(
            !chain["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "Input → Inserts → Volume & pan"),
            "the processing order lives in the count's tooltip"
        );
        label(&chain, "12 / 12 active");
        let footer = label(&chain, "+ Insert").clone();
        assert!(footer["max"][1].as_f64().unwrap() <= 600.0);
        let bypassed = app.click("01  First insert");
        assert_eq!(
            bypassed["project"]["tracks"][0]["inserts"][0]["enabled"],
            false
        );
        // Bypass is told by the switch, the muted name and the gray rail.
        assert!(
            !bypassed["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "Bypassed" || text == "Active")
        );
        label(&bypassed, "11 / 12 active");
        app.click("More");
        let moved = app.click("Down");
        assert_eq!(
            moved["project"]["tracks"][0]["inserts"][1]["id"],
            project.tracks[0].inserts[0].id
        );
        app.click("Edit");
        let undone = app.click("Undo");
        assert_eq!(undone["project"], bypassed["project"]);
        let (_, y) = center(label(&undone, "01  First insert"));
        let x = center(&footer).0;
        app.command(json!({"command":"scroll", "x":x, "y":y + 30.0, "delta":-5000.0}));
        for _ in 0..12 {
            app.inspect();
        }
        let scrolled = app.inspect();
        let bottom_footer = label(&scrolled, "+ Insert");
        assert!(
            (center(bottom_footer).1 - center(&footer).1).abs() < 1.0,
            "the add control must remain below the independently scrolling chain"
        );
        assert_eq!(scrolled["project"], bypassed["project"]);
        // The fixed footer can open the library even when the last insert is in view.
        assert_eq!(app.click("+ Insert")["popups"]["library"], true);
        let returned = app.key("Escape");
        assert_eq!(returned["popups"]["effects"], true);
        assert_eq!(returned["project"], bypassed["project"]);
        app.quit();
    }
}

#[test]
fn chooser_inserts_before_after_or_at_the_end_of_the_clicked_chain() {
    let mut app = Harness::new();
    fixture(&mut app, 2);
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    app.click("Window");
    let mut chain = app.click("Effects");
    let slots = |state: &Value| {
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .clone()
    };
    // The second card is explicitly the anchor, independent of which channel
    // or device the browser last displayed.
    for (action, occurrence, position) in [
        ("Insert before…", 1, 1),
        ("Insert after…", 1, 2),
        ("+ Insert", 0, 4),
    ] {
        let before = slots(&chain);
        let chooser =
            app.command(json!({"command":"click","label":action,"occurrence":occurrence}));
        assert_eq!(chooser["popups"]["library"], true);
        assert_eq!(chooser["library"]["primary_enabled"], true);
        if action != "+ Insert" {
            let anchor = before[occurrence]["name"].as_str().unwrap();
            assert!(
                chooser["library"]["destination"]
                    .as_str()
                    .unwrap()
                    .contains(anchor)
            );
            let cancelled = app.key("Escape");
            assert_eq!(slots(&cancelled), before);
            app.command(json!({"command":"click","label":action,"occurrence":occurrence}));
        }
        chain = app.click("Add to effect chain");
        assert_eq!(chain["popups"]["effects"], true);
        let mut after = slots(&chain);
        assert_eq!(after.len(), before.len() + 1);
        let inserted = after.remove(position);
        assert!(!before.iter().any(|slot| slot["id"] == inserted["id"]));
        assert_eq!(after, before);
        app.click("Edit");
        let undone = app.click("Undo");
        assert_eq!(slots(&undone), before);
        app.click("Edit");
        chain = app.click("Redo");
        assert_eq!(slots(&chain)[position], inserted);
    }
    app.quit();
}

#[test]
fn right_clicking_an_insert_card_opens_its_rack_commands() {
    let mut app = Harness::new();
    fixture(&mut app, 2);
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    app.click("Window");
    let initial = app.click("Effects");
    let original = &initial["project"]["tracks"][0]["inserts"];
    let context = |app: &mut Harness, state: &Value| {
        let (x, y) = center(label(state, "01  First insert"));
        app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}))
    };
    let mut chain = initial.clone();
    for (command, index) in [("Insert before…", 0), ("Insert after…", 1)] {
        let menu = context(&mut app, &chain);
        assert_eq!(&menu["project"]["tracks"][0]["inserts"], original);
        for command in [
            "Open editor",
            "Parameters…",
            "Insert before…",
            "Insert after…",
            "Bypass insert",
            "Up",
            "Down",
            "Remove insert",
        ] {
            assert!(
                menu["menu_label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["text"] == command),
                "Right-click card menu should offer {command}"
            );
        }
        let chooser = app.click(command);
        assert!(
            chooser["library"]["destination"]
                .as_str()
                .unwrap()
                .contains("First insert")
        );
        let added = app.click("Add to effect chain");
        let mut inserts = added["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .clone();
        inserts.remove(index);
        assert_eq!(inserts.as_slice(), original.as_array().unwrap().as_slice());
        app.click("Edit");
        chain = app.click("Undo");
    }
    context(&mut app, &chain);
    let bypassed = app.click("Bypass insert");
    assert_eq!(
        bypassed["project"]["tracks"][0]["inserts"][0]["enabled"],
        false
    );
    context(&mut app, &bypassed);
    let enabled = app.click("Enable insert");
    assert_eq!(&enabled["project"]["tracks"][0]["inserts"], original);
    context(&mut app, &enabled);
    let moved = app.click("Down");
    assert_eq!(moved["project"]["tracks"][0]["inserts"][1], original[0]);
    app.click("Edit");
    let restored = app.click("Undo");
    context(&mut app, &restored);
    let removed = app.click("Remove insert");
    assert_eq!(
        removed["project"]["tracks"][0]["inserts"],
        json!([original[1]])
    );
    app.click("Edit");
    assert_eq!(
        &app.click("Undo")["project"]["tracks"][0]["inserts"],
        original
    );
    app.quit();
}

#[test]
fn mixer_scale_marks_match_native_fader_gain_and_empty_chains_offer_insertion() {
    let mut app = Harness::new();
    fixture(&mut app, 0);
    let mixer = app.click("Mixer");
    // An empty chain shows one dashed "+ Insert" slot under the Effects header.
    let slot = label(&mixer, "+ Insert").clone();
    let summary = &mixer["mixer"]["channels"][0]["insert_summary"];
    assert!(slot["min"][1].as_f64().unwrap() >= summary["min"][1].as_f64().unwrap());
    assert!(slot["max"][1].as_f64().unwrap() <= summary["max"][1].as_f64().unwrap());
    let (x, y) = center(&slot);
    let library = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(library["popups"]["library"], true);
    assert_eq!(library["project"], mixer["project"]);
    app.key("Escape");
    let mixer = app.click("Mixer");
    let fader = &mixer["mixer"]["channels"][0]["fader"];
    let x = center(fader).0;
    for (mark, gain) in [("−6", 10.0_f64.powf(-6.0 / 20.0)), ("0", 1.0)] {
        let current = app.inspect();
        // Find the scale mark at the left of this channel's fader, not a count badge.
        let tick = current["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| {
                rect["text"] == mark
                    && rect["max"][0].as_f64().unwrap() < x
                    && center(rect).1 >= fader["min"][1].as_f64().unwrap()
            })
            .unwrap();
        let edited =
            app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":center(tick).1}));
        assert!(
            (edited["project"]["tracks"][0]["gain"].as_f64().unwrap() - gain).abs() < 0.015,
            "{mark} dB tick must match the native fader's gain"
        );
    }
    let effects = app.click("Effects");
    label(&effects, "No effects");
    label(&effects, "0 / 0 active");
    assert_eq!(app.click("+ Insert")["popups"]["library"], true);
    app.quit();
}
