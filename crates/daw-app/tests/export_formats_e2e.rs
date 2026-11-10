//! FLAC, Ogg Vorbis, MP3 and stem exports are chosen in the real Export
//! dialog, written by the running app and verified by decoding the files.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, MAX_IMPORTED_AUDIO_BYTES, Project, decode_audio_to_wav};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const RATE: u32 = 48_000;

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn has_prefix(state: &Value, prefix: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text.as_str().is_some_and(|text| text.starts_with(prefix)))
}

/// Failure context without the whole diagnostic state.
fn labels(state: &Value) -> String {
    state["visible_labels"].to_string()
}

/// A song named "Stem song" with Drums, Bass and Keys (two seconds each).
fn open_song(app: &mut Harness, master_gain: f32) -> Value {
    let mut project = Project::new();
    for (name, frequency, pan) in [
        ("Drums", 110.0_f32, -0.5),
        ("Bass", 220.0, 0.0),
        ("Keys", 660.0, 0.5),
    ] {
        let source = app.directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: RATE,
            channels: 2,
            samples: (0..RATE as usize * 2)
                .flat_map(|i| {
                    let value =
                        0.25 * (i as f32 / RATE as f32 * frequency * std::f32::consts::TAU).sin();
                    [value, value * 0.8]
                })
                .collect(),
        }
        .write_wav(&source)
        .unwrap();
        let id = project.add_audio_track(name);
        let track = project.track_mut(id).unwrap();
        track.gain = 0.9;
        track.pan = pan;
        track
            .clips
            .push(Clip::audio(name, 0.0, 4.0, source.to_string_lossy()));
    }
    project.master.gain = master_gain;
    let path = app.directory.path().join("Stem song.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.inspect()
}

fn dialog(app: &mut Harness) -> Value {
    app.click("File");
    let state = app.click("Export WAV…");
    assert_eq!(state["popups"]["export"], true);
    state
}

fn choose(app: &mut Harness, current: &str, choice: &str) -> Value {
    app.click(current);
    app.click(choice)
}

fn export_file(app: &mut Harness, path: &Path) -> Value {
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}))
}

fn decoded(app: &Harness, path: &Path) -> AudioData {
    let wav = app.directory.path().join(format!(
        "decoded-{}.wav",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&wav);
    decode_audio_to_wav(path, &wav, MAX_IMPORTED_AUDIO_BYTES).unwrap();
    AudioData::load_wav(wav).unwrap()
}

#[test]
fn the_mix_exports_as_flac_ogg_vorbis_and_mp3_with_their_own_settings() {
    let mut app = Harness::new();
    open_song(&mut app, 1.0);

    // The WAV render is the reference for every other file type.
    dialog(&mut app);
    choose(&mut app, "32-bit float", "24-bit PCM");
    app.click("Dither (TPDF)");
    let wav = app.directory.path().join("reference.wav");
    export_file(&mut app, &wav);
    let reference = AudioData::load_wav(&wav).unwrap();
    assert_eq!(reference.frames(), RATE as usize * 2 + RATE as usize / 5);

    // FLAC is lossless: same depth, bit-exact.
    let state = dialog(&mut app);
    assert!(has(&state, "48 kHz · Stereo · 24-bit PCM"));
    let state = choose(&mut app, "WAV", "FLAC");
    assert!(
        has(&state, "48 kHz · Stereo · 24-bit FLAC"),
        "{}",
        labels(&state)
    );
    let flac = app.directory.path().join("mix");
    let state = export_file(&mut app, &flac);
    // The extension follows the type.
    let flac = flac.with_extension("flac");
    assert_eq!(state["exported_file"], json!(flac));
    assert!(has(&state, "Exported · mix.flac"));
    assert_eq!(state["finishing"]["last_export"]["format"], json!("Flac"));
    assert_eq!(decoded(&app, &flac).samples, reference.samples);

    // Ogg Vorbis follows the mix closely at a high quality.
    dialog(&mut app);
    choose(&mut app, "FLAC", "Ogg Vorbis");
    let state = choose(&mut app, "5 · ≈160 kbit/s", "4 · ≈128 kbit/s");
    assert!(
        has(&state, "48 kHz · Stereo · Ogg Vorbis · quality 4"),
        "{}",
        labels(&state)
    );
    let ogg = app.directory.path().join("mix.ogg");
    export_file(&mut app, &ogg);
    let vorbis = decoded(&app, &ogg);
    assert_eq!(vorbis.frames(), reference.frames());
    assert_eq!((vorbis.sample_rate, vorbis.channels), (RATE, 2));
    let error: f64 = vorbis
        .samples
        .iter()
        .zip(&reference.samples)
        .map(|(a, b)| f64::from(a - b).powi(2))
        .sum();
    let signal: f64 = reference
        .samples
        .iter()
        .map(|a| f64::from(*a).powi(2))
        .sum();
    assert!(10.0 * (error / signal).log10() < -15.0);

    // MP3 has a bitrate, and refuses sample rates it cannot carry.
    dialog(&mut app);
    choose(&mut app, "Ogg Vorbis", "MP3");
    let state = choose(&mut app, "192 kbit/s", "256 kbit/s");
    assert!(has(&state, "48 kHz · Stereo · MP3 · 256 kbit/s"));
    let state = choose(&mut app, "Follow audio setup", "96 kHz");
    assert!(
        has_prefix(&state, "MP3 supports sample rates up to 48 kHz"),
        "{}",
        labels(&state)
    );
    app.command(
        json!({"command":"file_dialog","kind":"export","path":app.directory.path().join("never")}),
    );
    let refused = app.click("Export…");
    assert_eq!(refused["export_pending"], false);
    assert!(!app.directory.path().join("never.mp3").exists());
    choose(&mut app, "96 kHz", "Follow audio setup");
    let mp3 = app.directory.path().join("mix.mp3");
    let state = export_file(&mut app, &mp3);
    assert!(has(&state, "Exported · mix.mp3"));
    let lossy = decoded(&app, &mp3);
    assert_eq!((lossy.sample_rate, lossy.channels), (RATE, 2));
    assert!(lossy.frames().abs_diff(reference.frames()) <= 2_304);
    assert!(lossy.samples.iter().any(|sample| sample.abs() > 0.1));
    // The next launch of the dialog remembers the choice.
    let state = dialog(&mut app);
    assert!(has(&state, "48 kHz · Stereo · MP3 · 256 kbit/s"));
    app.quit();
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn stems(app: &mut Harness, folder: &Path, replace: Option<bool>) -> Value {
    app.command(json!({"command":"file_dialog","kind":"export_stems","path":folder,"replace_existing":replace}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}))
}

#[test]
fn stems_are_chosen_with_the_mouse_and_line_up_with_the_mix() {
    let mut app = Harness::new();
    open_song(&mut app, 0.5);
    let folder = app.directory.path().join("Stems");
    std::fs::create_dir(&folder).unwrap();

    let state = dialog(&mut app);
    assert!(
        !has(&state, "Through Master processing"),
        "stem options belong to Stems"
    );
    let state = app.click("Stems");
    assert_eq!(state["export_preferences"]["what"], "stems");
    assert!(has(&state, "3 of 3"));
    assert_eq!(state["export_stem_selection"].as_array().unwrap().len(), 3);
    assert!(has(&state, "Through Master processing"));

    // Select none leaves nothing to export; the dialog says so and cannot start.
    let state = app.click("Select none");
    assert!(has(&state, "0 of 3"));
    assert!(has(
        &state,
        "Choose at least one channel or include the full mix"
    ));
    app.command(json!({"command":"file_dialog","kind":"export_stems","path":folder}));
    assert_eq!(app.click("Export…")["export_pending"], false);
    assert!(names(&folder).is_empty());
    app.click("Select all");
    let state = app.click("Drums");
    assert!(has(&state, "2 of 3"));
    app.click("Include full mix");

    let state = stems(&mut app, &folder, None);
    assert_eq!(state["exported_stems"], 3);
    assert!(has(&state, "Exported · 3 stems · Stems"));
    assert_eq!(
        names(&folder),
        [
            "Stem song - 00 Mix.wav",
            "Stem song - 01 Bass.wav",
            "Stem song - 02 Keys.wav"
        ]
    );
    let load = |name: &str| AudioData::load_wav(folder.join(name)).unwrap();
    let (mix, bass, keys) = (
        load("Stem song - 00 Mix.wav"),
        load("Stem song - 01 Bass.wav"),
        load("Stem song - 02 Keys.wav"),
    );
    // Two seconds of audio plus the tail every file shares.
    assert_eq!(mix.frames(), RATE as usize * 2 + RATE as usize / 5);
    assert_eq!((bass.frames(), keys.frames()), (mix.frames(), mix.frames()));
    // Stems are taken before Master (gain 0.5), the mix after it: the bass
    // stem alone is louder than the whole mix scaled by the Master fader.
    let peak = |audio: &AudioData| {
        audio
            .samples
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()))
    };
    assert!(peak(&bass) > 0.1 && peak(&keys) > 0.1);
    assert!(peak(&mix) < peak(&bass) + peak(&keys));

    // Existing files are only replaced after a decision; none is a cancellation.
    let before: Vec<_> = names(&folder)
        .iter()
        .map(|n| {
            std::fs::metadata(folder.join(n))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    dialog(&mut app);
    let state = stems(&mut app, &folder, None);
    assert_eq!(state["export_pending"], false);
    let after: Vec<_> = names(&folder)
        .iter()
        .map(|n| {
            std::fs::metadata(folder.join(n))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    assert_eq!(before, after);

    // Replacing, now through Master and as FLAC into a fresh folder.
    let second = app.directory.path().join("Through Master");
    std::fs::create_dir(&second).unwrap();
    dialog(&mut app);
    app.click("Through Master processing");
    choose(&mut app, "WAV", "FLAC");
    let state = stems(&mut app, &second, Some(true));
    assert_eq!(state["exported_stems"], 4);
    assert_eq!(
        names(&second),
        [
            "Stem song - 00 Mix.flac",
            "Stem song - 01 Drums.flac",
            "Stem song - 02 Bass.flac",
            "Stem song - 03 Keys.flac"
        ]
    );
    let load = |name: &str| decoded(&app, &second.join(name));
    let (mix, drums, bass, keys) = (
        load("Stem song - 00 Mix.flac"),
        load("Stem song - 01 Drums.flac"),
        load("Stem song - 02 Bass.flac"),
        load("Stem song - 03 Keys.flac"),
    );
    // Through Master, 24-bit FLAC stems add up to the mix within one step.
    for index in 0..mix.samples.len() {
        let sum = drums.samples[index] + bass.samples[index] + keys.samples[index];
        assert!((sum - mix.samples[index]).abs() < 1e-5);
    }
    // Re-export offers the same stems explicitly as a replacement.
    app.click("File");
    let state = app.click("Re-export last stems (replace)");
    assert_eq!(state["export_pending"], true);
    app.command(json!({"command":"wait_export"}));
    assert_eq!(names(&second).len(), 4);
    app.quit();
}

#[test]
fn cancelling_a_running_stem_export_leaves_no_files() {
    let mut app = Harness::with_export_delay(5_000);
    open_song(&mut app, 1.0);
    let folder = app.directory.path().join("Cancelled");
    std::fs::create_dir(&folder).unwrap();
    dialog(&mut app);
    app.click("Stems");
    app.command(json!({"command":"file_dialog","kind":"export_stems","path":folder}));
    let running = app.click("Export…");
    assert_eq!(running["export_pending"], true);
    let state = app.click("Cancel export");
    let state = if state["export_pending"] == true {
        app.command(json!({"command":"wait_export"}))
    } else {
        state
    };
    assert_eq!(state["export_pending"], false);
    assert_eq!(state["status"], "Export canceled · No files written");
    assert_eq!(state["error"], false);
    assert!(names(&folder).is_empty());
    assert!(!has(&state, "Retry export"));
    app.quit();
}

fn application(app: &mut Harness, arguments: Value) -> Value {
    app.command(
        json!({"command":"application_tool","name":"document_action","arguments":arguments}),
    )
}

#[test]
fn the_application_tool_exports_formats_and_stems_and_lists_the_files() {
    let mut app = Harness::new();
    let song = open_song(&mut app, 1.0);
    let tracks = song["project"]["tracks"].as_array().unwrap();
    let ids: Vec<u64> = tracks.iter().map(|t| t["id"].as_u64().unwrap()).collect();

    let flac = app.directory.path().join("tool.flac");
    let result = application(
        &mut app,
        json!({"action":"export","path":flac,"format":"flac","bit_depth":16}),
    );
    assert_eq!(result["tool_result"]["ok"], true, "{result}");
    assert_eq!(result["tool_result"]["result"]["files"], json!([flac]));
    app.command(json!({"command":"wait_export"}));
    assert_eq!(
        decoded(&app, &flac).frames(),
        RATE as usize * 2 + RATE as usize / 5
    );

    // Invalid combinations are rejected before anything starts.
    for bad in [
        json!({"action":"export","path":app.directory.path().join("x.mp3"),"format":"mp3","sample_rate":96000}),
        json!({"action":"export","path":app.directory.path().join("x.flac"),"format":"flac","bit_depth":32}),
        json!({"action":"export","path":app.directory.path().join("x.wav"),"quality":3}),
        json!({"action":"export","path":app.directory.path().join("x.wav"),"channels":[1]}),
        json!({"action":"export","path":app.directory.path().join("missing"),"stems":true}),
        json!({"action":"export","path":app.directory.path(),"stems":true,"channels":[999]}),
    ] {
        let result = application(&mut app, bad.clone());
        assert_eq!(
            result["tool_result"]["ok"], false,
            "{bad}: {}",
            result["tool_result"]
        );
    }

    let folder = app.directory.path().join("Tool stems");
    std::fs::create_dir(&folder).unwrap();
    let result = application(
        &mut app,
        json!({"action":"export","path":folder,"stems":true,"format":"ogg","quality":6,
               "channels":[ids[1], ids[2]],"include_mix":true,"include_tails":false}),
    );
    assert_eq!(result["tool_result"]["ok"], true, "{result}");
    let listed: Vec<PathBuf> = result["tool_result"]["result"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f.as_str().unwrap()))
        .collect();
    assert_eq!(
        listed
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        [
            "Stem song - 00 Mix.ogg",
            "Stem song - 01 Bass.ogg",
            "Stem song - 02 Keys.ogg"
        ]
    );
    app.command(json!({"command":"wait_export"}));
    let status = application(
        &mut app,
        json!({"action":"export","path":folder,"stems":true,"format":"ogg","quality":6,
               "channels":[ids[1], ids[2]],"include_mix":true,"include_tails":false}),
    );
    // Existing files are never replaced unless asked.
    assert_eq!(status["tool_result"]["ok"], false);
    assert_eq!(names(&folder).len(), 3);
    for file in &listed {
        let audio = decoded(&app, file);
        assert_eq!(audio.frames(), RATE as usize * 2);
    }
    let before_replace = std::fs::metadata(&listed[1]).unwrap().len();
    // Replacing is explicit.
    let again = application(
        &mut app,
        json!({"action":"export","path":folder,"stems":true,"format":"ogg","channels":[ids[1]],
               "include_tails":false,"replace_existing":true}),
    );
    assert_eq!(again["tool_result"]["ok"], true, "{}", again["tool_result"]);
    app.command(json!({"command":"wait_export"}));
    assert_eq!(names(&folder).len(), 3);
    assert_ne!(
        std::fs::metadata(&listed[1]).unwrap().len(),
        before_replace,
        "the replaced file is the new, smaller encoding"
    );
    let document =
        app.command(json!({"command":"application_tool","name":"document_status","arguments":{}}));
    assert_eq!(
        document["tool_result"]["result"]["exported_files"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.quit();
}
