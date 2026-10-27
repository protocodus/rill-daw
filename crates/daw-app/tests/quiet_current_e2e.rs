//! A second short song exercises meter, audio-range variation and precise fades.
mod common;

use common::Harness;
use daw_core::{AudioData, ClipContent, Project};
use serde_json::{Value, json};
use std::path::Path;

const RATE: u32 = 48_000;
const PHRASE_FRAMES: usize = RATE as usize * 4;

fn menu(app: &mut Harness, name: &str, action: &str) -> Value {
    let opened = app.click(name);
    let occurrence = opened["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == action)
        .count()
        - 1;
    app.command(json!({"command":"click","label":action,"occurrence":occurrence}))
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn select(app: &mut Harness, id: u64) -> Value {
    menu(app, "Window", "Arrangement");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"];
    let (x, y) = center(rect);
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(state["selected_clip"], id);
    menu(app, "Window", "Clip Editor")
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

fn number(app: &mut Harness, rect: &Value, text: &str) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}));
    app.key("Enter")
}

fn fade(app: &mut Harness, key: &str, value: &str) -> Value {
    let state = app.key(key);
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| {
            rect["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" beats"))
        })
        .expect("precise fade duration")
        .clone();
    number(app, &rect, value)
}

fn write_phrase(path: &Path, voice: usize) {
    let mut noise = 0x745c_132d_u32;
    let samples = (0..PHRASE_FRAMES)
        .map(|frame| {
            let time = frame as f64 / f64::from(RATE);
            let beat = time * 1.5;
            let tau = std::f64::consts::TAU;
            let value = match voice {
                0 => {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    let random = f64::from(noise) / f64::from(u32::MAX) * 2.0 - 1.0;
                    let local = (beat % 1.0) / 1.5;
                    let attack = (local * 500.0).min(1.0);
                    if (beat as usize).is_multiple_of(3) {
                        (tau * (48.0 * local + 1.4 * (1.0 - (-22.0 * local).exp()))).sin()
                            * (-14.0 * local).exp()
                            * attack
                            * 0.22
                    } else {
                        random * (-38.0 * local).exp() * attack * 0.065
                    }
                }
                1 => {
                    let local = (beat % 3.0) / 1.5;
                    let frequency = if beat < 3.0 {
                        73.416_191_98
                    } else {
                        58.270_470_19
                    };
                    let envelope = (local * 70.0).min(1.0) * (-2.8 * local).exp();
                    ((tau * frequency * local).sin() + 0.15 * (tau * 2.0 * frequency * local).sin())
                        * envelope
                        * 0.17
                }
                _ => {
                    let notes = if beat < 3.0 {
                        [
                            293.664_768,
                            349.228_231,
                            440.0,
                            523.251_131,
                            440.0,
                            349.228_231,
                        ]
                    } else {
                        [
                            233.081_881,
                            293.664_768,
                            349.228_231,
                            440.0,
                            349.228_231,
                            293.664_768,
                        ]
                    };
                    let frequency = notes[((beat % 3.0) * 2.0) as usize % 6];
                    let local = (beat % 0.5) / 1.5;
                    let envelope = (local * 220.0).min(1.0) * (-8.0 * local).exp();
                    ((tau * frequency * local).sin()
                        + 0.24 * (tau * 2.0 * frequency * local).sin()
                        + 0.08 * (tau * 3.0 * frequency * local).sin())
                        * envelope
                        * 0.13
                }
            };
            value as f32
        })
        .collect();
    AudioData {
        sample_rate: RATE,
        channels: 1,
        samples,
    }
    .write_wav(path)
    .unwrap();
}

fn export(app: &mut Harness, path: &Path) -> AudioData {
    menu(app, "File", "Export WAV…");
    if app.inspect()["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn waltz_range_variation_and_fades_survive_portable_recall_and_exact_export() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.name = "Quiet Current".into();
    project.tempo = 90.0;
    project.beats_per_bar = 3;
    let seed = app.directory.path().join("Waltz template.json");
    project.save(&seed).unwrap();
    app.command(json!({"command":"open","path":seed}));
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let sources: Vec<_> = ["Brushes.wav", "Warm bass.wav", "Plucked current.wav"]
        .iter()
        .map(|name| app.directory.path().join(name))
        .collect();
    for (voice, source) in sources.iter().enumerate() {
        write_phrase(source, voice);
        app.command(json!({"command":"drop","paths":[source],"beat":0.0}));
        for _ in 0..3 {
            app.command(json!({"command":"key","key":"D","cmd":true}));
        }
        let state = app.inspect();
        let clips = state["project"]["tracks"][voice]["clips"]
            .as_array()
            .unwrap();
        assert_eq!(clips.len(), 4);
        for (phrase, clip) in clips.iter().enumerate() {
            assert_eq!(clip["start"], phrase as f64 * 6.0);
            assert_eq!(clip["length"], 6.0);
        }
    }
    let state = app.inspect();
    let brushes = state["project"]["tracks"][0]["clips"][3]["id"]
        .as_u64()
        .unwrap();
    let melody_first = state["project"]["tracks"][2]["clips"][0]["id"]
        .as_u64()
        .unwrap();
    let melody_last = state["project"]["tracks"][2]["clips"][3]["id"]
        .as_u64()
        .unwrap();
    select(&mut app, brushes);
    app.key("P");
    let state = app.click("Fit");
    let a = point(&state, 20.0);
    let b = point(&state, 21.0);
    let range = app
        .command(json!({"command":"pointer","phase":"drag","x":a.0,"y":a.1,"to_x":b.0,"to_y":b.1}));
    assert_eq!(range["audio_editor"]["range"], json!([20.0, 21.0]));
    let before = range["project"].clone();
    let variation = menu(&mut app, "Clip", "Duplicate selection");
    let added = variation["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(
        (
            added["start"].as_f64().unwrap(),
            added["length"].as_f64().unwrap()
        ),
        (21.0, 1.0)
    );
    assert_eq!(added["offset"], 2.0);
    assert_eq!(undo(&mut app, false)["project"], before);
    assert_eq!(undo(&mut app, true)["project"], variation["project"]);

    select(&mut app, melody_first);
    let before = app.inspect()["project"].clone();
    let attack = fade(&mut app, "I", "0.125");
    assert_eq!(
        attack["project"]["tracks"][2]["clips"][0]["content"]["Audio"]["fade_in"],
        0.125
    );
    assert_eq!(undo(&mut app, false)["project"], before);
    undo(&mut app, true);
    select(&mut app, melody_last);
    let before = app.inspect()["project"].clone();
    let release = fade(&mut app, "O", "1.5");
    assert_eq!(
        release["project"]["tracks"][2]["clips"][3]["content"]["Audio"]["fade_out"],
        1.5
    );
    assert_eq!(undo(&mut app, false)["project"], before);
    undo(&mut app, true);

    // Balance one voice in the existing arranger controls, retaining centered
    // brushes and bass. The final mix must contain a real stereo difference.
    let state = menu(&mut app, "Window", "Arrangement");
    let pan = &state["timeline"]["channels"].as_array().unwrap()[2]["pan"];
    let (x, y) = center(pan);
    let target = pan["min"][0].as_f64().unwrap()
        + (pan["max"][0].as_f64().unwrap() - pan["min"][0].as_f64().unwrap()) * 0.38;
    let state =
        app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":target,"to_y":y}));
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    project.validate().unwrap();
    assert_eq!(
        (project.tempo, project.beats_per_bar, project.end_beat()),
        (90.0, 3, 24.0)
    );
    assert!(project.tracks[2].pan < -0.1);
    let preview_path = app.directory.path().join("Quiet Current preview.wav");
    let preview = export(&mut app, &preview_path);
    assert_eq!(
        (preview.sample_rate, preview.channels, preview.frames()),
        (RATE, 2, RATE as usize * 16)
    );
    assert!(
        preview
            .samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() < 1.0)
    );
    for bar in preview.samples.as_chunks::<{ RATE as usize * 4 }>().0 {
        assert!(
            bar.iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>()
                > 0.5
        );
    }
    assert!(
        preview
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .any(|frame| (frame[0] - frame[1]).abs() > 0.001)
    );

    let bundle = app.directory.path().join("Quiet Current.rill");
    app.command(json!({"command":"file_dialog","kind":"save","path":bundle}));
    menu(&mut app, "File", "Save as…");
    app.command(json!({"command":"wait_project"}));
    for source in &sources {
        std::fs::remove_file(source).unwrap();
    }
    let mut app = app.restart();
    app.command(json!({"command":"file_dialog","kind":"open","path":bundle}));
    menu(&mut app, "File", "Open…");
    let restored = app.command(json!({"command":"wait_project"}));
    assert_eq!(restored["modified"], false);
    let recalled: Project = serde_json::from_value(restored["project"].clone()).unwrap();
    recalled.validate().unwrap();
    assert_eq!(
        (
            recalled.name.as_str(),
            recalled.tempo,
            recalled.beats_per_bar,
            recalled.end_beat()
        ),
        ("Quiet Current", 90.0, 3, 24.0)
    );
    assert_eq!(recalled.tracks[0].clips.len(), 5);
    for track in &recalled.tracks {
        for clip in &track.clips {
            let ClipContent::Audio { path, .. } = &clip.content else {
                panic!()
            };
            assert!(Path::new(path).is_file());
        }
    }
    let final_path = app.directory.path().join("Quiet Current.wav");
    assert_eq!(export(&mut app, &final_path).samples, preview.samples);
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        let destination = Path::new(&destination);
        std::fs::create_dir_all(destination).unwrap();
        std::fs::copy(bundle, destination.join("Quiet Current.rill")).unwrap();
        std::fs::copy(final_path, destination.join("Quiet Current.wav")).unwrap();
    }
    app.quit();
}
