//! Shape and override full-song automation through the production mouse UI.
mod common;

use common::Harness;
use daw_core::{AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const RATE: usize = 48_000;

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == label)
}

fn click_last(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| *item == label)
        .count();
    assert!(count > 0, "Missing {label}: {}", state["visible_labels"]);
    app.command(json!({"command":"click", "label":label, "occurrence":count-1}))
}

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

fn scroll_curve_popup(app: &mut Harness, down: bool) -> Value {
    let state = app.inspect();
    let plot = &state["automation_editor"]["plot"];
    let (_, y) = center(plot);
    app.command(json!({"command":"scroll",
        "x":plot["min"][0].as_f64().unwrap() - 21.0, "y":y,
        "delta":if down { -1000.0 } else { 1000.0 }}));
    for _ in 0..20 {
        app.inspect();
    }
    app.inspect()
}

fn click_visible_inspector(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let target = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .expect("Inspector action is visible");
    let (x, y) = center(target);
    // Let the scroll area's hover state follow the mouse before pressing.
    // A real pointer travels from its wheel target to the inspector first.
    app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    app.click(name)
}

fn lane(state: &Value, id: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()["automation"]
}

fn point(state: &Value, beat: f64, value: f64) -> (f64, f64) {
    let editor = &state["automation_editor"];
    let plot = &editor["plot"];
    (
        plot["min"][0].as_f64().unwrap()
            + (beat - editor["start_beat"].as_f64().unwrap())
                * editor["pixels_per_beat"].as_f64().unwrap(),
        plot["max"][1].as_f64().unwrap()
            - value * (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap()),
    )
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "{actual} != {expected}"
    );
}

fn fixture(dark: bool, density: f64) -> (Harness, u64, u64) {
    fixture_in(Harness::new(), dark, density)
}

fn fixture_in(mut app: Harness, dark: bool, density: f64) -> (Harness, u64, u64) {
    let audio = app.directory.path().join("held-source.wav");
    AudioData {
        sample_rate: RATE as u32,
        channels: 1,
        samples: vec![0.25; RATE * 4],
    }
    .write_wav(&audio)
    .unwrap();
    let mut project = Project::new();
    project.loop_enabled = false;
    let source = project.add_audio_track("Held source");
    let channel = project.track_mut(source).unwrap();
    channel.gain = 0.9;
    channel
        .clips
        .push(Clip::audio("Held tone", 0.0, 8.0, audio.to_string_lossy()));
    let automation = project
        .add_automation_track(
            "Held volume",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.automation_lane_mut(automation).unwrap().points = vec![
        AutomationPoint::new(2.0, 0.2),
        AutomationPoint::new(4.0, 0.8),
        AutomationPoint::new(6.0, 0.4),
    ];
    let path = app.directory.path().join("held-automation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"system_theme","dark":dark}));
    app.command(json!({"command":"viewport","width":1100,"height":760,"pixels_per_point":density}));
    (app, source, automation)
}

fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Automation engine did not settle: {}",
            state["engine"]
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pump_level(app: &mut Harness, identity: &Value, gain: f64) -> Value {
    let state = app.command(json!({"command":"pump_audio","frames":8192}));
    for field in ["engine_id", "generation"] {
        assert_eq!(
            state["engine"][field], identity["engine"][field],
            "Automation replaced {field}"
        );
    }
    assert_eq!(state["engine"]["playing"], true);
    assert_eq!(state["engine"]["render_errors"], 0);
    assert_eq!(state["playback_loading"], false);
    let samples = state["audio_output"].as_array().unwrap();
    // Existing queued frames may precede a live edit; the tail must contain
    // the new value after native control smoothing has completed.
    for frame in &samples[samples.len() - 2048..] {
        for channel in 0..2 {
            near(frame[channel].as_f64().unwrap(), 0.25 * gain, 0.00003);
        }
    }
    state
}

#[test]
fn compact_curve_keeps_readable_percentages_and_mouse_access_to_point_inspectors() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let (mut app, _, automation) = fixture(dark, density);
        app.click("Held volume");
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        menu(&mut app, "Window", "Automation");
        let state = app.command(json!({"command":"inspect", "paint":true}));
        let plot = &state["automation_editor"]["plot"];
        let left = plot["min"][0].as_f64().unwrap();
        let top = plot["min"][1].as_f64().unwrap();
        let bottom = plot["max"][1].as_f64().unwrap();
        assert!(bottom - top >= f64::from(daw_ui::tokens::PARAMETER_ROW_HEIGHT) - 0.1);
        assert!(
            top >= 0.0 && bottom <= 600.0,
            "The minimum curve must be visible"
        );
        let mut ticks: Vec<_> = state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|text| {
                ["0%", "25%", "50%", "75%", "100%"]
                    .iter()
                    .any(|tick| text["text"] == *tick)
                    && text["rect"]["max"][0].as_f64().unwrap() < left
                    && text["rect"]["min"][1].as_f64().unwrap() >= top - 21.0
                    && text["rect"]["max"][1].as_f64().unwrap() <= bottom + 21.0
            })
            .collect();
        ticks.sort_by(|a, b| {
            a["rect"]["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&b["rect"]["min"][1].as_f64().unwrap())
        });
        assert_eq!(ticks.first().unwrap()["text"], "100%");
        assert_eq!(ticks.last().unwrap()["text"], "0%");
        for pair in ticks.windows(2) {
            assert!(
                pair[1]["rect"]["min"][1].as_f64().unwrap()
                    - pair[0]["rect"]["max"][1].as_f64().unwrap()
                    >= f64::from(daw_ui::tokens::SPACE_XS) - 1.0 / density
            );
        }
        let initial = lane(&state, automation)["points"].clone();
        let (x, y) = point(&state, 4.0, 0.8);
        let selected = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
        assert_eq!(
            selected["automation_editor"]["selected_point"],
            initial[1]["id"]
        );
        assert_eq!(selected["project"], state["project"]);
        // Scroll over the value gutter, outside the timeline's pan surface.
        app.command(
            json!({"command":"scroll", "x":left - 21.0, "y":(top + bottom) * 0.5, "delta":-1000.0}),
        );
        for _ in 0..20 {
            app.inspect();
        }
        let scrolled = app.inspect();
        let exp = scrolled["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|label| label["text"] == "Exp")
            .expect("Segment inspector remains reachable by scrolling");
        assert!(exp["min"][1].as_f64().unwrap() >= 0.0 && exp["max"][1].as_f64().unwrap() <= 600.0);
        let shaped = click_visible_inspector(&mut app, "Exp");
        assert_eq!(lane(&shaped, automation)["points"][1]["curve"], 1.0);
        assert_eq!(lane(&shaped, automation)["points"][0], initial[0]);
        assert_eq!(lane(&shaped, automation)["points"][2], initial[2]);
        app.quit();
    }
}

#[test]
fn explicit_override_resume_and_curve_edits_keep_live_engine_and_audio() {
    let (mut app, _, automation) = fixture_in(Harness::with_virtual_audio(None, None), true, 1.0);
    app.click("Held volume");
    menu(&mut app, "Window", "Automation");
    app.click("Play");
    let running = wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    pump_level(&mut app, &running, 0.3);
    let overridden = app.click("Override automation");
    assert!(visible(&overridden, "Overridden"));
    near(
        lane(&overridden, automation)["override_value"]
            .as_f64()
            .unwrap(),
        0.2,
        0.00001,
    );
    let stable_override = app.inspect();
    let override_field = stable_override["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            label["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Override "))
        })
        .expect("Visible manual override number");
    let (x, y) = center(override_field);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"0.9"}));
    let manual = app.key("Enter");
    near(
        lane(&manual, automation)["override_value"]
            .as_f64()
            .unwrap(),
        0.6,
        0.00001,
    );
    wait(&mut app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    });
    pump_level(&mut app, &running, 0.9);
    let resumed = app.click("Resume automation");
    assert!(lane(&resumed, automation)["override_value"].is_null());
    wait(&mut app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    });
    pump_level(&mut app, &running, 0.3);
    let state = app.inspect();
    let (x, y) = point(&state, 2.0, 0.2);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let shaped = app.click("Exp");
    assert_eq!(lane(&shaped, automation)["points"][0]["curve"], 1.0);
    wait(&mut app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    });
    pump_level(&mut app, &running, 0.3);
    app.click("Stop");
    app.quit();
}

fn export(app: &mut Harness, filename: &str) -> AudioData {
    let path = app.directory.path().join(filename);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn assert_sample(audio: &AudioData, beat: f64, gain: f64) {
    assert_eq!(audio.sample_rate, RATE as u32);
    assert_eq!(audio.channels, 2);
    let frame = (beat * 0.5 * RATE as f64).round() as usize;
    for channel in 0..2 {
        near(
            f64::from(audio.samples[frame * 2 + channel]),
            0.25 * gain,
            0.00003,
        );
    }
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    menu(app, "Edit", if redo { "Redo" } else { "Undo" })
}

fn reopen_saved(mut app: Harness, filename: &str) -> (Harness, Value) {
    let path = app.directory.path().join(filename);
    app.command(json!({"command":"file_dialog","kind":"save","path":path}));
    menu(&mut app, "File", "Save as…");
    app.command(json!({"command":"wait_project"}));
    let mut app = app.restart();
    app.command(json!({"command":"file_dialog","kind":"open","path":path}));
    let mut state = menu(&mut app, "File", "Open…");
    if state["project_pending"] == true {
        state = app.command(json!({"command":"wait_project"}));
    }
    (app, state)
}

#[test]
fn segment_shapes_and_mouse_handles_preserve_neighbors_and_hold_song_endpoints() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let (mut app, _, automation) = fixture(dark, density);
        app.click("Held volume");
        let initial = menu(&mut app, "Window", "Automation");
        assert_eq!(lane(&initial, automation)["interpolation"], "Spline");
        let points = lane(&initial, automation)["points"].clone();
        let baseline = export(&mut app, "linear.wav");
        assert_sample(&baseline, 0.0, 0.3);
        assert_sample(&baseline, 1.0, 0.3); // Before the first point at beat two.
        assert_sample(&baseline, 3.0, 0.75);
        assert_sample(&baseline, 7.5, 0.6); // After the last point at beat six.

        let state = app.inspect();
        let (x, y) = point(&state, 2.0, 0.2);
        app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        let exponential = app.click("Exp");
        assert_eq!(lane(&exponential, automation)["points"][0]["curve"], 1.0);
        assert_eq!(lane(&exponential, automation)["points"][1], points[1]);
        assert_eq!(lane(&exponential, automation)["points"][2], points[2]);
        let audio = export(&mut app, "exponential.wav");
        assert_sample(&audio, 0.0, 0.3);
        assert_sample(&audio, 3.0, 0.4125);
        assert_sample(&audio, 7.5, 0.6);
        let logarithmic = app.click("Log");
        assert_eq!(lane(&logarithmic, automation)["points"][0]["curve"], -1.0);
        assert_sample(&export(&mut app, "logarithmic.wav"), 3.0, 1.0875);
        let linear = click_last(&mut app, "Linear");
        assert_eq!(lane(&linear, automation)["interpolation"], "Spline");
        assert_eq!(lane(&linear, automation)["points"], points);

        let first = points[0]["id"].as_u64().unwrap();
        let handle = linear["automation_editor"]["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|segment| segment["start"] == first)
            .unwrap();
        let (x, y) = center(&handle["rect"]);
        let dragged = app.command(json!({"command":"pointer","phase":"drag", "x":x,"y":y,
            "to_x":x,"to_y":y+32.0}));
        let curve = lane(&dragged, automation)["points"][0]["curve"]
            .as_f64()
            .unwrap();
        assert!(
            curve > 0.05 && curve <= 1.0,
            "Downward drag must bend rising segment toward Exp: {curve}"
        );
        assert_eq!(lane(&dragged, automation)["points"][1], points[1]);
        assert_eq!(lane(&dragged, automation)["points"][2], points[2]);
        assert_eq!(undo(&mut app, false)["project"], linear["project"]);
        assert_eq!(undo(&mut app, true)["project"], dragged["project"]);
        let (mut app, recalled) = reopen_saved(app, "shaped-automation.rill");
        assert_eq!(lane(&recalled, automation), lane(&dragged, automation));
        let rendered = export(&mut app, "recalled-curve.wav");
        assert_sample(&rendered, 0.0, 0.3);
        assert_sample(&rendered, 7.5, 0.6);
        app.quit();
    }
}

#[test]
fn compact_automation_keeps_slope_and_resume_controls_reachable() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let (mut app, _, automation) = fixture(dark, density);
        app.click("Held volume");
        menu(&mut app, "Window", "Automation");
        for (width, height) in [(800, 600), (1100, 720)] {
            app.command(json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}));
            scroll_curve_popup(&mut app, false);
            let state = app.click("Fit");
            let (x, y) = point(&state, 2.0, 0.2);
            app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
            scroll_curve_popup(&mut app, true);
            let shaped = click_visible_inspector(&mut app, "Exp");
            assert_eq!(lane(&shaped, automation)["points"][0]["curve"], 1.0);
            let shaped = scroll_curve_popup(&mut app, true);
            for name in ["Log", "Exp"] {
                let label = shaped["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|label| label["text"] == name)
                    .unwrap_or_else(|| {
                        panic!(
                            "Missing {name} at {width}×{height}: labels={} editor={} windows={} footer={}",
                            shaped["visible_labels"], shaped["automation_editor"], shaped["window_rects"],
                            json!(shaped["label_rects"].as_array().unwrap().iter().filter(|label| label["min"][1].as_f64().unwrap_or(0.0)>350.0).collect::<Vec<_>>())
                        )
                    });
                assert!(label["min"][0].as_f64().unwrap() >= 0.0);
                assert!(label["max"][0].as_f64().unwrap() <= f64::from(width));
                assert!(label["max"][1].as_f64().unwrap() <= f64::from(height));
            }
            scroll_curve_popup(&mut app, false);
            let overridden = click_visible_inspector(&mut app, "Override automation");
            let resume = overridden["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|label| label["text"] == "Resume automation")
                .expect("Resume remains visible");
            assert!(resume["max"][1].as_f64().unwrap() <= f64::from(height));
            let resumed = app.click("Resume automation");
            assert!(lane(&resumed, automation)["override_value"].is_null());
        }
        app.quit();
    }
}

#[test]
fn manual_gain_override_is_visible_saved_and_resumable_without_changing_curve_points() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let (mut app, source, automation) = fixture(dark, density);
        let original = app.click("Mixer");
        let channel = original["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == source)
            .unwrap();
        // The stopped control already reads the first automation value, not the
        // stored manual value (0.9), even before the first point is reached.
        assert!(visible(&original, "Automated"));
        assert!(
            visible(&original, "-10.5 dB"),
            "{}",
            original["visible_labels"]
        );
        let (x, y) = center(&channel["gain"]);
        app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
        app.command(json!({"command":"key","key":"A","cmd":true}));
        app.command(json!({"command":"text","text":"-6.0"}));
        let overridden = app.key("Enter");
        let gain = 10.0_f64.powf(-6.0 / 20.0);
        near(
            lane(&overridden, automation)["override_value"]
                .as_f64()
                .unwrap(),
            gain / 1.5,
            1e-6,
        );
        assert!(visible(&overridden, "Overridden"));
        assert_eq!(
            lane(&overridden, automation)["points"],
            lane(&original, automation)["points"]
        );
        let audio = export(&mut app, "manual-override.wav");
        for beat in [0.0, 1.0, 3.0, 5.0, 7.5] {
            assert_sample(&audio, beat, gain);
        }
        assert_eq!(undo(&mut app, false)["project"], original["project"]);
        assert_eq!(undo(&mut app, true)["project"], overridden["project"]);

        let (mut app, recalled) = reopen_saved(app, "overridden-automation.rill");
        assert_eq!(lane(&recalled, automation), lane(&overridden, automation));
        app.click("Held volume");
        let editor = menu(&mut app, "Window", "Automation");
        assert!(visible(&editor, "Overridden"));
        assert!(visible(&editor, "Resume automation"));
        let resumed = app.click("Resume automation");
        assert!(lane(&resumed, automation)["override_value"].is_null());
        assert!(visible(&resumed, "Automated"));
        assert_eq!(
            lane(&resumed, automation)["points"],
            lane(&original, automation)["points"]
        );
        let audio = export(&mut app, "resumed.wav");
        assert_sample(&audio, 0.0, 0.3);
        assert_sample(&audio, 3.0, 0.75);
        assert_sample(&audio, 7.5, 0.6);
        assert_eq!(undo(&mut app, false)["project"], recalled["project"]);
        let restored = undo(&mut app, true);
        assert_eq!(lane(&restored, automation), lane(&resumed, automation));
        app.quit();
    }
}
