//! Session-only arming/monitoring and atomic publication of completed captures.
//! Microphone ownership and disk writing belong to the engine; only finalized
//! files become document clips. Every recording is one undoable transaction.
use crate::app::{DawApp, PopupKind, SetupPage, WorkspaceView};
use anyhow::Result;
use daw_core::{
    AudioData, AudioInputChannels, Clip, MidiInputChannel, RecordedAudioPass, RecordingPlacement,
    Track, TrackKind,
};
use daw_engine::{
    DeviceInfo, InputRoute, InputSettings, MidiPortInfo, MidiRecordingResult, MidiRoute,
    MidiSettings, RecordingConfig, RecordingResult, RecordingTiming,
};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MonitorMode {
    #[default]
    Off,
    Auto,
    On,
}
impl MonitorMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Auto => "Auto",
            Self::On => "On",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AfterRecording {
    #[default]
    Continue,
    Pause,
    Stop,
}

struct Session {
    tracks: Vec<Track>,
    tempo: f64,
    loop_recording: bool,
    start: f64,
    placement: RecordingPlacement,
    directory: PathBuf,
    started: bool,
    audio_started: bool,
    midi_started: bool,
    timing_committed: bool,
    finishing: bool,
    after: AfterRecording,
    select_result: bool,
}
struct FinalizedRecording {
    result: RecordingResult,
    previews: HashMap<String, AudioData>,
}

#[derive(Default)]
pub(crate) struct RecordingUi {
    pub(crate) armed: HashSet<u64>,
    monitors: HashMap<u64, MonitorMode>,
    placement: RecordingPlacement,
    session: Option<Session>,
    finishing: Option<mpsc::Receiver<FinalizedRecording>>,
    signature: Option<(u64, InputSettings)>,
    midi_signature: Option<(u64, MidiSettings)>,
    timing_signature: Option<(u64, RecordingTiming, f64, u8)>,
    audio_complete: Option<FinalizedRecording>,
    midi_complete: Option<MidiRecordingResult>,
    pub(crate) midi_ports: Vec<MidiPortInfo>,
    virtual_signature: Option<(u64, daw_engine::VirtualIoSettings)>,
    failed_input: bool,
    pub(crate) rebuild_after: bool,
    last_commit: serde_json::Value,
    last_error: Option<String>,
}

impl DawApp {
    /// Shared control-side recording configuration. Call only after validating
    /// every requested route; no UI selection or popup is needed.
    pub(crate) fn configure_recording_controls(
        &mut self,
        controls: &[(u64, Option<bool>, Option<MonitorMode>)],
        placement: Option<RecordingPlacement>,
    ) {
        for &(track, armed, monitor) in controls {
            if let Some(armed) = armed
                && self.recording.armed.contains(&track) != armed
            {
                self.toggle_record_arm(track);
            }
            if let Some(monitor) = monitor {
                self.recording.monitors.insert(track, monitor);
            }
        }
        if let Some(placement) = placement {
            self.recording.placement = placement;
        }
        self.recording.failed_input = false;
    }

    pub(crate) fn retry_recording_input(&mut self) {
        self.recording.failed_input = false;
        self.recording.signature = None;
        self.recording.midi_signature = None;
    }

    pub(crate) fn recording_busy(&self) -> bool {
        self.recording.session.is_some()
    }
    pub(crate) fn recording_finalizing(&self) -> bool {
        self.recording.finishing.is_some()
            || self
                .recording
                .session
                .as_ref()
                .is_some_and(|session| session.finishing)
    }

    pub(crate) fn recording_engine_failed(&mut self, error: &anyhow::Error) {
        if self.recording_busy()
            || self
                .recording
                .monitors
                .values()
                .any(|mode| *mode != MonitorMode::Off)
        {
            self.recording.failed_input = true;
            self.recording.last_error = Some(format!("{error:#}"));
            self.finish_recording(AfterRecording::Pause);
        }
    }
    pub(crate) fn refresh_input_devices(&mut self) {
        if self.virtual_audio {
            self.input_devices = vec![DeviceInfo {
                name: "Rill virtual input".into(),
                is_default: true,
                supported_sample_rates: vec![44100, 48000, 88200, 96000],
                channels: self.virtual_io.input_channels,
            }];
        } else if !self.audio_device_enabled {
            self.input_devices.clear();
        } else {
            match daw_engine::list_input_devices() {
                Ok(devices) => self.input_devices = devices,
                Err(error) => self.message(Err(error.context("Enumerate audio inputs")), ""),
            }
        }
    }

    pub(crate) fn refresh_midi_devices(&mut self) {
        if self.virtual_audio {
            self.recording.midi_ports = vec![MidiPortInfo {
                id: "rill-virtual-midi".into(),
                name: "Rill virtual MIDI input".into(),
            }];
        } else if !self.audio_device_enabled {
            self.recording.midi_ports.clear();
        } else {
            match daw_engine::list_midi_inputs() {
                Ok(ports) => self.recording.midi_ports = ports,
                Err(error) => self.message(Err(error.context("Enumerate MIDI inputs")), ""),
            }
        }
    }

    fn monitor_enabled(&self, track: u64) -> bool {
        if self.document_guard.is_some() {
            return false;
        }
        match self
            .recording
            .monitors
            .get(&track)
            .copied()
            .unwrap_or_default()
        {
            MonitorMode::Off => false,
            MonitorMode::Auto => {
                self.recording.armed.contains(&track) && (!self.playing() || self.recording_busy())
            }
            MonitorMode::On => true,
        }
    }

    fn desired_midi_input(&self) -> MidiSettings {
        if self.settings.midi_input_device.is_none() {
            return MidiSettings::default();
        }
        let mut routes = Vec::new();
        for track in self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Midi)
        {
            let Some(input) = track.midi_input else {
                continue;
            };
            let monitor = self.monitor_enabled(track.id);
            if monitor || (self.recording_busy() && self.recording.armed.contains(&track.id)) {
                routes.push(MidiRoute {
                    track: track.id,
                    channel: match input {
                        MidiInputChannel::All => None,
                        MidiInputChannel::Channel { channel } => Some(channel),
                    },
                    monitor,
                });
            }
        }
        MidiSettings {
            port_id: self.settings.midi_input_device.clone(),
            routes,
        }
    }

    fn apply_recording_timing(&mut self) -> Result<()> {
        if let Some(engine) = self.engine.as_mut() {
            let options = RecordingTiming {
                precount_bars: self.project.precount_bars,
                click_enabled: self.project.metronome_enabled,
                click_gain: self.project.metronome_gain,
            };
            let signature = (
                engine.snapshot().engine_id,
                options,
                self.project.tempo,
                self.project.beats_per_bar,
            );
            if self.recording.timing_signature != Some(signature) {
                engine.configure_recording_timing(&self.project, options)?;
                self.recording.timing_signature = Some(signature);
            }
        }
        Ok(())
    }

    pub(crate) fn open_recording(&mut self, track: u64) {
        self.selected_track = Some(track);
        self.open_popup(PopupKind::Recording);
    }

    pub(crate) fn toggle_record_arm(&mut self, track: u64) {
        if self.recording_busy() {
            return;
        }
        if !self.recording.armed.remove(&track) {
            self.recording.armed.insert(track);
            if self
                .project
                .track(track)
                .is_some_and(|track| track.kind == TrackKind::Midi)
            {
                self.recording
                    .monitors
                    .entry(track)
                    .or_insert(MonitorMode::Auto);
            }
        }
        self.recording.failed_input = false;
    }

    pub(crate) fn begin_recording(&mut self) {
        if self.recording_busy() {
            return;
        }
        if self.project_io_pending() || self.state_sync_pending() || self.document_guard.is_some() {
            self.message(
                Err(anyhow::anyhow!(
                    "Wait for the current project operation before recording"
                )),
                "",
            );
            return;
        }
        let targets: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| {
                matches!(track.kind, TrackKind::Audio | TrackKind::Midi)
                    && self.recording.armed.contains(&track.id)
            })
            .cloned()
            .collect();
        if targets.is_empty() {
            if let Some(track) = self.selected_track.filter(|track| {
                self.project
                    .track(*track)
                    .is_some_and(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
            }) {
                self.open_recording(track);
            }
            self.status = "Arm an audio or MIDI track to record".into();
            return;
        }
        if targets.iter().any(|track| match track.kind {
            TrackKind::Audio => track.audio_input.is_none(),
            TrackKind::Midi => {
                track.midi_input.is_none() || self.settings.midi_input_device.is_none()
            }
            _ => true,
        }) {
            self.message(
                Err(anyhow::anyhow!(
                    "Choose an input for every armed audio or MIDI track"
                )),
                "",
            );
            return;
        }
        if !self.audio_device_enabled && !self.virtual_audio {
            self.message(
                Err(anyhow::anyhow!(
                    "An audio input/output device is required for recording"
                )),
                "",
            );
            return;
        }
        self.flush_pending_edit();
        self.history.seal_group();
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.recording.session = Some(Session {
            tracks: targets,
            tempo: self.project.tempo,
            loop_recording: self.project.loop_enabled,
            start: self.cursor,
            placement: self.recording.placement,
            directory: self
                .settings
                .scratch_dir
                .join("Recordings")
                .join(format!("{serial}-{}", daw_core::new_id())),
            started: false,
            audio_started: false,
            midi_started: false,
            timing_committed: false,
            finishing: false,
            after: AfterRecording::Continue,
            select_result: true,
        });
        self.recording.failed_input = false;
        self.recording.signature = None;
        self.recording.last_error = None;
        self.recording.last_commit = serde_json::Value::Null;
        self.recording.audio_complete = None;
        self.recording.midi_complete = None;
        self.recording.midi_signature = None;
        self.status = "Preparing recording…".into();
        self.error = false;
        if self.engine.is_none() && self.loading.is_none() {
            self.prepare_engine(false);
        }
    }

    /// Tool-driven capture leaves the user's editing selection in place, even
    /// when its asynchronously finalized clips are published later.
    pub(crate) fn begin_recording_from_tool(&mut self) {
        self.begin_recording();
        if let Some(session) = self.recording.session.as_mut() {
            session.select_result = false;
        }
    }

    pub(crate) fn finish_recording(&mut self, after: AfterRecording) {
        let Some(session) = self.recording.session.as_mut() else {
            return;
        };
        session.after = session.after.max(after);
        session.finishing = true;
        if !session.started {
            self.recording.session = None;
            if let Some(engine) = &self.engine {
                engine.cancel_recording_timing();
                self.desired_playing = engine.snapshot().playing;
            }
            if after >= AfterRecording::Pause {
                self.desired_playing = false;
                if let Some(engine) = &self.engine {
                    engine.set_playing(false);
                }
            }
            if after == AfterRecording::Stop {
                self.stop();
            }
            if std::mem::take(&mut self.recording.rebuild_after) {
                self.rebuild();
            }
            return;
        }
        if let Some(engine) = &self.engine {
            engine.request_stop_recording();
            engine.request_stop_midi_recording();
            engine.cancel_recording_timing();
            // Cancelling a count-in returns to stopped; finishing a captured
            // take with Continue keeps the backing transport running.
            self.desired_playing = engine.snapshot().playing;
            if after >= AfterRecording::Pause {
                engine.set_playing(false);
                self.desired_playing = false;
            }
        }
        if !self.error {
            self.status = "Finishing recording…".into();
        }
    }

    fn desired_input(&self) -> InputSettings {
        let playing = self.playing();
        let capturing = self.recording_busy();
        let mut routes = Vec::new();
        for track in self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
        {
            let Some(channels) = track.audio_input else {
                continue;
            };
            let armed = self.recording.armed.contains(&track.id);
            let monitor = self.document_guard.is_none()
                && match self
                    .recording
                    .monitors
                    .get(&track.id)
                    .copied()
                    .unwrap_or_default()
                {
                    MonitorMode::Off => false,
                    MonitorMode::Auto => armed && (!playing || capturing),
                    MonitorMode::On => true,
                };
            if monitor || (capturing && armed) {
                routes.push(InputRoute {
                    track: track.id,
                    channels,
                    monitor,
                });
            }
        }
        InputSettings {
            device_name: self.settings.input_device_name.clone(),
            routes,
        }
    }

    pub(crate) fn poll_recording(&mut self, ctx: &egui::Context) {
        self.recording.armed.retain(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
        });
        self.recording.monitors.retain(|id, _| {
            self.project
                .track(*id)
                .is_some_and(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
        });
        if let Some(engine) = self.engine.as_mut() {
            if self.virtual_audio {
                let signature = (engine.snapshot().engine_id, self.virtual_io);
                if self.recording.virtual_signature != Some(signature) {
                    if let Err(error) = engine.configure_virtual_io(self.virtual_io) {
                        self.message(Err(error), "");
                        return;
                    }
                    self.recording.virtual_signature = Some(signature);
                }
            }
            engine.poll_input();
            engine.poll_midi();
        }
        if let Err(error) = self.apply_recording_timing() {
            self.recording.failed_input = true;
            self.finish_recording(AfterRecording::Pause);
            self.message(Err(error), "");
        }
        let errors = self
            .engine
            .as_mut()
            .map(|engine| [engine.take_input_error(), engine.take_midi_error()])
            .unwrap_or_default();
        for error in errors.into_iter().flatten() {
            self.recording.failed_input = true;
            self.recording.last_error = Some(error.clone());
            self.finish_recording(AfterRecording::Pause);
            self.message(Err(anyhow::anyhow!(error)), "");
        }
        if let Some(result) = self
            .engine
            .as_mut()
            .and_then(|engine| engine.take_recording_result())
        {
            self.finish_recording(AfterRecording::Continue);
            let budget = crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(
                self.waveforms
                    .values()
                    .map(|audio| audio.samples.len() * size_of::<f32>())
                    .sum(),
            );
            let (tx, rx) = mpsc::channel();
            self.recording.finishing = Some(rx);
            std::thread::spawn(move || {
                let mut previews = HashMap::new();
                let mut remaining = budget;
                for take in &result.takes {
                    if let Ok(audio) = AudioData::load_wav_with_limit(
                        &take.path,
                        remaining.min(crate::project_io::WAVEFORM_FILE_BYTES),
                    ) {
                        remaining =
                            remaining.saturating_sub(audio.samples.len() * size_of::<f32>());
                        previews.insert(take.path.to_string_lossy().into_owned(), audio);
                    }
                }
                let _ = tx.send(FinalizedRecording { result, previews });
            });
        }
        if let Some(result) = self
            .engine
            .as_mut()
            .and_then(|engine| engine.take_midi_recording_result())
        {
            self.finish_recording(AfterRecording::Continue);
            self.recording.midi_complete = Some(result);
        }
        if let Some(receiver) = &self.recording.finishing {
            match receiver.try_recv() {
                Ok(done) => {
                    self.recording.finishing = None;
                    self.recording.audio_complete = Some(done);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.recording.finishing = None;
                    self.recording.audio_complete = Some(FinalizedRecording { result: RecordingResult { takes: Vec::new(), error: Some("Recording preview finalization stopped; captured files remain in the recording directory".into()), dropped_frames: 0 }, previews: HashMap::new() });
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let complete = self.recording.session.as_ref().is_some_and(|session| {
            session.finishing
                && session.started
                && (!session.audio_started || self.recording.audio_complete.is_some())
                && (!session.midi_started || self.recording.midi_complete.is_some())
        });
        if complete {
            let audio =
                self.recording
                    .audio_complete
                    .take()
                    .unwrap_or_else(|| FinalizedRecording {
                        result: RecordingResult {
                            takes: Vec::new(),
                            error: None,
                            dropped_frames: 0,
                        },
                        previews: HashMap::new(),
                    });
            let midi = self.recording.midi_complete.take().unwrap_or_default();
            self.publish_recording(audio, midi);
        }
        let wanted = self.desired_input();
        let midi_wanted = self.desired_midi_input();
        if self.recording.failed_input && !self.recording_busy() {
            if let Some(engine) = self.engine.as_mut() {
                let _ = engine.disable_input();
                let _ = engine.disable_midi();
            }
            self.recording.signature = None;
            self.recording.midi_signature = None;
        }
        let routes_mutable = self
            .recording
            .session
            .as_ref()
            .is_none_or(|session| !session.started);
        if !self.recording.failed_input && self.recording.finishing.is_none() && routes_mutable {
            if self.engine.is_none()
                && self.loading.is_none()
                && (!wanted.routes.is_empty() || !midi_wanted.routes.is_empty())
                && (self.audio_device_enabled || self.virtual_audio)
            {
                self.prepare_engine(false);
            }
            let configured = (|| -> Result<()> {
                let Some(engine) = self.engine.as_mut() else {
                    return Ok(());
                };
                let id = engine.snapshot().engine_id;
                let signature = (id, wanted.clone());
                if self.recording.signature.as_ref() != Some(&signature) {
                    if wanted.routes.is_empty() {
                        engine.disable_input()?;
                    } else {
                        engine.configure_input(&self.project, wanted)?;
                    }
                    self.recording.signature = Some(signature);
                }
                let signature = (id, midi_wanted.clone());
                if self.recording.midi_signature.as_ref() != Some(&signature) {
                    if midi_wanted.routes.is_empty() {
                        engine.disable_midi()?;
                    } else {
                        engine.configure_midi(&self.project, midi_wanted)?;
                    }
                    self.recording.midi_signature = Some(signature);
                }
                Ok(())
            })();
            if let Err(error) = configured {
                self.recording.failed_input = true;
                self.finish_recording(AfterRecording::Pause);
                self.recording.last_error = Some(format!("{error:#}"));
                self.message(Err(error), "");
            }
        }
        let can_start = !self.recording.failed_input
            && self.recording.session.as_ref().is_some_and(|session| {
                !session.started
                    && !session.finishing
                    && self.engine.as_ref().is_some_and(|engine| {
                        (!session
                            .tracks
                            .iter()
                            .any(|track| track.kind == TrackKind::Audio)
                            || engine.recording_snapshot().input_ready)
                            && (!session
                                .tracks
                                .iter()
                                .any(|track| track.kind == TrackKind::Midi)
                                || engine.midi_snapshot().input_ready)
                    })
            });
        if can_start {
            let session = self.recording.session.as_ref().unwrap();
            let audio: Vec<_> = session
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Audio)
                .map(|track| track.id)
                .collect();
            let midi: Vec<_> = session
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Midi)
                .map(|track| track.id)
                .collect();
            let directory = session.directory.clone();
            let started = (|| -> Result<()> {
                let engine = self.engine.as_mut().unwrap();
                engine.arm_recording_timing(!audio.is_empty(), !midi.is_empty())?;
                if !audio.is_empty() {
                    engine.start_recording(RecordingConfig {
                        directory,
                        tracks: audio,
                    })?;
                    let session = self.recording.session.as_mut().unwrap();
                    session.audio_started = true;
                    session.started = true;
                }
                if !midi.is_empty() {
                    engine.start_midi_recording(midi)?;
                    let session = self.recording.session.as_mut().unwrap();
                    session.midi_started = true;
                    session.started = true;
                }
                Ok(())
            })();
            if let Err(error) = started {
                self.recording.failed_input = true;
                self.recording.last_error = Some(format!("{error:#}"));
                self.finish_recording(AfterRecording::Pause);
                self.message(Err(error), "");
            }
        }
        let ready = self.recording.session.as_ref().is_some_and(|session| {
            session.started
                && !session.finishing
                && !session.timing_committed
                && self.engine.as_ref().is_some_and(|engine| {
                    (!session.audio_started || engine.recording_snapshot().active)
                        && (!session.midi_started || engine.midi_snapshot().active)
                })
        });
        if ready {
            match self.engine.as_ref().unwrap().commit_recording_timing() {
                Ok(()) => {
                    self.recording.session.as_mut().unwrap().timing_committed = true;
                    self.desired_playing = true;
                    self.status = "Recording".into();
                }
                Err(error) => {
                    self.finish_recording(AfterRecording::Pause);
                    self.message(Err(error), "");
                }
            }
        }
        if self.recording_busy()
            || self.engine.as_ref().is_some_and(|engine| {
                engine.recording_snapshot().input_opening || engine.midi_snapshot().input_opening
            })
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn publish_recording(&mut self, done: FinalizedRecording, midi: MidiRecordingResult) {
        let Some(session) = self.recording.session.take() else {
            return;
        };
        let paths: Vec<_> = done
            .result
            .takes
            .iter()
            .map(|take| take.path.clone())
            .collect();
        let result = (|| -> Result<Vec<daw_core::RecordedAudioCommit>> {
            let mut project = self.project.clone();
            let mut commits = Vec::new();
            for source in &session.tracks {
                let takes: Vec<_> = done
                    .result
                    .takes
                    .iter()
                    .filter(|take| take.track == source.id && take.frames > 0)
                    .collect();
                if takes.is_empty() {
                    continue;
                }
                let mut target = source.id;
                if project.track(target).is_none() {
                    target =
                        project.add_audio_track(format!("{} · recovered recording", source.name));
                    let bus = source.output_bus.filter(|id| project.track(*id).is_some());
                    let track = project.track_mut(target).unwrap();
                    track.gain = source.gain;
                    track.pan = source.pan;
                    track.output_bus = bus;
                    track.audio_input = source.audio_input;
                }
                let session_number = project.track(target).unwrap().clips.len() + 1;
                let passes = takes
                    .into_iter()
                    .enumerate()
                    .map(|(index, take)| {
                        let length =
                            take.frames as f64 / take.sample_rate as f64 * session.tempo / 60.0;
                        RecordedAudioPass {
                            clip: Clip::audio(
                                format!(
                                    "{} · Recording {session_number} · Take {}",
                                    source.name,
                                    index + 1
                                ),
                                take.start_beat,
                                length,
                                take.path.to_string_lossy(),
                            ),
                            complete: take.complete,
                        }
                    })
                    .collect();
                commits.push(project.insert_recorded_audio(
                    target,
                    passes,
                    session.placement,
                    session.loop_recording,
                )?);
            }
            let midi_count = midi.tracks.len();
            for recorded in &midi.tracks {
                if recorded.notes.is_empty() {
                    continue;
                }
                let Some(source) = session
                    .tracks
                    .iter()
                    .find(|track| track.id == recorded.track)
                else {
                    continue;
                };
                let target = if project.track(recorded.track).is_some() {
                    recorded.track
                } else {
                    let id =
                        project.add_midi_track(format!("{} · recovered recording", source.name));
                    let bus = source.output_bus.filter(|id| project.track(*id).is_some());
                    let track = project.track_mut(id).unwrap();
                    track.instrument = source.instrument.clone();
                    track.instrument_state = source.instrument_state.clone();
                    track.instrument_parameters = source.instrument_parameters.clone();
                    track.gain = source.gain;
                    track.pan = source.pan;
                    track.output_bus = bus;
                    track.midi_input = source.midi_input;
                    id
                };
                let clip = Clip::midi(
                    format!(
                        "{} · Recording {}",
                        source.name,
                        project.track(target).unwrap().clips.len() + 1
                    ),
                    recorded.start_beat,
                    recorded.length_beats,
                    recorded.notes.clone(),
                );
                let clip = project.insert_recorded_midi(target, clip)?;
                commits.push(daw_core::RecordedAudioCommit {
                    track: target,
                    clip,
                    group: None,
                });
            }
            if !commits.is_empty() {
                project.validate()?;
                self.begin_edit();
                self.project = project;
                self.waveforms.extend(done.previews);
                self.edit_label = Some(
                    if midi_count > 0 && !done.result.takes.is_empty() {
                        "Record MIDI and audio"
                    } else if midi_count > 0 {
                        "Record MIDI"
                    } else if session.placement == RecordingPlacement::Overdub {
                        "Record audio overdub"
                    } else {
                        "Record audio takes"
                    }
                    .into(),
                );
                self.changed();
                self.flush_pending_edit();
                if session.select_result
                    && let Some(first) = commits.first()
                {
                    self.selected_track = Some(first.track);
                    self.selected_clip = Some(first.clip);
                }
            }
            Ok(commits)
        })();
        match result {
            Ok(commits) => {
                self.recording.last_commit = serde_json::json!({"tracks":commits.iter().map(|commit| serde_json::json!({"track":commit.track,"clip":commit.clip,"group":commit.group})).collect::<Vec<_>>(), "files":paths, "takes":done.result.takes.len(), "midi_clips":midi.tracks.len()});
                let errors: Vec<_> = [done.result.error, midi.error]
                    .into_iter()
                    .flatten()
                    .collect();
                if !errors.is_empty() {
                    let error = errors.join("\n\n");
                    self.recording.failed_input = true;
                    self.recording.last_error = Some(error.clone());
                    let recovery = if paths.is_empty() {
                        format!(
                            "Recovered {} MIDI clip(s) in the project",
                            midi.tracks.len()
                        )
                    } else {
                        format!(
                            "Recovered {} recorded file(s) in {}",
                            paths.len(),
                            session.directory.display()
                        )
                    };
                    self.message(Err(anyhow::anyhow!("{error}\n\n{recovery}")), "");
                } else if !self.recording.failed_input && !self.error {
                    self.status = if commits.is_empty() {
                        "Recording cancelled · No input captured".into()
                    } else {
                        format!(
                            "Recorded {} audio take(s) · {} MIDI clip(s)",
                            done.result.takes.len(),
                            midi.tracks.len()
                        )
                    };
                }
            }
            Err(error) => self.message(
                Err(error.context(format!(
                    "Could not attach recorded audio; files remain in {}",
                    session.directory.display()
                ))),
                "",
            ),
        }
        self.recording.signature = None;
        self.recording.midi_signature = None;
        if let Some(engine) = &self.engine {
            engine.cancel_recording_timing();
        }
        if session.after == AfterRecording::Stop {
            self.stop();
        } else if session.after == AfterRecording::Pause {
            if let Some(engine) = &self.engine {
                engine.set_playing(false);
            }
            self.desired_playing = false;
        }
        if std::mem::take(&mut self.recording.rebuild_after) {
            self.rebuild();
        }
    }

    pub(crate) fn recording_region(&self, track: u64) -> Option<(f64, f64)> {
        let session = self.recording.session.as_ref()?;
        if !session.timing_committed
            || self
                .engine
                .as_ref()
                .is_some_and(|engine| engine.timing_snapshot().counting)
            || session.finishing
            || !session.tracks.iter().any(|source| source.id == track)
        {
            return None;
        }
        let looped = self
            .engine
            .as_ref()
            .is_some_and(|engine| engine.recording_snapshot().completed_passes > 0);
        let start = if session.loop_recording && (looped || self.cursor < session.start) {
            self.project.loop_start
        } else {
            session.start
        };
        Some((start, self.cursor.max(start + 0.01)))
    }

    pub(crate) fn recording_popup(&mut self, ctx: &egui::Context) {
        if !self.recording_open {
            return;
        }
        let track = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .filter(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
            .cloned();
        let title = track
            .as_ref()
            .map(|track| format!("Recording · {}", track.name))
            .unwrap_or_else(|| "Recording".into());
        let mut open = true;
        daw_ui::popup(title, ctx).id(egui::Id::new("recording_popup")).open(&mut open).default_width(460.0).default_height(560.0).vscroll(false).show(ctx, |ui| {
            let busy = self.recording_busy();
            daw_ui::popup_scroll_body(ui, "recording_inputs", 200.0, |ui| {
            if let Some(track) = &track {
                let midi = track.kind == TrackKind::Midi;
                ui.add_enabled_ui(!busy, |ui| {
                    if midi {
                        let mut device = self.settings.midi_input_device.clone();
                        daw_ui::form_grid(ui, "recording_midi_device", |ui| {
                            let label = daw_ui::text::label(ui, "MIDI device");
                            let selected = device.as_ref().map(|id| self.recording.midi_ports.iter().find(|port| &port.id == id).map(|port| port.name.as_str()).unwrap_or("Unavailable MIDI input")).unwrap_or("No MIDI input");
                            daw_ui::select("recording_midi_port", selected).show_ui(ui, |ui| {
                                daw_ui::selectable_value(ui, &mut device, None, "No MIDI input");
                                for port in &self.recording.midi_ports { daw_ui::selectable_value(ui, &mut device, Some(port.id.clone()), &port.name); }
                            }).response.labelled_by(label.id).on_hover_text("Input device shared by MIDI tracks; choose each track's channel below");
                            ui.end_row();
                        });
                        if device != self.settings.midi_input_device {
                            let mut settings = self.settings.clone(); settings.midi_input_device = device;
                            match settings.save() {
                                Ok(()) => { self.settings = settings; self.setup_draft = self.settings.clone(); self.recording.failed_input = false; self.recording.midi_signature = None; }
                                Err(error) => self.message(Err(error), ""),
                            }
                        }
                        if daw_ui::small_button(ui, "Refresh MIDI inputs").clicked() { self.refresh_midi_devices(); }
                        let mut input = track.midi_input;
                        daw_ui::form_grid(ui, "recording_midi_channel", |ui| {
                            let label = daw_ui::text::label(ui, "Channel");
                            daw_ui::select("track_midi_input", midi_input_label(input)).show_ui(ui, |ui| {
                                daw_ui::selectable_value(ui, &mut input, None, "No input");
                                daw_ui::selectable_value(ui, &mut input, Some(MidiInputChannel::All), "All channels");
                                for channel in 0..16 { let choice = Some(MidiInputChannel::Channel { channel }); daw_ui::selectable_value(ui, &mut input, choice, midi_input_label(choice)); }
                            }).response.labelled_by(label.id);
                            ui.end_row();
                        });
                        if input != track.midi_input { self.begin_edit(); self.project.track_mut(track.id).unwrap().midi_input = input; self.edit_label = Some("Choose MIDI input channel".into()); self.changed(); self.recording.failed_input = false; }
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            daw_ui::text::note(ui, self.settings.input_device_name.as_deref().unwrap_or("System default input"));
                            if daw_ui::button(ui, "Audio setup…").clicked() { self.setup_page = SetupPage::Audio; self.open_popup(PopupKind::Setup); }
                        });
                        let channels = self.input_devices.iter().find(|device| self.settings.input_device_name.as_ref().map_or(device.is_default, |name| name == &device.name)).map_or(2, |device| device.channels.min(32));
                        let mut input = track.audio_input;
                        daw_ui::form_grid(ui, "recording_input", |ui| {
                            let label = daw_ui::text::label(ui, "Input");
                            daw_ui::select("track_audio_input", input_label(input)).show_ui(ui, |ui| {
                                daw_ui::selectable_value(ui, &mut input, None, "No input");
                                for channel in 0..channels { let choice = Some(AudioInputChannels::Mono {channel}); daw_ui::selectable_value(ui, &mut input, choice, input_label(choice)); }
                                for left in (0..channels.saturating_sub(1)).step_by(2) { let choice = Some(AudioInputChannels::Stereo {left,right:left+1}); daw_ui::selectable_value(ui, &mut input, choice, input_label(choice)); }
                            }).response.labelled_by(label.id); ui.end_row();
                        });
                        if input != track.audio_input { self.begin_edit(); self.project.track_mut(track.id).unwrap().audio_input = input; self.edit_label = Some("Choose recording input".into()); self.changed(); self.recording.failed_input = false; }
                    }
                    let mut armed = self.recording.armed.contains(&track.id);
                    if daw_ui::checkbox(ui, &mut armed, "Arm recording").changed() { self.toggle_record_arm(track.id); }
                    let mut mode = self.recording.monitors.get(&track.id).copied().unwrap_or_default();
                    daw_ui::form_grid(ui, "recording_monitor", |ui| {
                        let label = daw_ui::text::label(ui, if midi { "Input monitor" } else { "Input monitor (dry)" });
                        daw_ui::select("input_monitor_mode", mode.label()).show_ui(ui, |ui| { for choice in [MonitorMode::Off,MonitorMode::Auto,MonitorMode::On] { daw_ui::selectable_value(ui, &mut mode, choice, choice.label()); } }).response.labelled_by(label.id); ui.end_row();
                        if !midi {
                            let label = daw_ui::text::label(ui, "Record into");
                            daw_ui::select("recording_placement", if self.recording.placement == RecordingPlacement::Take {"New take"} else {"Overdub layer"}).show_ui(ui, |ui| {
                                daw_ui::selectable_value(ui, &mut self.recording.placement, RecordingPlacement::Take, "New take");
                                daw_ui::selectable_value(ui, &mut self.recording.placement, RecordingPlacement::Overdub, "Overdub layer");
                            }).response.labelled_by(label.id); ui.end_row();
                        }
                    });
                    if mode != self.recording.monitors.get(&track.id).copied().unwrap_or_default() { self.recording.monitors.insert(track.id, mode); self.recording.failed_input = false; }
                });
                if midi {
                    if matches!(track.instrument, daw_core::Instrument::None) {
                        daw_ui::text::note(ui, "Choose an instrument to hear incoming MIDI.");
                        if daw_ui::button(ui, "Choose instrument…").clicked() { self.open_instrument_chooser(true); }
                    }
                    daw_ui::text::note(ui, if self.project.loop_enabled { "Loop passes build up in one new MIDI clip. Existing clips stay intact." } else { "Record notes into a new MIDI clip. Existing clips stay intact." });
                } else {
                    let peaks = self.engine.as_ref().map_or([0.0;2], |engine| engine.input_peaks(track.id));
                    daw_ui::input_meter(ui, peaks);
                    daw_ui::text::note(ui, "Dry monitoring follows channel and output-bus levels. Inserts and sends are bypassed; recorded audio stays unchanged.");
                    if self.project.loop_enabled { daw_ui::text::note(ui, "Loop recording keeps each pass as a separate take."); }
                }
                ui.separator();
            } else { daw_ui::text::note(ui, "Select an audio or MIDI track to choose its input."); }
            });
            let (mut precount, mut click, mut gain) = (self.project.precount_bars, self.project.metronome_enabled, self.project.metronome_gain);
            if daw_ui::recording_timing_controls(ui, &mut precount, &mut click, &mut gain, !busy) {
                self.begin_edit(); self.project.precount_bars = precount; self.project.metronome_enabled = click; self.project.metronome_gain = gain;
                self.edit_label = Some("Recording timing".into()); self.changed();
            }
            ui.horizontal_wrapped(|ui| {
                if daw_ui::record_button(ui, busy, !self.recording_finalizing()).clicked() {
                    if busy { self.finish_recording(AfterRecording::Continue); } else { self.begin_recording(); }
                }
                if self.recording.failed_input && daw_ui::button(ui, "Retry input").clicked() {
                    self.recording.failed_input = false; self.recording.signature = None; self.recording.midi_signature = None;
                }
                if let Some(clip) = self.selected().cloned() {
                    if daw_ui::button(ui, "Edit recording").clicked() { self.close_popups(); self.view = WorkspaceView::Clip; }
                    if clip.take_group().is_some() && daw_ui::button(ui, "Takes…").clicked() { self.open_audio_takes(&clip, None); }
                }
            });
        });
        self.recording_open &= open;
    }

    pub(crate) fn recording_state(&self) -> serde_json::Value {
        let snapshot = self
            .engine
            .as_ref()
            .map(|engine| engine.recording_snapshot())
            .unwrap_or_default();
        let midi = self
            .engine
            .as_ref()
            .map(|engine| engine.midi_snapshot())
            .unwrap_or_default();
        let timing = self
            .engine
            .as_ref()
            .map(|engine| engine.timing_snapshot())
            .unwrap_or_default();
        let phase = if self.recording_finalizing() {
            "finalizing"
        } else if self.recording_busy() {
            if timing.counting {
                "counting_in"
            } else if self
                .recording
                .session
                .as_ref()
                .is_some_and(|session| session.timing_committed)
            {
                "recording"
            } else {
                "starting"
            }
        } else {
            "idle"
        };
        serde_json::json!({"phase":phase,
            "midi":{"input_ready":midi.input_ready,"input_opening":midi.input_opening,"active":midi.active,"recording":midi.recording,"events":midi.events,"held_notes":midi.held_notes},
            "midi_input_device":self.settings.midi_input_device,
            "timing":{"armed":timing.armed,"counting":timing.counting,"capturing":timing.capturing,"remaining_beats":timing.remaining_beats,"remaining_frames":timing.remaining_frames,"captured_frames":timing.captured_frames},
            "armed":self.recording.armed,"monitors":self.recording.monitors.iter().map(|(id,mode)| (id.to_string(),mode.label())).collect::<HashMap<_,_>>(),
            "input_ready":snapshot.input_ready,"input_opening":snapshot.input_opening,"input_channels":snapshot.input_channels,
            "frames":snapshot.frames,"completed_passes":snapshot.completed_passes,"dropped_frames":snapshot.dropped_frames,"input_peaks":snapshot.input_peaks,
            "dummy_device":{"input_channels":self.virtual_io.input_channels,"input_latency_frames":self.virtual_io.input_latency_frames,"output_latency_frames":self.virtual_io.output_latency_frames},
            "last_commit":self.recording.last_commit,"error":self.recording.last_error,"input_device_name":self.settings.input_device_name,
            "placement":if self.recording.placement == RecordingPlacement::Take {"take"} else {"overdub"}})
    }

    pub(crate) fn recording_diagnostics(&self, ctx: &egui::Context) -> serde_json::Value {
        let mut state = self.recording_state();
        state["input_device_rect"] = serde_json::json!(
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("input_device_picker_rect")))
                .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
        );
        state
    }
}

fn input_label(input: Option<AudioInputChannels>) -> String {
    match input {
        None => "No input".into(),
        Some(AudioInputChannels::Mono { channel }) => format!("Mono {}", channel + 1),
        Some(AudioInputChannels::Stereo { left, right }) => {
            format!("Stereo {}–{}", left + 1, right + 1)
        }
    }
}

fn midi_input_label(input: Option<MidiInputChannel>) -> String {
    match input {
        None => "No input".into(),
        Some(MidiInputChannel::All) => "All channels".into(),
        Some(MidiInputChannel::Channel { channel }) => format!("Channel {}", channel + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Project;
    use daw_engine::{AudioEngine, DeviceSettings};
    use std::sync::{Arc, atomic::AtomicBool};

    fn midi_app(precount_bars: u8) -> (egui::Context, DawApp, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let context = egui::Context::default();
        let mut project = Project::new();
        project.precount_bars = precount_bars;
        let track = project.add_midi_track("Recording input");
        let engine = AudioEngine::start_virtual_cancellable(
            &project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let mut app = DawApp::with_context(&context, project, None);
        app.virtual_audio = true;
        app.settings = crate::settings::Settings {
            scratch_dir: directory.path().into(),
            midi_input_device: Some("rill-virtual-midi".into()),
            ..Default::default()
        };
        app.engine = Some(engine);
        app.recording.armed.insert(track);
        (context, app, directory)
    }

    fn prepared_replacement(app: &mut DawApp) -> AudioEngine {
        let (_, result) = app
            .loading
            .take()
            .expect("The requested graph rebuild must start")
            .recv_timeout(Duration::from_secs(5))
            .expect("Virtual graph preparation must finish");
        result.unwrap()
    }

    #[test]
    fn cancelling_count_in_does_not_resume_playback_on_a_later_rebuild() {
        let (context, mut app, _directory) = midi_app(1);
        app.begin_recording();
        app.poll_recording(&context);
        assert!(app.engine.as_ref().unwrap().timing_snapshot().counting);
        assert!(app.desired_playing);
        app.finish_recording(AfterRecording::Continue);
        app.poll_recording(&context);
        assert!(!app.recording_busy());
        assert!(!app.playing());
        assert!(!app.desired_playing);

        app.project.add_audio_track("Added after cancellation");
        app.rebuild();
        assert!(!app.desired_playing);
        assert!(!prepared_replacement(&mut app).snapshot().playing);
    }

    #[test]
    fn finishing_an_active_take_keeps_playback_running() {
        let (context, mut app, _directory) = midi_app(0);
        app.begin_recording();
        app.poll_recording(&context);
        app.engine.as_mut().unwrap().pump_virtual(512).unwrap();
        assert!(app.engine.as_ref().unwrap().timing_snapshot().capturing);
        app.finish_recording(AfterRecording::Continue);
        app.poll_recording(&context);
        assert!(!app.recording_busy());
        assert!(app.playing());
        assert!(app.desired_playing);
    }

    #[test]
    fn successful_recording_commit_preserves_an_unrelated_active_error_and_its_details() {
        let (context, mut app, _directory) = midi_app(0);
        app.begin_recording();
        app.poll_recording(&context);
        let engine = app.engine.as_mut().unwrap();
        engine.inject_virtual_midi(&[0x90, 60, 100], 0).unwrap();
        engine.pump_virtual(512).unwrap();
        app.message(
            Err(anyhow::anyhow!("Isolated worker did not respond")
                .context("Instrument processing failed")),
            "",
        );
        let status = app.status.clone();
        app.finish_recording(AfterRecording::Stop);
        assert_eq!(
            app.status, status,
            "Finishing must preserve the actual error"
        );
        app.poll_recording(&context);
        assert!(!app.recording_busy());
        assert_eq!(app.recording.last_commit["midi_clips"], 1);
        assert_eq!(app.project.tracks[0].clips.len(), 1);
        assert!(app.error);
        assert_eq!(app.status, status);
        app.show_status_error_details();
        let details = app.error_details_diagnostics();
        assert_eq!(details["summary"], "Instrument processing failed");
        assert!(
            details["details"]
                .as_str()
                .unwrap()
                .contains("Isolated worker did not respond")
        );
    }

    #[test]
    fn rebuild_during_recording_preparation_is_not_lost() {
        let (_context, mut app, _directory) = midi_app(1);
        let previous_engine = app.engine.as_ref().unwrap().snapshot().engine_id;
        app.begin_recording();
        assert!(!app.recording.session.as_ref().unwrap().started);
        app.project.add_audio_track("Added during preparation");
        app.rebuild();
        assert!(!app.recording_busy());
        assert!(!app.recording.rebuild_after);
        assert!(!app.desired_playing);
        let engine = prepared_replacement(&mut app);
        assert_ne!(engine.snapshot().engine_id, previous_engine);
        assert!(engine.try_update_project(&app.project, 1).unwrap());
    }
}
