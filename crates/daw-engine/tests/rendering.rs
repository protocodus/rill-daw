use daw_core::{
    AudioData, Clip, ClipContent, Effect, EffectSlot, Note, Project, Send, Track, TrackKind,
};
use daw_engine::{CompensationDelay, export_project_wav, export_wav, render};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEST_WAV: AtomicU64 = AtomicU64::new(0);

struct TestWav(PathBuf);
impl TestWav {
    fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Clock resolution can be coarser than nanoseconds, so parallel tests
        // also need a process-local counter to keep their input files distinct.
        let sequence = NEXT_TEST_WAV.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "daw-engine-{}-{nonce}-{sequence}.wav",
            std::process::id()
        ));
        AudioData {
            samples,
            sample_rate,
            channels: 1,
        }
        .write_wav(&path)
        .unwrap();
        Self(path)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for TestWav {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn empty_project() -> Project {
    let mut project = Project::new();
    project.tracks.clear();
    project.tempo = 120.0;
    project.loop_enabled = false;
    project
}

fn midi_track() -> Track {
    let mut track = Track::new("Piano", TrackKind::Midi);
    track.gain = 1.0;
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        2.0,
        vec![Note::new(69, 0.0, 1.0, 100)],
    ));
    track
}

#[test]
fn built_in_instrument_is_nonzero_finite_and_ends() {
    let mut project = empty_project();
    project.tracks.push(midi_track());
    let audio = render(&project, 8_000, 2.0).unwrap();
    assert_eq!(audio.len(), 16_000);
    assert!(audio.iter().all(|sample| sample.is_finite()));
    assert!(audio.iter().any(|sample| sample.abs() > 0.01));
    assert!(audio[10_000..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn nle_audio_offset_is_used_and_source_is_not_modified() {
    let mut source = vec![0.1; 4_000];
    source.extend(vec![0.6; 4_000]);
    let wav = TestWav::new(source, 8_000);
    let before = std::fs::read(&wav.0).unwrap();
    let mut project = empty_project();
    let mut track = Track::new("Audio", TrackKind::Audio);
    track.gain = 1.0;
    let mut clip = Clip::audio("Trimmed", 1.0, 1.0, wav.path());
    clip.offset = 1.0;
    track.clips.push(clip);
    project.tracks.push(track);
    let audio = render(&project, 8_000, 3.0).unwrap();
    assert!(audio[..8_000].iter().all(|sample| *sample == 0.0));
    assert!(
        audio[8_000..16_000]
            .iter()
            .all(|sample| (*sample - 0.6).abs() < 1e-6)
    );
    assert!(audio[16_000..].iter().all(|sample| *sample == 0.0));
    assert_eq!(std::fs::read(&wav.0).unwrap(), before);
}

#[test]
fn midi_source_offset_excludes_trimmed_notes() {
    let mut project = empty_project();
    let mut track = midi_track();
    track.clips[0].offset = 1.0;
    track.clips[0].length = 1.0;
    let audio = {
        project.tracks.push(track);
        render(&project, 8_000, 1.0).unwrap()
    };
    assert!(audio.iter().all(|sample| *sample == 0.0));
}

#[test]
fn audio_resamples_and_applies_clip_fades() {
    let wav = TestWav::new(vec![1.0; 8_000], 16_000);
    let mut project = empty_project();
    let mut track = Track::new("Audio", TrackKind::Audio);
    track.gain = 1.0;
    let mut clip = Clip::audio("Fade", 0.0, 1.0, wav.path());
    if let ClipContent::Audio {
        fade_in, fade_out, ..
    } = &mut clip.content
    {
        *fade_in = 0.25;
        *fade_out = 0.25;
    }
    track.clips.push(clip);
    project.tracks.push(track);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert_eq!(audio[0], 0.0);
    assert!((audio[1_000] - 0.5).abs() < 1e-6);
    assert!((audio[4_000] - 1.0).abs() < 1e-6);
    assert!(audio[7_998] < 0.002);
}

#[test]
fn compensation_aligns_paths_and_removes_export_preroll() {
    let mut impulse = vec![0.0; 4_000];
    impulse[0] = 0.25;
    impulse[300] = 0.5;
    let wav = TestWav::new(impulse, 8_000);
    let mut project = empty_project();
    for latency in [0, 37, 257] {
        let mut track = Track::new("Delayed", TrackKind::Audio);
        track.gain = 1.0;
        track.latency_samples = latency;
        track
            .clips
            .push(Clip::audio("Impulse", 0.0, 1.0, wav.path()));
        project.tracks.push(track);
    }
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert_eq!(audio[0], 0.75);
    assert_eq!(audio[600], 1.5);
    assert_eq!(audio.iter().filter(|sample| **sample != 0.0).count(), 4);
}

#[test]
fn delay_line_has_exact_sample_latency() {
    let mut delay = CompensationDelay::new(37);
    assert_eq!(delay.process([1.0, -1.0]), [0.0; 2]);
    for _ in 0..36 {
        assert_eq!(delay.process([0.0; 2]), [0.0; 2]);
    }
    assert_eq!(delay.process([0.0; 2]), [1.0, -1.0]);
    delay.reset();
    assert_eq!(delay.process([0.0; 2]), [0.0; 2]);
}

#[test]
fn mute_solo_and_pan_are_applied() {
    let mut project = empty_project();
    let mut audible = midi_track();
    audible.solo = true;
    audible.pan = -1.0;
    let mut inaudible = midi_track();
    inaudible.pan = 1.0;
    project.tracks.extend([audible, inaudible]);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert!(audio.chunks_exact(2).any(|frame| frame[0].abs() > 0.01));
    assert!(audio.chunks_exact(2).all(|frame| frame[1] == 0.0));
    project.tracks[0].mute = true;
    assert!(
        render(&project, 8_000, 1.0)
            .unwrap()
            .iter()
            .all(|sample| *sample == 0.0)
    );
}

#[test]
fn streaming_wav_export_matches_offline_render() {
    let mut project = empty_project();
    project.tracks.push(midi_track());
    let wav = TestWav::new(vec![0.0], 8_000);
    export_wav(&project, &wav.0, 8_000, 2.0).unwrap();
    let loaded = AudioData::load_wav(&wav.0).unwrap();
    assert_eq!(loaded.channels, 2);
    assert_eq!(loaded.samples, render(&project, 8_000, 2.0).unwrap());
}

#[test]
fn invalid_render_arguments_fail_without_panicking() {
    let project = empty_project();
    assert!(render(&project, 0, 1.0).is_err());
    assert!(render(&project, 48_000, f64::NAN).is_err());
    assert!(render(&project, 48_000, -1.0).is_err());
}

fn audio_track(wav: &TestWav, gain: f32) -> Track {
    let mut track = Track::new("Source", TrackKind::Audio);
    track.gain = gain;
    track
        .clips
        .push(Clip::audio("Source", 0.0, 1.0, wav.path()));
    track
}

fn effect(effect: Effect) -> EffectSlot {
    EffectSlot {
        id: daw_core::new_id(),
        name: "Test insert".into(),
        enabled: true,
        effect,
    }
}

#[test]
fn bus_sums_inputs_and_applies_inserts_before_its_fader() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    let mut bus = Track::new("Mix bus", TrackKind::Bus);
    bus.gain = 0.5;
    bus.inserts.push(effect(Effect::Gain { gain: 2.0 }));
    let mut a = audio_track(&wav, 0.5);
    a.output_bus = Some(bus.id);
    let mut b = audio_track(&wav, 0.25);
    b.output_bus = Some(bus.id);
    let mut project = empty_project();
    project.tracks.extend([bus, a, b]);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert!(audio.iter().all(|sample| (*sample - 0.75).abs() < 1e-6));
}

#[test]
fn sends_use_the_correct_pre_or_post_fader_tap_after_inserts() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    for (pre_fader, expected) in [(false, 0.75), (true, 1.5)] {
        let mut bus = Track::new("Return", TrackKind::Bus);
        bus.gain = 1.0;
        let mut source = audio_track(&wav, 0.25);
        source.inserts.push(effect(Effect::Gain { gain: 2.0 }));
        source.sends.push(Send {
            target_bus: bus.id,
            gain: 0.5,
            pre_fader,
        });
        let mut project = empty_project();
        project.tracks.extend([source, bus]);
        assert!(
            render(&project, 8_000, 1.0)
                .unwrap()
                .iter()
                .all(|sample| (*sample - expected).abs() < 1e-6)
        );
    }
}

#[test]
fn routing_compensates_bus_inputs_and_parallel_master_sends() {
    let mut samples = vec![0.0; 4_000];
    samples[0] = 0.1;
    let wav = TestWav::new(samples, 8_000);
    let mut bus = Track::new("Delayed bus", TrackKind::Bus);
    bus.gain = 1.0;
    bus.latency_samples = 53;
    let mut a = audio_track(&wav, 1.0);
    a.latency_samples = 37;
    a.output_bus = Some(bus.id);
    let mut b = audio_track(&wav, 1.0);
    b.latency_samples = 257;
    b.output_bus = Some(bus.id);
    let c = audio_track(&wav, 1.0);
    let mut d = audio_track(&wav, 1.0);
    d.latency_samples = 17;
    d.sends.push(Send {
        target_bus: bus.id,
        gain: 1.0,
        pre_fader: false,
    });
    let mut project = empty_project();
    project.tracks.extend([bus, a, b, c, d]);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert!((audio[0] - 0.5).abs() < 1e-6);
    assert!(audio[2..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn musical_delay_is_not_removed_by_processing_latency_compensation() {
    let mut samples = vec![0.0; 4_000];
    samples[0] = 0.5;
    let wav = TestWav::new(samples, 8_000);
    let mut source = audio_track(&wav, 1.0);
    source.latency_samples = 120;
    source.inserts.push(effect(Effect::Delay {
        time_ms: 10.0,
        feedback: 0.5,
        mix: 1.0,
    }));
    let mut project = empty_project();
    project.tracks.push(source);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert_eq!(audio[0], 0.0);
    assert_eq!(audio[160], 0.5);
    assert_eq!(audio[320], 0.25);
}

#[test]
fn solo_source_preserves_bus_path_and_excludes_other_sources() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    let mut bus = Track::new("Mix", TrackKind::Bus);
    bus.gain = 1.0;
    let mut a = audio_track(&wav, 0.2);
    a.solo = true;
    a.output_bus = Some(bus.id);
    let mut b = audio_track(&wav, 0.4);
    b.output_bus = Some(bus.id);
    let mut project = empty_project();
    project.tracks.extend([a, b, bus]);
    assert!(
        render(&project, 8_000, 1.0)
            .unwrap()
            .iter()
            .all(|sample| (*sample - 0.2).abs() < 1e-6)
    );
}

#[test]
fn solo_bus_includes_upstream_feeds_and_folder_mute_takes_priority() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    let mut folder = Track::new("Group", TrackKind::Folder);
    let mut bus = Track::new("Mix", TrackKind::Bus);
    bus.gain = 1.0;
    bus.solo = true;
    let mut a = audio_track(&wav, 0.2);
    a.output_bus = Some(bus.id);
    a.parent_folder = Some(folder.id);
    let mut b = audio_track(&wav, 0.4);
    b.output_bus = Some(bus.id);
    b.parent_folder = Some(folder.id);
    let c = audio_track(&wav, 0.8);
    let mut project = empty_project();
    project.tracks.extend([a, b, bus, c, folder.clone()]);
    assert!(
        render(&project, 8_000, 1.0)
            .unwrap()
            .iter()
            .all(|sample| (*sample - 0.6).abs() < 1e-6)
    );
    folder.mute = true;
    *project.tracks.last_mut().unwrap() = folder;
    assert!(
        render(&project, 8_000, 1.0)
            .unwrap()
            .iter()
            .all(|sample| *sample == 0.0)
    );
}

#[test]
fn bypassed_inserts_leave_audio_unchanged() {
    let wav = TestWav::new(vec![0.25; 4_000], 8_000);
    let mut source = audio_track(&wav, 1.0);
    let mut slot = effect(Effect::Gain { gain: 0.0 });
    slot.enabled = false;
    source.inserts.push(slot);
    let mut project = empty_project();
    project.tracks.push(source);
    assert!(
        render(&project, 8_000, 1.0)
            .unwrap()
            .iter()
            .all(|sample| *sample == 0.25)
    );
}

#[test]
fn full_project_export_preserves_cascaded_effect_tails() {
    let mut samples = vec![0.0; 4_000];
    samples[3_999] = 0.5;
    let source = TestWav::new(samples, 8_000);
    let output = TestWav::new(vec![0.0], 8_000);
    let mut track = audio_track(&source, 1.0);
    let mut bus = Track::new("Echo return", TrackKind::Bus);
    bus.gain = 1.0;
    track.output_bus = Some(bus.id);
    for node in [&mut track, &mut bus] {
        node.inserts.push(effect(Effect::Delay {
            time_ms: 150.0,
            feedback: 0.0,
            mix: 1.0,
        }));
    }
    let mut project = empty_project();
    project.tracks.extend([track, bus]);
    export_project_wav(&project, &output.0, 8_000).unwrap();
    let result = AudioData::load_wav(&output.0).unwrap();
    assert_eq!(result.frames(), 6_400);
    assert_eq!(result.samples[6_399 * 2], 0.5);
}

#[test]
fn failed_export_preserves_the_existing_destination() {
    let source = TestWav::new(vec![0.25; 4_000], 8_000);
    let output = TestWav::new(vec![0.123], 8_000);
    let before = std::fs::read(&output.0).unwrap();
    let mut track = audio_track(&source, f32::MAX);
    track.inserts.push(effect(Effect::Gain { gain: 16.0 }));
    let mut project = empty_project();
    project.tracks.push(track);
    assert!(export_project_wav(&project, &output.0, 8_000).is_err());
    assert_eq!(std::fs::read(&output.0).unwrap(), before);
}
