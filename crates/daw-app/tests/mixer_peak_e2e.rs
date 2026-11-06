//! Peak capture, hold and reset through the real render worker and virtual device.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn channel(state: &Value, track: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()
}

fn center(rect: &Value) -> [f64; 2] {
    std::array::from_fn(|axis| {
        (rect["min"][axis].as_f64().unwrap() + rect["max"][axis].as_f64().unwrap()) * 0.5
    })
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let [x, y] = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 1.0
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn assert_badge_color(state: &Value, rect: &Value, color: eframe::egui::Color32) {
    assert!(
        state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| contains(rect, &text["rect"])
                && text["colors"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(color.to_array()))),
        "The peak badge must paint its actual semantic color: {rect}"
    );
}

fn assert_switch_color(
    state: &Value,
    rect: &Value,
    label: &str,
    color: eframe::egui::Color32,
    active: bool,
) {
    let expected = if active {
        daw_ui::theme::contrast_text(color)
    } else {
        color
    };
    assert!(
        state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text["text"] == label
                && contains(rect, &text["rect"])
                && text["colors"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(expected.to_array())))
    );
    if active {
        assert!(
            state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| shape["fill"] == json!(color.to_array())
                    && contains(rect, &shape["rect"])),
            "Active {label} must paint a solid operation-color fill"
        );
    }
}

#[test]
fn every_channel_and_master_hold_transients_until_mouse_reset_and_switches_are_color_coded() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::with_virtual_audio(None, None);
        app.command(
            json!({"command":"viewport", "width":1600, "height":1200, "pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme", "dark":dark}));
        let colors = if dark {
            daw_ui::theme::DARK
        } else {
            daw_ui::theme::LIGHT
        };
        let path = app.directory.path().join("peaks.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 2,
            samples: (0..192_000)
                .flat_map(|frame| {
                    if frame < 1024 {
                        [0.8, 0.4]
                    } else if frame < 4096 {
                        [0.2, 0.1]
                    } else {
                        [0.0; 2]
                    }
                })
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        let mut project = Project::new();
        let source = project.add_audio_track("Peak source");
        let silent = project.add_midi_track("Silent MIDI");
        let bus = project.add_bus("Peak bus");
        let send = project.add_bus("Quiet return");
        let track = project.track_mut(source).unwrap();
        track.gain = 1.5;
        track
            .clips
            .push(Clip::audio("Peak pulse", 0.0, 8.0, path.to_string_lossy()));
        project.set_output(source, Some(bus)).unwrap();
        project.add_send(source, send, 0.2, false).unwrap();
        let path = app.directory.path().join("peaks.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.click("Play");
        wait_until(&mut app, |state| {
            state["engine"]["playing"] == true && state["playback_loading"] == false
        });
        // The transient occurs while the mixer is closed and is followed by
        // over two seconds of digital silence before any peak badge is drawn.
        for _ in 0..16 {
            app.command(json!({"command":"pump_audio", "frames":8192}));
        }
        app.click("Mixer");
        let held = paint(&mut app);
        for (id, expected, clipped) in [
            (source, 1.2, true),
            (bus, 1.2, true),
            (send, 0.24, false),
            (silent, 0.0, false),
        ] {
            let channel = channel(&held, id);
            assert!(
                (channel["peak_hold"][0].as_f64().unwrap() - expected).abs() < 0.001,
                "{channel}"
            );
            assert_eq!(channel["peak_clipped"], clipped);
            assert_badge_color(
                &held,
                &channel["meter_readout"],
                if clipped {
                    colors.warning
                } else {
                    colors.muted
                },
            );
        }
        assert_eq!(held["mixer"]["master"]["peak_clipped"], true);
        assert_eq!(held["mixer"]["master"]["peak_hold"][0], 1.0);
        assert_badge_color(
            &held,
            &held["mixer"]["master"]["meter_readout"],
            colors.warning,
        );
        app.click("Pause");
        let paused = app.inspect();
        for _ in 0..120 {
            app.inspect();
        }
        let still_held = app.inspect();
        assert_eq!(channel(&still_held, source)["peaks"], json!([0.0, 0.0]));
        assert_eq!(
            channel(&still_held, source)["peak_hold"],
            channel(&paused, source)["peak_hold"]
        );
        app.click("Stop");
        let stopped = app.inspect();
        assert!(stopped["engine"].is_null());
        assert_eq!(
            channel(&stopped, source)["peak_hold"],
            channel(&paused, source)["peak_hold"],
            "Stopping must retain the highest measured peak until reset"
        );
        let cleared = click_rect(&mut app, &channel(&stopped, source)["meter_readout"]);
        assert_eq!(channel(&cleared, source)["peak_hold"], json!([0.0, 0.0]));
        assert_eq!(
            channel(&cleared, bus)["peak_clipped"],
            true,
            "Reset must affect only its channel"
        );
        for id in [bus, send, silent] {
            let rect = channel(&app.inspect(), id)["meter_readout"].clone();
            click_rect(&mut app, &rect);
        }
        let rect = app.inspect()["mixer"]["master"]["meter_readout"].clone();
        let cleared = click_rect(&mut app, &rect);
        assert_eq!(
            cleared["project"], paused["project"],
            "Peak reset is telemetry only"
        );
        assert_eq!(cleared["history"], paused["history"]);
        for id in [source, bus, send, silent] {
            assert_eq!(channel(&cleared, id)["peak_hold"], json!([0.0, 0.0]));
        }
        assert_eq!(cleared["mixer"]["master"]["peak_hold"], json!([0.0, 0.0]));
        assert_eq!(cleared["mixer"]["master"]["peak_clipped"], false);
        let clear_paint = paint(&mut app);
        assert_badge_color(
            &clear_paint,
            &channel(&clear_paint, source)["meter_readout"],
            colors.muted,
        );
        for (label, key, color) in [("Mute", "mute", colors.mute), ("Solo", "solo", colors.solo)] {
            let idle = paint(&mut app);
            assert_switch_color(&idle, &channel(&idle, source)[key], label, color, false);
            click_rect(&mut app, &channel(&idle, source)[key]);
            let active = paint(&mut app);
            assert_switch_color(&active, &channel(&active, source)[key], label, color, true);
        }
        for key in ["mute", "solo"] {
            let rect = channel(&app.inspect(), source)[key].clone();
            click_rect(&mut app, &rect);
        }
        app.click("Play");
        wait_until(&mut app, |state| {
            state["engine"]["playing"] == true && state["playback_loading"] == false
        });
        app.command(json!({"command":"pump_audio", "frames":8192}));
        app.click("Stop");
        let latched_again = app.inspect();
        assert_eq!(
            channel(&latched_again, source)["peak_clipped"],
            true,
            "A new overload after reset must be measured again"
        );
        assert_eq!(latched_again["mixer"]["master"]["peak_clipped"], true);
        app.command(json!({"command":"open", "path":path}));
        app.click("Mixer");
        let replaced = app.inspect();
        for id in [source, bus, send, silent] {
            assert_eq!(channel(&replaced, id)["peak_hold"], json!([0.0, 0.0]));
        }
        assert_eq!(replaced["mixer"]["master"]["peak_hold"], json!([0.0, 0.0]));
        app.quit();
    }
}
