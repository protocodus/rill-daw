//! Stems are the song rendered with one channel soloed; they share one extent
//! and are staged and committed together.
use daw_core::{AudioData, Clip, Project};
use daw_engine::{
    ExportFormat, ExportOptions, ExportRange, StemRequest, WavEncoding,
    export_project_wav_with_report, export_stems, plan_stems,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const RATE: u32 = 8_000;

fn tone(path: &Path, frequency: f32, seconds: f32) {
    let frames = (RATE as f32 * seconds) as usize;
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..frames)
            .flat_map(|i| {
                let value =
                    0.3 * (i as f32 / RATE as f32 * frequency * std::f32::consts::TAU).sin();
                [value, value * 0.5]
            })
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

struct Song {
    _directory: tempfile::TempDir,
    project: Project,
    a: u64,
    b: u64,
    bus: u64,
}

/// Two tracks that also send to a reverb-less bus, plus a muted track.
fn song(seconds: f32) -> Song {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    project.name = "Song".into();
    let mut ids = Vec::new();
    for (name, frequency, gain, pan) in [
        ("Bass", 110.0, 0.8, -0.3),
        ("Lead/Keys: 1?", 440.0, 0.6, 0.4),
    ] {
        let source = directory
            .path()
            .join(format!("{name}.wav").replace(['/', ':', '?'], "_"));
        tone(&source, frequency, seconds);
        let id = project.add_audio_track(name);
        let track = project.track_mut(id).unwrap();
        track.gain = gain;
        track.pan = pan;
        track.clips.push(Clip::audio(
            name,
            0.0,
            f64::from(seconds) * 2.0,
            source.to_string_lossy(),
        ));
        ids.push(id);
    }
    let bus = project.add_bus("Return");
    project.track_mut(bus).unwrap().gain = 0.5;
    project.add_send(ids[0], bus, 0.5, false).unwrap();
    project.add_send(ids[1], bus, 0.25, true).unwrap();
    Song {
        _directory: directory,
        project,
        a: ids[0],
        b: ids[1],
        bus,
    }
}

fn request(directory: &Path, channels: Vec<u64>) -> StemRequest {
    StemRequest {
        directory: directory.to_owned(),
        song: "Song".into(),
        channels,
        include_mix: true,
        through_master: false,
        replace_existing: false,
    }
}

fn wav_options() -> ExportOptions {
    ExportOptions {
        include_effect_tails: false,
        ..Default::default()
    }
}

fn load(path: &Path) -> AudioData {
    AudioData::load_wav(path).unwrap()
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn track_stems_sum_to_the_mix_including_sends_and_share_one_length() {
    let song = song(2.0);
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let stems = export_stems(
        &song.project,
        &request(out.path(), vec![song.a, song.b]),
        RATE,
        wav_options(),
        &cancel,
    )
    .unwrap();
    assert_eq!(
        stems
            .iter()
            .map(|s| s.plan.file_name.as_str())
            .collect::<Vec<_>>(),
        [
            "Song - 00 Mix.wav",
            "Song - 01 Bass.wav",
            "Song - 02 Lead-Keys- 1-.wav"
        ]
    );
    let audio: Vec<_> = stems.iter().map(|s| load(&s.path)).collect();
    // Four beats at 120 BPM and 8 kHz.
    assert!(
        audio
            .iter()
            .all(|a| a.frames() == 16_000 && a.sample_rate == RATE)
    );
    assert!(stems.iter().all(|s| s.report.frames == 16_000));
    // The mix matches an ordinary export, and the stems add up to it (the
    // bus is linear and fed only by the selected tracks).
    let reference = out.path().join("reference.wav");
    export_project_wav_with_report(&song.project, &reference, RATE, wav_options()).unwrap();
    assert_eq!(audio[0].samples, load(&reference).samples);
    let mut worst = 0.0_f32;
    for index in 0..audio[0].samples.len() {
        let sum = audio[1].samples[index] + audio[2].samples[index];
        worst = worst.max((sum - audio[0].samples[index]).abs());
    }
    assert!(worst < 1e-6, "worst difference {worst}");
    // Each stem is audibly its own channel, send return included.
    let energy = |a: &AudioData| a.samples.iter().map(|s| s * s).sum::<f32>();
    assert!(energy(&audio[1]) > 0.0 && energy(&audio[2]) > 0.0);
    assert!(energy(&audio[1]) != energy(&audio[2]));
    assert!(files(out.path()).iter().all(|name| !name.ends_with(".tmp")));
}

#[test]
fn a_bus_stem_is_everything_routed_into_it() {
    let mut song = song(1.0);
    // Route the second track fully through its own bus.
    let group = song.project.add_bus("Group");
    song.project.set_output(song.b, Some(group)).unwrap();
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut stems_request = request(out.path(), vec![song.b, group]);
    stems_request.include_mix = false;
    let stems = export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel).unwrap();
    let (track, bus) = (load(&stems[0].path), load(&stems[1].path));
    assert_eq!(track.samples, bus.samples);
}

#[test]
fn master_processing_is_left_off_stems_unless_requested() {
    let mut song = song(1.0);
    song.project.master.gain = 0.5;
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut stems_request = request(out.path(), vec![song.a, song.b]);
    let stems = export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel).unwrap();
    let audio: Vec<_> = stems.iter().map(|s| load(&s.path)).collect();
    // The mix keeps the Master fader; the stems are taken before it.
    for index in 0..audio[0].samples.len() {
        let sum = audio[1].samples[index] + audio[2].samples[index];
        assert!((sum * 0.5 - audio[0].samples[index]).abs() < 1e-6);
    }
    let again = tempfile::tempdir().unwrap();
    stems_request.directory = again.path().to_owned();
    stems_request.through_master = true;
    let through =
        export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel).unwrap();
    let (first, second) = (load(&through[1].path), load(&through[2].path));
    for index in 0..audio[0].samples.len() {
        let sum = first.samples[index] + second.samples[index];
        assert!((sum - audio[0].samples[index]).abs() < 1e-6);
    }
}

#[test]
fn stems_share_the_tail_and_ranges_and_every_format_is_available() {
    let song = song(1.0);
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for (format, encoding) in [
        (ExportFormat::Flac, WavEncoding::Pcm24),
        (ExportFormat::VORBIS_DEFAULT, WavEncoding::Float32),
        (ExportFormat::MP3_DEFAULT, WavEncoding::Float32),
    ] {
        let directory = out.path().join(format.extension());
        std::fs::create_dir(&directory).unwrap();
        let stems = export_stems(
            &song.project,
            &request(&directory, vec![song.a, song.bus]),
            RATE,
            ExportOptions {
                format,
                encoding,
                range: Some(ExportRange {
                    start_beat: 0.5,
                    end_beat: 1.5,
                }),
                ..wav_options()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(stems.len(), 3);
        assert!(
            stems
                .iter()
                .all(|s| s.report.frames == 4_000 && s.report.format == format)
        );
        assert!(
            stems
                .iter()
                .all(|s| s.path.extension().unwrap() == format.extension())
        );
    }
}

#[test]
fn names_are_sanitised_numbered_and_never_silently_replaced() {
    let mut song = song(1.0);
    let twin = song.project.add_audio_track("Bass");
    let plans = plan_stems(
        &song.project,
        "My: Song?",
        &[twin, song.a],
        false,
        ExportFormat::Flac,
    )
    .unwrap();
    assert_eq!(
        plans
            .iter()
            .map(|p| p.file_name.as_str())
            .collect::<Vec<_>>(),
        ["My- Song- - 01 Bass.flac", "My- Song- - 02 Bass.flac"]
    );
    assert!(plan_stems(&song.project, "x", &[], false, ExportFormat::Wav).is_err());
    assert!(
        plan_stems(
            &song.project,
            "x",
            &[song.a, song.a],
            false,
            ExportFormat::Wav
        )
        .is_err()
    );
    assert!(plan_stems(&song.project, "x", &[9_999], false, ExportFormat::Wav).is_err());
    let long = "é".repeat(200);
    let plan = plan_stems(&song.project, &long, &[song.a], false, ExportFormat::Wav).unwrap();
    assert!(plan[0].file_name.len() < 255);

    song.project.track_mut(song.b).unwrap().name = "  ..  ".into();
    let out = tempfile::tempdir().unwrap();
    let existing: PathBuf = out.path().join("Song - 01 Bass.wav");
    std::fs::write(&existing, b"keep me").unwrap();
    let cancel = AtomicBool::new(false);
    let mut stems_request = request(out.path(), vec![song.a, song.b]);
    let error = export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Song - 01 Bass.wav"), "{error}");
    assert_eq!(files(out.path()), ["Song - 01 Bass.wav"]);
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
    stems_request.replace_existing = true;
    export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel).unwrap();
    assert_eq!(
        files(out.path()),
        [
            "Song - 00 Mix.wav",
            "Song - 01 Bass.wav",
            "Song - 02 Channel.wav"
        ]
    );
    assert_ne!(std::fs::read(&existing).unwrap(), b"keep me");
}

#[test]
fn cancellation_leaves_no_files_even_after_some_stems_finished() {
    let song = song(30.0);
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    // Already cancelled: nothing is created.
    cancel.store(true, Ordering::Release);
    let stems_request = request(out.path(), vec![song.a, song.b]);
    let error =
        export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel).unwrap_err();
    assert!(error.to_string().contains("cancel"));
    assert!(files(out.path()).is_empty());
    // Cancelled while rendering: stop as soon as the first staging file exists.
    cancel.store(false, Ordering::Release);
    let result = std::thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            while files(out.path()).is_empty() {
                std::thread::yield_now();
            }
            cancel.store(true, Ordering::Release);
        });
        let result = export_stems(&song.project, &stems_request, RATE, wav_options(), &cancel);
        cancel.store(true, Ordering::Release);
        watcher.join().unwrap();
        result
    });
    assert!(format!("{:#}", result.unwrap_err()).contains("cancel"));
    assert!(files(out.path()).is_empty(), "{:?}", files(out.path()));
}
