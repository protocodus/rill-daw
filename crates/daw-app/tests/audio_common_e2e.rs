//! Common audio edits via the running app: real gestures, undo and WAV oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};

fn fixture(reverse: bool, crossings: bool) -> Harness {
    let mut app = Harness::new();
    let path = app.directory.path().join("common-source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: (0..192000)
            .flat_map(|frame| {
                let value = if crossings {
                    if frame % 100 == 8 {
                        0.0
                    } else if frame % 100 < 8 {
                        -0.4
                    } else {
                        0.4
                    }
                } else {
                    0.1 + frame as f32 / 800000.0
                };
                [value, -value * 0.5]
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Common audio");
    project.track_mut(track).unwrap().gain = 1.0;
    let mut clip = Clip::audio("Edit source", 1.0, 4.0, path.to_string_lossy());
    clip.offset = 1.0;
    if let ClipContent::Audio { reverse: value, .. } = &mut clip.content {
        *value = reverse;
    }
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("common.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Edit source");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn selected(state: &Value) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == state["selected_clip"])
        .unwrap()
}
fn point(state: &Value, beat: f64) -> (f64, f64) {
    let view = &state["audio_editor"];
    (
        view["rect"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap(),
        (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) * 0.5,
    )
}
fn drag(app: &mut Harness, start: f64, end: f64, shift: bool) -> Value {
    let state = app.inspect();
    let at = point(&state, start);
    let to = point(&state, end);
    app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":to.0,"to_y":to.1,"shift":shift}))
}
fn at(app: &mut Harness, beat: f64, shift: bool) -> Value {
    let at = point(&app.inspect(), beat);
    app.command(json!({"command":"pointer","phase":"click","x":at.0,"y":at.1,"shift":shift}))
}
fn chord(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key","key":key,"cmd":true}))
}
fn dismiss_menu(app: &mut Harness) {
    app.command(json!({"command":"pointer","phase":"click","x":4.0,"y":4.0}));
}
fn range(app: &mut Harness) -> Value {
    app.key("P");
    app.click("Fit");
    let state = drag(app, 2.0, 3.0, false);
    assert_eq!(state["audio_editor"]["range"], json!([2.0, 3.0]));
    state
}
fn export(app: &mut Harness, filename: &str) -> AudioData {
    let path = app.directory.path().join(filename);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn near(value: &Value, expected: f64) {
    assert!(
        (value.as_f64().unwrap() - expected).abs() < 1e-8,
        "{value} != {expected}"
    );
}
fn frames(data: &AudioData, start: f64, end: f64) -> &[f32] {
    &data.samples[(start * 24000.0).round() as usize * 2..(end * 24000.0).round() as usize * 2]
}

#[test]
fn range_cut_and_delete_leave_silent_gaps_and_undo_in_one_step() {
    let mut app = fixture(false, false);
    let original = app.inspect()["project"].clone();
    let baseline = export(&mut app, "baseline.wav");
    range(&mut app);
    let cut = chord(&mut app, "X");
    assert_eq!(
        cut["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let edited = export(&mut app, "cut.wav");
    assert_eq!(frames(&edited, 0.0, 2.0), frames(&baseline, 0.0, 2.0));
    assert!(
        frames(&edited, 2.0, 3.0)
            .iter()
            .all(|sample| *sample == 0.0)
    );
    assert_eq!(frames(&edited, 3.0, 5.0), frames(&baseline, 3.0, 5.0));
    assert_eq!(chord(&mut app, "Z")["project"], original);
    range(&mut app);
    app.key("Delete");
    assert_eq!(export(&mut app, "deleted.wav").samples, edited.samples);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    range(&mut app);
    app.click("Edit");
    app.click("Delete");
    assert_eq!(export(&mut app, "menu-delete.wav").samples, edited.samples);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.quit();
}

#[test]
fn ripple_delete_closes_time_on_one_track_and_moves_later_audio_with_one_undo() {
    let mut app = fixture(false, false);
    let mut project: Project = serde_json::from_value(app.inspect()["project"].clone()).unwrap();
    let track = project.tracks[0].id;
    let ClipContent::Audio { path, .. } = &project.tracks[0].clips[0].content else {
        unreachable!()
    };
    let mut later = Clip::audio("Later", 6.0, 1.0, path);
    later.offset = 7.0;
    project.track_mut(track).unwrap().clips.push(later);
    let other = project.add_audio_track("Other track");
    project.track_mut(other).unwrap().mute = true;
    let path = app.directory.path().join("ripple.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Edit source");
    app.click("Window");
    app.click("Clip Editor");
    let before = app.inspect()["project"].clone();
    let baseline = export(&mut app, "ripple-before.wav");
    range(&mut app);
    app.click("Clip");
    let rippled = app.click("Ripple delete selection");
    let later = rippled["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["name"] == "Later")
        .unwrap();
    near(&later["start"], 5.0);
    assert_eq!(rippled["project"]["tracks"][1], before["tracks"][1]);
    let rendered = export(&mut app, "ripple-after.wav");
    let expected: Vec<_> = frames(&baseline, 0.0, 2.0)
        .iter()
        // Preserve the exporter's release tail after the arrangement too.
        .chain(&baseline.samples[3 * 24000 * 2..])
        .copied()
        .collect();
    assert_eq!(
        rendered.samples.len(),
        expected.len(),
        "Ripple export duration"
    );
    let mismatch = rendered
        .samples
        .iter()
        .zip(&expected)
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected);
    assert!(
        mismatch.is_none(),
        "First ripple sample mismatch: {mismatch:?}"
    );
    assert_eq!(chord(&mut app, "Z")["project"], before);
    app.quit();
}

#[test]
fn whole_clip_cut_can_paste_back_after_returning_to_arranger() {
    let mut app = fixture(false, false);
    let original = app.inspect()["project"].clone();
    let baseline = export(&mut app, "baseline.wav");
    at(&mut app, 1.0, false);
    let cut = chord(&mut app, "X");
    assert!(
        cut["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(cut["view"], "Arrangement");
    app.click("Edit");
    let pasted = app.click("Paste");
    assert_eq!(
        pasted["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(export(&mut app, "restored.wav").samples, baseline.samples);
    chord(&mut app, "Z");
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.quit();
}

#[test]
fn range_copy_paste_and_duplicate_preserve_samples_and_project_recall() {
    let mut app = fixture(true, false);
    let baseline = export(&mut app, "baseline.wav");
    let original = app.inspect()["project"].clone();
    range(&mut app);
    assert_eq!(chord(&mut app, "C")["project"], original);
    at(&mut app, 4.0, false);
    let pasted = chord(&mut app, "V");
    near(&selected(&pasted)["start"], 4.0);
    near(&selected(&pasted)["length"], 1.0);
    assert!(selected(&pasted)["link_group"].is_null());
    let rendered = export(&mut app, "pasted.wav");
    assert_eq!(frames(&rendered, 4.0, 5.0), frames(&baseline, 2.0, 3.0));
    assert_eq!(frames(&rendered, 0.0, 4.0), frames(&baseline, 0.0, 4.0));
    let path = app.directory.path().join("edited.rill");
    app.command(json!({"command":"save","path":path}));
    app.command(json!({"command":"open","path":path}));
    assert_eq!(export(&mut app, "recalled.wav").samples, rendered.samples);
    // Reload the original fixture to test duplicate as a separate transaction.
    let path = app.directory.path().join("common.rill");
    app.command(json!({"command":"open","path":path}));
    app.click("Edit source");
    app.click("Window");
    app.click("Clip Editor");
    let original = app.inspect()["project"].clone();
    range(&mut app);
    let duplicate = chord(&mut app, "D");
    near(&selected(&duplicate)["start"], 3.0);
    near(&selected(&duplicate)["length"], 1.0);
    let duplicated = export(&mut app, "duplicated.wav");
    assert_eq!(frames(&duplicated, 3.0, 4.0), frames(&baseline, 2.0, 3.0));
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.quit();
}

#[test]
fn duplicate_menu_names_the_active_audio_scope_and_matches_the_rendered_result() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture(true, false);
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100.0,"height":760.0,"pixels_per_point":density}),
        );
        let original = app.inspect()["project"].clone();
        let baseline = export(&mut app, "baseline.wav");
        let visible = |state: &Value, label: &str| {
            state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == label)
        };

        let whole_clip = app.click("Clip");
        assert!(visible(&whole_clip, "Duplicate clip"));
        assert!(!visible(&whole_clip, "Duplicate selection"));
        app.key("Escape");
        range(&mut app);
        let selection = app.click("Clip");
        assert!(visible(&selection, "Duplicate selection"));
        assert!(!visible(&selection, "Duplicate clip"));
        let cleared = app.click("Clear selection");
        assert!(cleared["audio_editor"]["range"].is_null());
        assert_eq!(cleared["project"], original);
        assert!(visible(&app.click("Clip"), "Duplicate clip"));
        app.key("Escape");

        range(&mut app);
        app.click("Clip");
        let duplicated = app.click("Duplicate selection");
        near(&selected(&duplicated)["start"], 3.0);
        near(&selected(&duplicated)["length"], 1.0);
        let rendered = export(&mut app, "duplicated.wav");
        assert_eq!(frames(&rendered, 3.0, 4.0), frames(&baseline, 2.0, 3.0));
        assert_eq!(chord(&mut app, "Z")["project"], original);

        app.click("Clip");
        let duplicated = app.click("Duplicate clip");
        near(&selected(&duplicated)["start"], 5.0);
        near(&selected(&duplicated)["length"], 4.0);
        let rendered = export(&mut app, "whole-clip.wav");
        assert_eq!(frames(&rendered, 5.0, 9.0), frames(&baseline, 1.0, 5.0));
        assert_eq!(chord(&mut app, "Z")["project"], original);
        app.quit();
    }
}

#[test]
fn slip_drag_moves_audio_inside_fixed_edges_and_retains_reverse_stereo_mapping() {
    for reverse in [false, true] {
        let mut app = fixture(reverse, false);
        let original = app.inspect()["project"].clone();
        let source = AudioData::load_wav(app.directory.path().join("common-source.wav")).unwrap();
        let state = app.key("L");
        assert_eq!(state["audio_editor"]["tool"], "Slip");
        let slipped = drag(&mut app, 3.0, 3.5, false);
        near(&selected(&slipped)["start"], 1.0);
        near(&selected(&slipped)["length"], 4.0);
        near(
            &selected(&slipped)["offset"],
            if reverse { 1.5 } else { 0.5 },
        );
        let rendered = export(&mut app, "slipped.wav");
        let mut expected = if reverse {
            frames(&source, 1.5, 5.5)
                .as_chunks::<2>()
                .0
                .iter()
                .rev()
                .flat_map(|frame| frame.iter().copied())
                .collect::<Vec<_>>()
        } else {
            frames(&source, 0.5, 4.5).to_vec()
        };
        assert_eq!(frames(&rendered, 1.0, 5.0), expected.as_mut_slice());
        assert_eq!(chord(&mut app, "Z")["project"], original);
        app.quit();
    }
}

#[test]
fn zero_crossing_snap_changes_cursor_cut_and_trim_together_and_shift_bypasses() {
    let mut app = fixture(false, true);
    let original = app.inspect()["project"].clone();
    near(&at(&mut app, 2.0, false)["audio_editor"]["cursor"], 2.0);
    app.click("Grid 0.25");
    assert_eq!(
        app.click("Source zero crossings")["audio_editor"]["snap_zero_crossings"],
        true
    );
    dismiss_menu(&mut app);
    let expected = 2.0 + 8.0 / 24000.0;
    near(
        &at(&mut app, 2.0, false)["audio_editor"]["cursor"],
        expected,
    );
    let bypass = at(&mut app, 2.0, true)["audio_editor"]["cursor"]
        .as_f64()
        .unwrap();
    assert!((bypass - 2.0).abs() < 1e-6);
    app.key("E");
    let cut = at(&mut app, 2.0, false);
    let clips = cut["project"]["tracks"][0]["clips"].as_array().unwrap();
    let right = clips
        .iter()
        .max_by(|a, b| {
            a["start"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["start"].as_f64().unwrap())
        })
        .unwrap();
    near(&right["start"], expected);
    let rendered = export(&mut app, "zero-cut.wav");
    let boundary = (expected * 24000.0).round() as usize * 2;
    assert_eq!(&rendered.samples[boundary..boundary + 2], &[0.0, 0.0]);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.key("P");
    at(&mut app, 2.0, false);
    app.click("Clip");
    let trimmed = app.click("Set start");
    near(&selected(&trimmed)["start"], expected);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.quit();
}

#[test]
fn nudge_uses_shared_sample_millisecond_and_grid_steps_in_both_workspaces() {
    let mut app = fixture(false, false);
    let original = app.inspect()["project"].clone();
    app.click("Nudge");
    app.click("1 sample");
    dismiss_menu(&mut app);
    let nudged = chord(&mut app, "ArrowRight");
    near(&selected(&nudged)["start"], 1.0 + 1.0 / 24000.0);
    let source = AudioData::load_wav(app.directory.path().join("common-source.wav")).unwrap();
    let rendered = export(&mut app, "sample-nudge.wav");
    assert_eq!(
        &rendered.samples[24001 * 2..24001 * 2 + 2048],
        &source.samples[24000 * 2..24000 * 2 + 2048]
    );
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.click("Nudge");
    app.click("1 ms");
    dismiss_menu(&mut app);
    near(&selected(&chord(&mut app, "ArrowRight"))["start"], 1.002);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.click("Arranger");
    near(&selected(&chord(&mut app, "ArrowRight"))["start"], 1.002);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.click("Nudge");
    app.click("Grid");
    dismiss_menu(&mut app);
    near(&selected(&chord(&mut app, "ArrowLeft"))["start"], 0.75);
    assert_eq!(chord(&mut app, "Z")["project"], original);
    app.quit();
}

#[test]
fn compact_common_controls_work_in_narrow_and_wide_editors() {
    let mut app = fixture(false, false);
    for width in [900.0, 1600.0] {
        app.command(json!({"command":"viewport","width":width,"height":760.0}));
        for label in ["Slip", "Nudge", "Clip", "View"] {
            let state = app.inspect();
            assert!(
                state["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == label),
                "Missing {label} at {width}"
            );
        }
        app.click("Nudge");
        app.click("1 sample");
        dismiss_menu(&mut app);
        app.click("Clip");
        app.click("Copy");
        app.click("View");
        app.key("Escape");
        assert_eq!(app.inspect()["view"], "Clip");
    }
    app.quit();
}
