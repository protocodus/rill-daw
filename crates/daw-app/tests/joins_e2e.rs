//! Joining through real popup controls, with source-window and sample oracles.
mod common;
use common::Harness;
use daw_core::{AudioCrossfade, AudioData, Clip, ClipContent, CrossfadeCurve, Note, Project};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

fn open(app: &mut Harness, project: &Project, clip: &str) {
    let path = app.directory.path().join("join-fixture.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click(clip);
}
fn join_dialog(app: &mut Harness) {
    app.click("Edit");
    let state = app.click("Join clips…");
    assert_eq!(state["popups"]["join"], true, "{state}");
}
fn wait_join(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["join_pending"] == false {
            assert_eq!(state["error"], false, "{state}");
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Audio join did not finish: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn document(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn near(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 3e-6,
        "{actual} != {expected}"
    );
}

#[test]
fn midi_join_clips_visible_note_windows_undo_redo_and_portable_recall() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Join MIDI");
    let mut a = Clip::midi(
        "Trimmed phrase",
        1.0,
        2.0,
        vec![
            Note::new(60, 0.5, 1.5, 90),
            Note::new(62, 2.5, 1.0, 100),
            Note::new(65, 4.0, 1.0, 80),
        ],
    );
    a.offset = 1.0;
    let b = Clip::midi(
        "Second phrase",
        4.0,
        2.0,
        vec![Note::new(69, 0.5, 3.0, 110)],
    );
    let outside = Clip::midi("Keep separate", 10.0, 1.0, vec![]);
    let old_ids: HashSet<_> = [&a, &b]
        .into_iter()
        .flat_map(|clip| {
            let ClipContent::Midi { notes } = &clip.content else {
                unreachable!()
            };
            notes
                .iter()
                .map(|note| note.id)
                .chain([clip.id])
                .collect::<Vec<_>>()
        })
        .collect();
    project.track_mut(track).unwrap().clips = vec![a, b, outside.clone()];
    open(&mut app, &project, "Trimmed phrase");
    join_dialog(&mut app);
    app.click("Keep separate"); // Exercise selection without changing the final scope.
    app.click("Keep separate");
    let joined = document(&app.click("Join"));
    let clips = &joined.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[1], outside);
    let clip = &clips[0];
    assert_eq!(
        (clip.name.as_str(), clip.start, clip.length, clip.offset),
        ("Joined MIDI", 1.0, 5.0, 0.0)
    );
    assert!(!old_ids.contains(&clip.id));
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("Joined MIDI became audio")
    };
    assert_eq!(
        notes
            .iter()
            .map(|n| (n.pitch, n.start, n.length, n.velocity))
            .collect::<Vec<_>>(),
        vec![(60, 0.0, 1.0, 90), (62, 1.5, 0.5, 100), (69, 3.5, 1.5, 110)]
    );
    assert!(notes.iter().all(|note| !old_ids.contains(&note.id)));
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(document(&undone), project);
    let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    assert_eq!(document(&redone), joined);
    let archive = app.directory.path().join("joined-midi.rill");
    app.command(json!({"command":"save", "path":archive}));
    let reopened = app.command(json!({"command":"open", "path":archive}));
    assert_eq!(document(&reopened), joined);
    app.quit();
}

#[test]
fn audio_join_bakes_edits_and_crossfade_once_and_reopens_without_scratch_source() {
    let mut app = Harness::new();
    let a_path = app.directory.path().join("first.wav");
    let b_path = app.directory.path().join("second.wav");
    for (path, second) in [(&a_path, false), (&b_path, true)] {
        AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: (0..192000)
                .flat_map(|frame| {
                    let value = if second {
                        0.35 - frame as f32 / 2000000.0
                    } else {
                        0.1 + frame as f32 / 1000000.0
                    };
                    [value, -value * 0.5]
                })
                .collect(),
        }
        .write_wav(path)
        .unwrap();
    }
    let source_bytes = [
        std::fs::read(&a_path).unwrap(),
        std::fs::read(&b_path).unwrap(),
    ];
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Join audio");
    let mut a = Clip::audio("First take", 1.0, 4.0, a_path.to_string_lossy());
    a.offset = 0.25;
    if let ClipContent::Audio {
        gain,
        fade_in,
        fade_out,
        ..
    } = &mut a.content
    {
        *gain = 0.75;
        *fade_in = 0.5;
        *fade_out = 0.5;
    }
    let mut b = Clip::audio("Second take", 3.0, 4.0, b_path.to_string_lossy());
    b.offset = 0.5;
    if let ClipContent::Audio {
        gain,
        reverse,
        crossfade,
        ..
    } = &mut b.content
    {
        *gain = 0.8;
        *reverse = true;
        *crossfade = Some(AudioCrossfade {
            duration: 1.0,
            curve: CrossfadeCurve::SMOOTH,
        });
    }
    let channel = project.track_mut(track).unwrap();
    channel.gain = 0.5;
    channel.pan = -0.25;
    channel.clips = vec![a, b];
    open(&mut app, &project, "First take");
    let before = export(&mut app, "before.wav");
    for (frame, expected) in [(30000, 0.021), (84000, 0.0915251)] {
        near(before.samples[frame * 2], expected);
        near(before.samples[frame * 2 + 1], -expected * 0.5 * 0.75);
    }
    join_dialog(&mut app);
    app.click("Join");
    let state = wait_join(&mut app);
    assert_eq!(state["popups"]["join"], false);
    let joined = document(&state);
    let channel = joined.track(track).unwrap();
    assert_eq!(
        (channel.gain, channel.pan, channel.clips.len()),
        (0.5, -0.25, 1)
    );
    let clip = &channel.clips[0];
    assert_eq!((clip.start, clip.length, clip.offset), (1.0, 6.0, 0.0));
    let ClipContent::Audio {
        path,
        gain,
        fade_in,
        fade_out,
        reverse,
        crossfade,
        ..
    } = &clip.content
    else {
        panic!("Joined audio became MIDI")
    };
    assert_eq!(
        (*gain, *fade_in, *fade_out, *reverse, *crossfade),
        (1.0, 0.0, 0.0, false, None)
    );
    let baked_path = std::path::PathBuf::from(path);
    let baked = AudioData::load_wav(&baked_path).unwrap();
    assert_eq!(
        (baked.sample_rate, baked.channels, baked.frames()),
        (48000, 2, 144000)
    );
    let after = export(&mut app, "after.wav");
    assert_eq!(before.samples.len(), after.samples.len());
    assert!(
        before
            .samples
            .iter()
            .zip(&after.samples)
            .all(|(a, b)| (a - b).abs() < 3e-6)
    );
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(document(&undone), project);
    let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
    assert_eq!(document(&redone), joined);
    let archive = app.directory.path().join("joined-audio.rill");
    app.command(json!({"command":"save", "path":archive}));
    std::fs::remove_file(&baked_path).unwrap();
    let reopened = app.command(json!({"command":"open", "path":archive}));
    assert_eq!(reopened["error"], false);
    let restored = export(&mut app, "restored.wav");
    assert_eq!(restored.samples, after.samples);
    assert_eq!(std::fs::read(a_path).unwrap(), source_bytes[0]);
    assert_eq!(std::fs::read(b_path).unwrap(), source_bytes[1]);
    app.quit();
}

#[test]
fn audio_join_requires_overlapping_regions_and_cancel_leaves_document_untouched() {
    let mut app = Harness::new();
    let source = app.directory.path().join("constant.wav");
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: vec![0.2; 192000 * 2],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Overlap selection");
    project.track_mut(track).unwrap().clips = vec![
        Clip::audio("Base", 0.0, 8.0, source.to_string_lossy()),
        Clip::audio("Middle", 2.0, 4.0, source.to_string_lossy()),
        Clip::audio("Nested", 3.0, 1.0, source.to_string_lossy()),
    ];
    open(&mut app, &project, "Base");
    join_dialog(&mut app);
    let invalid = app.click("Nested");
    assert!(
        invalid["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Include overlapping clips before joining")
    );
    assert_eq!(document(&app.click("Join")), project); // Disabled submit cannot start a job.
    assert_eq!(app.inspect()["join_pending"], false);
    app.click("Cancel");
    assert_eq!(document(&app.inspect()), project);
    join_dialog(&mut app);
    app.click("Join");
    assert_eq!(
        document(&wait_join(&mut app))
            .track(track)
            .unwrap()
            .clips
            .len(),
        1
    );
    app.quit();
}

#[test]
fn cancel_running_audio_join_retires_worker_cleans_output_and_allows_next_join() {
    let mut app = Harness::new();
    let source = app.directory.path().join("tiny-source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: vec![0.125; 256 * 2],
    }
    .write_wav(&source)
    .unwrap();
    let source_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Cancel join");
    // The virtual gap would produce 476 MiB, below the 512 MiB source cap.
    // It keeps the streamed worker busy without allocating a large fixture.
    project.track_mut(track).unwrap().clips = vec![
        Clip::audio("First tiny take", 0.0, 1.0, source.to_string_lossy()),
        Clip::audio("Distant end", 2599.0, 1.0, source.to_string_lossy()),
    ];
    open(&mut app, &project, "First tiny take");
    join_dialog(&mut app);
    let running = app.click("Join");
    assert_eq!(running["join_pending"], true, "{running}");
    assert_eq!(document(&running), project);
    let scratch = std::path::PathBuf::from(running["scratch_dir"].as_str().unwrap());
    // While a real worker is running the primary action is disabled.
    let attempted_second = app.click("Join");
    assert_eq!(attempted_second["join_pending"], true);
    assert_eq!(document(&attempted_second), project);
    let canceled = app.click("Cancel");
    assert_eq!(canceled["popups"]["join"], false);
    assert_eq!(document(&canceled), project);
    join_dialog(&mut app);
    let reopened = app.inspect();
    assert_eq!(document(&reopened), project);
    // Cancellation may already have finished by the time the menu reopens.
    // If it is still retiring, reopening must retain the occupied worker slot.
    if reopened["join_pending"] == true {
        assert_eq!(document(&app.click("Join")), project);
    }
    let retired = wait_join(&mut app);
    assert_eq!(document(&retired), project);
    let joins = scratch.join("joins");
    assert!(
        !joins.exists() || std::fs::read_dir(&joins).unwrap().next().is_none(),
        "Cancelled source or temporary output remains in {}",
        joins.display()
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    app.click("Cancel");
    project.track_mut(track).unwrap().clips[1].start = 2.0;
    open(&mut app, &project, "First tiny take");
    join_dialog(&mut app);
    app.click("Join");
    let completed = document(&wait_join(&mut app));
    let clips = &completed.track(track).unwrap().clips;
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].length, 3.0);
    let ClipContent::Audio { path, .. } = &clips[0].content else {
        unreachable!()
    };
    assert_eq!(AudioData::load_wav(path).unwrap().frames(), 72000);
    app.quit();
}

#[test]
fn join_identifies_linked_instances_by_visible_one_based_beat_ranges() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Linked sections");
    let clip = Clip::midi("Shared phrase", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let original = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let second = project.clone_clip(track, original).unwrap();
    project.clone_clip(track, second).unwrap();
    open(&mut app, &project, "Shared phrase");
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    join_dialog(&mut app);
    let state = app.inspect();
    for label in [
        "Beats 1.00 – 5.00",
        "Beats 5.00 – 9.00",
        "Beats 9.00 – 13.00",
        "Join",
        "Cancel",
    ] {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == label)
            .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
        assert!(rect["max"][1].as_f64().unwrap() < 696.0, "{rect}");
    }
    app.command(json!({"command":"click","label":"Shared phrase","occurrence":1}));
    assert!(
        app.inspect()["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Select at least two clips")
    );
    app.command(json!({"command":"click","label":"Shared phrase","occurrence":2}));
    let state = app.click("Join");
    let joined = document(&state);
    let clips = &joined.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().any(|clip| clip.id == second));
    assert!(
        clips
            .iter()
            .any(|clip| clip.length == 12.0 && clip.link_group.is_none())
    );
    app.quit();
}
