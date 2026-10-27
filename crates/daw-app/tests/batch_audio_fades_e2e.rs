//! Batch anti-click edits through the running app, with PCM and original-file oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, FadeCurve, Note, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture() -> (Harness, u64, u64, u64, u64, PathBuf, Vec<u8>) {
    let mut app = Harness::new();
    let source = app.directory.path().join("original.wav");
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: (0..192000)
            .flat_map(|i| {
                let value = 0.3 + (i as f32 * 0.02).sin() * 0.1;
                [value, -value * 0.5]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let first = project.add_audio_track("Protected musical fades");
    let a = Clip::audio("A musical phrase", 0.0, 2.0, source.to_string_lossy());
    let a_id = a.id;
    project.track_mut(first).unwrap().clips.push(a);
    project
        .set_audio_fades(
            first,
            a_id,
            0.5,
            0.5,
            FadeCurve::SCurve,
            FadeCurve::EqualPower,
        )
        .unwrap();
    let ghost = project.clone_clip(first, a_id).unwrap();
    project.move_clip(first, ghost, 8.0).unwrap();
    let second = project.add_audio_track("Cut edges");
    let mut parent = Clip::audio("B cut cue", 0.0, 4.0, source.to_string_lossy());
    parent.source_rate = 2.0;
    if let ClipContent::Audio {
        fade_in,
        fade_out,
        fade_in_curve,
        fade_out_curve,
        reverse,
        ..
    } = &mut parent.content
    {
        *fade_in = 3.0;
        *fade_out = 3.0;
        *fade_in_curve = FadeCurve::SCurve;
        *fade_out_curve = FadeCurve::EqualPower;
        *reverse = true;
    }
    let mut b = parent.audio_region(1.0, 3.0).unwrap();
    b.start = 3.0;
    let b_id = b.id;
    project.track_mut(second).unwrap().clips.push(b);
    let midi = project.add_midi_track("MIDI included in selection");
    let m = Clip::midi("MIDI phrase", 0.0, 2.0, vec![Note::new(60, 0.0, 1.0, 100)]);
    let m_id = m.id;
    project.track_mut(midi).unwrap().clips.push(m);
    let path = app.directory.path().join("batch.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.key("Q");
    app.click("Fit");
    (app, a_id, b_id, m_id, ghost, source, bytes)
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn select(app: &mut Harness, id: u64, shift: bool) -> Value {
    let state = app.inspect();
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,"shift":shift}))
}

fn edit(app: &mut Harness, label: &str) -> Value {
    app.click("Edit");
    app.click(label)
}
fn chord(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key","key":key,"cmd":true}))
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn mixed_selection_smooths_cut_audio_in_one_undo_and_preserves_other_sources_and_fades() {
    let (mut app, a, b, midi, ghost, source, bytes) = fixture();
    let baseline = export(&mut app, "before.wav");
    select(&mut app, a, false);
    select(&mut app, b, true);
    select(&mut app, midi, true);
    let before = app.inspect();
    let selection = before["selected_clips"].as_array().unwrap();
    assert_eq!(selection.len(), 3, "mixed clip selection: {selection:?}");
    for id in [a, b, midi] {
        assert!(
            selection.iter().any(|item| item[1] == id),
            "missing selected clip {id}: {selection:?}"
        );
    }
    let edited = edit(&mut app, "Smooth edit boundaries");
    assert_eq!(
        clip(&edited, a),
        clip(&before, a),
        "existing musical fades remain exact"
    );
    assert_eq!(
        clip(&edited, ghost),
        clip(&before, ghost),
        "unselected ghost remains exact"
    );
    assert_eq!(clip(&edited, midi), clip(&before, midi));
    let mut restored = clip(&edited, b).clone();
    let boundary = restored["content"]["Audio"]
        .as_object_mut()
        .unwrap()
        .remove("boundary_fades")
        .unwrap();
    assert_eq!(
        restored,
        *clip(&before, b),
        "source mapping and inherited envelope stay intact"
    );
    assert!((boundary["fade_in"].as_f64().unwrap() - 0.012).abs() < 1e-10);
    assert!((boundary["fade_out"].as_f64().unwrap() - 0.020).abs() < 1e-10);
    assert_eq!(
        edited["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    let smoothed = export(&mut app, "after.wav");
    let frames_per_beat = baseline.sample_rate as f64 * 0.5;
    let start = (3.0 * frames_per_beat).round() as usize * 2;
    let inside =
        ((3.0 * frames_per_beat) + 0.003 * baseline.sample_rate as f64).ceil() as usize * 2;
    let last = ((5.0 * frames_per_beat) - 0.005 * baseline.sample_rate as f64).floor() as usize * 2;
    let end = (5.0 * frames_per_beat).round() as usize * 2;
    assert_eq!(&baseline.samples[..start], &smoothed.samples[..start]);
    assert_eq!(
        &baseline.samples[inside..last],
        &smoothed.samples[inside..last]
    );
    assert_eq!(&baseline.samples[end..], &smoothed.samples[end..]);
    assert_eq!(&smoothed.samples[start..start + 2], &[0.0, 0.0]);
    assert_eq!(chord(&mut app, "Z")["project"], before["project"]);
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}))["project"],
        edited["project"]
    );
    assert_eq!(export(&mut app, "redo.wav").samples, smoothed.samples);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    app.quit();
}

fn field(app: &mut Harness, title: &str, value: &str) {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| {
            rect["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" ms") && text.starts_with(title))
        })
        .unwrap();
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    chord(app, "A");
    app.command(json!({"command":"text","text":value}));
    app.key("Enter");
}

#[test]
fn configured_fades_cancel_cleanly_apply_once_and_survive_portable_recall() {
    let (mut app, _, b, _, _, source, bytes) = fixture();
    select(&mut app, b, false);
    let before = app.inspect();
    edit(&mut app, "Boundary fade settings…");
    field(&mut app, "3", "4.25");
    field(&mut app, "5", "12.5");
    app.click("Cancel");
    let cancelled = app.inspect();
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    edit(&mut app, "Boundary fade settings…");
    field(&mut app, "3", "4.25");
    field(&mut app, "5", "12.5");
    let edited = app.click("Apply boundary fades");
    let boundary = &clip(&edited, b)["content"]["Audio"]["boundary_fades"];
    assert!((boundary["fade_in"].as_f64().unwrap() - 0.017).abs() < 1e-10);
    assert!((boundary["fade_out"].as_f64().unwrap() - 0.05).abs() < 1e-10);
    let same = edit(&mut app, "Smooth edit boundaries");
    assert_eq!(same["project"], edited["project"]);
    assert_eq!(
        same["history"], edited["history"],
        "reapplying minimum fades creates no undo step"
    );
    let expected = export(&mut app, "configured.wav");
    let path = app.directory.path().join("portable.rill");
    app.command(json!({"command":"save","path":path}));
    app.command(json!({"command":"open","path":path}));
    let recalled = app.inspect();
    assert_eq!(
        clip(&recalled, b)["content"]["Audio"]["boundary_fades"],
        *boundary
    );
    assert_eq!(export(&mut app, "recalled.wav").samples, expected.samples);
    select(&mut app, b, false);
    edit(&mut app, "Boundary fade settings…");
    let removed = app.click("Remove boundary fades");
    assert!(clip(&removed, b)["content"]["Audio"]["boundary_fades"].is_null());
    let restored = chord(&mut app, "Z");
    assert_eq!(
        clip(&restored, b)["content"]["Audio"]["boundary_fades"],
        *boundary
    );
    assert_eq!(export(&mut app, "restored.wav").samples, expected.samples);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    app.quit();
}
