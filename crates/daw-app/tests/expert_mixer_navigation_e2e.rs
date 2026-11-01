//! Direct clip properties, visible routing, and distinct default bus names.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn edge(rect: &Value, corner: &str, axis: usize) -> f64 {
    rect[corner][axis].as_f64().unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (edge(rect, "min", 0) + edge(rect, "max", 0)) * 0.5,
        (edge(rect, "min", 1) + edge(rect, "max", 1)) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn tracks(state: &Value) -> &[Value] {
    state["project"]["tracks"].as_array().unwrap()
}

fn track(state: &Value, id: u64) -> &Value {
    tracks(state)
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], before[key], "Viewing changed {key}");
    }
}

fn open_project(app: &mut Harness, project: &Project, filename: &str) -> Value {
    let path = app.directory.path().join(filename);
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}))
}

fn audio_source(app: &Harness) -> std::path::PathBuf {
    let path = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 144_000],
    }
    .write_wav(&path)
    .unwrap();
    path
}

fn start_keypad(app: &mut Harness) -> Value {
    let state = app.inspect();
    let start = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            label["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Beat "))
        })
        .unwrap();
    let keypad = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| {
            label["text"] == "123"
                && (center(label).1 - center(start).1).abs() < 12.0
                && edge(label, "min", 0) > edge(start, "max", 0)
        })
        .min_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .unwrap();
    click_rect(app, keypad)
}

fn clip_properties_workflow(audio: bool) {
    for dark in [false, true] {
        let mut app = Harness::new();
        let source = audio_source(&app);
        let mut project = Project::new();
        let audio_track = project.add_audio_track("Recorded instrument");
        project
            .track_mut(audio_track)
            .unwrap()
            .clips
            .push(Clip::audio(
                "Audio phrase",
                2.0,
                4.0,
                source.to_string_lossy(),
            ));
        let midi_track = project.add_midi_track("Played instrument");
        project
            .track_mut(midi_track)
            .unwrap()
            .clips
            .push(Clip::midi(
                "MIDI phrase",
                2.0,
                4.0,
                vec![Note::new(60, 0.0, 1.0, 100)],
            ));
        open_project(&mut app, &project, "direct-properties.rill");
        app.command(json!({"command":"viewport", "width":1440, "height":1000}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        let index = usize::from(!audio);
        let target = project.tracks[index].clips[0].id;
        let owner = project.tracks[index].id;
        app.click(if audio { "MIDI phrase" } else { "Audio phrase" });
        let before = app.click("Fit");
        assert_ne!(before["selected_clip"], target);
        let rect = &before["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == target)
            .unwrap()["visible_rect"];
        let (x, y) = center(rect);
        let choices = app.command(json!({"command":"pointer", "phase":"click",
            "button":"secondary", "x":x, "y":y}));
        unchanged(&choices, &before);
        assert_eq!(choices["selected_clip"], before["selected_clip"]);
        assert!(
            choices["menu_label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label["text"] == "Properties")
        );
        let properties = app.click("Properties");
        unchanged(&properties, &before);
        assert_eq!(properties["selected_track"], owner);
        assert_eq!(properties["selected_clip"], target);
        assert_eq!(properties["view"], "Arrangement");
        assert_eq!(properties["popups"]["properties"], true);
        assert!(visible(
            &properties,
            if audio {
                "Properties of Audio phrase"
            } else {
                "Properties of MIDI phrase"
            }
        ));

        start_keypad(&mut app);
        app.click("Clear");
        let draft = app.click("9");
        unchanged(&draft, &properties);
        let cancelled = app.click("Cancel");
        unchanged(&cancelled, &properties);
        assert_eq!(cancelled["selected_clip"], target);
        let returned = app.click("Arranger");
        unchanged(&returned, &before);
        assert_eq!(returned["view"], "Arrangement");
        assert_eq!(returned["popups"]["properties"], false);
        assert_eq!(returned["selected_clip"], target);

        // Reopen the same direct command and commit a mouse-only exact start.
        app.command(json!({"command":"pointer", "phase":"click",
            "button":"secondary", "x":x, "y":y}));
        app.click("Properties");
        start_keypad(&mut app);
        app.click("Clear");
        for digit in ["8", ".", "5"] {
            app.click(digit);
        }
        let edited = app.click("Apply value");
        let mut expected = before["project"].clone();
        expected["tracks"][index]["clips"][0]["start"] = json!(7.5);
        assert_eq!(
            edited["project"], expected,
            "Only the clicked occurrence moves"
        );
        assert_eq!(
            edited["history"]["undo"].as_array().unwrap().len(),
            before["history"]["undo"].as_array().unwrap().len() + 1
        );
        let returned = app.click("Arranger");
        assert_eq!(returned["view"], "Arrangement");
        assert_eq!(returned["selected_clip"], target);
        assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn audio_clip_context_properties_targets_its_own_timing_and_returns_to_arranger() {
    clip_properties_workflow(true);
}

#[test]
fn midi_clip_context_properties_targets_its_own_timing_and_returns_to_arranger() {
    clip_properties_workflow(false);
}

fn within(rect: &Value, viewport: &Value) {
    for axis in 0..2 {
        assert!(
            edge(rect, "min", axis) >= edge(viewport, "min", axis) - 0.5,
            "Leading edge outside viewport: {rect} / {viewport}"
        );
        assert!(
            edge(rect, "max", axis) <= edge(viewport, "max", axis) + 0.5,
            "Trailing edge outside viewport: {rect} / {viewport}"
        );
    }
}

fn same_controls(state: &Value, before: &Value, ids: &[u64]) {
    for id in ids {
        for control in ["fader", "pan", "mute", "solo", "meter", "target", "sends"] {
            for corner in ["min", "max"] {
                for axis in 0..2 {
                    assert!(
                        (edge(&channel(state, *id)[control], corner, axis)
                            - edge(&channel(before, *id)[control], corner, axis))
                        .abs()
                            < 0.1,
                        "Channel {id} {control} moved after a notice or routing change"
                    );
                }
            }
        }
    }
}

fn settled_source(app: &mut Harness, source: u64) -> Value {
    let state = app.inspect();
    let (x, y) = center(&channel(&state, source)["meter"]);
    app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    for _ in 0..4 {
        app.inspect();
    }
    app.inspect()
}

fn visible_routes(state: &Value, source: u64, destination: u64) {
    let viewport = &state["mixer"]["reorder_viewport"];
    assert_eq!(state["mixer"]["flow_source"], source);
    for (kind, caption, target) in [
        ("Output", "Output → Master", Value::Null),
        ("Post send", "Post send → Room", json!(destination)),
    ] {
        let route = state["mixer"]["routes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|route| route["source"] == source && route["kind"] == kind)
            .unwrap();
        assert_eq!(route["target"], target);
        assert_eq!(route["source_clipped"], false);
        assert_eq!(route["target_clipped"], false);
        within(&route["lane"], viewport);
        within(label(state, caption), viewport);
        for endpoint in ["from", "to"] {
            for axis in 0..2 {
                let point = route[endpoint][axis].as_f64().unwrap();
                assert!(
                    point >= edge(viewport, "min", axis) && point <= edge(viewport, "max", axis)
                );
            }
        }
    }
    let rail = &channel(state, source)["fader_scale"]["rail"];
    within(rail, viewport);
    assert!(
        edge(rail, "max", 1) - edge(rail, "min", 1) >= 80.0,
        "Fader must retain useful mouse travel"
    );
}

#[test]
fn output_and_one_send_stay_visible_without_scrolling_through_notices_and_undo() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let audio = audio_source(&app);
        let mut project = Project::new();
        let source = project.add_audio_track("Source");
        project.track_mut(source).unwrap().clips.push(Clip::audio(
            "Phrase",
            0.0,
            2.0,
            audio.to_string_lossy(),
        ));
        let other = project.add_midi_track("Other");
        let room = project.add_bus("Room");
        open_project(&mut app, &project, "visible-mixer-routes.rill");
        app.command(json!({"command":"viewport", "width":1440, "height":1000}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.click("Mixer");
        let before = settled_source(&mut app, source);
        let fader = channel(&before, source)["fader_scale"]["handle_center"].clone();
        app.click("Add send…");
        app.click("Room");
        let sent = settled_source(&mut app, source);
        // Ordinary edit confirmation is status metadata; the export below
        // exercises the visible notice that consumes workspace height.
        assert_eq!(sent["status"], "Send added");
        visible_routes(&sent, source, room);
        same_controls(&sent, &before, &[source, other, room]);

        let output = app.directory.path().join("Visible routes.wav");
        app.command(json!({"command":"export", "path":output}));
        app.command(json!({"command":"wait_export"}));
        let exported = settled_source(&mut app, source);
        assert!(visible(&exported, "Exported · Visible routes.wav"));
        unchanged(&exported, &sent);
        visible_routes(&exported, source, room);
        same_controls(&exported, &before, &[source, other, room]);
        app.click("Dismiss");
        let dismissed = settled_source(&mut app, source);
        visible_routes(&dismissed, source, room);
        same_controls(&dismissed, &before, &[source, other, room]);

        // These are the fader coordinates captured before either notice or send.
        let x = fader[0].as_f64().unwrap();
        let y = fader[1].as_f64().unwrap();
        let adjusted = app.command(json!({"command":"pointer", "phase":"drag",
            "x":x, "y":y, "to_x":x, "to_y":y+35.0}));
        assert!(
            track(&adjusted, source)["gain"].as_f64().unwrap()
                < track(&sent, source)["gain"].as_f64().unwrap()
        );
        assert_eq!(track(&adjusted, other), track(&sent, other));
        assert_eq!(menu(&mut app, "Edit", "Undo")["project"], sent["project"]);
        let restored_send = settled_source(&mut app, source);
        visible_routes(&restored_send, source, room);
        same_controls(&restored_send, &before, &[source, other, room]);
        assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
        same_controls(
            &settled_source(&mut app, source),
            &before,
            &[source, other, room],
        );
        app.quit();
    }
}

fn open_source_send(app: &mut Harness, source: u64) -> Value {
    let state = settled_source(app, source);
    click_rect(app, &channel(&state, source)["sends"])
}

fn assert_bus_choices(state: &Value) {
    let labels = state["menu_label_rects"].as_array().unwrap();
    for name in ["Bus 1", "Bus 2", "New bus and send"] {
        assert!(
            labels.iter().any(|label| label["text"] == name),
            "Missing distinct choice {name}"
        );
    }
    assert!(!labels.iter().any(|label| label["text"] == "New bus"));
}

#[test]
fn default_buses_are_numbered_and_destination_choices_stay_distinct_from_creation() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    open_project(&mut app, &project, "numbered-buses.rill");
    let first = menu(&mut app, "Track", "Add bus");
    assert_eq!(tracks(&first)[1]["name"], "Bus 1");
    let second = menu(&mut app, "Track", "Add bus");
    assert_eq!(tracks(&second)[2]["name"], "Bus 2");
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    app.click("Mixer");
    let before = settled_source(&mut app, source);
    let choices = open_source_send(&mut app, source);
    unchanged(&choices, &before);
    assert_bus_choices(&choices);
    let routed = app.click("Bus 1");
    assert_eq!(tracks(&routed).len(), 3);
    assert_eq!(
        track(&routed, source)["sends"][0]["target_bus"],
        tracks(&first)[1]["id"]
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    open_source_send(&mut app, source);
    let created = app.click("New bus and send");
    assert_eq!(tracks(&created)[3]["name"], "Bus 3");
    assert_eq!(
        track(&created, source)["sends"][0]["target_bus"],
        tracks(&created)[3]["id"]
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    app.quit();
}

#[test]
fn numbered_bus_creation_skips_existing_custom_names_and_undo_keeps_them() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_midi_track("Lead");
    let first = project.add_bus("Bus 1");
    project.track_mut(first).unwrap().pan = -0.2;
    let second = project.add_bus("Bus 2");
    project.track_mut(second).unwrap().gain = 0.4;
    open_project(&mut app, &project, "custom-numbered-buses.rill");
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    app.click("Mixer");
    let before = settled_source(&mut app, source);
    assert_bus_choices(&open_source_send(&mut app, source));
    let created = app.click("New bus and send");
    assert_eq!(tracks(&created)[3]["name"], "Bus 3");
    assert_eq!(track(&created, first), track(&before, first));
    assert_eq!(track(&created, second), track(&before, second));
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    let added = app.click("Add bus");
    assert_eq!(tracks(&added)[3]["name"], "Bus 3");
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    app.quit();
}
