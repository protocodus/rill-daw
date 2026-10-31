//! Arranger label sizing, separated loop range, and nested visual track groups.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let source = app.directory.path().join("Original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.05; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.loop_enabled = true;
    project.loop_start = 0.25;
    project.loop_end = 3.75;
    for name in ["Drums", "Bass"] {
        let id = project.add_audio_track(name);
        project.track_mut(id).unwrap().clips.push(Clip::audio(
            format!("{name} phrase"),
            0.0,
            4.0,
            source.to_string_lossy(),
        ));
    }
    let path = app.directory.path().join("Layout.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn track(state: &Value, id: u64) -> &Value {
    state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["track"] == id)
        .unwrap()
}
fn select(app: &mut Harness, id: u64) -> Value {
    let state = app.click("Fit");
    let (x, y) = center(&track(&state, id)["name"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}
fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "selected_track", "selected_clip"] {
        assert_eq!(state[field], before[field], "View sizing changed {field}");
    }
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn label_and_all_row_sizes_resize_without_edits_and_loop_range_has_its_own_lane() {
    let mut app = fixture();
    let audio = export(&mut app, "Before.wav");
    for (width, density, dark) in [(800., 1., false), (1400., 2., true)] {
        app.command(
            json!({"command":"viewport","width":width,"height":900,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let before = app.click("Fit");
        let old_width = before["timeline"]["header_width"].as_f64().unwrap();
        let r = &before["timeline"]["header_resize"];
        let x = (r["min"][0].as_f64().unwrap() + r["max"][0].as_f64().unwrap()) * 0.5;
        let y = before["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.;
        let resized = app
            .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+60.,"to_y":y}));
        assert!(
            (resized["timeline"]["header_width"].as_f64().unwrap() - old_width - 60.).abs() < 0.01
        );
        unchanged(&resized, &before);
        assert!(
            (resized["timeline"]["canvas"]["min"][0].as_f64().unwrap()
                - before["timeline"]["canvas"]["min"][0].as_f64().unwrap()
                - 60.)
                .abs()
                < 0.01,
            "Resized header and timeline origin must align: width {}, canvas {}, prior canvas {}",
            resized["timeline"]["header_width"],
            resized["timeline"]["canvas"],
            before["timeline"]["canvas"]
        );
        let old_height = resized["timeline"]["row_height"].as_f64().unwrap();
        let (x, y) = center(&resized["timeline"]["row_resize"][0]["rect"]);
        let resized = app
            .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x,"to_y":y+20.}));
        assert!(
            (resized["timeline"]["row_height"].as_f64().unwrap() - old_height - 20.).abs() < 0.01
        );
        unchanged(&resized, &before);
        for t in resized["timeline"]["tracks"].as_array().unwrap() {
            let r = &t["header"];
            assert!(
                (r["max"][1].as_f64().unwrap() - r["min"][1].as_f64().unwrap() - old_height - 20.)
                    .abs()
                    < 0.01
            );
            assert!(t["name"]["min"][1].as_f64().unwrap() >= r["min"][1].as_f64().unwrap() + 4.);
        }
        let state = app.command(json!({"command":"inspect","paint":true}));
        let indicator = &state["timeline"]["loop_indicator"];
        assert!(
            indicator["min"][1].as_f64().unwrap()
                >= state["timeline"]["ruler"]["max"][1].as_f64().unwrap() + 2.
        );
        assert!(
            state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| shape["rect"] == *indicator),
            "Loop lane must actually be painted"
        );
        assert_eq!(export(&mut app, "After.wav").samples, audio.samples);
    }
    app.quit();
}

#[test]
fn tracks_menu_creates_subfolders_and_moves_tracks_with_cycle_safe_choices_and_recall() {
    let mut app = fixture();
    app.command(json!({"command":"viewport","width":800,"height":600}));
    let original_audio = export(&mut app, "Ungrouped.wav");
    let state = app.inspect();
    let zoom = state["timeline"]["pixels_per_beat"].clone();
    let row_height = state["timeline"]["row_height"].clone();
    let drums = state["project"]["tracks"][0]["id"].as_u64().unwrap();
    let bass = state["project"]["tracks"][1]["id"].as_u64().unwrap();
    app.click("Tracks");
    let state = app.click("Add folder");
    let root = state["selected_track"].as_u64().unwrap();
    app.click("Tracks");
    let state = app.click("Add subfolder");
    let child = state["selected_track"].as_u64().unwrap();
    assert_eq!(state["project"]["tracks"][3]["parent_folder"], root);
    assert_eq!(state["timeline"]["pixels_per_beat"], zoom);
    assert_eq!(state["timeline"]["row_height"], row_height);
    assert!(
        track(&state, child)["name"]["min"][1].as_f64().unwrap()
            >= state["timeline"]["canvas"]["min"][1].as_f64().unwrap()
    );
    assert!(
        track(&state, child)["name"]["max"][1].as_f64().unwrap() < 600.0,
        "A newly created subfolder should scroll into view"
    );
    for (id, folder) in [(drums, "New subfolder"), (bass, "New folder")] {
        select(&mut app, id);
        app.click("Tracks");
        app.click("Move to folder");
        app.click(folder);
    }
    let state = app.click("Fit");
    assert_eq!(track(&state, root)["depth"], 0);
    assert_eq!(track(&state, child)["depth"], 1);
    assert_eq!(track(&state, drums)["depth"], 2);
    assert_eq!(track(&state, bass)["depth"], 1);
    assert!(
        track(&state, drums)["name"]["min"][0].as_f64().unwrap()
            > track(&state, child)["name"]["min"][0].as_f64().unwrap()
    );
    // A root cannot be placed inside its own descendant.
    let state = app.click("Fit");
    let (x, y) = center(&track(&state, root)["name"]);
    let child_labels = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "New subfolder")
        .count();
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,"button":"secondary"}));
    let state = app.click("Move to folder");
    assert_eq!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|label| *label == "New subfolder")
            .count(),
        child_labels,
        "A descendant must not be offered again as a folder destination"
    );
    app.key("Escape");
    app.key("Escape");
    // Clicking the folder header collapses all descendants; Undo restores them.
    let state = select(&mut app, root);
    assert_eq!(state["timeline"]["tracks"].as_array().unwrap().len(), 1);
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(state["timeline"]["tracks"].as_array().unwrap().len(), 4);
    let saved = app.directory.path().join("Nested.rill");
    app.command(json!({"command":"save","path":saved}));
    app.command(json!({"command":"open","path":saved}));
    let state = app.click("Fit");
    assert_eq!(track(&state, drums)["depth"], 2);
    assert_eq!(state["project"]["tracks"][0]["clips"][0]["offset"], 0.0);
    assert_eq!(
        export(&mut app, "Grouped.wav").samples,
        original_audio.samples
    );
    app.quit();
}

#[test]
fn buses_keep_a_distinct_shared_identity_at_narrow_and_compact_sizes_without_changing_channels() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Bus original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.08; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Source");
    let bus = project.add_bus("Rhythm room with a deliberately long name");
    let midi = project.add_midi_track("Guide");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Tone",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    project.set_output(audio, Some(bus)).unwrap();
    let path = app.directory.path().join("Bus identity.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open","path":path}));
    let original_audio = export(&mut app, "Bus before.wav");
    for (width, density, dark, labels, rows) in
        [(800., 1., false, 224., 56.), (1400., 2., true, 360., 144.)]
    {
        app.command(
            json!({"command":"viewport","width":width,"height":900,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let state = app.inspect();
        let (x, _) = center(&state["timeline"]["header_resize"]);
        let y = state["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.;
        let delta = labels - state["timeline"]["header_width"].as_f64().unwrap();
        let state = app.command(
            json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+delta,"to_y":y}),
        );
        let (x, y) = center(&state["timeline"]["row_resize"][0]["rect"]);
        let delta = rows - state["timeline"]["row_height"].as_f64().unwrap();
        app.command(
            json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x,"to_y":y+delta}),
        );
        let state = app.command(json!({"command":"inspect","paint":true}));
        unchanged(&state, &before);
        let heading = track(&state, bus);
        assert_eq!(heading["bus"], true);
        assert!(track(&state, audio)["bus_badge"].is_null());
        assert!(track(&state, midi)["bus_badge"].is_null());
        let badge = &heading["bus_badge"];
        assert!(
            heading["name"]["max"][0].as_f64().unwrap() + 4. <= badge["min"][0].as_f64().unwrap()
        );
        assert!(badge["max"][0].as_f64().unwrap() < heading["header"]["max"][0].as_f64().unwrap());
        assert!(badge["max"][1].as_f64().unwrap() < heading["header"]["max"][1].as_f64().unwrap());
        assert_ne!(heading["header_fill"], track(&state, audio)["header_fill"]);
        assert!(
            state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| shape["rect"] == heading["header"]
                    && shape["fill"] == heading["header_fill"])
        );
        assert_eq!(
            state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|label| *label == "BUS")
                .count(),
            1
        );
        for (id, destination) in [
            (audio, "Rhythm room with a deliberately long name"),
            (bus, "Master"),
            (midi, "Master"),
        ] {
            let sections = &track(&state, id)["sections"];
            if rows < f64::from(daw_ui::TRACK_HEADER_HEIGHT) {
                assert!(sections["routing"].is_null());
                assert!(
                    sections["details"].is_object(),
                    "Compact headers retain access to channel details"
                );
            } else {
                let routing = &sections["routing"];
                assert!(routing.is_object());
                for text in ["Routing", destination] {
                    assert!(
                        state["label_rects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|label| { label["text"] == text && contains(routing, label) }),
                        "{text} must stay inside its dedicated routing section"
                    );
                }
            }
        }
    }
    assert_eq!(
        export(&mut app, "Bus after.wav").samples,
        original_audio.samples
    );
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
    app.quit();
}

fn group(state: &Value, id: u64) -> &Value {
    state["timeline"]["folder_containers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["folder"] == id)
        .unwrap()
}
fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        outer["min"][axis].as_f64().unwrap() <= inner["min"][axis].as_f64().unwrap() + 0.01
            && outer["max"][axis].as_f64().unwrap() >= inner["max"][axis].as_f64().unwrap() - 0.01
    })
}

#[test]
fn recursive_folder_borders_enclose_descendant_lanes_with_common_grid_and_clipped_scroll_edges() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Folder original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.06; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let outer = project.add_folder("Outer");
    let kick = project.add_audio_track("Kick");
    let middle = project.add_folder("Middle");
    let inner = project.add_folder("Inner");
    let bass = project.add_audio_track("Bass");
    for (id, parent) in [
        (kick, outer),
        (middle, outer),
        (inner, middle),
        (bass, inner),
    ] {
        project.set_parent_folder(id, Some(parent)).unwrap();
    }
    for (id, color) in [
        (outer, [190, 70, 90]),
        (middle, [70, 155, 210]),
        (inner, [130, 180, 70]),
    ] {
        project.track_mut(id).unwrap().color = color;
    }
    for id in [kick, bass] {
        project.track_mut(id).unwrap().clips.push(Clip::audio(
            "Original cue",
            0.0,
            4.0,
            source.to_string_lossy(),
        ));
    }
    let path = app.directory.path().join("Colored groups.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open","path":path}));
    let audio = export(&mut app, "Before groups.wav");
    for (width, height, density, dark) in [(1400., 900., 1., false), (800., 700., 2., true)] {
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let state = app.click("Fit");
        let (x, _) = center(&state["timeline"]["header_resize"]);
        let y = state["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.0;
        let target = if width > 1000.0 { 360.0 } else { 224.0 };
        let delta = target - state["timeline"]["header_width"].as_f64().unwrap();
        app.command(
            json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+delta,"to_y":y}),
        );
        app.click("Fit");
        let state = app.command(json!({"command":"inspect","paint":true}));
        unchanged(&state, &before);
        assert_eq!(
            state["timeline"]["folder_containers"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        for (a, b) in [(outer, middle), (middle, inner)] {
            assert!(contains(
                &group(&state, a)["rect"],
                &group(&state, b)["rect"]
            ));
            assert!(
                group(&state, a)["rect"]["max"][1].as_f64().unwrap()
                    > group(&state, b)["rect"]["max"][1].as_f64().unwrap() + 4.
            );
        }
        for (id, folder) in [
            (outer, outer),
            (kick, outer),
            (middle, middle),
            (inner, inner),
            (bass, inner),
        ] {
            let header = &track(&state, id)["header"];
            assert!(contains(&group(&state, folder)["rect"], header));
            assert!(
                header["min"][0].as_f64().unwrap()
                    >= group(&state, folder)["rect"]["min"][0].as_f64().unwrap() + 4.
            );
            let color_strip = state["timeline"]["track_colors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|strip| strip["track"] == id)
                .unwrap();
            assert!(contains(
                &group(&state, folder)["rect"],
                &color_strip["rect"]
            ));
            for label in state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|label| {
                    let text = label["text"].as_str().unwrap_or("");
                    matches!(
                        text,
                        "M" | "S" | "FOLDER" | "Routing" | "Input" | "Instrument"
                    ) && label["min"][1].as_f64().unwrap() >= header["min"][1].as_f64().unwrap()
                        && label["max"][1].as_f64().unwrap() <= header["max"][1].as_f64().unwrap()
                })
            {
                assert!(
                    contains(header, label),
                    "Header control escaped the recursive left padding: {label}"
                );
            }
        }
        for (id, folder) in [(kick, outer), (bass, inner)] {
            let clip = state["timeline"]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|clip| clip["track"] == id)
                .unwrap();
            assert!(contains(
                &group(&state, folder)["rect"],
                &clip["visible_rect"]
            ));
        }
        let clips = state["timeline"]["clips"].as_array().unwrap();
        assert_eq!(
            clips[0]["rect"]["min"][0], clips[1]["rect"]["min"][0],
            "Depth cannot change the beat mapping"
        );
        for container in state["timeline"]["folder_containers"].as_array().unwrap() {
            for border in container["borders"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|border| !border.is_null())
            {
                assert!(
                    state["painted_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(
                            |shape| shape["rect"] == *border && shape["fill"] == container["color"]
                        ),
                    "Each colored container edge must actually be painted"
                );
            }
        }
    }
    // Increase global row height, then scroll the ancestor headers out of view.
    let state = app.inspect();
    let (x, y) = center(&state["timeline"]["row_resize"][0]["rect"]);
    app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x,"to_y":y+60.}));
    let state = app.inspect();
    let (x, y) = center(&state["timeline"]["canvas"]);
    let amount = state["timeline"]["row_height"].as_f64().unwrap() * 1.5;
    let state = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-amount}));
    unchanged(&state, &before);
    let root = group(&state, outer);
    assert!(
        root["rect"]["min"][1].as_f64().unwrap()
            < state["timeline"]["canvas"]["min"][1].as_f64().unwrap()
    );
    assert!(
        root["borders"][0].is_null(),
        "Scrolling must not invent a top cap"
    );
    app.command(json!({"command":"scroll","x":x,"y":y,"delta":100000.}));
    let state = app.inspect();
    let (x, y) = center(&track(&state, outer)["name"]);
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        state["timeline"]["folder_containers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(state["timeline"]["tracks"].as_array().unwrap().len(), 1);
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(state["project"], before["project"]);
    assert_eq!(export(&mut app, "After groups.wav").samples, audio.samples);
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
    app.quit();
}
