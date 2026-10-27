//! Instrument setup guidance and recovery through the real headless app.
//! Plug-in fixtures contain discovery metadata only and never produce audio.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn window(app: &mut Harness, label: &str) -> Value {
    app.click("Window");
    app.click(label)
}

fn latest(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count();
    assert!(count > 0, "Missing {label}: {}", state["visible_labels"]);
    app.command(json!({"command":"click", "label":label, "occurrence":count-1}))
}

fn wait(app: &mut Harness, condition: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if condition(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Instrument setup did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn preserved(state: &Value, baseline: &Value) {
    for key in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "view",
    ] {
        assert_eq!(state[key], baseline[key], "Browsing changed {key}");
    }
}

fn metadata(root: &std::path::Path, bundle: &str, vendor: &str, name: &str, kind: &str) {
    let resources = root.join(format!("{bundle}.vst3/Contents/Resources"));
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("moduleinfo.json"),
        json!({
            "Name":bundle, "Version":"1", "Factory Info":{"Vendor":vendor},
            "Classes":[{"CID":"44444444444444444444444444444444",
                "Category":"Audio Module Class", "Name":name,
                "Sub Categories":[kind], "Version":"1", "SDKVersion":"VST3.7.9",
                "Cardinality":2147483647}]
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn empty_and_effect_only_libraries_offer_plugin_setup_and_restore_the_assignment_caller() {
    for effect_only in [false, true] {
        let inventory = tempfile::tempdir().unwrap();
        if effect_only {
            metadata(
                inventory.path(),
                "NoviceEffects",
                "Novice tests",
                "Novice Echo",
                "Fx",
            );
        }
        let mut app = Harness::with_vst_path(inventory.path());
        app.click("Track");
        app.click("Add MIDI track");
        let baseline = wait(&mut app, |state| {
            visible(state, "No VST3 instruments found") && visible(state, "Scan")
        });
        assert_eq!(baseline["popups"]["library"], true);
        assert_eq!(baseline["library"]["total"], 0);
        assert!(baseline["library"]["selected"].is_null());
        assert!(visible(
            &baseline,
            "MIDI notes stay silent until a VST3 instrument is assigned."
        ));
        assert!(visible(
            &baseline,
            "Install a VST3 instrument, then Scan. Use Plug-in folders if it is installed elsewhere."
        ));
        assert!(!visible(&baseline, "No matching instruments"));
        assert!(!visible(&baseline, "Clear filters"));
        if effect_only {
            assert!(
                baseline["plugin_catalog"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["name"] == "Novice Echo")
            );
        } else {
            assert!(baseline["plugin_catalog"].as_array().unwrap().is_empty());
        }

        let setup = app.click("Plug-in folders");
        assert_eq!(setup["popups"]["setup"], true);
        assert_eq!(setup["popups"]["library"], false);
        assert!(visible(&setup, "VST3 SCAN DIRECTORIES"));
        assert!(visible(&setup, "Folder path"));
        preserved(&setup, &baseline);
        let original_settings = setup["settings"].clone();
        app.click("Folder path");
        app.command(json!({"command":"text", "text":inventory.path()}));
        let draft = app.click("Add folder");
        assert_eq!(draft["setup_draft"]["vst_dirs"], json!([inventory.path()]));
        let cancelled = app.click("Cancel");
        assert_eq!(cancelled["popups"]["setup"], false);
        assert_eq!(cancelled["popups"]["library"], true);
        assert_eq!(cancelled["settings"], original_settings);
        assert!(visible(&cancelled, "Assign instrument"));
        preserved(&cancelled, &baseline);

        app.click("Scan");
        let scanned = wait(&mut app, |state| {
            visible(state, "No VST3 instruments found") && visible(state, "Scan")
        });
        preserved(&scanned, &baseline);
        let caller = app.key("Escape");
        assert_eq!(caller["popups"]["library"], false);
        assert_eq!(caller["view"], "Arrangement");
        preserved(&caller, &baseline);
        app.quit();
    }
}

#[test]
fn unmatched_searches_clear_both_search_and_developer_without_assigning_a_stale_device() {
    let inventory = common::plugin_fixture();
    metadata(
        inventory.path(),
        "NoviceKeys",
        "Other novice developer",
        "Novice Keys",
        "Instrument",
    );
    let mut app = Harness::with_vst_path(inventory.path());
    let baseline = window(&mut app, "Library");
    common::wait_library_device(&mut app, "Novice Keys");
    app.click("All developers");
    latest(&mut app, "Other novice developer");
    assert_eq!(app.inspect()["library"]["total"], 1);
    app.click("Search instruments");
    app.command(json!({"command":"text", "text":"no such instrument 987654"}));
    let missing = wait(&mut app, |state| visible(state, "No matching instruments"));
    assert!(visible(
        &missing,
        "Try another search or clear the filters."
    ));
    assert!(!visible(&missing, "No VST3 instruments found"));
    assert_eq!(missing["library"]["total"], 0);
    assert!(missing["library"]["selected"].is_null());
    preserved(&app.key("Enter"), &baseline);
    let cleared = app.click("Clear filters");
    assert!(visible(&cleared, "Search instruments"));
    assert!(visible(&cleared, "All developers"));
    common::wait_library_device(&mut app, "Rill Fixture A");
    common::wait_library_device(&mut app, "Novice Keys");
    assert_eq!(app.inspect()["library"]["total"], 3);
    preserved(&app.inspect(), &baseline);

    app.click("Effects");
    common::wait_library_device(&mut app, "Rill Fixture FX");
    app.click("Search effects");
    app.command(json!({"command":"text", "text":"no such effect 987654"}));
    let missing = wait(&mut app, |state| visible(state, "No matching effects"));
    assert!(missing["library"]["selected"].is_null());
    app.click("Clear filters");
    common::wait_library_device(&mut app, "Rill Fixture FX");
    preserved(&app.inspect(), &baseline);
    app.quit();
}

#[test]
fn silent_instrument_overview_opens_assignment_and_restores_the_clip_editor() {
    let inventory = tempfile::tempdir().unwrap();
    let mut app = Harness::with_vst_path(inventory.path());
    let mut project = Project::new();
    let track = project.add_midi_track("Unassigned melody");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Quiet phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 2.0, 100), Note::new(64, 2.0, 2.0, 90)],
    ));
    let path = app.directory.path().join("silent-instrument.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Quiet phrase");
    let baseline = window(&mut app, "Clip Editor");
    assert_eq!(baseline["view"], "Clip");
    // Utilities retain the Clip Editor in their navigation snapshot/backdrop;
    // their active workspace field is Arrangement until the caller is restored.
    let mut utility_baseline = baseline.clone();
    utility_baseline["view"] = json!("Arrangement");

    let overview = window(&mut app, "Instrument");
    assert!(visible(&overview, "Instrument of Unassigned melody"));
    assert!(visible(&overview, "No instrument assigned"));
    assert!(visible(
        &overview,
        "This MIDI track is silent until you choose a VST3 instrument."
    ));
    assert_eq!(
        overview["popup_context"]["source"],
        "Clip Editor · Quiet phrase"
    );
    preserved(&overview, &utility_baseline);
    let chooser = app.click("Choose instrument");
    assert_eq!(chooser["popups"]["library"], true);
    assert_eq!(chooser["popups"]["instrument"], false);
    assert!(visible(&chooser, "Assign instrument"));
    assert_eq!(
        chooser["popup_context"]["source"],
        "Instrument · Unassigned melody"
    );
    preserved(&chooser, &utility_baseline);
    let returned = app.key("Escape");
    assert_eq!(returned["popups"]["instrument"], true);
    assert_eq!(returned["popups"]["library"], false);
    assert!(visible(&returned, "No instrument assigned"));
    assert_eq!(
        returned["popup_context"]["source"],
        "Clip Editor · Quiet phrase"
    );
    preserved(&returned, &utility_baseline);
    let caller = app.key("Escape");
    assert_eq!(caller["popups"]["instrument"], false);
    assert!(!caller["midi_editor"].is_null());
    assert_eq!(caller["view"], "Clip");
    preserved(&caller, &baseline);
    app.quit();
}
