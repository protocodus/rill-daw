//! Consolidation tools run against the real song, preserving output and undo.
mod common;
use common::Harness;
use daw_core::{AudioCrossfade, AudioData, Clip, ClipContent, CrossfadeCurve, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn open(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("tool-join.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}

fn args(state: &Value, track: u64, clips: &[u64]) -> Value {
    json!({"session_id":state["song_session"]["id"],"expected_revision":state["song_session"]["revision"],"track_id":track,"clip_ids":clips,"name":"Consolidated"})
}

fn join(app: &mut Harness, args: Value) -> Value {
    app.command(json!({"command":"song_tool","name":"join_clips","arguments":args}))
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn assert_same_audio(before: &AudioData, after: &AudioData) {
    assert_eq!(before.sample_rate, after.sample_rate);
    assert_eq!(before.samples.len(), after.samples.len());
    let difference = before
        .samples
        .iter()
        .zip(&after.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0_f32, f32::max);
    assert!(difference < 3e-6, "maximum sample difference: {difference}");
}

type AudioFixture = (Project, u64, Vec<u64>, Vec<(std::path::PathBuf, Vec<u8>)>);

fn audio_fixture(app: &mut Harness, long: bool) -> AudioFixture {
    let mut sources = Vec::new();
    for second in [false, true] {
        let path = app
            .directory
            .path()
            .join(if second { "second.wav" } else { "first.wav" });
        AudioData {
            sample_rate: 48_000,
            channels: 2,
            samples: (0..if long { 128 } else { 96_000 })
                .flat_map(|frame| {
                    let value = if second {
                        0.35 - frame as f32 / 2_000_000.0
                    } else {
                        0.1 + frame as f32 / 1_000_000.0
                    };
                    [value, -value * 0.5]
                })
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        sources.push((path.clone(), std::fs::read(path).unwrap()));
    }
    let mut project = Project::new();
    let track = project.add_audio_track("Audio joins");
    let mut a = Clip::audio(
        "First",
        if long { 0.0 } else { 1.0 },
        if long { 1.0 } else { 3.0 },
        sources[0].0.to_string_lossy(),
    );
    let mut b = Clip::audio(
        "Second",
        if long { 2599.0 } else { 2.5 },
        if long { 1.0 } else { 3.0 },
        sources[1].0.to_string_lossy(),
    );
    if !long {
        a.offset = 0.25;
        b.offset = 0.5;
        if let ClipContent::Audio {
            gain,
            fade_in,
            fade_out,
            ..
        } = &mut a.content
        {
            *gain = 0.75;
            *fade_in = 0.2;
            *fade_out = 0.25;
        }
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
                duration: 0.5,
                curve: CrossfadeCurve::SMOOTH,
            });
        }
    }
    let ids = vec![a.id, b.id];
    let channel = project.track_mut(track).unwrap();
    channel.gain = 0.5;
    channel.pan = -0.25;
    channel.clips = vec![a, b];
    (project, track, ids, sources)
}

#[test]
fn audio_join_tool_preserves_pcm_single_undo_and_portable_project() {
    let mut app = Harness::new();
    let (project, track, ids, sources) = audio_fixture(&mut app, false);
    let before = open(&mut app, &project);
    let baseline = export(&mut app, "before.wav");
    let joined = join(&mut app, args(&before, track, &ids));
    assert_eq!(joined["tool_result"]["ok"], true, "{joined}");
    assert_eq!(joined["history"]["undo"].as_array().unwrap().len(), 1);
    let channel = &joined["project"]["tracks"][0];
    assert_eq!(channel["gain"], before["project"]["tracks"][0]["gain"]);
    assert_eq!(channel["pan"], before["project"]["tracks"][0]["pan"]);
    assert_eq!(channel["clips"].as_array().unwrap().len(), 1);
    let clip = &channel["clips"][0];
    assert_eq!(clip["name"], "Consolidated");
    assert_eq!(clip["start"], 1.0);
    assert_eq!(clip["length"], 4.5);
    assert_eq!(clip["content"]["Audio"]["gain"], 1.0);
    assert_eq!(clip["content"]["Audio"]["reverse"], false);
    let derived = std::path::PathBuf::from(clip["content"]["Audio"]["path"].as_str().unwrap());
    assert!(derived.exists());
    assert!(
        !joined["tool_result"]
            .to_string()
            .contains(derived.to_str().unwrap())
    );
    assert_same_audio(&baseline, &export(&mut app, "joined.wav"));
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    assert!(
        derived.exists(),
        "Undo must preserve the source needed by Redo"
    );
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], joined["project"]);
    let archive = app.directory.path().join("joined-portable.rill");
    app.command(json!({"command":"save","path":archive}));
    std::fs::remove_file(&derived).unwrap();
    let reopened = app.command(json!({"command":"open","path":archive}));
    assert_eq!(
        reopened["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_same_audio(&baseline, &export(&mut app, "reopened.wav"));
    for (path, bytes) in sources {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    app.quit();
}

#[test]
fn midi_join_tool_preserves_visible_note_windows_and_rejects_bad_or_stale_requests() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("MIDI joins");
    let mut a = Clip::midi(
        "Trimmed",
        1.0,
        2.0,
        vec![Note::new(60, 0.5, 1.5, 90), Note::new(62, 2.5, 1.0, 100)],
    );
    a.offset = 1.0;
    let b = Clip::midi("Later", 4.0, 2.0, vec![Note::new(69, 0.5, 3.0, 110)]);
    let ids = vec![a.id, b.id];
    project.track_mut(track).unwrap().clips = vec![a, b];
    let before = open(&mut app, &project);
    for bad in [vec![ids[0]], vec![ids[0], ids[0]], vec![ids[0], u64::MAX]] {
        let failed = join(&mut app, args(&before, track, &bad));
        assert_eq!(failed["tool_result"]["ok"], false, "{failed}");
        assert_eq!(failed["project"], before["project"]);
        assert_eq!(failed["history"], before["history"]);
    }
    let joined = join(&mut app, args(&before, track, &ids));
    assert_eq!(joined["tool_result"]["ok"], true, "{joined}");
    let notes = joined["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    let visible: Vec<_> = notes
        .iter()
        .map(|note| {
            (
                note["pitch"].as_u64().unwrap(),
                note["start"].as_f64().unwrap(),
                note["length"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        visible,
        vec![(60, 0.0, 1.0), (62, 1.5, 0.5), (69, 3.5, 1.5)]
    );
    assert_eq!(joined["history"]["undo"].as_array().unwrap().len(), 1);
    let stale = join(&mut app, args(&before, track, &ids));
    assert_eq!(stale["tool_result"]["error"]["code"], "conflict");
    assert_eq!(stale["project"], joined["project"]);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.quit();
}

fn enter_prompt(app: &mut Harness, state: &Value) {
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":"Join the two audio clips."}));
}

#[test]
fn cancelled_or_stale_assistant_audio_join_cleans_derived_sources() {
    for stale in [false, true] {
        let mut app = Harness::new();
        let (project, track, ids, sources) = audio_fixture(&mut app, true);
        let before = open(&mut app, &project);
        app.command(json!({"command":"assistant_script","turns":[{"calls":[{"id":"join","name":"join_clips","arguments":args(&before,track,&ids)}]},{"text":"Joined."}]}));
        app.click("Window");
        let opened = app.click("Assistant");
        enter_prompt(&mut app, &opened);
        app.click("Send");
        let deadline = Instant::now() + Duration::from_secs(5);
        let running = loop {
            let state = app.inspect();
            if state["tool_join_pending"] == true {
                break state;
            }
            assert!(Instant::now() < deadline, "Join did not start: {state}");
        };
        assert_eq!(running["project"], before["project"]);
        let expected = if stale {
            app.click("Track");
            app.click("Add audio track")
        } else {
            app.click("Stop")
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let retired = loop {
            let state = app.inspect();
            if state["tool_join_pending"] == false {
                break state;
            }
            assert!(Instant::now() < deadline, "Join did not retire: {state}");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(retired["project"], expected["project"]);
        assert_eq!(retired["history"], expected["history"]);
        let joins =
            std::path::PathBuf::from(retired["scratch_dir"].as_str().unwrap()).join("joins");
        assert!(
            !joins.exists() || std::fs::read_dir(&joins).unwrap().next().is_none(),
            "Cancelled files remain in {}",
            joins.display()
        );
        for (path, bytes) in sources {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        app.quit();
    }
}
