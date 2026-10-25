//! Output-only metronome and a shared, sample-clock recording start gate.
//! No plugin, file I/O, allocation, or locking belongs in these callbacks.
use super::{AudioEngine, Controls};
use anyhow::{Result, ensure};
use daw_core::Project;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

const IDLE: usize = 0;
const ARMED: usize = 1;
const COUNTING: usize = 2;
const STARTING: usize = 3;
const CAPTURING: usize = 4;
const CANCELLED: usize = 5;
const PREPARING_COUNT: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordingTiming {
    pub precount_bars: u8,
    pub click_enabled: bool,
    pub click_gain: f32,
}
impl Default for RecordingTiming {
    fn default() -> Self {
        Self {
            precount_bars: 0,
            click_enabled: false,
            click_gain: 0.25,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordingTimingSnapshot {
    pub armed: bool,
    pub counting: bool,
    pub capturing: bool,
    pub remaining_beats: f64,
    pub remaining_frames: u64,
    pub captured_frames: u64,
}

pub(super) struct TimingShared {
    phase: AtomicUsize,
    sample_rate: AtomicU32,
    tempo: AtomicU64,
    beats_per_bar: AtomicU8,
    precount_bars: AtomicU8,
    click_enabled: AtomicBool,
    click_gain: AtomicU32,
    audio: AtomicBool,
    midi: AtomicBool,
    was_playing: AtomicBool,
    remaining: AtomicU64,
    total: AtomicU64,
    captured: AtomicU64,
    click_age: AtomicU64,
    click_accent: AtomicBool,
}
#[derive(Clone, Copy)]
pub(super) struct FrameGate {
    pub hold_transport: bool,
    pub capture_allowed: bool,
    counting: bool,
    count_pulse: Option<u64>,
}
impl TimingShared {
    pub(super) fn new() -> Self {
        Self {
            phase: AtomicUsize::new(IDLE),
            sample_rate: AtomicU32::new(48000),
            tempo: AtomicU64::new(120.0_f64.to_bits()),
            beats_per_bar: AtomicU8::new(4),
            precount_bars: AtomicU8::new(0),
            click_enabled: AtomicBool::new(false),
            click_gain: AtomicU32::new(0.25_f32.to_bits()),
            audio: AtomicBool::new(false),
            midi: AtomicBool::new(false),
            was_playing: AtomicBool::new(false),
            remaining: AtomicU64::new(0),
            total: AtomicU64::new(0),
            captured: AtomicU64::new(0),
            click_age: AtomicU64::new(u64::MAX),
            click_accent: AtomicBool::new(false),
        }
    }
    pub(super) fn busy(&self) -> bool {
        matches!(
            self.phase.load(Ordering::Acquire),
            ARMED | PREPARING_COUNT | COUNTING | STARTING | CAPTURING
        )
    }
    pub(super) fn sync_project(&self, project: &Project) -> Result<()> {
        ensure!(
            project.tempo.is_finite()
                && project.tempo > 0.0
                && (1..=32).contains(&project.beats_per_bar),
            "Invalid metronome tempo or time signature"
        );
        ensure!(
            !self.busy()
                || (self.tempo.load(Ordering::Acquire) == project.tempo.to_bits()
                    && self.beats_per_bar.load(Ordering::Acquire) == project.beats_per_bar),
            "Finish recording before changing tempo or time signature"
        );
        self.tempo.store(project.tempo.to_bits(), Ordering::Release);
        self.beats_per_bar
            .store(project.beats_per_bar, Ordering::Release);
        Ok(())
    }
    pub(super) fn configure(
        &self,
        project: &Project,
        settings: RecordingTiming,
        sample_rate: u32,
    ) -> Result<()> {
        ensure!(
            settings.precount_bars <= 2,
            "Choose Off, one bar or two bars of pre-count"
        );
        ensure!(
            settings.click_gain.is_finite() && (0.0..=1.0).contains(&settings.click_gain),
            "Metronome volume must be between zero and one"
        );
        self.sync_project(project)?;
        self.sample_rate.store(sample_rate, Ordering::Release);
        self.precount_bars
            .store(settings.precount_bars, Ordering::Release);
        self.click_enabled
            .store(settings.click_enabled, Ordering::Release);
        self.click_gain
            .store(settings.click_gain.to_bits(), Ordering::Release);
        Ok(())
    }
    pub(super) fn cancel(&self, controls: &Controls) {
        let phase = self.phase.swap(CANCELLED, Ordering::AcqRel);
        self.remaining.store(0, Ordering::Release);
        if matches!(phase, ARMED | PREPARING_COUNT | COUNTING | STARTING)
            && !self.was_playing.load(Ordering::Acquire)
        {
            controls.playing.store(false, Ordering::Release);
        }
    }
    pub(super) fn count_frames(&self) -> Option<u64> {
        (self.phase.load(Ordering::Acquire) == COUNTING)
            .then(|| self.remaining.load(Ordering::Acquire))
    }
    pub(super) fn holds_transport(&self) -> bool {
        matches!(
            self.phase.load(Ordering::Acquire),
            ARMED | PREPARING_COUNT | COUNTING
        ) && !self.was_playing.load(Ordering::Acquire)
    }
    pub(super) fn legacy_capture_allowed(&self) -> bool {
        self.phase.load(Ordering::Acquire) == IDLE
    }
    pub(super) fn can_start(&self) -> bool {
        matches!(self.phase.load(Ordering::Acquire), STARTING | CAPTURING)
    }
    pub(super) fn keeps_clock_running(&self) -> bool {
        self.click_enabled.load(Ordering::Acquire)
    }
    pub(super) fn suspends_midi_monitor(&self) -> bool {
        matches!(
            self.phase.load(Ordering::Acquire),
            ARMED | PREPARING_COUNT | COUNTING
        ) && !self.was_playing.load(Ordering::Acquire)
    }
    pub(super) fn waiting_for_timeline(&self) -> bool {
        self.phase.load(Ordering::Acquire) == PREPARING_COUNT
    }
    pub(super) fn frame_before(&self, controls: &Controls, timeline_ready: bool) -> FrameGate {
        let mut phase = self.phase.load(Ordering::Acquire);
        if phase == PREPARING_COUNT
            && timeline_ready
            && self
                .phase
                .compare_exchange(
                    PREPARING_COUNT,
                    COUNTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        {
            phase = COUNTING;
        }
        if phase == COUNTING {
            let remaining = self.remaining.load(Ordering::Relaxed);
            let elapsed = self.total.load(Ordering::Relaxed).saturating_sub(remaining);
            let pulse = self.pulse(elapsed);
            if remaining <= 1 {
                if self
                    .phase
                    .compare_exchange(COUNTING, STARTING, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    self.remaining.store(0, Ordering::Release);
                    controls.playing.store(true, Ordering::Release);
                    // Stop may cancel between the phase transition and this
                    // transport write. Do not let the callback resume a
                    // transport the user has just stopped.
                    if self.phase.load(Ordering::Acquire) == CANCELLED {
                        controls.playing.store(false, Ordering::Release);
                    }
                }
            } else {
                let _ = self.remaining.compare_exchange(
                    remaining,
                    remaining - 1,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
            }
            return FrameGate {
                hold_transport: true,
                capture_allowed: false,
                counting: true,
                count_pulse: pulse,
            };
        }
        FrameGate {
            hold_transport: matches!(phase, ARMED | PREPARING_COUNT)
                && !self.was_playing.load(Ordering::Acquire),
            capture_allowed: matches!(phase, IDLE | STARTING | CAPTURING),
            counting: false,
            count_pulse: None,
        }
    }
    pub(super) fn observe_frame(&self, position: Option<u64>, capture_allowed: bool) {
        if capture_allowed
            && position.is_some()
            && matches!(self.phase.load(Ordering::Acquire), STARTING | CAPTURING)
        {
            let _ = self.phase.compare_exchange(
                STARTING,
                CAPTURING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            self.captured.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn pulse(&self, frame: u64) -> Option<u64> {
        let tempo = f64::from_bits(self.tempo.load(Ordering::Relaxed));
        let beat_frames = f64::from(self.sample_rate.load(Ordering::Relaxed)) * 60.0 / tempo;
        let beat = (frame as f64 / beat_frames).round();
        ((beat * beat_frames).round() as u64 == frame).then_some(beat as u64)
    }
    pub(super) fn click(&self, gate: FrameGate, position: Option<u64>) -> f32 {
        let enabled =
            gate.counting || (self.click_enabled.load(Ordering::Relaxed) && position.is_some());
        if !enabled {
            self.click_age.store(u64::MAX, Ordering::Relaxed);
            return 0.0;
        }
        let pulse = if gate.counting {
            gate.count_pulse
        } else {
            position.and_then(|frame| self.pulse(frame))
        };
        if let Some(beat) = pulse {
            self.click_age.store(0, Ordering::Relaxed);
            self.click_accent.store(
                beat.is_multiple_of(u64::from(self.beats_per_bar.load(Ordering::Relaxed))),
                Ordering::Relaxed,
            );
        }
        let age = self.click_age.load(Ordering::Relaxed);
        let rate = self.sample_rate.load(Ordering::Relaxed);
        let length = (rate / 50).max(1); // 20 ms, smoothly zero at both ends.
        if age >= u64::from(length) {
            return 0.0;
        }
        self.click_age.store(age + 1, Ordering::Relaxed);
        let accent = self.click_accent.load(Ordering::Relaxed);
        let t = age as f32 / rate as f32;
        let phase = age as f32 / length as f32;
        let envelope = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
        let gain = f32::from_bits(self.click_gain.load(Ordering::Relaxed))
            * if accent { 1.0 } else { 0.65 };
        (std::f32::consts::TAU * if accent { 1760.0 } else { 1320.0 } * t).sin() * envelope * gain
    }
}
impl AudioEngine {
    pub fn configure_recording_timing(
        &self,
        project: &Project,
        settings: RecordingTiming,
    ) -> Result<()> {
        self.controls
            .timing
            .configure(project, settings, self.sample_rate)?;
        if settings.click_enabled {
            self.controls
                .finished_generation
                .store(u64::MAX, Ordering::Release);
        }
        Ok(())
    }
    /// Arm before preparing either backend. While already playing, preparation
    /// preserves backing audio and pre-count is skipped.
    pub fn arm_recording_timing(&self, audio: bool, midi: bool) -> Result<()> {
        let timing = &self.controls.timing;
        ensure!(audio || midi, "Select an audio or MIDI recording target");
        ensure!(!timing.busy(), "A recording start is already armed");
        let playing = self.controls.playing.load(Ordering::Acquire);
        timing.was_playing.store(playing, Ordering::Relaxed);
        timing.audio.store(audio, Ordering::Relaxed);
        timing.midi.store(midi, Ordering::Relaxed);
        let frames = if playing {
            0
        } else {
            (f64::from(timing.precount_bars.load(Ordering::Relaxed))
                * f64::from(timing.beats_per_bar.load(Ordering::Relaxed))
                * f64::from(self.sample_rate)
                * 60.0
                / f64::from_bits(timing.tempo.load(Ordering::Relaxed)))
            .round() as u64
        };
        timing.total.store(frames, Ordering::Relaxed);
        timing.remaining.store(frames, Ordering::Relaxed);
        timing.captured.store(0, Ordering::Relaxed);
        timing.phase.store(ARMED, Ordering::Release);
        Ok(())
    }
    /// Called once both audio and MIDI backends have finished preparation.
    pub fn commit_recording_timing(&self) -> Result<()> {
        let timing = &self.controls.timing;
        ensure!(
            timing.phase.load(Ordering::Acquire) == ARMED,
            "Arm recording before starting its clock"
        );
        ensure!(
            !timing.audio.load(Ordering::Acquire) || self.controls.input.active(),
            "Audio recording is still preparing"
        );
        ensure!(
            !timing.midi.load(Ordering::Acquire) || self.controls.midi.active(),
            "MIDI recording is still preparing"
        );
        let phase = if timing.remaining.load(Ordering::Relaxed) > 0 {
            PREPARING_COUNT
        } else {
            STARTING
        };
        timing.phase.store(phase, Ordering::Release);
        if phase == STARTING {
            self.controls.playing.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn cancel_recording_timing(&self) {
        self.controls.timing.cancel(&self.controls);
    }
    pub fn timing_snapshot(&self) -> RecordingTimingSnapshot {
        let timing = &self.controls.timing;
        let phase = timing.phase.load(Ordering::Acquire);
        let remaining = timing.remaining.load(Ordering::Acquire);
        RecordingTimingSnapshot {
            armed: matches!(
                phase,
                ARMED | PREPARING_COUNT | COUNTING | STARTING | CAPTURING
            ),
            // Preparing the timeline is part of the visible count-in. Report
            // it immediately, even before the first output callback arrives.
            counting: matches!(phase, PREPARING_COUNT | COUNTING),
            capturing: phase == CAPTURING,
            remaining_frames: remaining,
            remaining_beats: remaining as f64
                * f64::from_bits(timing.tempo.load(Ordering::Relaxed))
                / (f64::from(self.sample_rate) * 60.0),
            captured_frames: timing.captured.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceSettings, InputRoute, InputSettings, RecordingConfig};
    use daw_core::{AudioData, AudioInputChannels, Clip};
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    fn engine(project: &Project) -> AudioEngine {
        AudioEngine::start_virtual_cancellable(
            project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
    #[test]
    fn saved_metronome_is_ready_for_the_first_playback_callback() {
        let mut project = Project::new();
        project.metronome_enabled = true;
        project.metronome_gain = 0.75;
        let mut engine = engine(&project);
        // A device may consume audio before the UI gets another frame to
        // configure timing. The saved click must already be present.
        engine.set_playing(true);
        let frames = engine.pump_virtual(960).unwrap();
        assert!(frames.iter().any(|frame| frame[0].abs() > 0.6));
        assert!(engine.snapshot().playing);
        assert!((engine.position_beats() - 0.04).abs() < 1e-8);
    }
    fn ready(engine: &AudioEngine) {
        let until = Instant::now() + Duration::from_secs(5);
        while !engine.recording_snapshot().active {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn finish(engine: &mut AudioEngine) -> crate::RecordingResult {
        engine.request_stop_recording();
        engine.cancel_recording_timing();
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = engine.take_recording_result() {
                return result;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn audio_count_in_holds_cursor_clicks_with_toggle_off_and_excludes_preroll() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Input");
        project.track_mut(track).unwrap().gain = 1.0;
        let mut engine = engine(&project);
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: None,
                    routes: vec![InputRoute {
                        track,
                        channels: AudioInputChannels::Mono { channel: 0 },
                        monitor: true,
                    }],
                },
            )
            .unwrap();
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    precount_bars: 1,
                    click_enabled: false,
                    click_gain: 0.5,
                },
            )
            .unwrap();
        engine.arm_recording_timing(true, false).unwrap();
        assert!(
            engine.commit_recording_timing().is_err(),
            "Do not start until the writer is ready"
        );
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        ready(&engine);
        engine.commit_recording_timing().unwrap();
        assert!(engine.timing_snapshot().counting);
        assert_eq!(engine.timing_snapshot().remaining_frames, 96000);
        let count = engine.pump_virtual_input(&vec![0.0; 96000 * 2], 2).unwrap();
        assert_eq!(engine.position_beats(), 0.0);
        assert_eq!(engine.recording_snapshot().frames, 0);
        assert_eq!(engine.timing_snapshot().remaining_frames, 0);
        assert!(!engine.timing_snapshot().capturing);
        for onset in [0, 24000, 48000, 72000] {
            assert_eq!(count[onset], [0.0; 2]);
            assert!(
                count[onset + 1..onset + 960]
                    .iter()
                    .any(|frame| frame[0].abs() > 0.2)
            );
            assert!(
                count[onset + 960..onset + 24000]
                    .iter()
                    .all(|frame| *frame == [0.0; 2])
            );
        }
        let accent = count[..960]
            .iter()
            .map(|frame| frame[0] * frame[0])
            .sum::<f32>();
        let ordinary = count[24000..24960]
            .iter()
            .map(|frame| frame[0] * frame[0])
            .sum::<f32>();
        assert!((accent / ordinary - 1.0 / 0.65_f32.powi(2)).abs() < 0.01);
        let monitored = engine.pump_virtual_input(&vec![0.2; 512 * 2], 2).unwrap();
        assert!(
            monitored[256..]
                .iter()
                .all(|frame| (frame[0] - 0.2).abs() < 1e-6)
        );
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.takes[0].frames, 512);
        assert!(
            AudioData::load_wav(&result.takes[0].path)
                .unwrap()
                .samples
                .iter()
                .all(|sample| *sample == 0.2)
        );
    }
    #[test]
    fn two_bar_count_and_click_callback_are_allocation_free_and_cancel_quietly() {
        let controls = Controls::new();
        let project = Project::new();
        controls
            .timing
            .configure(
                &project,
                RecordingTiming {
                    precount_bars: 2,
                    click_enabled: false,
                    click_gain: 0.25,
                },
                48000,
            )
            .unwrap();
        controls.timing.phase.store(COUNTING, Ordering::Release);
        controls.timing.total.store(192000, Ordering::Release);
        controls.timing.remaining.store(192000, Ordering::Release);
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                for _ in 0..512 {
                    let gate = controls.timing.frame_before(&controls, true);
                    let _ = controls.timing.click(gate, None);
                }
            }),
            0
        );
        assert_eq!(
            controls.timing.remaining.load(Ordering::Relaxed),
            192000 - 512
        );
        controls.timing.cancel(&controls);
        assert!(!controls.playing.load(Ordering::Relaxed));
        assert!(!controls.timing.busy());
        assert!(
            !controls
                .timing
                .frame_before(&controls, true)
                .capture_allowed
        );
    }
    #[test]
    fn metronome_toggle_is_live_preserves_engine_and_is_absent_from_export() {
        let mut project = Project::new();
        let track = project.add_midi_track("Empty timeline");
        project
            .track_mut(track)
            .unwrap()
            .clips
            .push(Clip::midi("Silence", 0.0, 8.0, vec![]));
        let mut engine = engine(&project);
        let id = engine.snapshot().engine_id;
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    click_enabled: true,
                    ..RecordingTiming::default()
                },
            )
            .unwrap();
        engine.set_playing(true);
        let click = engine.pump_virtual(48000).unwrap();
        assert!(click.iter().any(|frame| frame[0].abs() > 0.1));
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    click_enabled: false,
                    ..RecordingTiming::default()
                },
            )
            .unwrap();
        assert!(
            engine
                .pump_virtual(12000)
                .unwrap()
                .iter()
                .all(|frame| *frame == [0.0; 2])
        );
        assert_eq!(engine.snapshot().engine_id, id);
        assert!(
            crate::render(&project, 48000, 2.0)
                .unwrap()
                .iter()
                .all(|sample| *sample == 0.0)
        );
    }
    #[test]
    fn record_while_playing_skips_count_and_preserves_backing_position() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Input");
        let midi = project.add_midi_track("Timeline");
        project
            .track_mut(midi)
            .unwrap()
            .clips
            .push(Clip::midi("Silence", 0.0, 8.0, vec![]));
        let mut engine = engine(&project);
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: None,
                    routes: vec![InputRoute {
                        track,
                        channels: AudioInputChannels::Mono { channel: 0 },
                        monitor: false,
                    }],
                },
            )
            .unwrap();
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    precount_bars: 2,
                    ..RecordingTiming::default()
                },
            )
            .unwrap();
        engine.set_playing(true);
        engine.pump_virtual_input(&vec![0.0; 512 * 2], 2).unwrap();
        let beat = engine.position_beats();
        engine.arm_recording_timing(true, false).unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        ready(&engine);
        engine.commit_recording_timing().unwrap();
        assert_eq!(engine.timing_snapshot().remaining_frames, 0);
        engine.pump_virtual_input(&vec![0.1; 512 * 2], 2).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!((result.takes[0].start_beat - beat).abs() < 1e-9);
        assert_eq!(result.takes[0].frames, 512);
    }
    #[test]
    fn empty_project_click_keeps_its_clock_until_disabled() {
        let project = Project::new();
        let mut engine = engine(&project);
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    click_enabled: true,
                    ..RecordingTiming::default()
                },
            )
            .unwrap();
        engine.set_playing(true);
        let samples = engine.pump_virtual(144000).unwrap();
        assert!(engine.snapshot().playing);
        assert!((engine.position_beats() - 6.0).abs() < 1e-8);
        for beat in 0..6 {
            assert!(
                samples[beat * 24000 + 1..beat * 24000 + 960]
                    .iter()
                    .any(|frame| frame[0].abs() > 0.1)
            );
        }
        engine
            .configure_recording_timing(&project, RecordingTiming::default())
            .unwrap();
        engine.pump_virtual(2048).unwrap();
        assert!(!engine.snapshot().playing);
    }
    #[test]
    fn mixed_count_prepares_timeline_before_count_and_captures_auto_midi_first_frame() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        let audio = project.add_audio_track("Audio");
        let midi = project.add_midi_track("MIDI");
        let mut engine = engine(&project);
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: None,
                    routes: vec![InputRoute {
                        track: audio,
                        channels: AudioInputChannels::Mono { channel: 0 },
                        monitor: false,
                    }],
                },
            )
            .unwrap();
        engine
            .configure_midi(
                &project,
                crate::MidiSettings {
                    port_id: Some("rill-virtual-midi".into()),
                    routes: vec![crate::MidiRoute {
                        track: midi,
                        channel: None,
                        monitor: true,
                    }],
                },
            )
            .unwrap();
        // Establish paused monitoring first, exercising the real preview→timeline
        // transition rather than only an unmonitored dummy recording.
        let until = Instant::now() + Duration::from_secs(5);
        while !engine.controls.render_preview.load(Ordering::Acquire)
            || !engine.controls.auditioning.load(Ordering::Acquire)
        {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        engine.pump_virtual_input(&vec![0.0; 512 * 2], 2).unwrap();
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    precount_bars: 1,
                    ..RecordingTiming::default()
                },
            )
            .unwrap();
        engine.arm_recording_timing(true, true).unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![audio],
            })
            .unwrap();
        engine.start_midi_recording(vec![midi]).unwrap();
        ready(&engine);
        engine.commit_recording_timing().unwrap();
        engine.inject_virtual_midi(&[0x90, 60, 100], 100).unwrap();
        engine.inject_virtual_midi(&[0x80, 60, 0], 200).unwrap();
        engine.pump_virtual_input(&vec![0.1; 96000 * 2], 2).unwrap();
        assert_eq!(engine.position_beats(), 0.0);
        assert!(!engine.controls.render_preview.load(Ordering::Acquire));
        assert_eq!(engine.recording_snapshot().frames, 0);
        assert_eq!(engine.midi_snapshot().events, 0);
        engine.inject_virtual_midi(&[0x90, 64, 99], 0).unwrap();
        engine.inject_virtual_midi(&[0x80, 64, 0], 240).unwrap();
        engine.pump_virtual_input(&vec![0.2; 512 * 2], 2).unwrap();
        engine.request_stop_midi_recording();
        let audio_result = finish(&mut engine);
        assert!(audio_result.error.is_none(), "{:?}", audio_result.error);
        assert_eq!(audio_result.takes[0].frames, 512);
        assert_eq!(audio_result.takes[0].start_beat, 0.0);
        let midi_result = engine.take_midi_recording_result().unwrap();
        assert!(midi_result.error.is_none(), "{:?}", midi_result.error);
        assert_eq!(midi_result.tracks.len(), 1);
        assert_eq!(midi_result.tracks[0].notes.len(), 1);
        let note = &midi_result.tracks[0].notes[0];
        assert_eq!(note.pitch, 64);
        assert_eq!(note.start, 0.0);
        assert!((note.length - 0.01).abs() < 1e-8);
    }
}
