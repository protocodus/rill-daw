//! Phrase editing through pointer, menus, native clipboard and keyboard input.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Note, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let source = app.directory.path().join("Original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..192_000)
            .flat_map(|frame| {
                let sample = (frame as f32 * std::f32::consts::TAU / 81.0).sin() * 0.1;
                [sample, sample * 0.7]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    for (name, midi) in [
        ("Audio A", false),
        ("MIDI B", true),
        ("Audio C", false),
        ("MIDI D", true),
    ] {
        let track = if midi {
            project.add_midi_track(name)
        } else {
            project.add_audio_track(name)
        };
        for (suffix, start) in [("one", 0.0), ("two", 2.0)] {
            let mut clip = if midi {
                Clip::midi(
                    format!("{name} {suffix}"),
                    start,
                    0.5,
                    vec![Note::new(60, 0.0, 0.5, 90)],
                )
            } else {
                Clip::audio(
                    format!("{name} {suffix}"),
                    start,
                    1.0,
                    source.to_string_lossy(),
                )
            };
            if !midi {
                clip.offset = 2.0;
            }
            project.track_mut(track).unwrap().clips.push(clip);
        }
    }
    // Keep the source palette outside the phrase and give its rests visible room.
    let track = project.tracks[0].id;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Palette",
        7.0,
        1.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("Phrase.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1300,"height":1000}));
    app.click("Fit");
    app
}
fn rect_center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn clip_rect(state: &Value, id: u64) -> &Value {
    &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"]
}
fn click_clip(app: &mut Harness, id: u64, shift: bool) -> Value {
    let state = app.inspect();
    let p = rect_center(clip_rect(&state, id));
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1,"shift":shift}))
}
fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}
fn select_only(app: &mut Harness, id: u64) -> Value {
    let state = app.inspect();
    let p = rect_center(clip_rect(&state, id));
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":p.0,"y":p.1}));
    app.click("Select only this clip")
}
fn selected(state: &Value) -> Vec<u64> {
    state["timeline"]["selected_clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_u64().unwrap())
        .collect()
}
fn chosen(app: &mut Harness) -> (Value, u64, u64) {
    let state = app.inspect();
    let a = state["project"]["tracks"][0]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    let b = state["project"]["tracks"][1]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    select_only(app, a);
    let state = click_clip(app, b, true);
    (state, a, b)
}
fn time_point(state: &Value, beat: f64, track: usize, fraction: f64) -> (f64, f64) {
    let timeline = &state["timeline"];
    let row = &timeline["tracks"][track]["header"];
    (
        timeline["canvas"]["min"][0].as_f64().unwrap()
            + (beat - timeline["start_beat"].as_f64().unwrap())
                * timeline["pixels_per_beat"].as_f64().unwrap(),
        row["min"][1].as_f64().unwrap()
            + (row["max"][1].as_f64().unwrap() - row["min"][1].as_f64().unwrap()) * fraction,
    )
}
fn phrase(app: &mut Harness, start: f64, end: f64) -> Value {
    app.click("Cursor");
    let state = app.inspect();
    let a = time_point(&state, start, 0, 0.03);
    let b = time_point(&state, end, 1, 0.97);
    app.command(json!({"command":"pointer","phase":"drag","x":a.0,"y":a.1,"to_x":b.0,"to_y":b.1}))
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

#[test]
fn noncontiguous_selection_and_group_duplicate_are_mouse_accessible_and_one_undo() {
    let mut app = fixture();
    let before = app.inspect();
    let a = before["project"]["tracks"][0]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    let b = before["project"]["tracks"][1]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    click_clip(&mut app, a, false);
    let state = app.inspect();
    let p = rect_center(clip_rect(&state, b));
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":p.0,"y":p.1}));
    let added = app.click("Add to selection");
    assert_eq!(selected(&added), vec![a, b]);
    assert_eq!(added["project"], before["project"]);
    assert_eq!(added["history"], before["history"]);
    let duplicated = menu(&mut app, "Edit", "Duplicate");
    assert_eq!(selected(&duplicated).len(), 2);
    assert_eq!(
        duplicated["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        duplicated["project"]["tracks"][1]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let restored = menu(&mut app, "Edit", "Undo");
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(selected(&restored), vec![a, b]);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], duplicated["project"]);
    let id = selected(&redone)[0];
    let p = rect_center(clip_rect(&redone, id));
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":p.0,"y":p.1}));
    let removed = app.click("Remove from selection");
    assert_eq!(selected(&removed).len(), 1);
    app.quit();
}

#[test]
fn time_block_duplicate_preserves_rests_and_clips_partial_source_windows() {
    let mut app = fixture();
    let before = app.inspect();
    let selected_phrase = phrase(&mut app, 0.0, 4.0);
    assert_eq!(
        selected_phrase["timeline"]["selection_span"],
        json!([0.0, 4.0])
    );
    assert_eq!(selected(&selected_phrase).len(), 4);
    assert_eq!(selected_phrase["project"], before["project"]);
    let copied = app.command(json!({"command":"key","key":"D","cmd":true}));
    assert_eq!(copied["timeline"]["selection_span"], json!([4.0, 8.0]));
    for track in [0, 1] {
        let starts = copied["project"]["tracks"][track]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|clip| selected(&copied).contains(&clip["id"].as_u64().unwrap()))
            .map(|clip| clip["start"].as_f64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(starts, vec![4.0, 6.0]);
    }
    let repeated = app.command(json!({"command":"key","key":"D","cmd":true}));
    assert_eq!(repeated["timeline"]["selection_span"], json!([8.0, 12.0]));
    assert_eq!(undo(&mut app, false)["project"], copied["project"]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.click("Fit");
    let partial = phrase(&mut app, 0.25, 2.5);
    assert_eq!(partial["timeline"]["selection_span"], json!([0.25, 2.5]));
    let duplicate = menu(&mut app, "Edit", "Duplicate");
    let ids = selected(&duplicate);
    let audio = duplicate["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|clip| ids.contains(&clip["id"].as_u64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(audio.len(), 2);
    assert_eq!(audio[0]["start"], 2.5);
    assert_eq!(audio[0]["length"], 0.75);
    assert_eq!(audio[0]["offset"], 2.25);
    assert_eq!(audio[1]["length"], 0.5);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn marquee_group_move_shift_copy_and_arrow_selection_preserve_relative_placement() {
    let mut app = fixture();
    let before = app.inspect();
    let a = time_point(&before, 1.5, 0, 0.02);
    let b = time_point(&before, 0.0, 1, 0.98);
    let marked = app
        .command(json!({"command":"pointer","phase":"drag","x":a.0,"y":a.1,"to_x":b.0,"to_y":b.1}));
    assert_eq!(selected(&marked).len(), 2);
    assert_eq!(marked["project"], before["project"]);
    let first = selected(&marked)[0];
    let p = rect_center(clip_rect(&marked, first));
    let scale = marked["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let moved = app.command(
        json!({"command":"pointer","phase":"drag","x":p.0,"y":p.1,"to_x":p.0+scale,"to_y":p.1}),
    );
    assert_eq!(moved["project"]["tracks"][0]["clips"][0]["start"], 1.0);
    assert_eq!(moved["project"]["tracks"][1]["clips"][0]["start"], 1.0);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let (marked, _, _) = chosen(&mut app);
    let p = rect_center(clip_rect(&marked, first));
    let copied=app.command(json!({"command":"pointer","phase":"drag","shift":true,"x":p.0,"y":p.1,"to_x":p.0+scale*4.0,"to_y":p.1}));
    assert_eq!(selected(&copied).len(), 2);
    assert_eq!(
        copied["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(copied["project"]["tracks"][0]["clips"][0]["start"], 0.0);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    click_clip(&mut app, first, false);
    let selection_only = app.key("ArrowRight");
    assert_eq!(selection_only["project"], before["project"]);
    assert_ne!(selection_only["selected_clip"], first);
    app.quit();
}

#[test]
fn phrase_cut_keeps_outside_material_and_original_sources_and_undo_restores_selection() {
    let mut app = fixture();
    let before = app.inspect();
    let original = std::fs::read(app.directory.path().join("Original.wav")).unwrap();
    let selected_phrase = phrase(&mut app, 0.25, 0.75);
    let cut = app.command(json!({"command":"clipboard","action":"cut"}));
    assert_eq!(cut["project"]["tracks"][0]["clips"][0]["length"], 0.25);
    let project: Project = serde_json::from_value(cut["project"].clone()).unwrap();
    project.validate().unwrap();
    let source = project.tracks[0]
        .clips
        .iter()
        .find(|clip| clip.start == 0.75)
        .unwrap();
    assert_eq!(source.offset, 2.75);
    assert_eq!(
        std::fs::read(app.directory.path().join("Original.wav")).unwrap(),
        original
    );
    let restored = undo(&mut app, false);
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(selected(&restored), selected(&selected_phrase));
    assert_eq!(
        restored["timeline"]["selection_span"],
        selected_phrase["timeline"]["selection_span"]
    );
    app.quit();
}

#[test]
fn arranger_slip_and_fade_handles_edit_sources_non_destructively_and_fit_selection_recalls_view() {
    let mut app = fixture();
    let before = app.inspect();
    let id = before["project"]["tracks"][0]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    let selected = click_clip(&mut app, id, false);
    app.click("View");
    let fitted = app.click("Fit selection");
    assert_eq!(fitted["project"], before["project"]);
    assert_eq!(fitted["history"], before["history"]);
    assert!(
        fitted["timeline"]["pixels_per_beat"].as_f64().unwrap()
            > selected["timeline"]["pixels_per_beat"].as_f64().unwrap()
    );
    app.click("View");
    let recalled = app.click("Previous view");
    for field in ["start_beat", "pixels_per_beat", "row_offset", "row_height"] {
        assert_eq!(
            recalled["timeline"][field], selected["timeline"][field],
            "Previous view {field}"
        );
    }
    app.click("Slip");
    let view = app.inspect();
    let p = rect_center(clip_rect(&view, id));
    let scale = view["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let slipped = app.command(
        json!({"command":"pointer","phase":"drag","x":p.0,"y":p.1,"to_x":p.0+scale*0.5,"to_y":p.1}),
    );
    let clip = &slipped["project"]["tracks"][0]["clips"][0];
    assert_eq!(clip["start"], 0.0);
    assert_eq!(clip["length"], 1.0);
    assert_eq!(clip["offset"], 1.5);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.click("Select");
    let view = app.inspect();
    let handle = &view["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["fade_out_handle"];
    let p = rect_center(handle);
    let faded=app.command(json!({"command":"pointer","phase":"drag","x":p.0,"y":p.1,"to_x":p.0-scale*0.25,"to_y":p.1}));
    let project: Project = serde_json::from_value(faded["project"].clone()).unwrap();
    let ClipContent::Audio { fade_out, .. } = project.tracks[0].clips[0].content else {
        panic!("audio")
    };
    assert!(
        fade_out > 0.15 && fade_out < 0.4,
        "fadehandle changed duration: {fade_out}"
    );
    assert_eq!(project.tracks[0].clips[0].start, 0.0);
    assert_eq!(project.tracks[0].clips[0].length, 1.0);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let view = app.inspect();
    let handle = &view["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["fade_out_handle"];
    let p = rect_center(handle);
    let panned = app.command(json!({"command":"pointer","phase":"drag","button":"middle","x":p.0,"y":p.1,"to_x":p.0-20.0,"to_y":p.1+8.0}));
    assert_eq!(
        panned["project"], view["project"],
        "Panning across a fade handle never edits audio"
    );
    assert_eq!(panned["history"], view["history"]);
    app.quit();
}

#[test]
fn compatible_group_track_transfer_and_retained_take_sources_survive_duplicate_and_delete() {
    let mut app = fixture();
    let before = app.inspect();
    let (chosen_state, a, _) = chosen(&mut app);
    let p = rect_center(clip_rect(&chosen_state, a));
    let destination = time_point(&chosen_state, 0.5, 2, 0.5);
    let moved = app.command(
        json!({"command":"pointer","phase":"drag","x":p.0,"y":p.1,"to_x":p.0,"to_y":destination.1}),
    );
    assert!(
        moved["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|clip| clip["id"] != a)
    );
    assert!(
        moved["project"]["tracks"][2]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|clip| clip["id"] == a)
    );
    assert_eq!(selected(&moved).len(), 2);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let mut project: Project = serde_json::from_value(before["project"].clone()).unwrap();
    let track = project.tracks[0].id;
    let source = project.tracks[0].clips[0].id;
    let alternate = project.tracks[0].clips[1].id;
    let group = project
        .group_audio_takes(track, &[source, alternate])
        .unwrap();
    let retained = project.tracks[0]
        .clips
        .iter()
        .filter(|clip| clip.is_take_source())
        .cloned()
        .collect::<Vec<_>>();
    let comp = project.audio_take_regions(track, group)[0].id;
    let path = app.directory.path().join("Takes.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Fit");
    click_clip(&mut app, comp, false);
    let duplicated = menu(&mut app, "Edit", "Duplicate");
    let duplicate = selected(&duplicated)[0];
    assert!(
        duplicated["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == duplicate)
            .unwrap()["audio_take"]
            .is_null()
    );
    menu(&mut app, "Edit", "Delete");
    let after: Project = serde_json::from_value(app.inspect()["project"].clone()).unwrap();
    after.validate().unwrap();
    for original in retained {
        assert_eq!(after.clip(track, original.id), Some(&original));
    }
    app.quit();
}

#[test]
fn leading_rests_stop_at_zero_and_undo_abandons_a_held_audio_gesture() {
    let mut app = fixture();
    phrase(&mut app, 1.0, 4.0);
    for _ in 0..8 {
        app.command(json!({"command":"key","key":"ArrowLeft","cmd":true}));
    }
    let limited = app.inspect();
    assert_eq!(limited["timeline"]["selection_span"], json!([0.0, 3.0]));
    let project: Project = serde_json::from_value(limited["project"].clone()).unwrap();
    project.validate().unwrap();
    let id = project.tracks[0].clips[0].id;
    app.click("Select");
    click_clip(&mut app, id, false);
    for slip in [true, false] {
        app.click(if slip { "Slip" } else { "Select" });
        let before = app.inspect();
        let p = if slip {
            rect_center(clip_rect(&before, id))
        } else {
            let clip = before["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|clip| clip["id"] == id)
                .unwrap();
            rect_center(&clip["fade_out_handle"])
        };
        let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
        let direction = if slip { 1.0 } else { -1.0 };
        app.command(json!({"command":"pointer","phase":"down","x":p.0,"y":p.1}));
        let edited = app.command(
            json!({"command":"pointer","phase":"move","x":p.0+direction*scale*0.25,"y":p.1}),
        );
        assert_ne!(
            edited["project"], before["project"],
            "The held gesture really edited audio"
        );
        let restored = undo(&mut app, false);
        assert_eq!(restored["project"], before["project"]);
        let continued = app.command(
            json!({"command":"pointer","phase":"move","x":p.0+direction*scale*0.5,"y":p.1}),
        );
        assert_eq!(
            continued["project"], before["project"],
            "Undo abandons the held gesture"
        );
        app.command(json!({"command":"pointer","phase":"up","x":p.0+direction*scale*0.5,"y":p.1}));
    }
    app.quit();
}

#[test]
fn nested_folder_selection_fit_accounts_for_fixed_closing_strips_at_each_density() {
    let mut app = fixture();
    let old: Project = serde_json::from_value(app.inspect()["project"].clone()).unwrap();
    let ClipContent::Audio { path: source, .. } = &old.tracks[0].clips[0].content else {
        panic!("audio")
    };
    let mut project = Project::new();
    let outer = project.add_folder("Outer");
    let inner = project.add_folder("Inner");
    project.set_parent_folder(inner, Some(outer)).unwrap();
    let first = project.add_audio_track("Nested track");
    project.set_parent_folder(first, Some(inner)).unwrap();
    let first_clip = Clip::audio("Nested cue", 4.0, 1.5, source);
    let first_id = first_clip.id;
    project.track_mut(first).unwrap().clips.push(first_clip);
    let last = project.add_audio_track("Root track");
    let last_clip = Clip::audio("Root cue", 4.5, 1.5, source);
    let last_id = last_clip.id;
    project.track_mut(last).unwrap().clips.push(last_clip);
    for index in 0..12 {
        project.add_audio_track(format!("Empty {index}"));
    }
    let path = app.directory.path().join("Nested fit.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    for (width, height, density) in [(800.0, 800.0, 1.0), (1300.0, 1000.0, 2.0)] {
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        let before = app.click("Fit");
        select_only(&mut app, first_id);
        click_clip(&mut app, last_id, true);
        app.click("View");
        let fitted = app.click("Fit selection");
        let canvas = &fitted["timeline"]["canvas"];
        let headers = fitted["timeline"]["tracks"].as_array().unwrap();
        let first_row = &headers.iter().find(|row| row["track"] == first).unwrap()["header"];
        let last_row = &headers.iter().find(|row| row["track"] == last).unwrap()["header"];
        let gap_top = first_row["min"][1].as_f64().unwrap() - canvas["min"][1].as_f64().unwrap();
        let gap_bottom = canvas["max"][1].as_f64().unwrap() - last_row["max"][1].as_f64().unwrap();
        assert!(
            (-0.1..=16.0).contains(&gap_top),
            "Selected top lane must be visible with its fit padding: {gap_top}"
        );
        assert!(
            (-0.1..=16.0).contains(&gap_bottom),
            "Fixed folder strips must not leave an oversized fit gap: {gap_bottom}"
        );
        assert_eq!(fitted["project"], before["project"]);
        assert_eq!(fitted["history"], before["history"]);
        app.click("View");
        let recalled = app.click("Previous view");
        assert!(
            (recalled["timeline"]["row_height"].as_f64().unwrap()
                - before["timeline"]["row_height"].as_f64().unwrap())
            .abs()
                < 0.01
        );
    }
    app.quit();
}

#[test]
fn native_mixed_clipboard_pastes_once_and_a_fresh_editor_copy_replaces_it() {
    let mut app = fixture();
    let (before, audio_id, _) = chosen(&mut app);
    let bytes = std::fs::read(app.directory.path().join("Original.wav")).unwrap();
    let copied = app.command(json!({"command":"clipboard","action":"copy","with_key":true}));
    assert_eq!(copied["project"], before["project"]);
    assert_eq!(copied["history"], before["history"]);
    app.click("Cursor");
    let state = app.inspect();
    let p = time_point(&state, 4.0, 0, 0.65);
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1}));
    let pasted = app.command(json!({"command":"clipboard","action":"paste","with_key":true}));
    assert_eq!(selected(&pasted).len(), 2);
    assert_eq!(
        pasted["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        pasted["project"]["tracks"][1]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let selected_ids = selected(&pasted);
    for track in [0, 1] {
        let additions = pasted["project"]["tracks"][track]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|clip| selected_ids.contains(&clip["id"].as_u64().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            additions.len(),
            1,
            "Native event plus key must create only one copy on each track"
        );
        assert_eq!(additions[0]["start"], 4.0);
    }
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.click("Select");
    select_only(&mut app, audio_id);
    menu(&mut app, "Window", "Clip Editor");
    let fresh = app.command(json!({"command":"clipboard","action":"copy","with_key":true}));
    assert_eq!(fresh["view"], "Clip");
    assert_eq!(fresh["project"], before["project"]);
    app.click("Arranger");
    app.click("Cursor");
    let state = app.inspect();
    let p = time_point(&state, 5.0, 0, 0.65);
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1}));
    let pasted_audio = app.command(json!({"command":"clipboard","action":"paste","with_key":true}));
    assert_eq!(
        pasted_audio["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        pasted_audio["project"]["tracks"][1]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "A fresh audio editor copy invalidates the older mixed selection clipboard"
    );
    let project: Project = serde_json::from_value(pasted_audio["project"].clone()).unwrap();
    project.validate().unwrap();
    assert!(project.tracks[0].clips.iter().any(|clip| clip.start == 5.0));
    assert_eq!(
        std::fs::read(app.directory.path().join("Original.wav")).unwrap(),
        bytes
    );
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn header_resize_stays_on_labels_and_a_near_zero_cursor_drag_selects_audio() {
    let mut app = fixture();
    let before = app.inspect();
    let width = before["timeline"]["header_width"].as_f64().unwrap();
    let gutter = &before["timeline"]["header_resize"];
    assert!(
        gutter["max"][0].as_f64().unwrap()
            < before["timeline"]["canvas"]["min"][0].as_f64().unwrap()
    );
    let x = rect_center(gutter).0;
    let y = before["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.0;
    let resized =
        app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+40.0,"to_y":y}));
    assert!((resized["timeline"]["header_width"].as_f64().unwrap() - width - 40.0).abs() < 0.01);
    assert_eq!(resized["project"], before["project"]);
    assert_eq!(resized["history"], before["history"]);
    app.click("Cursor");
    let view = app.inspect();
    let left = view["timeline"]["canvas"]["min"][0].as_f64().unwrap();
    let a = time_point(&view, 0.0, 0, 0.03);
    let b = time_point(&view, 4.0, 1, 0.97);
    let selected_phrase = app.command(
        json!({"command":"pointer","phase":"drag","x":left+2.0,"y":a.1,"to_x":b.0,"to_y":b.1}),
    );
    assert_eq!(
        selected_phrase["timeline"]["selection_span"],
        json!([0.0, 4.0])
    );
    assert_eq!(selected(&selected_phrase).len(), 4);
    assert_eq!(
        selected_phrase["timeline"]["header_width"], view["timeline"]["header_width"],
        "The first audio pixels belong to time selection, not label resizing"
    );
    assert_eq!(selected_phrase["project"], before["project"]);
    assert_eq!(selected_phrase["history"], before["history"]);
    app.quit();
}

#[test]
fn additive_marquee_keeps_offscreen_clips_and_empty_viewport_clicks_clear_or_seek() {
    let mut app = fixture();
    let before = app.inspect();
    let first = before["project"]["tracks"][0]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    let last_track = before["project"]["tracks"][3]["id"].as_u64().unwrap();
    let last = before["project"]["tracks"][3]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    select_only(&mut app, first);
    let view = app.command(json!({"command":"viewport","width":1300,"height":650}));
    let x = view["timeline"]["canvas"]["min"][0].as_f64().unwrap() + 200.0;
    let y = view["timeline"]["canvas"]["min"][1].as_f64().unwrap() + 40.0;
    let scrolled = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-1200.0}));
    assert!(
        !scrolled["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|clip| clip["id"] == first),
        "The earlier selection must really be offscreen"
    );
    let row = &scrolled["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["track"] == last_track)
        .unwrap()["header"];
    let left = scrolled["timeline"]["canvas"]["min"][0].as_f64().unwrap();
    let start = left
        + (1.5 - scrolled["timeline"]["start_beat"].as_f64().unwrap())
            * scrolled["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let top = row["min"][1].as_f64().unwrap() + 3.0;
    let bottom = row["max"][1].as_f64().unwrap() - 3.0;
    let marked=app.command(json!({"command":"pointer","phase":"drag","shift":true,"x":start,"y":top,"to_x":left+2.0,"to_y":bottom}));
    assert_eq!(selected(&marked), vec![first, last]);
    let blank_y = bottom + 23.0;
    assert!(blank_y < marked["timeline"]["canvas"]["max"][1].as_f64().unwrap());
    let cleared = app.command(json!({"command":"pointer","phase":"click","x":start,"y":blank_y}));
    assert!(selected(&cleared).is_empty());
    assert_eq!(cleared["project"], before["project"]);
    assert_eq!(cleared["history"], before["history"]);
    app.click("Cursor");
    let view = app.inspect();
    let row = &view["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["track"] == last_track)
        .unwrap()["header"];
    let x = view["timeline"]["canvas"]["min"][0].as_f64().unwrap()
        + (3.0 - view["timeline"]["start_beat"].as_f64().unwrap())
            * view["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let y = row["max"][1].as_f64().unwrap() + 20.0;
    let sought = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(sought["timeline"]["cursor"], 3.0);
    assert_eq!(sought["project"], before["project"]);
    assert_eq!(sought["history"], before["history"]);
    app.quit();
}
