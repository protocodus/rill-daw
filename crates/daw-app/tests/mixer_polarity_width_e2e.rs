//! The mixer strip's polarity switch and stereo width rail edit the channel,
//! undo as one step each and reach the exported audio.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}

fn pointer(app: &mut Harness, rect: &Value, button: &str) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,"button":button}))
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn middle_frame(audio: &AudioData) -> [f32; 2] {
    let frame = audio.samples.len() / 4;
    [audio.samples[frame * 2], audio.samples[frame * 2 + 1]]
}

#[test]
fn polarity_switch_and_width_rail_edit_the_channel_and_the_export() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Stereo.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: [0.4, 0.1].repeat(48_000),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Overheads");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Overheads",
        0.0,
        1.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("polarity-width.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app.click("Mixer");
    let dry = export(&mut app, "Dry.wav");
    let [left, right] = middle_frame(&dry);
    assert!(left > right && right > 0.0, "{left} {right}");

    let channel = &state["mixer"]["channels"][0];
    let inverted = pointer(&mut app, &channel["polarity"], "primary");
    assert_eq!(inverted["project"]["tracks"][0]["invert_polarity"], true);
    let flipped = middle_frame(&export(&mut app, "Inverted.wav"));
    assert!((flipped[0] + left).abs() < 1e-4 && (flipped[1] + right).abs() < 1e-4);

    pointer(
        &mut app,
        &inverted["mixer"]["channels"][0]["stereo_width"],
        "secondary",
    );
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"0"}));
    let mono = app.key("Enter");
    assert_eq!(mono["project"]["tracks"][0]["stereo_width"], 0.0);
    let folded = middle_frame(&export(&mut app, "Mono.wav"));
    let sum = -(left + right) / 2.0;
    assert!((folded[0] - sum).abs() < 1e-4 && (folded[1] - sum).abs() < 1e-4);

    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"]["tracks"][0]["stereo_width"], 1.0);
    assert_eq!(undone["project"]["tracks"][0]["invert_polarity"], true);
    app.click("Edit");
    let restored = app.click("Undo");
    // An upright channel omits the field from the project file.
    assert_eq!(
        restored["project"]["tracks"][0].get("invert_polarity"),
        None
    );
    app.quit();
}
