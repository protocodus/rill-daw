//! Track and mixer ordering is a mouse operation on stable song identities.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationTarget, Clip, Effect, EffectSlot, Note, Project};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
enum View {
    Arrangement,
    Mixer,
}

impl View {
    fn key(self) -> &'static str {
        match self {
            Self::Arrangement => "timeline",
            Self::Mixer => "mixer",
        }
    }
    fn items(self) -> &'static str {
        match self {
            Self::Arrangement => "tracks",
            Self::Mixer => "channels",
        }
    }
    fn source(self) -> &'static str {
        match self {
            Self::Arrangement => "reorder_grip",
            Self::Mixer => "reorder_source",
        }
    }
    fn target(self) -> &'static str {
        match self {
            Self::Arrangement => "reorder_target",
            Self::Mixer => "drop",
        }
    }
}

fn item(state: &Value, view: View, id: u64) -> &Value {
    state[view.key()][view.items()]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["track"] == id)
        .unwrap_or_else(|| panic!("Missing track {id}: {}", state[view.key()]))
}

fn point(rect: &Value, x: f64, y: f64) -> (f64, f64) {
    let at = |axis: usize, fraction: f64| {
        let min = rect["min"][axis].as_f64().unwrap();
        min + (rect["max"][axis].as_f64().unwrap() - min) * fraction
    };
    (at(0, x), at(1, y))
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1}))
}

fn document_unchanged(before: &Value, after: &Value) {
    for field in [
        "project",
        "history",
        "revision",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            before[field], after[field],
            "A pending/cancelled drag changed {field}"
        );
    }
}

fn ids(state: &Value) -> Vec<u64> {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_u64().unwrap())
        .collect()
}

fn tracks(state: &Value) -> BTreeMap<u64, Value> {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| (track["id"].as_u64().unwrap(), track.clone()))
        .collect()
}

fn visible_order(state: &Value, view: View) -> Vec<u64> {
    state[view.key()][view.items()]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["track"].as_u64().unwrap())
        .collect()
}

fn channel_order(state: &Value) -> Vec<u64> {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| matches!(track["kind"].as_str(), Some("Audio" | "Midi" | "Bus")))
        .map(|track| track["id"].as_u64().unwrap())
        .collect()
}

fn menu(app: &mut Harness, parent: &str, child: &str) -> Value {
    app.click(parent);
    app.click(child)
}

fn start_drag(
    app: &mut Harness,
    view: View,
    source: u64,
    target: u64,
    placement: &str,
) -> (Value, Value, (f64, f64)) {
    let before = app.inspect();
    let from = point(&item(&before, view, source)[view.source()], 0.5, 0.5);
    let fraction = match placement {
        "Before" => 0.15,
        "After" => 0.85,
        "IntoFolder" => 0.5,
        _ => unreachable!(),
    };
    let to = match view {
        View::Arrangement => point(&item(&before, view, target)[view.target()], 0.55, fraction),
        View::Mixer => point(&item(&before, view, target)[view.target()], fraction, 0.2),
    };
    pointer(app, "down", from);
    pointer(app, "move", (from.0 + 9.0, from.1 + 9.0));
    let pending = pointer(app, "move", to);
    document_unchanged(&before, &pending);
    (before, pending, to)
}

fn drag(
    app: &mut Harness,
    view: View,
    source: u64,
    target: u64,
    placement: &str,
) -> (Value, Value) {
    let (before, pending, to) = start_drag(app, view, source, target, placement);
    let preview = &pending[view.key()]["reorder_preview"];
    assert_eq!(
        preview["source"], source,
        "Missing reorder preview: {pending}"
    );
    assert_eq!(preview["target"], target);
    assert_eq!(preview["placement"], placement);
    pointer(app, "up", to);
    // Commit occurs at the end of the release frame; inspect its newly laid-out
    // rows rather than diagnostics painted just before the commit.
    let after = app.inspect();
    assert_eq!(after["error"], false, "{after}");
    assert_eq!(after["selected_track"], before["selected_track"]);
    assert_eq!(after["selected_clip"], before["selected_clip"]);
    (before, after)
}

fn undo_redo(app: &mut Harness, before: &Value, after: &Value) -> Value {
    assert_eq!(
        after["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1,
        "A drop is one undoable edit"
    );
    let undone = menu(app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_eq!(undone["selected_track"], before["selected_track"]);
    let redone = menu(app, "Edit", "Redo");
    assert_eq!(redone["project"], after["project"]);
    assert_eq!(redone["selected_track"], after["selected_track"]);
    redone
}

fn fixture() -> (Project, [u64; 5]) {
    let mut project = Project::new();
    let audio = project.add_audio_track("Drums");
    let midi = project.add_midi_track("Keys");
    let bus = project.add_bus("Room");
    let folder = project.add_folder("Group");
    let automation = project
        .add_automation_track(
            "Volume envelope",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 2.0, 88)],
    ));
    let slot = EffectSlot::new(
        "Saved insert",
        Effect::Vst3 {
            path: "/unavailable/reorder-fixture.vst3".into(),
            class_id: None,
        },
    );
    project.track_mut(audio).unwrap().inserts.push(slot);
    project.track_mut(audio).unwrap().inserts[0].state = vec![1, 3, 5, 7];
    project.set_output(audio, Some(bus)).unwrap();
    project.add_send(midi, bus, 0.4, true).unwrap();
    project.automation_lane_mut(automation).unwrap().points[0].curve = 0.6;
    project
        .set_automation_override(automation, Some(0.4))
        .unwrap();
    (project, [audio, midi, bus, folder, automation])
}

fn open(app: &mut Harness, project: &Project, dark: bool, density: f64) -> Value {
    let path = app.directory.path().join("track-order.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"system_theme","dark":dark}));
    app.command(json!({"command":"viewport","width":1400,"height":1100,"pixels_per_point":density}))
}

#[test]
fn every_arranger_track_kind_reorders_by_mouse_with_one_history_entry_and_portable_recall() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (project, [audio, midi, bus, folder, automation]) = fixture();
        open(&mut app, &project, dark, density);
        let initial = app.click("Phrase");
        assert_eq!(initial["selected_track"], midi);
        assert_eq!(
            initial["selected_clip"],
            project.track(midi).unwrap().clips[0].id
        );
        let content = tracks(&initial);
        let mut expected = vec![audio, midi, bus, folder, automation];
        for (source, target, placement) in [
            (audio, midi, "After"),
            (automation, audio, "Before"),
            (folder, midi, "Before"),
            (bus, folder, "Before"),
            (midi, audio, "After"),
        ] {
            let (before, after) = drag(&mut app, View::Arrangement, source, target, placement);
            expected.retain(|id| *id != source);
            let index = expected.iter().position(|id| *id == target).unwrap()
                + usize::from(placement == "After");
            expected.insert(index, source);
            assert_eq!(ids(&after), expected);
            assert_eq!(visible_order(&after, View::Arrangement), expected);
            assert_eq!(tracks(&after), content, "Only order may change");
            undo_redo(&mut app, &before, &after);
        }

        // Escape and dropping onto oneself must not create history or change selection.
        let (before, _, to) = start_drag(&mut app, View::Arrangement, audio, bus, "Before");
        app.key("Escape");
        let cancelled = pointer(&mut app, "up", to);
        document_unchanged(&before, &cancelled);
        let (before, _, to) = start_drag(&mut app, View::Arrangement, audio, audio, "Before");
        let same = pointer(&mut app, "up", to);
        document_unchanged(&before, &same);

        let final_state = app.click("Mixer");
        assert_eq!(
            visible_order(&final_state, View::Mixer),
            channel_order(&final_state)
        );
        let bundle = app.directory.path().join("reordered.rill");
        app.command(json!({"command":"save","path":bundle}));
        app = app.restart();
        let recalled = app.command(json!({"command":"open","path":bundle}));
        assert_eq!(recalled["project"], final_state["project"]);
        assert_eq!(ids(&recalled), expected);
        app.quit();
    }
}

#[test]
fn folder_drag_moves_hidden_descendants_and_reparenting_rejects_cycles() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let first = project.add_audio_track("Outside");
    let folder = project.add_folder("Ensemble");
    let child = project.add_audio_track("Inside");
    let nested = project.add_folder("Nested");
    let midi = project.add_midi_track("Nested keys");
    let lane = project
        .add_automation_track(
            "Inside pan",
            AutomationTarget {
                track: child,
                parameter: AutomationParameter::TrackPan,
            },
        )
        .unwrap();
    let bus = project.add_bus("Return");
    for (track, parent) in [
        (child, folder),
        (nested, folder),
        (midi, nested),
        (lane, folder),
    ] {
        project.set_parent_folder(track, Some(parent)).unwrap();
    }
    project.track_mut(folder).unwrap().collapsed = true;
    project.set_output(first, Some(bus)).unwrap();
    let initial = open(&mut app, &project, true, 1.0);
    assert_eq!(
        visible_order(&initial, View::Arrangement),
        vec![first, folder, bus]
    );
    let (before, after) = drag(&mut app, View::Arrangement, folder, bus, "After");
    assert_eq!(
        ids(&after),
        vec![first, bus, folder, child, nested, midi, lane]
    );
    assert_eq!(tracks(&after), tracks(&before));
    undo_redo(&mut app, &before, &after);
    menu(&mut app, "Edit", "Undo");
    let expanded = app.click("Ensemble");
    assert!(
        !expanded["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == folder)
            .unwrap()["collapsed"]
            .as_bool()
            .unwrap()
    );
    // Expanded rows can exceed the viewport at the comfortable default height.
    // Fit exposes both drag endpoints before testing the hierarchy operation.
    app.click("Fit");

    let (before, pending, to) =
        start_drag(&mut app, View::Arrangement, folder, nested, "IntoFolder");
    assert!(
        pending["timeline"]["reorder_preview"].is_null(),
        "A folder cannot be dropped inside its descendant"
    );
    let invalid = pointer(&mut app, "up", to);
    document_unchanged(&before, &invalid);

    let (before, after) = drag(&mut app, View::Arrangement, bus, nested, "IntoFolder");
    assert_eq!(
        ids(&after),
        vec![first, folder, child, nested, midi, bus, lane]
    );
    let mut expected_content = tracks(&before);
    expected_content.get_mut(&bus).unwrap()["parent_folder"] = json!(nested);
    assert_eq!(tracks(&after), expected_content);
    undo_redo(&mut app, &before, &after);
    let mixer = app.click("Mixer");
    assert_eq!(
        visible_order(&mixer, View::Mixer),
        vec![first, child, midi, bus]
    );
    app.quit();
}

#[test]
fn mixer_channels_cross_bus_boundaries_and_share_arranger_order_without_changing_sound_data() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (mut project, [audio, midi, bus, _, _]) = fixture();
        let last = project.add_audio_track("Last channel");
        open(&mut app, &project, dark, density);
        let mixer = app.click("Mixer");
        let content = tracks(&mixer);
        assert_eq!(
            visible_order(&mixer, View::Mixer),
            vec![audio, midi, bus, last]
        );
        for (source, target, placement, order) in [
            (bus, audio, "Before", vec![bus, audio, midi, last]),
            (last, bus, "After", vec![bus, last, audio, midi]),
            (midi, bus, "Before", vec![midi, bus, last, audio]),
        ] {
            let (before, after) = drag(&mut app, View::Mixer, source, target, placement);
            assert_eq!(visible_order(&after, View::Mixer), order);
            assert_eq!(channel_order(&after), order);
            assert_eq!(tracks(&after), content);
            let redone = undo_redo(&mut app, &before, &after);
            assert_eq!(visible_order(&redone, View::Mixer), order);
            let arranger = app.click("Arranger");
            assert_eq!(visible_order(&arranger, View::Arrangement), ids(&arranger));
            app.click("Mixer");
        }
        let (before, _, to) = start_drag(&mut app, View::Mixer, audio, midi, "Before");
        app.key("Escape");
        document_unchanged(&before, &pointer(&mut app, "up", to));
        app.quit();
    }
}

#[test]
fn compact_track_titles_and_mixer_dragging_scroll_to_offscreen_destinations() {
    for view in [View::Arrangement, View::Mixer] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let order: Vec<_> = (1..=10)
            .map(|index| {
                let id = project.add_audio_track(format!("Channel {index:02}"));
                project.track_mut(id).unwrap().collapsed = true;
                id
            })
            .collect();
        open(&mut app, &project, false, 1.0);
        app.command(json!({"command":"viewport","width":800,"height":600}));
        // A normal title click still selects a track before the same title is
        // used as a drag source. The subsequent drag must preserve that owner.
        let selected = app.click("Channel 02");
        assert_eq!(selected["selected_track"], order[1]);
        if matches!(view, View::Mixer) {
            app.click("Mixer");
        }
        let before = app.inspect();
        let source = order[0];
        let last = *order.last().unwrap();
        let from = point(&item(&before, view, source)["name"], 0.5, 0.5);
        pointer(&mut app, "down", from);
        pointer(&mut app, "move", (from.0 + 9.0, from.1 + 9.0));
        let viewport = match view {
            View::Arrangement => &before["timeline"]["canvas"],
            View::Mixer => &before["mixer"]["reorder_viewport"],
        };
        let edge = match view {
            View::Arrangement => (140.0, viewport["max"][1].as_f64().unwrap() - 3.0),
            View::Mixer => (
                viewport["max"][0].as_f64().unwrap() - 3.0,
                point(viewport, 0.5, 0.3).1,
            ),
        };
        pointer(&mut app, "move", edge);
        let mut scrolled = app.inspect();
        let axis = usize::from(matches!(view, View::Arrangement));
        let bound = viewport["max"][axis].as_f64().unwrap() - 4.0;
        let mut destination = None;
        for _ in 0..140 {
            document_unchanged(&before, &scrolled);
            destination = scrolled[view.key()][view.items()]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["track"] == last)
                .map(|item| &item[view.target()])
                .filter(|rect| {
                    rect["min"][axis].as_f64().unwrap()
                        + (rect["max"][axis].as_f64().unwrap()
                            - rect["min"][axis].as_f64().unwrap())
                            * 0.85
                        < bound
                })
                .cloned();
            if destination.is_some() {
                break;
            }
            scrolled = app.inspect();
        }
        let destination = destination.unwrap_or_else(|| {
            panic!(
                "Edge dragging did not reveal the final track: {}",
                scrolled[view.key()]
            )
        });
        let to = match view {
            View::Arrangement => point(&destination, 0.55, 0.85),
            View::Mixer => point(&destination, 0.85, 0.2),
        };
        let preview = pointer(&mut app, "move", to);
        assert_eq!(preview[view.key()]["reorder_preview"]["target"], last);
        assert_eq!(preview[view.key()]["reorder_preview"]["placement"], "After");
        pointer(&mut app, "up", to);
        let after = app.inspect();
        let mut expected = order[1..].to_vec();
        expected.push(source);
        assert_eq!(ids(&after), expected);
        assert_eq!(tracks(&before), tracks(&after));
        undo_redo(&mut app, &before, &after);
        app.quit();
    }
}
