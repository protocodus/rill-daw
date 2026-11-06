//! The real meter's held signal and the fader index share the same physical
//! pixel row. A unity source makes measured signal equal the edited fader gain.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

fn point(value: &Value) -> [f64; 2] {
    [number(&value[0]), number(&value[1])]
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn click_rect(app: &mut Harness, rect: &Value) {
    let at = std::array::from_fn::<_, 2, _>(|axis| {
        (number(&rect["min"][axis]) + number(&rect["max"][axis])) * 0.5
    });
    app.command(json!({"command":"pointer", "phase":"click", "x":at[0], "y":at[1]}));
}

fn actual_fader_index(state: &Value, control: &Value, density: f64, unity: bool) -> f64 {
    let center = point(&control["fader_scale"]["handle_center"]);
    if unity {
        let points: Vec<_> = state["painted_circles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|shape| {
                let at = point(&shape["center"]);
                (at[0] - center[0]).abs() * density <= 0.51
                    && (at[1] - center[1]).abs() * density <= 0.51
                    && number(&shape["radius"]) <= 2.0
                    && number(&shape["fill"][3]) > 0.0
                    && number(&shape["stroke_width"]) == 0.0
            })
            .collect();
        assert_eq!(points.len(), 1, "Unity uses one small filled point");
        return number(&points[0]["center"][1]);
    }
    let indexes: Vec<_> = state["painted_lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|line| {
            let a = point(&line["points"][0]);
            let b = point(&line["points"][1]);
            ((a[1] - b[1]).abs() < 1e-6
                && ((a[0] + b[0]) * 0.5 - center[0]).abs() * density <= 0.51
                && (a[1] - center[1]).abs() * density <= 0.51
                && (b[0] - a[0]).abs() >= 8.0
                && (b[0] - a[0]).abs() <= 20.0)
                .then_some(a[1])
        })
        .collect();
    assert_eq!(indexes.len(), 1, "A non-unity cap has one precise groove");
    indexes[0]
}

fn assert_meter_alignment(state: &Value, id: u64, density: f64, unity: bool) {
    let control = channel(state, id);
    let gain = number(&state["project"]["tracks"][0]["gain"]);
    let meter = &control["meter"];
    let left = number(&meter["min"][0]);
    let right = number(&meter["max"][0]);
    let top = number(&meter["min"][1]);
    let bottom = number(&meter["max"][1]);
    let index = actual_fader_index(state, control, density, unity);
    let native_center = point(&control["fader_scale"]["handle_center"]);
    let caps: Vec<_> = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| {
            let min = point(&shape["rect"]["min"]);
            let max = point(&shape["rect"]["max"]);
            (10.0..=25.0).contains(&(max[0] - min[0]))
                && (5.0..=16.0).contains(&(max[1] - min[1]))
                && ((min[0] + max[0]) * 0.5 - native_center[0]).abs() * density <= 0.51
                && ((min[1] + max[1]) * 0.5 - index).abs() * density < 0.01
                && number(&shape["stroke_width"]) > 0.0
                && number(&shape["stroke"][3]) > 0.0
                && number(&shape["fill"][3]) > 0.0
        })
        .collect();
    assert_eq!(caps.len(), 1, "The cap must center precisely on its index");
    for corner in ["min", "max"] {
        for coordinate in point(&caps[0]["rect"][corner]) {
            let pixel = coordinate * density;
            assert!(
                (pixel - pixel.round()).abs() < 0.01,
                "Cap edge must lie on a physical pixel: {pixel}"
            );
        }
    }

    // Read all three narrow troughs from actual submitted painter rectangles.
    // Equal outer rows prevent the cap and meter from appearing vertically
    // offset even when their independently calibrated level indices agree.
    let troughs_in = |min_x: f64, max_x: f64| -> Vec<&Value> {
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|shape| {
                let min = point(&shape["rect"]["min"]);
                let max = point(&shape["rect"]["max"]);
                (2.0..=8.0).contains(&(max[0] - min[0]))
                    && max[1] - min[1] > 80.0
                    && min[0] >= min_x - 0.51 / density
                    && max[0] <= max_x + 0.51 / density
                    && number(&shape["stroke_width"]) > 0.0
                    && number(&shape["stroke"][3]) > 0.0
                    && number(&shape["fill"][3]) > 0.0
            })
            .collect()
    };
    let rail = &control["fader_scale"]["rail"];
    let fader_trough = troughs_in(number(&rail["min"][0]), number(&rail["max"][0]));
    let meter_troughs = troughs_in(left, right);
    assert_eq!(fader_trough.len(), 1, "One actual fader trough");
    assert_eq!(meter_troughs.len(), 2, "Two actual stereo meter troughs");
    for trough in meter_troughs {
        for corner in ["min", "max"] {
            assert!(
                (number(&trough["rect"][corner][1]) - number(&fader_trough[0]["rect"][corner][1]))
                    .abs()
                    * density
                    < 0.01,
                "Painted fader and meter trough {corner} edges must align"
            );
        }
    }

    // Inspect submitted meter strokes, rather than assuming the component's
    // reported outer rectangle proves its actual calibrated drawing aligned.
    let holds: Vec<_> = state["painted_lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|line| {
            let a = point(&line["points"][0]);
            let b = point(&line["points"][1]);
            (a[0].min(b[0]) >= left - 0.51 / density
                && a[0].max(b[0]) <= right + 0.51 / density
                && a[0] != b[0]
                && a[1] == b[1]
                && a[1] >= top
                && a[1] <= bottom)
                .then_some(a[1])
        })
        .collect();
    assert_eq!(
        holds.len(),
        2,
        "Two real stereo peak-hold strokes: {holds:?}"
    );
    for y in holds {
        assert!(
            (y - index).abs() * density < 0.01,
            "Meter and fader index must share a pixel row at {density}×: {y} vs {index}"
        );
        let pixel = y * density;
        assert!(
            ((pixel - 0.5).round() + 0.5 - pixel).abs() < 0.01,
            "The calibrated index must land on a physical pixel center: {pixel}"
        );
    }
    for measured in control["peaks"].as_array().unwrap() {
        assert!((number(measured) - gain).abs() < 2e-6);
    }
    for key in ["min_position", "max_position"] {
        let y = number(&control["fader_scale"][key][1]);
        assert!(y >= top && y <= bottom, "Fader travel fits the meter");
    }
}

#[test]
fn rendered_signal_and_mouse_edited_fader_share_pixel_rows_at_multiple_scales() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let mut app = Harness::with_virtual_audio(None, None);
        let audio = app.directory.path().join("Unity source.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 2,
            samples: vec![1.0; 96_000],
        }
        .write_wav(&audio)
        .unwrap();
        let mut project = Project::new();
        let id = project.add_audio_track("Calibrated fader");
        let track = project.track_mut(id).unwrap();
        track.gain = 0.6;
        track
            .clips
            .push(Clip::audio("Unity", 0.0, 2.0, audio.to_string_lossy()));
        let path = app.directory.path().join("Aligned sliders.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1280, "height":900,
            "pixels_per_point":density}));
        app.click("Mixer");

        for height in [900, 1183] {
            app.command(json!({"command":"viewport", "width":1280, "height":height,
                "pixels_per_point":density}));
            for fraction in [0.55, 0.75, 1.0] {
                let before = app.inspect();
                let control = channel(&before, id);
                click_rect(&mut app, &control["meter_readout"]);
                let min = point(&control["fader_scale"]["min_position"]);
                let max = point(&control["fader_scale"]["max_position"]);
                let at = std::array::from_fn::<_, 2, _>(|axis| {
                    min[axis] + (max[axis] - min[axis]) * fraction
                });
                let edited = app.command(json!({"command":"pointer", "phase":"click",
                    "x":at[0], "y":at[1]}));
                let expected = f64::from(daw_ui::gain_untaper(fraction as f32));
                assert!((number(&edited["project"]["tracks"][0]["gain"]) - expected).abs() < 1e-4);
                app.click("Play");
                wait_until(&mut app, |state| {
                    state["engine"]["playing"] == true && state["playback_loading"] == false
                });
                app.command(json!({"command":"pump_audio", "frames":512}));
                let paint = painted(&mut app);
                assert_meter_alignment(&paint, id, density, fraction == 0.75);
                for key in ["project", "history", "modified"] {
                    assert_eq!(
                        paint[key], edited[key],
                        "Painting and metering changed {key}"
                    );
                }
                app.click("Stop");
                let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
                assert_eq!(
                    undone["project"], before["project"],
                    "Fader edit is reversible"
                );
            }
        }
        app.quit();
    }
}
