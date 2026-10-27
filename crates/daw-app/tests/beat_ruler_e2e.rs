//! Close zoom keeps musical and clock positions visible between bar lines.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Note, Project,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Editor {
    Arrangement,
    Midi,
    Audio,
    Automation,
}

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}

fn fixture(editor: Editor, meter: u8, dark: bool, density: f64) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1100,"height":760,"pixels_per_point":density}));
    app.command(json!({"command":"system_theme","dark":dark}));
    let source = app.directory.path().join("ruler-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 100.0;
    project.beats_per_bar = meter;
    let midi = project.add_midi_track("Beat guide");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Guide phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.25, 0.25, 90), Note::new(64, 1.25, 0.25, 90)],
    ));
    let audio = project.add_audio_track("Beat audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        0.0,
        8.0,
        source.to_string_lossy(),
    ));
    let lane = project
        .add_automation_track(
            "Gain curve",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![
        AutomationPoint::new(0.25, 0.4),
        AutomationPoint::new(8.0, 0.8),
    ];
    let path = app.directory.path().join("ruler-fixture.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    match editor {
        Editor::Arrangement => {
            app.click("Guide phrase");
        }
        Editor::Midi => {
            app.click("Guide phrase");
            menu(&mut app, "Window", "Clip Editor");
            app.command(json!({"command":"key","key":"A","cmd":true}));
        }
        Editor::Audio => {
            app.click("Audio phrase");
            menu(&mut app, "Window", "Clip Editor");
        }
        Editor::Automation => {
            app.click("Gain curve");
            menu(&mut app, "Window", "Automation");
        }
    }
    app
}

fn view(state: &Value, editor: Editor) -> (f64, f64, [f64; 4]) {
    let (value, rect) = match editor {
        Editor::Arrangement => (&state["timeline"], &state["timeline"]["canvas"]),
        Editor::Midi => (&state["midi_editor"], &state["midi_editor"]["plot"]),
        Editor::Audio => (&state["audio_editor"], &state["audio_editor"]["rect"]),
        Editor::Automation => (
            &state["automation_editor"],
            &state["automation_editor"]["plot"],
        ),
    };
    let rect = if let Some(values) = rect.as_array() {
        std::array::from_fn(|index| values[index].as_f64().unwrap())
    } else {
        [
            rect["min"][0].as_f64().unwrap(),
            rect["min"][1].as_f64().unwrap(),
            rect["max"][0].as_f64().unwrap(),
            rect["max"][1].as_f64().unwrap(),
        ]
    };
    (
        value["start_beat"].as_f64().unwrap(),
        value["pixels_per_beat"].as_f64().unwrap(),
        rect,
    )
}

fn navigate(app: &mut Harness, editor: Editor, pixels: f64, start: f64) -> Value {
    let initial = app.inspect();
    let (_, previous, rect) = view(&initial, editor);
    let (x, y) = ((rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5);
    let zoom = app.command(
        json!({"command":"scroll","x":x,"y":y,"delta":(pixels/previous).ln()/0.005,"ctrl":true}),
    );
    let (current, pixels, rect) = view(&zoom, editor);
    app.command(
        json!({"command":"scroll","x":(rect[0]+rect[2])*0.5,"y":(rect[1]+rect[3])*0.5,
        "delta":(current-start)*pixels,"shift":true}),
    );
    app.inspect()
}

fn selection(state: &Value, editor: Editor) -> Value {
    let notes = if matches!(editor, Editor::Midi) {
        state["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|note| note["selected"] == true)
            .map(|note| note["id"].clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    json!([
        state["selected_track"],
        state["selected_clip"],
        notes,
        state["automation_editor"]["point_beat"]
    ])
}

fn ruler_label<'a>(state: &'a Value, text: &str, plot: [f64; 4]) -> Option<&'a Value> {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            label["text"] == text
                && label["min"][1].as_f64().unwrap() >= plot[1] - 64.0
                && label["max"][1].as_f64().unwrap() <= plot[1]
        })
}

#[test]
fn close_zoom_rulers_show_aligned_beat_and_clock_anchors_in_all_music_editors() {
    for (meter, dark, density) in [(3, false, 1.0), (4, true, 2.0)] {
        for editor in [
            Editor::Arrangement,
            Editor::Midi,
            Editor::Audio,
            Editor::Automation,
        ] {
            let mut app = fixture(editor, meter, dark, density);
            let original = app.inspect();
            let selected = selection(&original, editor);
            // The complete visible view lies between whole beats, and hence
            // between bar lines: it still needs meaningful position anchors.
            let close = navigate(&mut app, editor, 1600.0, 0.125);
            let (start, pixels, rect) = view(&close, editor);
            assert!((start - 0.125).abs() < 0.0001, "{editor:?}: {start}");
            assert!(start + (rect[2] - rect[0]) / pixels < 1.0);
            for text in ["1.1.2", "0:00.15"] {
                let label = ruler_label(&close, text, rect).unwrap_or_else(|| {
                    panic!("{editor:?} lacks {text}: {}", close["visible_labels"])
                });
                let expected =
                    rect[0] + (0.25 - start) * pixels + f64::from(daw_ui::tokens::SPACE_XS);
                assert!(
                    (label["min"][0].as_f64().unwrap() - expected).abs() < 1.0,
                    "{editor:?}: {label}"
                );
            }
            if matches!(editor, Editor::Midi) {
                let note = &close["midi_editor"]["notes"][0]["rect"];
                let label = ruler_label(&close, "1.1.2", rect).unwrap();
                assert!(
                    (label["min"][0].as_f64().unwrap() - 4.0 - note["min"][0].as_f64().unwrap())
                        .abs()
                        < 1.0
                );
            }
            let wide = navigate(&mut app, editor, 40.0, 0.0);
            for state in [&close, &wide] {
                assert_eq!(state["project"], original["project"], "{editor:?}");
                assert_eq!(state["history"], original["history"], "{editor:?}");
                assert_eq!(state["modified"], original["modified"], "{editor:?}");
                assert_eq!(selection(state, editor), selected, "{editor:?}");
            }
            let (_, _, rect) = view(&wide, editor);
            assert!(ruler_label(&wide, "1.1.2", rect).is_none());
            assert!(ruler_label(&wide, "1", rect).is_some());
            app.quit();
        }
    }
}
