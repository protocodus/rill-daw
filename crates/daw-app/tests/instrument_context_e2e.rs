//! Contextual plugin navigation and on-demand automation through the running app.
//! The metadata-only fixture never opens a native window or an audio device.
mod common;

use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Instrument, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn menu(app: &mut Harness, item: &str) -> Value {
    app.click("Window");
    app.click(item)
}

fn latest(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count();
    assert!(count > 0, "missing {label}: {}", state["visible_labels"]);
    app.command(json!({"command":"click", "label":label, "occurrence":count-1}))
}

#[test]
fn plugin_overview_hides_parameters_until_requested_and_retains_channel_context() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    let channel = project.add_midi_track("Evening lead 🎹");
    let plugin = fixture
        .path()
        .join("RillFixture.vst3")
        .to_string_lossy()
        .into_owned();
    project
        .set_instrument(
            channel,
            Instrument::Vst3 {
                path: plugin.clone(),
                class_id: Some("11111111111111111111111111111111".into()),
            },
        )
        .unwrap();
    project
        .track_mut(channel)
        .unwrap()
        .clips
        .push(Clip::midi("Lead phrase", 0.0, 8.0, vec![]));
    project
        .track_mut(channel)
        .unwrap()
        .inserts
        .push(EffectSlot::new(
            "Echo",
            Effect::Vst3 {
                path: plugin,
                class_id: Some("33333333333333333333333333333333".into()),
            },
        ));
    let path = app.directory.path().join("context.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Lead phrase");
    let baseline = app.inspect();

    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        let overview = menu(&mut app, "Instrument");
        assert!(visible(&overview, "Instrument of Evening lead 🎹"));
        assert!(visible(&overview, "Open editor"));
        assert!(visible(&overview, "Automate…"));
        assert!(!visible(&overview, "Search parameters"));
        assert!(!visible(&overview, "Read parameters"));
        assert!(!visible(&overview, "Automate parameter"));
        assert_eq!(overview["native_editor"]["open"], false);

        let chooser = latest(&mut app, "Automate…");
        assert_eq!(chooser["native_editor"]["parameters_open"], true);
        assert!(visible(&chooser, "Search parameters"));
        assert!(visible(&chooser, "Back"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = app.inspect();
            assert_eq!(state["native_editor"]["open"], false);
            if !state["native_editor"]["error"].is_null() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "metadata fixture must fail cleanly: {state}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let overview = latest(&mut app, "Back");
        assert_eq!(overview["native_editor"]["parameters_open"], false);
        assert!(visible(&overview, "Automate…"));
        assert!(!visible(&overview, "Search parameters"));
        latest(&mut app, "Automate…");
        let overview = app.key("Escape");
        assert_eq!(overview["popups"]["instrument"], true);
        assert_eq!(overview["native_editor"]["parameters_open"], false);

        let effects = latest(&mut app, "Effects");
        assert!(visible(&effects, "Effects of Evening lead 🎹"));
        assert!(visible(&effects, "01  Echo"));
        assert_eq!(effects["selected_track"], channel);
        // The metadata fixture contains no executable plugin, so attempting its
        // editor fails in the isolated helper without creating a native window.
        let effect = latest(&mut app, "Open editor");
        assert!(visible(&effect, "Echo of Evening lead 🎹"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.inspect()["native_editor"]["error"].is_null() {
            assert!(
                Instant::now() < deadline,
                "fixture editor did not fail cleanly"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let instrument = latest(&mut app, "Instrument");
        assert!(
            visible(&instrument, "Instrument of Evening lead 🎹"),
            "effect editor must link to the channel's actual instrument"
        );
        assert!(instrument["native_editor"]["effect"].is_null());
        latest(&mut app, "Effects");
        let routing = latest(&mut app, "Routing");
        assert!(visible(&routing, "Routing of Evening lead 🎹"));
        assert_eq!(routing["selected_track"], channel);
        let instrument = latest(&mut app, "Instrument");
        assert!(visible(&instrument, "Instrument of Evening lead 🎹"));
        assert_eq!(instrument["selected_track"], channel);
        let channel_view = latest(&mut app, "Channel");
        assert_eq!(channel_view["selected_track"], channel);
        let mixer = latest(&mut app, "Mixer");
        assert_eq!(mixer["view"], "Mixer");
        assert_eq!(mixer["selected_track"], channel);
        let channel_view = app.key("Escape");
        assert_eq!(channel_view["selected_track"], channel);
        assert_ne!(channel_view["view"], "Mixer");
        assert_eq!(channel_view["project"], baseline["project"]);
        assert_eq!(channel_view["history"], baseline["history"]);
        menu(&mut app, "Arrangement");
    }
    app.quit();
}
