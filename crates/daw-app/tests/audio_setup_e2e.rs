//! Audio quality choices travel through Setup and the production export worker.
mod common;
use common::Harness;
use serde_json::json;

fn setup(app: &mut Harness) {
    app.click("Window");
    app.click("Setup");
    app.click("Audio");
}
fn render(app: &mut Harness, name: &str) -> daw_core::AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    daw_core::AudioData::load_wav(path).unwrap()
}
fn rms(audio: &daw_core::AudioData) -> f64 {
    let skip = audio.sample_rate as usize / 20 * audio.channels;
    let samples = &audio.samples[skip..audio.samples.len() - skip];
    (samples
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

#[test]
fn quality_and_sample_rate_switches_preserve_project_and_affect_real_exports() {
    let mut app = Harness::new();
    let source = app.directory.path().join("ultrasonic.wav");
    daw_core::AudioData {
        sample_rate: 96_000,
        channels: 1,
        samples: (0..96_000)
            .map(|i| (std::f64::consts::TAU * 28_000.0 * i as f64 / 96_000.0).sin() as f32 * 0.2)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let state = app.command(json!({"command":"drop","paths":[source]}));
    let before = state["project"].clone();
    assert_eq!(state["settings"]["resample_quality"], "High");
    setup(&mut app);
    app.click("High quality");
    app.click("Fast");
    let state = app.click("Cancel");
    assert_eq!(state["settings"]["resample_quality"], "High");
    assert_eq!(state["project"], before);
    setup(&mut app);
    app.click("High quality");
    app.click("Fast");
    let state = app.click("Apply");
    assert_eq!(state["settings"]["resample_quality"], "Fast");
    assert_eq!(state["project"], before);
    let fast = render(&mut app, "fast.wav");
    assert_eq!(fast.sample_rate, 48_000);
    assert!(rms(&fast) > 0.05);
    setup(&mut app);
    app.click("Fast");
    app.click("High quality");
    app.click("Apply");
    let high = render(&mut app, "high.wav");
    assert!(
        rms(&high) < rms(&fast) * 0.05,
        "High quality must reject downsample aliasing: {} vs {}",
        rms(&high),
        rms(&fast)
    );
    setup(&mut app);
    app.click("48000 Hz");
    app.click("96000 Hz");
    let state = app.click("Apply");
    assert_eq!(state["settings"]["sample_rate"], 96_000);
    assert_eq!(state["project"], before);
    let full = render(&mut app, "96k.wav");
    assert_eq!(full.sample_rate, 96_000);
    assert_eq!(full.frames(), 115_200); // one-second source plus the renderer's 0.2-second safety tail
    assert!(rms(&full) > 0.05);
    let path = app.directory.path().join("quality.rill");
    app.command(json!({"command":"save","path":path}));
    let state = app.command(json!({"command":"open","path":path}));
    assert_eq!(state["settings"]["sample_rate"], 96_000);
    assert_eq!(state["settings"]["resample_quality"], "High");
    let settings: serde_json::Value = serde_json::from_slice(
        &std::fs::read(app.directory.path().join("config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["sample_rate"], 96_000);
    assert_eq!(settings["resample_quality"], "High");
    app.quit();
}
