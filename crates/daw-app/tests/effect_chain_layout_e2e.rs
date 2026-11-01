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
        let count = label(&mixer, "12");
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
        label(&chain, "Input → Inserts → Volume & pan");
        label(&chain, "12 / 12 active");
        let footer = label(&chain, "+ Insert").clone();
        assert!(footer["max"][1].as_f64().unwrap() <= 600.0);
        let bypassed = app.click("01  First insert");
        assert_eq!(
            bypassed["project"]["tracks"][0]["inserts"][0]["enabled"],
            false
        );
        label(&bypassed, "Bypassed");
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
fn mixer_scale_marks_match_native_fader_gain_and_empty_chains_offer_insertion() {
    let mut app = Harness::new();
    fixture(&mut app, 0);
    let mixer = app.click("Mixer");
    label(&mixer, "No inserts");
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
