//! Bounce uses the production export renderer, then publishes one portable asset.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn click_clip(app: &mut Harness, id: u64, shift: bool) {
    let state = app.inspect();
    let clip = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap();
    let (x, y) = center(&clip["visible_rect"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,"shift":shift}));
}
fn choose(app: &mut Harness, previous: &str, next: &str) {
    app.click(previous);
    app.click(next);
}
fn rename(app: &mut Harness, previous: &str, name: &str) {
    let state = app.inspect();
    let labels = state["label_rects"].as_array().unwrap();
    let anchor = labels
        .iter()
        .rev()
        .find(|rect| rect["text"] == "Name")
        .unwrap();
    let (_, anchor_y) = center(anchor);
    let rect = labels
        .iter()
        .filter(|rect| rect["text"] == previous)
        .min_by(|a, b| {
            (center(a).1 - anchor_y)
                .abs()
                .total_cmp(&(center(b).1 - anchor_y).abs())
        })
        .unwrap();
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":name}));
    // An explicit pointer action commits the text without relying on a shortcut.
    app.click("Name");
}
fn open(app: &mut Harness, project: &Project) {
    let path = app.directory.path().join("Bounce source.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1500,"height":1100}));
    app.click("Fit");
}
fn audio_source(app: &Harness, name: &str, value: f32) -> PathBuf {
    let path = app.directory.path().join(name);
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [value, -value * 0.5]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}
fn pool(state: &Value) -> &[Value] {
    state["project"]["samples"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn bounce_options(app: &mut Harness) {
    app.click("File");
    app.click("Bounce audio…");
}

#[test]
fn selected_clips_bounce_named_quality_sample_with_undo_and_portable_recall() {
    let mut app = Harness::new();
    let first = audio_source(&app, "First.wav", 0.2);
    let second = audio_source(&app, "Second.wav", 0.3);
    let excluded = audio_source(&app, "Excluded.wav", 0.5);
    let mut project = Project::new();
    project.master.gain = 1.0;
    let a = project.add_audio_track("First source");
    let b = project.add_audio_track("Second source");
    let c = project.add_audio_track("Unselected source");
    let ca = Clip::audio("First selected", 2.0, 1.0, first.to_string_lossy());
    let cb = Clip::audio("Second selected", 3.0, 1.0, second.to_string_lossy());
    let ids = [ca.id, cb.id];
    for (track, clip) in [
        (a, ca),
        (b, cb),
        (
            c,
            Clip::audio("Leave out", 2.0, 2.0, excluded.to_string_lossy()),
        ),
    ] {
        let track = project.track_mut(track).unwrap();
        track.gain = 1.0;
        track.clips.push(clip);
    }
    open(&mut app, &project);
    click_clip(&mut app, ids[0], false);
    click_clip(&mut app, ids[1], true);
    let before = app.inspect();
    bounce_options(&mut app);
    app.click("Project samples");
    rename(&mut app, "Selected clips", "Layered percussion");
    choose(&mut app, "32-bit float", "24-bit PCM");
    choose(&mut app, "Follow audio setup", "44.1 kHz");
    app.click("Include effect tails");
    app.click("Bounce");
    let done = app.command(json!({"command":"wait_export"}));
    assert_eq!(pool(&done).len(), 1, "{done}");
    assert_eq!(pool(&done)[0]["name"], "Layered percussion");
    assert_eq!(done["project"]["tracks"], before["project"]["tracks"]);
    let output = PathBuf::from(pool(&done)[0]["path"].as_str().unwrap());
    let reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(
        (
            reader.spec().sample_rate,
            reader.spec().bits_per_sample,
            reader.duration()
        ),
        (44_100, 24, 44_100)
    );
    let wav = AudioData::load_wav(&output).unwrap();
    assert!(
        (wav.samples[10_000 * 2] - 0.2).abs() < 1e-4,
        "Unselected source leaked into bounce"
    );
    assert!((wav.samples[30_000 * 2] - 0.3).abs() < 1e-4);
    app.click("Edit");
    let undone = app.click("Undo");
    assert!(pool(&undone).is_empty());
    app.click("Edit");
    let redone = app.click("Redo");
    assert_eq!(pool(&redone), pool(&done));
    let saved = app.directory.path().join("Portable samples.rill");
    app.command(json!({"command":"save","path":saved}));
    // The project collection owns the media even when no instrument uses it.
    let bytes = std::fs::read(&output).unwrap();
    std::fs::remove_file(&output).unwrap();
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(pool(&reopened)[0]["name"], "Layered percussion");
    assert_eq!(
        std::fs::read(pool(&reopened)[0]["path"].as_str().unwrap()).unwrap(),
        bytes
    );
    app.quit();
}

#[test]
fn multi_note_midi_clip_and_loop_share_file_bounce_pipeline() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("R1 source");
    project.track_mut(track).unwrap().instrument = Instrument::builtin("r1-mini");
    let clip = Clip::midi(
        "Clap gesture",
        2.0,
        2.0,
        vec![
            Note::new(48, 0.0, 0.05, 100),
            Note::new(55, 0.15, 0.05, 90),
            Note::new(60, 0.3, 0.05, 80),
        ],
    );
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project.loop_enabled = true;
    project.loop_start = 2.0;
    project.loop_end = 4.0;
    open(&mut app, &project);
    click_clip(&mut app, id, false);
    let before = app.inspect();
    bounce_options(&mut app);
    app.click("Include effect tails");
    let output = app.directory.path().join("Gesture.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":output}));
    app.click("Bounce…");
    let done = app.command(json!({"command":"wait_export"}));
    assert_eq!(done["project"], before["project"]);
    assert_eq!(done["history"], before["history"]);
    let wav = AudioData::load_wav(&output).unwrap();
    assert_eq!(wav.frames(), 48_000);
    for (start, end) in [(0, 1200), (3600, 4800), (7200, 8400)] {
        let rms = (wav.samples[start * 2..end * 2]
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            / ((end - start) * 2) as f32)
            .sqrt();
        assert!(rms > 0.001, "Missing MIDI gesture hit at frame {start}");
    }
    bounce_options(&mut app);
    choose(&mut app, "Selected clips", "Loop range");
    let loop_path = app.directory.path().join("Loop.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":loop_path}));
    app.click("Bounce…");
    app.command(json!({"command":"wait_export"}));
    assert_eq!(AudioData::load_wav(loop_path).unwrap().samples, wav.samples);
    app.quit();
}

#[test]
fn canceled_or_stale_project_bounce_publishes_no_sample_or_managed_file() {
    for stale in [false, true] {
        let mut app = Harness::with_export_delay(1500);
        let source = audio_source(&app, "Transient.wav", 0.2);
        let mut project = Project::new();
        let track = project.add_audio_track("Transient source");
        let clip = Clip::audio("Transient", 0.0, 1.0, source.to_string_lossy());
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        open(&mut app, &project);
        click_clip(&mut app, id, false);
        bounce_options(&mut app);
        app.click("Project samples");
        app.click("Bounce");
        if stale {
            app.click("Track");
            app.click("Add audio track");
        } else {
            app.click("Cancel export");
        }
        app.raw_command(json!({"command":"wait_export"}));
        let done = app.inspect();
        assert!(pool(&done).is_empty());
        let folder = Path::new(done["scratch_dir"].as_str().unwrap()).join("samples");
        assert!(
            std::fs::read_dir(&folder).unwrap().next().is_none(),
            "Unpublished bounce leaked its managed asset"
        );
        if stale {
            let review = app.click("Retry export");
            assert_eq!(review["popups"]["export"], true);
            assert!(
                review["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "Bounce audio")
            );
            app.click("Bounce");
            let retried = app.command(json!({"command":"wait_export"}));
            assert_eq!(pool(&retried).len(), 1, "{retried}");
            assert_eq!(pool(&retried)[0]["name"], "Transient");
            assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        }
        app.quit();
    }
}

#[test]
fn loaded_sample_exports_through_the_same_dialog_without_reapplying_track_gain() {
    let mut app = Harness::new();
    let source = audio_source(&app, "Original recording.wav", 0.2);
    let mut project = Project::new();
    let track = project.add_midi_track("Sampler source");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project.track_mut(track).unwrap().gain = 0.1;
    project
        .load_device_sample(
            track,
            daw_core::DeviceSample {
                slot: 0,
                path: source.to_string_lossy().into_owned(),
                name: "Original recording".into(),
            },
        )
        .unwrap();
    open(&mut app, &project);
    app.click("Sampler source");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Sample actions");
    app.click("Sample actions");
    app.click("Export sample…");
    let state = app.inspect();
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Loaded sample")
    );
    app.click("Keyboard");
    app.click("Clear");
    for key in ["h", "i", "t"] {
        app.click(key);
    }
    app.click("Keyboard");
    let named = app.inspect();
    assert_eq!(named["file_dialog_defaults"]["wav"], "hit.wav");
    let output = app.directory.path().join("Reusable recording.wav");
    app.command(json!({"command":"file_dialog","kind":"export","path":output}));
    app.click("Bounce…");
    let done = app.command(json!({"command":"wait_export"}));
    assert!(pool(&done).is_empty());
    let wav = AudioData::load_wav(output).unwrap();
    assert_eq!(wav.frames(), 48_000);
    assert!((wav.samples[20_000] - 0.2).abs() < 1e-6);
    assert!((wav.samples[20_001] + 0.1).abs() < 1e-6);
    app.quit();
}

#[test]
fn application_export_uses_the_same_selected_source_and_project_sample_transaction() {
    let mut app = Harness::new();
    let source = audio_source(&app, "Tool source.wav", 0.25);
    let mut project = Project::new();
    let track = project.add_audio_track("Tool source");
    let clip = Clip::audio("Selected for tool", 3.0, 1.0, source.to_string_lossy());
    let id = clip.id;
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(clip);
    open(&mut app, &project);
    click_clip(&mut app, id, false);
    let before = app.inspect();
    let invalid = app.command(json!({"command":"application_tool","name":"document_action","arguments":{
        "action":"export","source":"selected_clips","destination":"project_samples","name":"Tool hit","path":"not-a-managed-destination.wav"
    }}));
    assert_eq!(invalid["tool_result"]["ok"], false);
    assert_eq!(invalid["project"], before["project"]);
    let accepted = app.command(json!({"command":"application_tool","name":"document_action","arguments":{
        "action":"export","source":"selected_clips","destination":"project_samples","name":"Tool hit",
        "sample_rate":44_100,"bit_depth":16,"include_tails":false
    }}));
    assert_eq!(accepted["tool_result"]["ok"], true, "{accepted}");
    let done = app.command(json!({"command":"wait_export"}));
    assert_eq!(pool(&done).len(), 1);
    assert_eq!(pool(&done)[0]["name"], "Tool hit");
    assert_eq!(done["history"]["undo"][0], "Bounce to project samples");
    let path = pool(&done)[0]["path"].as_str().unwrap();
    let reader = hound::WavReader::open(path).unwrap();
    assert_eq!(
        (
            reader.spec().bits_per_sample,
            reader.spec().sample_rate,
            reader.duration()
        ),
        (16, 44_100, 22_050)
    );
    let wav = AudioData::load_wav(path).unwrap();
    assert!((wav.samples[10_000] - 0.25).abs() < 1e-4);
    assert_eq!(done["project"]["tracks"], before["project"]["tracks"]);
    app.quit();
}
