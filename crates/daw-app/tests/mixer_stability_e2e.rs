//! Transient notices and routing changes must not move a mixer control under the mouse.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn edge(rect: &Value, corner: &str, axis: usize) -> f64 {
    rect[corner][axis].as_f64().unwrap()
}

fn center(rect: &Value) -> [f64; 2] {
    [
        (edge(rect, "min", 0) + edge(rect, "max", 0)) * 0.5,
        (edge(rect, "min", 1) + edge(rect, "max", 1)) * 0.5,
    ]
}

fn click_at(app: &mut Harness, position: [f64; 2]) -> Value {
    app.command(json!({"command":"pointer", "phase":"click",
        "x":position[0], "y":position[1]}))
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn settled(app: &mut Harness) -> Value {
    for _ in 0..12 {
        app.inspect();
    }
    painted(app)
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn unchanged(state: &Value, before: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], before[key], "Layout changed {key}");
    }
}

fn same_controls(state: &Value, before: &Value, ids: &[u64]) {
    for id in ids {
        for control in ["fader", "pan", "mute", "solo", "meter", "target", "sends"] {
            let actual = &channel(state, *id)[control];
            let previous = &channel(before, *id)[control];
            for corner in ["min", "max"] {
                for axis in 0..2 {
                    assert!(
                        (edge(actual, corner, axis) - edge(previous, corner, axis)).abs() < 0.1,
                        "Channel {id} {control} moved at {corner}[{axis}]: {previous} -> {actual}"
                    );
                }
            }
        }
    }
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn fixture(app: &mut Harness, dark: bool, density: f64, height: f64) -> (u64, u64) {
    let source = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio source");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Phrase",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI source");
    project.add_bus("Room");
    project.add_bus("Delay");
    let path = app.directory.path().join("mixer-stability.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    app.command(
        json!({"command":"viewport", "width":1200.0, "height":height,
        "pixels_per_point":density}),
    );
    app.click("Mixer");
    (audio, midi)
}

fn ready(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["engine"]["playing"] == true && state["playback_loading"] == false {
            return settled(app);
        }
        assert!(Instant::now() < deadline, "Playback did not become ready");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn mixer_controls_stay_under_the_pointer_during_output_failure_and_recovery() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::with_virtual_audio(None, None);
        let (audio, midi) = fixture(&mut app, dark, density, 1000.0);
        app.click("Play");
        let before = ready(&mut app);
        let mute_position = center(&channel(&before, audio)["mute"]);
        let failed = app.command(json!({"command":"output_error"}));
        assert_eq!(failed["engine"]["output_failed"], true);
        assert!(visible(&failed, "Retry playback"));
        let failed = settled(&mut app);
        same_controls(&failed, &before, &[audio, midi]);
        unchanged(&failed, &before);

        app.click("Retry playback");
        let resumed = ready(&mut app);
        assert_eq!(resumed["engine"]["output_failed"], false);
        same_controls(&resumed, &before, &[audio, midi]);
        app.command(json!({"command":"output_error"}));
        same_controls(&settled(&mut app), &before, &[audio, midi]);
        // Reuse the coordinate observed before either asynchronous failure.
        // Undo reloads playback state and may replace its notice, so exercise
        // Retry before editing instead of expecting a stale notice afterward.
        let muted = click_at(&mut app, mute_position);
        assert_eq!(track(&muted, audio)["mute"], true);
        assert_eq!(track(&muted, midi)["mute"], false);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        same_controls(&settled(&mut app), &before, &[audio, midi]);
        app.quit();
    }
}

#[test]
fn mixer_export_completion_preserves_control_geometry_and_mouse_target() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (audio, midi) = fixture(&mut app, dark, density, 1000.0);
        let before = settled(&mut app);
        let solo_position = center(&channel(&before, midi)["solo"]);
        let output = app.directory.path().join("Finished mix.wav");
        app.command(json!({"command":"export", "path":output}));
        app.command(json!({"command":"wait_export"}));
        let completed = settled(&mut app);
        assert!(visible(&completed, "Exported · Finished mix.wav"));
        same_controls(&completed, &before, &[audio, midi]);
        unchanged(&completed, &before);
        let soloed = click_at(&mut app, solo_position);
        assert_eq!(track(&soloed, midi)["solo"], true);
        assert_eq!(track(&soloed, audio)["solo"], false);
        app.click("Dismiss");
        same_controls(&settled(&mut app), &before, &[audio, midi]);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn mixer_send_creation_keeps_existing_controls_fixed_and_routes_reachable() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (audio, midi) = fixture(&mut app, dark, density, 1000.0);
        let before = settled(&mut app);
        assert_eq!(before["selected_track"], audio);
        let mute_position = center(&channel(&before, audio)["mute"]);
        for (index, destination) in ["Room", "Delay"].into_iter().enumerate() {
            app.click("Add send…");
            app.click(destination);
            let sent = settled(&mut app);
            assert_eq!(
                track(&sent, audio)["sends"].as_array().unwrap().len(),
                index + 1
            );
            same_controls(&sent, &before, &[audio, midi]);
        }
        let sent_project = app.inspect()["project"].clone();
        let muted = click_at(&mut app, mute_position);
        assert_eq!(track(&muted, audio)["mute"], true);
        assert_eq!(undo(&mut app)["project"], sent_project);

        // Reveal the additional send lanes with ordinary scrolling. The
        // workspace header remains stationary and no routing edit is implied.
        let back = before["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "Back")
            .unwrap()
            .clone();
        let before_scroll = settled(&mut app);
        app.command(json!({"command":"scroll", "x":90.0, "y":550.0, "delta":-250.0}));
        let scrolled = settled(&mut app);
        unchanged(&scrolled, &before_scroll);
        assert!(visible(&scrolled, "Post send → Room"));
        assert!(visible(&scrolled, "Post send → Delay"));
        let now_back = scrolled["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "Back")
            .unwrap();
        assert_eq!(now_back["min"], back["min"]);
        assert_eq!(now_back["max"], back["max"]);
        let viewport = &scrolled["mixer"]["reorder_viewport"];
        let output = &channel(&scrolled, audio)["target"];
        assert!(edge(output, "min", 1) >= edge(viewport, "min", 1));
        assert!(edge(output, "max", 1) <= edge(viewport, "max", 1));
        app.quit();
    }
}

#[test]
fn mixer_notice_changes_keep_an_existing_short_window_scroll_position() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::with_virtual_audio(None, None);
        let (audio, midi) = fixture(&mut app, dark, density, 720.0);
        app.command(json!({"command":"viewport", "width":800.0, "height":720.0}));
        app.click("Play");
        let top = ready(&mut app);
        app.command(json!({"command":"scroll", "x":90.0, "y":350.0, "delta":-80.0}));
        let before = settled(&mut app);
        assert!(
            edge(&channel(&before, audio)["pan"], "min", 1)
                < edge(&channel(&top, audio)["pan"], "min", 1) - 20.0,
            "The test must start at an actually scrolled position"
        );
        app.command(json!({"command":"output_error"}));
        let failed = settled(&mut app);
        assert!(visible(&failed, "Retry playback"));
        same_controls(&failed, &before, &[audio, midi]);
        let viewport = &failed["mixer"]["reorder_viewport"];
        let mute = &channel(&failed, audio)["mute"];
        assert!(edge(mute, "min", 1) >= edge(viewport, "min", 1));
        assert!(edge(mute, "max", 1) <= edge(viewport, "max", 1));
        app.click("Dismiss");
        same_controls(&settled(&mut app), &before, &[audio, midi]);
        let muted = click_at(&mut app, center(&channel(&before, audio)["mute"]));
        assert_eq!(track(&muted, audio)["mute"], true);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        same_controls(&settled(&mut app), &before, &[audio, midi]);
        app.quit();
    }
}
