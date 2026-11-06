//! Delivery choices are exercised through the running app, then verified in
//! the resulting WAV files rather than merely trusting the options dialog.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::path::Path;

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn options(app: &mut Harness) -> Value {
    app.click("File");
    let state = app.click("Export WAV…");
    assert_eq!(state["popups"]["export"], true);
    state
}

fn choose(app: &mut Harness, previous: &str, choice: &str) -> Value {
    app.click(previous);
    app.click(choice)
}

fn finish(app: &mut Harness, path: &Path) -> Value {
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
    app.click("Export…");
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["exported_file"], json!(path));
    state
}

fn unchanged(state: &Value, baseline: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "revision",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(state[field], baseline[field], "Delivery changed {field}");
    }
}

fn fixture(app: &mut Harness, loud: bool, tempo: f64, start: f64, end: f64) -> Value {
    let source = app.directory.path().join("Delivery source.wav");
    let amplitude = if loud { 2.5 } else { 0.3 };
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000 * 5)
            .flat_map(|frame| {
                let sample = (frame as f32 * 0.071).sin() * amplitude;
                [sample, sample * 0.75]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = tempo;
    project.loop_enabled = true;
    project.loop_start = start;
    project.loop_end = end;
    let track = project.add_audio_track("Delivery source");
    let track = project.track_mut(track).unwrap();
    track.gain = 1.0;
    track.latency_samples = 257;
    track.clips.push(Clip::audio(
        "Delivery phrase",
        0.0,
        8.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("Delivery.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Delivery phrase");
    app.inspect()
}

#[derive(Debug)]
struct WavHeader {
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    frames: u64,
}

fn header(path: &Path) -> WavHeader {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut position = 12;
    let mut format = None;
    let mut data_bytes = None;
    while position + 8 <= bytes.len() {
        let length =
            u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap()) as usize;
        let body = position + 8;
        assert!(body + length <= bytes.len(), "Truncated WAV chunk");
        match &bytes[position..position + 4] {
            b"fmt " => {
                assert!(length >= 16);
                let tag = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
                let tag = if tag == 0xfffe {
                    assert!(length >= 40, "Extensible WAV needs its subtype GUID");
                    u16::from_le_bytes(bytes[body + 24..body + 26].try_into().unwrap())
                } else {
                    tag
                };
                format = Some((
                    tag,
                    u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap()),
                    u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap()),
                ));
            }
            b"data" => data_bytes = Some(length as u64),
            _ => {}
        }
        position = body + length + length % 2;
    }
    let (format, channels, rate, bits) = format.expect("A WAV needs its format chunk");
    WavHeader {
        format,
        channels,
        rate,
        bits,
        frames: data_bytes.unwrap() / u64::from(channels) / u64::from(bits / 8),
    }
}

#[test]
fn pcm_and_float_delivery_use_the_chosen_rate_without_reconfiguring_playback_or_editing() {
    let mut app = Harness::new();
    let baseline = fixture(&mut app, true, 120.0, 1.0, 3.0);
    options(&mut app);
    choose(&mut app, "32-bit float", "16-bit PCM");
    choose(&mut app, "Follow audio setup", "44.1 kHz");
    app.click("Include effect tails");
    let pcm16 = app.directory.path().join("Delivery 16.wav");
    let completed = finish(&mut app, &pcm16);
    unchanged(&completed, &baseline);
    assert_eq!(completed["settings"]["sample_rate"], 48_000);
    let report = &completed["finishing"]["last_export"]["report"];
    assert!(report["sample_peak"].as_f64().unwrap() > 1.0);
    assert!(report["clipped_samples"].as_u64().unwrap() > 0);
    assert_eq!(report["dither"], true);
    assert!(
        completed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text
                .as_str()
                .is_some_and(|text| text.to_lowercase().contains("clipped"))),
        "PCM overload must be disclosed in the completed delivery result"
    );
    let wav = header(&pcm16);
    assert_eq!(
        (wav.format, wav.bits, wav.rate, wav.channels, wav.frames),
        (1, 16, 44_100, 2, 44_100 * 4)
    );
    assert!(
        AudioData::load_wav(&pcm16)
            .unwrap()
            .samples
            .iter()
            .all(|sample| (-1.0..=1.0).contains(sample))
    );

    options(&mut app);
    choose(&mut app, "16-bit PCM", "24-bit PCM");
    choose(&mut app, "44.1 kHz", "48 kHz");
    app.click("Dither (TPDF)");
    let pcm24 = app.directory.path().join("Delivery 24.wav");
    let completed = finish(&mut app, &pcm24);
    unchanged(&completed, &baseline);
    assert_eq!(
        completed["finishing"]["last_export"]["report"]["dither"],
        false
    );
    let wav = header(&pcm24);
    assert_eq!(
        (wav.format, wav.bits, wav.rate, wav.channels, wav.frames),
        (1, 24, 48_000, 2, 48_000 * 4)
    );

    options(&mut app);
    choose(&mut app, "24-bit PCM", "32-bit float");
    choose(&mut app, "48 kHz", "96 kHz");
    let float = app.directory.path().join("Delivery float.wav");
    let completed = finish(&mut app, &float);
    unchanged(&completed, &baseline);
    assert_eq!(completed["settings"]["sample_rate"], 48_000);
    assert_eq!(
        completed["finishing"]["last_export"]["report"]["clipped_samples"],
        0
    );
    let wav = header(&float);
    assert_eq!(
        (wav.format, wav.bits, wav.rate, wav.channels, wav.frames),
        (3, 32, 96_000, 2, 96_000 * 4)
    );
    assert!(
        AudioData::load_wav(&float)
            .unwrap()
            .samples
            .iter()
            .any(|sample| sample.abs() > 1.1),
        "Float delivery must preserve headroom instead of inheriting device clipping"
    );

    // Cancelling the destination picker retains the last successful delivery.
    options(&mut app);
    app.command(json!({"command":"file_dialog", "kind":"export", "path":null}));
    let cancelled = app.click("Export…");
    unchanged(&cancelled, &baseline);
    assert_eq!(cancelled["exported_file"], json!(float));
    app.quit();
}

#[test]
fn loop_delivery_is_the_exact_compensated_timeline_slice_and_never_appends_tails() {
    let mut app = Harness::new();
    let baseline = fixture(&mut app, false, 137.0, 1.125, 5.875);
    options(&mut app);
    app.click("Include effect tails");
    let full = app.directory.path().join("Reference.wav");
    unchanged(&finish(&mut app, &full), &baseline);
    let full = AudioData::load_wav(full).unwrap();

    options(&mut app);
    // Keep the tail preference on, then choose exact loop delivery. A range
    // export must ignore that preference rather than including later material.
    app.click("Include effect tails");
    choose(&mut app, "Arrangement", "Loop range");
    let output = app.directory.path().join("Exact loop.wav");
    let done = finish(&mut app, &output);
    unchanged(&done, &baseline);
    let looped = AudioData::load_wav(&output).unwrap();
    let samples_per_beat: f64 = 48_000.0 * 60.0 / 137.0;
    let start = (1.125 * samples_per_beat).round() as usize;
    let end = (5.875 * samples_per_beat).round() as usize;
    assert_eq!(looped.frames(), end - start);
    assert_eq!(looped.samples, full.samples[start * 2..end * 2]);
    assert_eq!(header(&output).frames, (end - start) as u64);
    assert!(has(&done, "Exported · Exact loop.wav"));
    app.quit();
}

fn edit_number(app: &mut Harness, prefix: &str, value: &str) -> Value {
    let state = app.inspect();
    let field = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| {
            field["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("Missing {prefix}: {}", state["visible_labels"]));
    let x = (field["min"][0].as_f64().unwrap() + field["max"][0].as_f64().unwrap()) * 0.5;
    let y = (field["min"][1].as_f64().unwrap() + field["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

#[test]
fn retry_and_reexport_retain_delivery_settings_and_range_while_only_reexport_uses_later_edits() {
    let mut app = Harness::new();
    fixture(&mut app, false, 120.0, 1.0, 3.0);
    options(&mut app);
    choose(&mut app, "32-bit float", "24-bit PCM");
    choose(&mut app, "Follow audio setup", "44.1 kHz");
    choose(&mut app, "Arrangement", "Loop range");
    app.click("Dither (TPDF)");
    let directory = app.directory.path().join("Missing parent");
    let output = directory.join("Retained delivery.wav");
    app.command(json!({"command":"file_dialog", "kind":"export", "path":output}));
    app.click("Export…");
    app.raw_command(json!({"command":"wait_export"}));
    let failed = app.inspect();
    assert!(has(&failed, "Retry export"));
    assert_eq!(failed["export_run"]["sample_rate"], 44_100);

    // Change both musical timing and future delivery preferences through the
    // UI. The retained retry must remain the reviewed failed mix/specification.
    app.click("Range");
    edit_number(&mut app, "End beat ", "6");
    edit_number(&mut app, "Start beat ", "5");
    app.key("Escape");
    let state = app.inspect();
    let tempo = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| {
            field["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" BPM"))
        })
        .unwrap();
    let prefix = tempo["text"].as_str().unwrap().to_owned();
    edit_number(&mut app, &prefix, "60");
    app.click("Delivery phrase");
    options(&mut app);
    choose(&mut app, "24-bit PCM", "16-bit PCM");
    choose(&mut app, "44.1 kHz", "96 kHz");
    choose(&mut app, "Loop range", "Arrangement");
    app.click("Dither (TPDF)");
    let edited = app.click("Cancel");
    assert_eq!(edited["project"]["tempo"], 60.0);
    assert_eq!(edited["project"]["loop_start"], 4.0);
    assert_eq!(edited["project"]["loop_end"], 5.0);
    let preferences = edited["settings"]["export"].clone();

    std::fs::create_dir(&directory).unwrap();
    app.command(json!({"command":"file_dialog", "kind":"export", "path":null}));
    app.click("Retry export");
    let retried = app.command(json!({"command":"wait_export"}));
    unchanged(&retried, &edited);
    assert_eq!(
        retried["file_dialog_pending"], true,
        "Retry never reopens a picker"
    );
    let wav = header(&output);
    assert_eq!((wav.rate, wav.bits, wav.frames), (44_100, 24, 44_100));
    assert_eq!(retried["settings"]["export"], preferences);
    assert_eq!(retried["finishing"]["last_export"]["dither"], false);

    app.click("File");
    app.click("Re-export last WAV (replace)");
    let reexported = app.command(json!({"command":"wait_export"}));
    unchanged(&reexported, &edited);
    assert_eq!(reexported["settings"]["export"], preferences);
    assert_eq!(reexported["file_dialog_pending"], true);
    assert_eq!(reexported["finishing"]["last_export"]["dither"], false);
    let wav = header(&output);
    assert_eq!(
        (wav.rate, wav.bits, wav.frames),
        (44_100, 24, 44_100 * 2),
        "Re-export uses current tempo with the previous captured 1–3 beat range"
    );
    app.quit();
}
