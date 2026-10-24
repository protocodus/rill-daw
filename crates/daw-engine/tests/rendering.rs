use daw_core::{
    AudioData, AutomationLane, AutomationParameter, AutomationPoint, AutomationTarget, Clip,
    ClipContent, Effect, EffectSlot, Interpolation, Note, Project, Send, Track, TrackKind,
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
fn unassigned_midi_track_is_silent_without_an_integrated_instrument() {
    let mut project = empty_project();
    project.tracks.push(midi_track());
    let audio = render(&project, 8_000, 2.0).unwrap();
    assert_eq!(audio.len(), 16_000);
    assert!(audio.iter().all(|sample| sample.is_finite()));
    assert!(audio.iter().all(|sample| *sample == 0.0));
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
    // The f32 FFT converter introduces a few ulps of rounding at unity.
    assert!((audio[4_000] - 1.0).abs() < 5e-6, "center {}", audio[4_000]);
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
    let wav = TestWav::new(vec![0.25; 4_000], 8_000);
    let mut audible = audio_track(&wav, 1.0);
    audible.solo = true;
    audible.pan = -1.0;
    let mut inaudible = audio_track(&wav, 1.0);
    inaudible.pan = 1.0;
    project.tracks.extend([audible, inaudible]);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert!(
        audio
            .as_chunks::<2>()
            .0
            .iter()
            .any(|frame| frame[0].abs() > 0.01)
    );
    assert!(audio.as_chunks::<2>().0.iter().all(|frame| frame[1] == 0.0));
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
    let source = TestWav::new(vec![0.25; 4_000], 8_000);
    project.tracks.push(audio_track(&source, 1.0));
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
    EffectSlot::new("Test insert", effect)
}

#[test]
fn bus_sums_inputs_and_applies_its_fader() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    let mut bus = Track::new("Mix bus", TrackKind::Bus);
    bus.gain = 0.5;
    let mut a = audio_track(&wav, 0.5);
    a.output_bus = Some(bus.id);
    let mut b = audio_track(&wav, 0.25);
    b.output_bus = Some(bus.id);
    let mut project = empty_project();
    project.tracks.extend([bus, a, b]);
    let audio = render(&project, 8_000, 1.0).unwrap();
    assert!(audio.iter().all(|sample| (*sample - 0.375).abs() < 1e-6));
}

#[test]
fn sends_use_the_correct_pre_or_post_fader_tap() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    for (pre_fader, expected) in [(false, 0.375), (true, 0.75)] {
        let mut bus = Track::new("Return", TrackKind::Bus);
        bus.gain = 1.0;
        let mut source = audio_track(&wav, 0.25);
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
    let mut slot = effect(Effect::Vst3 {
        path: "missing-effect.vst3".into(),
        class_id: None,
    });
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
fn failed_export_preserves_the_existing_destination() {
    let source = TestWav::new(vec![1.0; 4_000], 8_000);
    let output = TestWav::new(vec![0.123], 8_000);
    let before = std::fs::read(&output.0).unwrap();
    let track = audio_track(&source, f32::MAX);
    let mut project = empty_project();
    project.tracks.push(track);
    project.tracks.push(audio_track(&source, f32::MAX));
    assert!(export_project_wav(&project, &output.0, 8_000).is_err());
    assert_eq!(std::fs::read(&output.0).unwrap(), before);
}

fn automate(
    project: &mut Project,
    target: u64,
    parameter: AutomationParameter,
    points: &[(f64, f32)],
    interpolation: Interpolation,
) -> u64 {
    let mut track = Track::new("Automation", TrackKind::Automation);
    track.automation = Some(AutomationLane {
        target: AutomationTarget {
            track: target,
            parameter,
        },
        points: points
            .iter()
            .map(|&(beat, value)| AutomationPoint::new(beat, parameter.normalize(value)))
            .collect(),
        interpolation,
    });
    let id = track.id;
    project.tracks.push(track);
    id
}

#[test]
fn gain_and_pan_automation_are_sample_accurate() {
    let wav = TestWav::new(vec![0.25; 4_000], 8_000);
    let track = audio_track(&wav, 1.0);
    let target = track.id;
    let mut project = empty_project();
    project.tracks.push(track);
    automate(
        &mut project,
        target,
        AutomationParameter::TrackGain,
        &[(0.0, 0.0), (1.0, 1.0)],
        Interpolation::Linear,
    );
    automate(
        &mut project,
        target,
        AutomationParameter::TrackPan,
        &[(0.0, -1.0), (1.0, 1.0)],
        Interpolation::Linear,
    );
    let output = render(&project, 8_000, 1.0).unwrap();
    for (index, frame) in output.as_chunks::<2>().0.iter().enumerate() {
        let beat = index as f32 / 4_000.0;
        let pan = beat * 2.0 - 1.0;
        let expected = [
            0.25 * beat * (1.0 - pan.max(0.0)),
            0.25 * beat * (1.0 + pan.min(0.0)),
        ];
        assert!((frame[0] - expected[0]).abs() < 1e-6, "Left frame {index}");
        assert!((frame[1] - expected[1]).abs() < 1e-6, "Right frame {index}");
    }
}

#[test]
fn step_and_smooth_automation_render_the_requested_curve() {
    let wav = TestWav::new(vec![0.25; 4_000], 8_000);
    for interpolation in [Interpolation::Step, Interpolation::Smooth] {
        let track = audio_track(&wav, 1.0);
        let target = track.id;
        let mut project = empty_project();
        project.tracks.push(track);
        automate(
            &mut project,
            target,
            AutomationParameter::TrackGain,
            &[(0.0, 0.0), (0.5, 1.0), (1.0, 1.0)],
            interpolation,
        );
        let output = render(&project, 8_000, 1.0).unwrap();
        assert!(output.iter().all(|value| (0.0..=0.250_001).contains(value)));
        assert!((output[4_000] - 0.25).abs() < 1e-6);
        match interpolation {
            Interpolation::Step => assert!(output[..4_000].iter().all(|value| *value == 0.0)),
            // PCHIP between (0,0), (.5,1), (1,1): its midpoint is .6875.
            Interpolation::Smooth => assert!((output[2_000] - 0.25 * 0.6875).abs() < 1e-6),
            Interpolation::Linear => unreachable!(),
        }
    }
}

#[test]
fn muted_empty_and_folder_muted_lanes_restore_manual_values() {
    let wav = TestWav::new(vec![1.0; 4_000], 8_000);
    let track = audio_track(&wav, 0.3);
    let target = track.id;
    let mut project = empty_project();
    project.tracks.push(track);
    let lane = automate(
        &mut project,
        target,
        AutomationParameter::TrackGain,
        &[(0.0, 0.0)],
        Interpolation::Linear,
    );
    project.track_mut(lane).unwrap().mute = true;
    // Solo on a control lane must not silence the audio graph.
    project.track_mut(lane).unwrap().solo = true;
    let assert_manual = |project: &Project| {
        assert!(
            render(project, 8_000, 1.0)
                .unwrap()
                .iter()
                .all(|value| (*value - 0.3).abs() < 1e-6)
        )
    };
    assert_manual(&project);
    let folder = project.add_folder("Muted controls");
    project.track_mut(folder).unwrap().mute = true;
    project.track_mut(lane).unwrap().mute = false;
    project.set_parent_folder(lane, Some(folder)).unwrap();
    assert_manual(&project);
    project.track_mut(folder).unwrap().mute = false;
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points
        .clear();
    assert_manual(&project);
}

#[test]
fn automated_sends_and_bus_faders_follow_compensated_musical_time() {
    let wav = TestWav::new(vec![0.1; 4_000], 8_000);
    for pre_fader in [false, true] {
        let mut source = audio_track(&wav, 1.0);
        source.latency_samples = 37;
        let target = source.id;
        let mut bus = Track::new("Automated bus", TrackKind::Bus);
        bus.latency_samples = 53;
        let bus_id = bus.id;
        // A zero manual send must still exist when its automation is active.
        source.sends.push(Send {
            target_bus: bus_id,
            gain: 0.0,
            pre_fader,
        });
        let mut parallel = audio_track(&wav, 2.0);
        parallel.latency_samples = 257;
        let mut project = empty_project();
        project.tracks.extend([bus, source, parallel]);
        for (track, parameter, from, to) in [
            (target, AutomationParameter::TrackGain, 0.0, 1.0),
            (
                target,
                AutomationParameter::SendGain { bus: bus_id },
                0.0,
                1.0,
            ),
            (bus_id, AutomationParameter::TrackGain, 1.0, 0.5),
        ] {
            automate(
                &mut project,
                track,
                parameter,
                &[(0.0, from), (1.0, to)],
                Interpolation::Linear,
            );
        }
        let output = render(&project, 8_000, 1.0).unwrap();
        for (index, frame) in output.as_chunks::<2>().0.iter().enumerate() {
            let beat = index as f32 / 4_000.0;
            let main = 0.1 * beat;
            let send = 0.1 * beat * (1.0 - beat * 0.5) * if pre_fader { 1.0 } else { beat };
            let expected = 0.2 + main + send;
            assert!(
                (frame[0] - expected).abs() < 2e-6,
                "Frame {index}: {} vs {expected}",
                frame[0]
            );
            assert!((frame[1] - expected).abs() < 2e-6);
        }
    }
}
