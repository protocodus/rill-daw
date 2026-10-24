mod common;

use common::Harness;
use daw_core::{AudioData, ClipContent, Project, TrackKind};
use serde_json::json;
use std::path::Path;

#[test]
fn launch_and_quit_without_a_display_or_audio_device() {
    let mut app = Harness::new();
    let state = app.inspect();
    assert_eq!(state["project"]["name"], "Untitled session");
    assert_eq!(state["export_pending"], false);
    assert_eq!(state["error"], false);
    assert!(
        !app.directory
            .path()
            .join("scratch/recovery.daw.json")
            .exists()
    );
    app.quit();
}

#[test]
fn file_menu_quit_closes_the_application() {
    let mut app = Harness::new();
    app.click("File");
    let response = app.raw_command(json!({"command": "click", "label": "Quit"}));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["event"], "exiting", "{response}");
    app.wait_exit();
}

#[test]
fn five_audio_tracks_drop_export_stereo_song_and_exit() {
    let mut app = Harness::new();
    let durations = [0.35, 0.6, 0.9, 1.25, 1.7];
    let rates = [44_100, 48_000, 32_000, 48_000, 22_050];
    let starts = [0.0, 0.5, 1.0, 1.5, 2.0];
    for index in 0..5 {
        app.click("Track");
        app.click("Add audio track");
        let file = app.directory.path().join(format!("source-{index}.wav"));
        write_audio(
            &file,
            rates[index],
            durations[index],
            index % 2 == 0,
            200.0 * (index + 1) as f32,
        );
        let state = app.command(
            json!({"command": "drop", "paths": [file], "track": index, "beat": starts[index]}),
        );
        let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
        assert_eq!(project.tracks.len(), index + 1);
        let track = &project.tracks[index];
        assert_eq!(track.kind, TrackKind::Audio);
        assert_eq!(track.clips.len(), 1);
        let clip = &track.clips[0];
        assert!((clip.start - starts[index]).abs() < 1e-9);
        assert!((clip.length - durations[index] * project.tempo / 60.0).abs() < 0.0001);
        assert!(matches!(clip.content, ClipContent::Audio { .. }));
    }
    let before = app.inspect();
    let project: Project = serde_json::from_value(before["project"].clone()).unwrap();
    project.validate().unwrap();
    assert!(project.tracks.iter().all(|track| track.clips.len() == 1));
    let destination = app.directory.path().join("song.wav");
    app.command(json!({"command": "export", "path": destination}));
    let state = app.command(json!({"command": "wait_export"}));
    assert_eq!(state["export_pending"], false);
    assert!(state["status"].as_str().unwrap().starts_with("Exported "));
    let audio = AudioData::load_wav(&destination).unwrap();
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.sample_rate, 48_000);
    // Full-song export includes its documented 200 ms minimum release tail.
    let expected = project.end_beat() * 60.0 / project.tempo + 0.2;
    assert!((audio.duration_seconds() - expected).abs() <= 1.0 / 48_000.0);
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    let mut channel_peaks = [0.0_f32; 2];
    let mut stereo_difference = 0.0_f32;
    for frame in audio.samples.chunks_exact(2) {
        for channel in 0..2 {
            channel_peaks[channel] = channel_peaks[channel].max(frame[channel].abs());
        }
        stereo_difference = stereo_difference.max((frame[0] - frame[1]).abs());
    }
    assert!(channel_peaks.iter().all(|peak| *peak > 0.01));
    assert!(
        stereo_difference > 0.005,
        "Stereo source channels must survive the mix"
    );
    // Each source has a distinct frequency. Correlate a steady 100 ms window
    // with that tone so the suite fails if any one of the five tracks is silent.
    for (index, beat) in starts.iter().enumerate() {
        let start =
            ((beat * 60.0 / project.tempo + 0.1) * f64::from(audio.sample_rate)).round() as usize;
        let count = audio.sample_rate as usize / 10;
        let frequency = 200.0 * (index + 1) as f64;
        let (mut real, mut imaginary) = (0.0, 0.0);
        for frame in 0..count {
            let phase =
                std::f64::consts::TAU * frequency * frame as f64 / f64::from(audio.sample_rate);
            let sample = f64::from(audio.samples[(start + frame) * 2]);
            real += sample * phase.cos();
            imaginary += sample * phase.sin();
        }
        let amplitude = 2.0 * real.hypot(imaginary) / count as f64;
        assert!(
            amplitude > 0.025,
            "Track {index}'s {frequency} Hz tone is missing: amplitude {amplitude}"
        );
    }
    assert!(
        audio.samples[audio.samples.len() - 4800..]
            .iter()
            .all(|sample| sample.abs() < 1e-6),
        "Export must include trailing silence"
    );
    assert_float_wav(&destination);
    let exiting = app.raw_command(json!({"command": "quit"}));
    assert_eq!(exiting["ok"], true, "{exiting}");
    assert_eq!(exiting["event"], "exiting");
    let recovered = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(
        recovered, project,
        "Closing must preserve the edited session in isolated recovery storage"
    );
    app.wait_exit();
}

#[test]
fn dropping_into_empty_arrangement_creates_audio_tracks() {
    let mut app = Harness::new();
    let files: Vec<_> = [0.25, 0.75]
        .into_iter()
        .enumerate()
        .map(|(index, duration)| {
            let file = app.directory.path().join(format!("new-track-{index}.wav"));
            write_audio(&file, 48_000, duration, true, 220.0);
            file
        })
        .collect();
    let state = app.command(json!({"command": "drop", "paths": files, "beat": 1.5}));
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    assert_eq!(project.tracks.len(), 2);
    for track in &project.tracks {
        assert_eq!(track.kind, TrackKind::Audio);
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].start, 1.5);
    }
    app.quit();
}

fn write_audio(path: &Path, sample_rate: u32, duration: f64, stereo: bool, frequency: f32) {
    let channels = if stereo { 2 } else { 1 };
    let frames = (duration * f64::from(sample_rate)).round() as usize;
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        let phase = frame as f32 / sample_rate as f32 * std::f32::consts::TAU;
        samples.push((phase * frequency).sin() * 0.12);
        if stereo {
            samples.push((phase * (frequency + 80.0)).sin() * 0.08);
        }
    }
    AudioData {
        sample_rate,
        channels,
        samples,
    }
    .write_wav(path)
    .unwrap();
}

fn assert_float_wav(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut offset = 12;
    loop {
        assert!(offset + 8 <= bytes.len(), "Missing WAV format chunk");
        let length = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        if &bytes[offset..offset + 4] == b"fmt " {
            assert!(length >= 16 && start + length <= bytes.len());
            let format = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap());
            assert_eq!(
                u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()),
                32
            );
            if format == 0xfffe {
                assert!(length >= 40);
                assert_eq!(
                    u16::from_le_bytes(bytes[start + 24..start + 26].try_into().unwrap()),
                    3
                );
            } else {
                assert_eq!(format, 3, "Expected IEEE floating-point WAV");
            }
            break;
        }
        offset = start + length + length % 2;
    }
}
