//! Shared controls remain usable in focused views at both desktop densities.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn window(app: &mut Harness, name: &str) -> Value {
    app.click("Window");
    app.click(name)
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn accessible(state: &Value, labels: &[&str]) {
    for label in labels {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|rect| rect["text"] == *label)
            .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
        let left = rect["min"][0].as_f64().unwrap();
        let top = rect["min"][1].as_f64().unwrap();
        let right = rect["max"][0].as_f64().unwrap();
        let bottom = rect["max"][1].as_f64().unwrap();
        assert!(left >= 0.0 && top >= 0.0 && right <= 1100.0 && bottom <= 720.0);
        assert!(
            right > left && bottom - top >= 13.0,
            "Clipped {label}: {rect}"
        );
    }
}

fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    accessible(&state, &[label]);
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == label)
        .count()
        - 1;
    app.command(json!({"command":"click","label":label,"occurrence":occurrence}))
}

fn appearance(app: &mut Harness, density: f64, dark: bool) {
    app.command(json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}));
    let state = app.command(json!({"command":"system_theme","dark":dark}));
    assert_eq!(
        state["effective_theme"],
        if dark { "Dark" } else { "Light" }
    );
    assert!(state["text_sizes"]["body"].as_f64().unwrap() >= 15.0);
}

fn text_rect(state: &Value, label: &str) -> Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]))
        .clone()
}

fn label_count(state: &Value, label: &str) -> usize {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == label)
        .count()
}

#[test]
fn setup_tabs_selects_and_folder_input_stay_accessible_and_cancel_discards_drafts() {
    let mut app = Harness::new();
    let original = app.inspect();
    let folder = app.directory.path().join("Plug-ins 🎹");
    std::fs::create_dir(&folder).unwrap();
    for density in [1.0, 2.0] {
        for dark in [false, true] {
            appearance(&mut app, density, dark);
            let state = window(&mut app, "Setup");
            assert_eq!(state["popups"]["setup"], true);
            accessible(
                &state,
                &[
                    "Audio",
                    "Appearance",
                    "Storage",
                    "Plug-ins",
                    "Apply",
                    "Cancel",
                ],
            );

            let state = app.click("Audio");
            accessible(
                &state,
                &[
                    "System default",
                    "48000 Hz",
                    "High quality",
                    "Refresh devices",
                ],
            );
            app.click("48000 Hz");
            accessible(&app.inspect(), &["44100 Hz", "96000 Hz"]);
            assert_eq!(app.click("96000 Hz")["setup_draft"]["sample_rate"], 96_000);
            let buffer = format!("{} samples", original["settings"]["buffer_frames"]);
            app.click(&buffer);
            let next_buffer = if buffer == "512 samples" {
                "1024 samples"
            } else {
                "512 samples"
            };
            let state = app.click(next_buffer);
            assert_ne!(
                state["setup_draft"]["buffer_frames"],
                original["settings"]["buffer_frames"]
            );
            app.click("High quality");
            assert_eq!(app.click("Fast")["setup_draft"]["resample_quality"], "Fast");

            app.click("Appearance");
            app.click("System");
            let theme = if dark { "Light" } else { "Dark" };
            let draft = app.click(theme);
            assert_eq!(draft["setup_draft"]["appearance"], theme);
            assert_eq!(draft["settings"], original["settings"]);

            let storage = app.click("Storage");
            accessible(
                &storage,
                &["SCRATCH DIRECTORY", "Choose…", "Recover session", "Cancel"],
            );
            let plugins = app.click("Plug-ins");
            accessible(
                &plugins,
                &["Folder path", "Add folder", "Browse…", "Apply", "Cancel"],
            );
            app.click("Folder path");
            app.command(json!({"command":"text","text":folder.to_string_lossy()}));
            let draft = app.click("Add folder");
            assert!(
                draft["setup_draft"]["vst_dirs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|path| path == &json!(folder))
            );
            accessible(&draft, &["Remove", "Apply", "Cancel"]);
            let cancelled = app.click("Cancel");
            assert_eq!(cancelled["popups"]["setup"], false);
            assert_eq!(cancelled["settings"], original["settings"]);
            assert_eq!(cancelled["project"], original["project"]);
            assert_eq!(cancelled["modified"], false);
            assert_eq!(cancelled["device_open"], false);
            assert_eq!(
                cancelled["effective_theme"],
                if dark { "Dark" } else { "Light" }
            );
        }
    }
    app.quit();
}

#[test]
fn properties_controls_and_library_effect_mixer_navigation_work_in_both_themes_and_densities() {
    let fixture = common::plugin_fixture();
    // Keep our developer visible above unrelated system plug-ins in the native
    // scrolling picker; no installed device catalog is required by this test.
    let metadata_path = fixture
        .path()
        .join("RillFixture.vst3/Contents/Resources/moduleinfo.json");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    metadata["Factory Info"]["Vendor"] = json!("000 Rill component tests");
    std::fs::write(metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    project.add_audio_track("Calm controls");
    let path = app.directory.path().join("controls.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open","path":path}))["project"].clone();
    app.click("Calm controls");

    for density in [1.0, 2.0] {
        for dark in [false, true] {
            appearance(&mut app, density, dark);
            let properties = window(&mut app, "Properties");
            accessible(
                &properties,
                &[
                    "Calm controls",
                    "Compact track",
                    "Top level",
                    "Mute",
                    "Solo",
                ],
            );
            assert_eq!(
                app.click("Compact track")["project"]["tracks"][0]["collapsed"],
                true
            );
            assert_eq!(
                app.click("Compact track")["project"]["tracks"][0]["collapsed"],
                false
            );
            assert_eq!(app.click("Mute")["project"]["tracks"][0]["mute"], true);
            assert_eq!(app.click("Mute")["project"]["tracks"][0]["mute"], false);
            front(&mut app, "Calm controls");
            app.command(json!({"command":"key","key":"A","cmd":true}));
            let edited = app.command(json!({"command":"text","text":"Calm 🎹 東京"}));
            assert_eq!(edited["project"]["tracks"][0]["name"], "Calm 🎹 東京");
            app.command(json!({"command":"key","key":"A","cmd":true}));
            app.command(json!({"command":"text","text":"Calm controls"}));
            let closed = app.key("Escape");
            assert_eq!(closed["project"], original);
            assert_eq!(closed["popups"]["properties"], false);

            let effects = window(&mut app, "Effects");
            accessible(&effects, &["+ Insert"]);
            let library = app.click("+ Insert");
            assert_eq!(library["popups"]["library"], true);
            assert_eq!(library["popups"]["effects"], false);
            app.click("Instruments");
            app.click("Effects");
            accessible(
                &app.inspect(),
                &["Search effects", "All developers", "Manage"],
            );
            app.click("Search effects");
            app.command(json!({"command":"text","text":"Rill Fixture FX"}));
            common::wait_visible(&mut app, "Add to effect chain");
            app.click("All developers");
            let filtered = front(&mut app, "000 Rill component tests");
            accessible(
                &filtered,
                &["Rill Fixture FX", "Add to effect chain", "More"],
            );
            assert!(!visible(&filtered, "Rill Fixture A"));
            assert_eq!(
                filtered["project"], original,
                "Browsing never inserts a plug-in"
            );
            let restored = app.key("Escape");
            assert_eq!(restored["popups"]["effects"], true);
            accessible(&restored, &["+ Insert"]);
            app.key("Escape");

            let mixer = window(&mut app, "Mixer");
            assert_eq!(mixer["view"], "Mixer");
            accessible(&mixer, &["Calm controls", "Ch 1 > Master", "Back"]);
            let back = app.click("Back");
            assert_eq!(back["view"], "Arrangement");
            assert_eq!(back["project"], original);
            assert_eq!(back["settings"]["appearance"], "System");
        }
    }
    app.quit();
}

#[test]
fn long_library_catalog_scrolls_without_moving_search_or_device_actions() {
    let fixture = common::plugin_fixture();
    let name = |index: usize| {
        format!("Catalog item {index:03} — Spacious instrument with a long descriptive name 🎹")
    };
    let classes: Vec<_> = (0..80)
        .map(|index| {
            json!({
                "CID":format!("{:032X}", 10_000 + index),
                "Category":"Audio Module Class", "Name":name(index),
                "Sub Categories":["Instrument"], "Version":"1.0",
                "SDKVersion":"VST3.7.9", "Cardinality":2147483647,
            })
        })
        .collect();
    std::fs::write(
        fixture
            .path()
            .join("RillFixture.vst3/Contents/Resources/moduleinfo.json"),
        json!({
            "Name":"Component catalog", "Version":"1.0",
            "Factory Info":{"Vendor":"000 Rill component catalog"},
            "Classes":classes,
        })
        .to_string(),
    )
    .unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    let original = app.inspect()["project"].clone();

    for density in [1.0, 2.0] {
        for dark in [false, true] {
            appearance(&mut app, density, dark);
            window(&mut app, "Library");
            let current_search = if visible(&app.inspect(), "Search instruments") {
                "Search instruments"
            } else {
                "Catalog item 013"
            };
            app.click(current_search);
            app.command(json!({"command":"key","key":"A","cmd":true}));
            // Reducing the result set first resets a previous catalog scroll.
            app.command(json!({"command":"text","text":"Catalog item 000"}));
            common::wait_visible(&mut app, &name(0));
            app.click("Catalog item 000");
            app.command(json!({"command":"key","key":"A","cmd":true}));
            app.command(json!({"command":"text","text":"Catalog item"}));
            let before = app.inspect();
            accessible(
                &before,
                &[
                    "Catalog item",
                    "All developers",
                    "Manage",
                    "Add instrument track",
                    "More",
                ],
            );
            assert_eq!(
                label_count(&before, &name(0)),
                2,
                "First device is shown in catalog and details"
            );

            let catalog = &before["library"]["catalog"];
            let x =
                (catalog["min"][0].as_f64().unwrap() + catalog["max"][0].as_f64().unwrap()) * 0.5;
            let y =
                (catalog["min"][1].as_f64().unwrap() + catalog["max"][1].as_f64().unwrap()) * 0.5;
            let scrolled = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-10000.0}));
            assert!(scrolled["library"]["scroll_offset"].as_f64().unwrap() > 0.0);
            assert_eq!(
                label_count(&scrolled, &name(0)),
                1,
                "Selected detail stays while its catalog row scrolls away"
            );
            assert!(
                visible(&scrolled, &name(79)),
                "Last catalog row must be reachable"
            );
            for label in [
                "Instruments",
                "Effects",
                "Catalog item",
                "All developers",
                "Manage",
                "Add instrument track",
            ] {
                assert_eq!(
                    text_rect(&scrolled, label),
                    text_rect(&before, label),
                    "Scrolling catalog moved {label}"
                );
            }
            accessible(&scrolled, &["Catalog item", "Add instrument track", "More"]);
            let selected = app.click(&name(79));
            assert_eq!(
                label_count(&selected, &name(79)),
                2,
                "Click must select the last device"
            );
            accessible(&selected, &["Add instrument track", "More"]);

            // Keyboard browsing must reveal a device beyond the visible rows.
            for _ in 0..12 {
                app.key("ArrowUp");
            }
            let mut keyboard = app.inspect();
            for _ in 0..20 {
                if label_count(&keyboard, &name(67)) == 2 {
                    break;
                }
                keyboard = app.inspect();
            }
            assert_eq!(
                label_count(&keyboard, &name(67)),
                2,
                "Arrow selection must scroll into view"
            );
            accessible(&keyboard, &["Catalog item", "Add instrument track", "More"]);

            app.click("Catalog item");
            app.command(json!({"command":"key","key":"A","cmd":true}));
            let filtered = app.command(json!({"command":"text","text":"Catalog item 013"}));
            assert_eq!(label_count(&filtered, &name(13)), 2);
            assert!(!visible(&filtered, &name(79)));
            accessible(
                &filtered,
                &["Catalog item 013", "Add instrument track", "More"],
            );
            assert_eq!(
                filtered["project"], original,
                "Browsing must not instantiate a device"
            );
            app.key("Escape");
        }
    }
    app.quit();
}
