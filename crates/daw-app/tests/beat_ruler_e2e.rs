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

fn ruler_vertical_bounds(state: &Value, editor: Editor, plot: [f64; 4]) -> (f64, f64) {
    // The arranger has a separate labelled loop lane between ruler and canvas.
    // Use its actual ruler bounds rather than assuming a fixed canvas offset.
    match editor {
        Editor::Arrangement => (
            state["timeline"]["ruler"]["min"][1].as_f64().unwrap(),
            state["timeline"]["ruler"]["max"][1].as_f64().unwrap(),
        ),
        Editor::Midi | Editor::Audio => {
            let view = if matches!(editor, Editor::Midi) {
                "midi_editor"
            } else {
                "audio_editor"
            };
            (
                state[view]["ruler"]["min"][1].as_f64().unwrap(),
                state[view]["ruler"]["max"][1].as_f64().unwrap(),
            )
        }
        Editor::Automation => (
            plot[1] - 12.0 - f64::from(daw_ui::tokens::TIMELINE_RULER_HEIGHT),
            plot[1] - 12.0,
        ),
    }
}

fn ruler_label<'a>(
    state: &'a Value,
    editor: Editor,
    text: &str,
    plot: [f64; 4],
) -> Option<&'a Value> {
    let (top, bottom) = ruler_vertical_bounds(state, editor, plot);
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            label["text"] == text
                && label["min"][1].as_f64().unwrap() >= top
                && label["max"][1].as_f64().unwrap() <= bottom
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
                let label = ruler_label(&close, editor, text, rect).unwrap_or_else(|| {
                    panic!("{editor:?} lacks {text}: {}", close["visible_labels"])
                });
                let expected =
                    rect[0] + (0.25 - start) * pixels + f64::from(daw_ui::tokens::SPACE_XS);
                assert!(
                    (label["min"][0].as_f64().unwrap() - expected).abs() < 1.0,
                    "{editor:?}: {label}"
                );
            }
            let clock = ruler_label(&close, editor, "0:00.15", rect).unwrap();
            let (_, ruler_bottom) = ruler_vertical_bounds(&close, editor, rect);
            assert!(
                clock["max"][1].as_f64().unwrap() + f64::from(daw_ui::tokens::SPACE_MD)
                    <= ruler_bottom,
                "{editor:?}: the clock needs space above the cursor/loop strip: {clock}"
            );
            if matches!(editor, Editor::Midi) {
                let local = ruler_label(&close, editor, "Local", rect).unwrap();
                assert!(
                    (local["max"][1].as_f64().unwrap() - clock["max"][1].as_f64().unwrap()).abs()
                        < 1.0,
                    "Local time origin must stay on the clock baseline: {local}, {clock}"
                );
                let note = &close["midi_editor"]["notes"][0]["rect"];
                let label = ruler_label(&close, editor, "1.1.2", rect).unwrap();
                assert!(
                    (label["min"][0].as_f64().unwrap()
                        - f64::from(daw_ui::tokens::SPACE_XS)
                        - note["min"][0].as_f64().unwrap())
                    .abs()
                        < 1.0,
                    "The note and ruler must share a time anchor: label {label}, note {note}"
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
            assert!(ruler_label(&wide, editor, "1.1.2", rect).is_none());
            assert!(ruler_label(&wide, editor, "1", rect).is_some());
            app.quit();
        }
    }
}

#[test]
fn zoomed_arranger_positions_distant_events_on_the_ruler_time_axis() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Long arrangement");
    let start = 900_000.281_25;
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Off-grid attack",
        start,
        0.25,
        vec![Note::new(60, 0.0, 0.125, 96)],
    ));
    let sustained = project.add_midi_track("Long clip");
    let note_start = start - 0.125;
    project.track_mut(sustained).unwrap().clips.push(Clip::midi(
        "Late note",
        0.0,
        start,
        vec![Note::new(64, note_start, 0.0625, 100)],
    ));
    let source = app.directory.path().join("cue.wav");
    AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.1; 8000],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Long audio event");
    let mut cue = Clip::audio("Tail fade", 0.0, start, source.to_string_lossy());
    if let daw_core::ClipContent::Audio { fade_out, .. } = &mut cue.content {
        *fade_out = 0.125;
    }
    project.track_mut(audio).unwrap().clips.push(cue);
    let path = app.directory.path().join("distant-event.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    navigate(&mut app, Editor::Arrangement, 1600.0, start - 0.25);
    let state = app.command(json!({"command":"inspect","paint":true}));
    let (first, pixels, canvas) = view(&state, Editor::Arrangement);
    let expected = canvas[0] + (start - first) * pixels;
    assert!(expected > canvas[0] && expected < canvas[2]);
    let clips = state["timeline"]["clips"].as_array().unwrap();
    let clip = clips.iter().find(|clip| clip["track"] == track).unwrap();
    assert!((clip["rect"]["min"][0].as_f64().unwrap() - expected).abs() < 1.0);
    let clip = clips
        .iter()
        .find(|clip| clip["track"] == sustained)
        .unwrap();
    assert!((clip["rect"]["max"][0].as_f64().unwrap() - expected).abs() < 1.0);
    let note_x = canvas[0] + (note_start - first) * pixels;
    let audio_clip = clips.iter().find(|clip| clip["track"] == audio).unwrap();
    let handle = &audio_clip["fade_out_handle"];
    let handle_x = (handle["min"][0].as_f64().unwrap() + handle["max"][0].as_f64().unwrap()) / 2.0;
    assert!(
        (handle_x - note_x).abs() < 1.0,
        "the fade and note share one beat"
    );
    let top = clip["content_rect"]["min"][1].as_f64().unwrap();
    let bottom = clip["content_rect"]["max"][1].as_f64().unwrap();
    assert!(
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| {
                let rect = &shape["rect"];
                (rect["min"][0].as_f64().unwrap() - note_x).abs() < 1.0
                    && rect["min"][1].as_f64().unwrap() >= top
                    && rect["max"][1].as_f64().unwrap() <= bottom
            }),
        "the note near the end of a long clip must share the ruler's time axis"
    );
    app.quit();
}

#[test]
fn elapsed_clock_labels_omit_empty_hours_but_keep_real_hour_and_second_values() {
    let mut app = Harness::new();
    let mut project = Project::new();
    // One beat per second and per bar makes each expected elapsed time a
    // primary ruler anchor, including one beyond five hours.
    project.tempo = 60.0;
    project.beats_per_bar = 1;
    let track = project.add_midi_track("Long session");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Clock guide",
        0.0,
        19_000.0,
        vec![Note::new(60, 0.0, 1.0, 90)],
    ));
    let path = app.directory.path().join("elapsed-clock.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":760,
            "pixels_per_point":density}));
        for (seconds, expected) in [(15.0, "0:15"), (165.0, "2:45"), (18_955.0, "5:15:55")] {
            let state = navigate(&mut app, Editor::Arrangement, 160.0, seconds);
            let (_, _, rect) = view(&state, Editor::Arrangement);
            let label =
                ruler_label(&state, Editor::Arrangement, expected, rect).unwrap_or_else(|| {
                    panic!(
                        "Missing elapsed time {expected}: {}",
                        state["visible_labels"]
                    )
                });
            assert!(label["max"][0].as_f64().unwrap() <= rect[2]);
            for field in ["project", "history", "modified"] {
                assert_eq!(
                    state[field], original[field],
                    "Viewing a time changed {field}"
                );
            }
        }
    }
    app.quit();
}

#[test]
fn four_bar_guides_follow_meter_changes_through_tracks_empty_space_and_the_ruler() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.beats_per_bar = 4;
    project.set_meter_change(3, 3).unwrap();
    let track = project.add_midi_track("Meter guide");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Cue", 0.0, 2.0, vec![]));
    let path = app.directory.path().join("four-bar-guides.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));
    let editor = Editor::Arrangement;

    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1280,"height":800,"pixels_per_point":density}),
        );
        navigate(&mut app, editor, 36.0, 0.0);
        let state = app.command(json!({"command":"inspect","paint":true}));
        let (start, pixels, rect) = view(&state, editor);
        let middle_y = |area: &Value| {
            (area["min"][1].as_f64().unwrap() + area["max"][1].as_f64().unwrap()) * 0.5
        };
        let row_y = middle_y(&state["timeline"]["tracks"][0]["header"]);
        let empty_y = middle_y(&state["timeline"]["empty_canvas"]);
        // The grid is neutral. A more prominent line is brighter in the
        // dark theme and darker in the light theme.
        let strength = |beat: f64, y: f64| {
            let x = rect[0] + (beat - start) * pixels;
            let line = state["painted_lines"]
                .as_array()
                .unwrap()
                .iter()
                .find(|line| {
                    let p = &line["points"];
                    (p[0][0].as_f64().unwrap() - x).abs() <= 0.6
                        && (p[1][0].as_f64().unwrap() - x).abs() <= 0.6
                        && p[0][1].as_f64().unwrap() < y
                        && y < p[1][1].as_f64().unwrap()
                })
                .unwrap_or_else(|| panic!("No visible grid line at beat {beat}, ({x},{y})"));
            let rgb: i64 = (0..3).map(|i| line["stroke"][i].as_i64().unwrap()).sum();
            if dark { rgb } else { -rgb }
        };
        // Bar 5 falls at beat 14 after the change to 3/4. Its
        // predecessor is bar 4 at beat 11, with beat 12 inside it.
        assert!(strength(14.0, row_y) > strength(11.0, row_y));
        assert!(strength(11.0, row_y) > strength(12.0, row_y));
        assert!(strength(14.0, empty_y) > strength(11.0, empty_y));

        navigate(&mut app, editor, 8.0, 0.0);
        let wide = app.command(json!({"command":"inspect","paint":true}));
        let (start, pixels, rect) = view(&wide, editor);
        for (bar, beat) in [("1", 0.0), ("5", 14.0), ("9", 26.0)] {
            let label = ruler_label(&wide, editor, bar, rect)
                .unwrap_or_else(|| panic!("Missing bar {bar} from the song overview"));
            let expected = rect[0] + (beat - start) * pixels + f64::from(daw_ui::tokens::SPACE_XS);
            assert!((label["min"][0].as_f64().unwrap() - expected).abs() < 1.0);
        }
        assert!(ruler_label(&wide, editor, "4", rect).is_none());
        let (_, bottom) = ruler_vertical_bounds(&wide, editor, rect);
        let unlabeled_bar = rect[0] + (11.0 - start) * pixels;
        assert!(
            wide["painted_lines"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| {
                    let p = &line["points"];
                    (p[0][0].as_f64().unwrap() - unlabeled_bar).abs() < 0.6
                        && (p[1][0].as_f64().unwrap() - unlabeled_bar).abs() < 0.6
                        && (p[1][1].as_f64().unwrap() - bottom).abs() <= 0.6
                        && (2.0..=6.0).contains(&(bottom - p[0][1].as_f64().unwrap()))
                }),
            "unlabeled bar 4 still needs its musical tick"
        );
        assert_eq!(wide["project"], original["project"]);
        assert_eq!(wide["history"], original["history"]);
    }
    app.quit();
}
