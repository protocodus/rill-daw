//! Terminal actions stay at the bottom of short, long and compact dialogs.
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
fn at_bottom(state: &Value, label: &str) {
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
    let gap = bottom - bounds(action)[3];
    assert!(
        (16.0..=30.0).contains(&gap),
        "{label} is {gap:.2} points above the dialog bottom: {window}"
    );
}

#[test]
fn join_generator_and_take_actions_remain_at_the_bottom_in_both_appearances() {
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
        at_bottom(&settled(&mut app), "Cancel");
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
        at_bottom(&settled(&mut app), "Back to editor");
        app.click("Back to editor");
        assert_eq!(app.inspect()["project"], opened["project"]);
        app.quit();
    }
}
