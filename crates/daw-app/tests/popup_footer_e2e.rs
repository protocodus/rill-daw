//! Terminal actions stay at the bottom of dialogs, and follow the content of
//! pages, which fill the workspace.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn settled(app: &mut Harness) -> Value {
    for _ in 0..8 {
        app.inspect();
    }
    app.inspect()
}
fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}
fn in_window(state: &Value, label: &str) -> ([f64; 4], [f64; 4]) {
    let window = state["window_rects"].as_array().unwrap().last().unwrap();
    let [left, top, right, bottom] = bounds(window);
    let action = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|item| {
            item["text"] == label && {
                let [x0, y0, x1, y1] = bounds(item);
                x0 >= left && x1 <= right && y0 >= top && y1 <= bottom
            }
        })
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    (bounds(window), bounds(action))
}
fn at_bottom(state: &Value, label: &str) {
    let (window, action) = in_window(state, label);
    let gap = window[3] - action[3];
    assert!(
        (16.0..=30.0).contains(&gap),
        "{label} is {gap:.2} points above the dialog bottom: {window:?}"
    );
}
/// A page's footer sits right after its content, not at the workspace bottom.
fn hugs(state: &Value, label: &str, max_height: f64) {
    let (window, action) = in_window(state, label);
    assert!(
        action[3] - window[1] < max_height,
        "{label} is not right after the page content: {window:?} {action:?}"
    );
}

#[test]
fn join_generator_and_take_actions_stay_with_their_content_in_both_appearances() {
    for (dark, width, height, density) in [(false, 1600, 1200, 1.0), (true, 800, 600, 2.0)] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        let mut project = Project::new();
        let track = project.add_midi_track("Footer synth");
        project.track_mut(track).unwrap().clips = vec![
            Clip::midi(
                "Footer phrase",
                0.0,
                8.0,
                vec![Note::new(60, 0.0, 1.0, 100)],
            ),
            Clip::midi("Other phrase", 8.0, 8.0, vec![]),
        ];
        let path = app.directory.path().join("footers.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.click("Footer phrase");
        app.click("Edit");
        app.click("Join clips…");
        let join = settled(&mut app);
        in_window(&join, "Cancel");
        // The subject lives in the title, and the two-row list leaves no
        // blank band between its last row and the footer divider.
        let title = label(&join, "Join clips · Footer synth");
        let last_row = label(&join, "Beats 9.00 – 17.00");
        let cancel = label(&join, "Cancel");
        assert!(
            cancel[1] - last_row[3] < 2.0 * daw_ui::tokens::CONTROL_HEIGHT as f64,
            "Join did not hug its list: {title:?} {last_row:?} {cancel:?}"
        );
        app.click("Cancel");
        app.click("Window");
        app.click("Clip Editor");
        app.click("Generate…");
        at_bottom(&settled(&mut app), "Cancel");
        app.click("Melody");
        at_bottom(&settled(&mut app), "Cancel");
        app.click("Cancel");
        let wave = app.directory.path().join("source.wav");
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.1; 192000],
        }
        .write_wav(&wave)
        .unwrap();
        let audio = project.add_audio_track("Footer takes");
        project.track_mut(audio).unwrap().clips.push(Clip::audio(
            "Original take",
            0.0,
            8.0,
            wave.to_string_lossy(),
        ));
        project.save(&path).unwrap();
        let opened = app.command(json!({"command":"open","path":path}));
        app.click("Original take");
        app.click("Window");
        app.click("Clip Editor");
        app.click("Clip");
        app.click("Takes…");
        in_window(&settled(&mut app), "Back to editor");
        app.click("Back to editor");
        assert_eq!(app.inspect()["project"], opened["project"]);
        app.quit();
    }
}

fn label(state: &Value, text: &str) -> [f64; 4] {
    bounds(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|item| item["text"] == text)
            .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"])),
    )
}

#[test]
fn footer_windows_hug_short_content_and_put_the_primary_action_rightmost() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1600,"height":1200}));
    app.click("Window");
    app.click("Setup");
    let setup = settled(&mut app);
    // A short form leaves no blank band: its actions follow right after it,
    // far above the 1200 pt viewport budget.
    hugs(&setup, "Apply", 700.0);
    let (apply, cancel) = (label(&setup, "Apply"), label(&setup, "Cancel"));
    assert!(
        apply[0] > cancel[2],
        "Apply must be right of Cancel: {apply:?} {cancel:?}"
    );
    assert!(
        (apply[1] - cancel[1]).abs() < 1.0,
        "Footer actions share one row"
    );
    let resampling = label(&setup, "Resampling");
    let buffer = label(&setup, "Buffer");
    assert!(
        (resampling[0] - buffer[0]).abs() < 0.5,
        "Form labels share one column"
    );
    app.click("Cancel");
    app.click("Help");
    app.click("About RILL");
    let about = settled(&mut app);
    hugs(&about, "Close", 400.0);
    app.quit();
}
