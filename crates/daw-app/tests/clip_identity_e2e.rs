//! Hover distinguishes repeated clip names without changing the arrangement.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

const NAME: &str = "New pattern · 夜のシンセ phrase with a long name";

fn clip_view(state: &Value, id: u64) -> &Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 0.01
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 0.01
    })
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "selected_clips",
        "selected_notes",
        "selection_span",
    ] {
        assert_eq!(state[field], before[field], "Clip hover changed {field}");
    }
    assert_eq!(state["timeline"]["cursor"], before["timeline"]["cursor"]);
}

fn hover(app: &mut Harness, id: u64) -> Value {
    let state = app.inspect();
    let (x, y) = center(&clip_view(&state, id)["visible_rect"]);
    app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    // Inspect advances the headless app's ordinary frames, including the
    // production hover delay. No wall-clock sleep or tooltip override is needed.
    let prefix = format!("{NAME}\n");
    for _ in 0..80 {
        let state = app.command(json!({"command":"inspect", "paint":true}));
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text.as_str().is_some_and(|text| text.starts_with(&prefix)))
        {
            return state;
        }
    }
    panic!("Clip {id} never exposed its occurrence tooltip");
}

#[test]
fn identical_clip_names_reveal_track_position_and_link_scope_without_editing() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.tempo = 112.0;
    project.beats_per_bar = 4;
    let track = project.add_midi_track("Occurrence synth");
    let first = Clip::midi(NAME, 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
    let first_id = first.id;
    let second = Clip::midi(NAME, 4.0, 4.0, vec![Note::new(64, 0.0, 1.0, 100)]);
    let second_id = second.id;
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let third_id = project.clone_clip(track, second_id).unwrap();
    // Fit a thirty-second arrangement so these one-bar titles are truncated at
    // both desktop widths, just like a row of repeated beginner patterns.
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("End marker", 52.0, 4.0, vec![]));
    let path = app.directory.path().join("clip-identity.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));

    let mut saw_icon_only_title = false;
    for (width, density, dark) in [(800, 1.0, false), (1440, 2.0, true)] {
        app.command(json!({"command":"viewport", "width":width, "height":900,
            "pixels_per_point":density}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        let fitted = app.click("Fit");
        let (x, y) = center(&clip_view(&fitted, first_id)["visible_rect"]);
        let before = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
        assert_eq!(before["selected_clip"], first_id);

        for (id, position, clock, linked) in [
            (first_id, "Bar 1 · Beat 1.00", "0:00.000–0:02.143", false),
            (second_id, "Bar 2 · Beat 1.00", "0:02.143–0:04.286", true),
            (third_id, "Bar 3 · Beat 1.00", "0:04.286–0:06.429", true),
        ] {
            let state = hover(&mut app, id);
            unchanged(&state, &before);
            let expected = format!("{NAME}\nOccurrence synth · {position}\n{clock}");
            let hints: Vec<_> = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|label| {
                    label["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with(&format!("{NAME}\n")))
                })
                .collect();
            assert_eq!(hints.len(), 1, "Only the hovered occurrence needs a hint");
            let text = hints[0]["text"].as_str().unwrap();
            assert!(
                text.starts_with(&expected),
                "Wrong occurrence identity: {text}"
            );
            assert_eq!(text.contains("Ghost clip"), linked);
            if linked {
                assert!(
                    text.contains("Edits and name are shared. Position and trim are independent.")
                );
            }
            assert!(hints[0]["min"][0].as_f64().unwrap() >= 0.0);
            assert!(hints[0]["min"][1].as_f64().unwrap() >= 0.0);
            assert!(hints[0]["max"][0].as_f64().unwrap() <= state["viewport"][0].as_f64().unwrap());
            assert!(hints[0]["max"][1].as_f64().unwrap() <= state["viewport"][1].as_f64().unwrap());

            for visible_id in [first_id, second_id, third_id] {
                let clip = clip_view(&state, visible_id);
                let original = clip_view(&before, visible_id);
                for field in [
                    "rect",
                    "visible_rect",
                    "title_rect",
                    "content_rect",
                    "link_icon",
                ] {
                    assert_eq!(clip[field], original[field], "Hover moved clip {field}");
                }
                assert!(contains(&clip["rect"], &clip["title_rect"]));
                assert!(contains(&clip["rect"], &clip["content_rect"]));
                assert!(
                    clip["title_rect"]["max"][0].as_f64().unwrap()
                        - clip["title_rect"]["min"][0].as_f64().unwrap()
                        < 120.0
                );
                let title = state["painted_text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|text| {
                        text["text"] == NAME && contains(&clip["title_rect"], &text["rect"])
                    });
                let title_right = clip["title_rect"]["max"][0].as_f64().unwrap();
                let text_left = if clip["link_icon"].is_null() {
                    clip["title_rect"]["min"][0].as_f64().unwrap()
                } else {
                    assert!(contains(&clip["title_rect"], &clip["link_icon"]));
                    (clip["link_icon"]["max"][0].as_f64().unwrap()
                        + f64::from(daw_ui::tokens::SPACE_XS))
                    .min(title_right)
                };
                if text_left < title_right {
                    assert!(
                        title.is_some(),
                        "Truncated title must remain inside its clip when text has room: width={width}, density={density}, hovered={id}, checked={visible_id}, clip={clip}, painted_text={}",
                        state["painted_text"]
                    );
                } else {
                    // A narrow ghost reserves its whole title for the chain
                    // icon. Its full name must still be exposed by hover above.
                    assert_eq!(clip["linked"], true);
                    assert!(
                        title.is_none(),
                        "Icon-only titles must not paint cut-off text"
                    );
                    saw_icon_only_title = true;
                }
            }
        }
    }
    assert!(
        saw_icon_only_title,
        "The narrow fixture must exercise icon-only identity"
    );
    app.quit();
}
