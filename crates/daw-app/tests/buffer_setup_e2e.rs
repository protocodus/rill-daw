//! Recommended buffering is explicit, rate-aware and preserves manual choices.
mod common;

use common::Harness;
use serde_json::json;

fn setup(app: &mut Harness) {
    app.click("Window");
    app.click("Setup");
}

#[test]
fn balanced_buffer_recommendation_scales_with_rate_without_overriding_manual_settings() {
    let mut app = Harness::new();
    assert_eq!(app.inspect()["settings"]["buffer_frames"], 256);
    setup(&mut app);
    app.click("256 samples");
    app.click("128 samples");
    assert_eq!(app.click("Apply")["settings"]["buffer_frames"], 128);

    setup(&mut app);
    let suggested = app.click("Recommended");
    assert_eq!(suggested["setup_draft"]["buffer_frames"], 256);
    assert_eq!(suggested["settings"]["buffer_frames"], 128);
    assert_eq!(app.click("Cancel")["settings"]["buffer_frames"], 128);

    setup(&mut app);
    app.click("48000 Hz");
    app.click("96000 Hz");
    let suggested = app.click("Recommended");
    assert_eq!(suggested["setup_draft"]["buffer_frames"], 512);
    let applied = app.click("Apply");
    assert_eq!(applied["settings"]["buffer_frames"], 512);
    assert_eq!(applied["settings"]["sample_rate"], 96_000);
    let mut app = app.restart();
    assert_eq!(app.inspect()["settings"]["buffer_frames"], 512);
    setup(&mut app);
    app.click("96000 Hz");
    app.click("48000 Hz");
    assert_eq!(
        app.click("Recommended")["setup_draft"]["buffer_frames"],
        256
    );
    assert_eq!(app.click("Apply")["settings"]["buffer_frames"], json!(256));
    app.quit();
}
