//! Session-only arming/monitoring and atomic publication of completed captures.
//! Microphone ownership and disk writing belong to the engine; only finalized
//! files become document clips. Every recording is one undoable transaction.
use crate::app::{DawApp, PopupKind, SetupPage, WorkspaceView};
use anyhow::Result;
use daw_core::{
    AudioData, AudioInputChannels, Clip, RecordedAudioPass, RecordingPlacement, Track, TrackKind,
};
use daw_engine::{DeviceInfo, InputRoute, InputSettings, RecordingConfig, RecordingResult};
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
    finishing: bool,
    after: AfterRecording,
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
    virtual_signature: Option<(u64, daw_engine::VirtualIoSettings)>,
    failed_input: bool,
    pub(crate) rebuild_after: bool,
    last_commit: serde_json::Value,
    last_error: Option<String>,
}

impl DawApp {
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

    pub(crate) fn open_recording(&mut self, track: u64) {
        self.selected_track = Some(track);
        self.refresh_input_devices();
        self.open_popup(PopupKind::Recording);
    }

    pub(crate) fn toggle_record_arm(&mut self, track: u64) {
        if self.recording_busy() {
            return;
        }
        if !self.recording.armed.remove(&track) {
            self.recording.armed.insert(track);
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
                track.kind == TrackKind::Audio && self.recording.armed.contains(&track.id)
            })
            .cloned()
            .collect();
        if targets.is_empty() {
            if let Some(track) = self.selected_track.filter(|track| {
                self.project
                    .track(*track)
                    .is_some_and(|track| track.kind == TrackKind::Audio)
            }) {
                self.open_recording(track);
            }
            self.status = "Arm an audio track to record".into();
            return;
        }
        if targets.iter().any(|track| track.audio_input.is_none()) {
            self.message(
                Err(anyhow::anyhow!("Choose an input for every armed track")),
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
            finishing: false,
            after: AfterRecording::Continue,
        });
        self.recording.failed_input = false;
        self.recording.signature = None;
        self.recording.last_error = None;
        self.recording.last_commit = serde_json::Value::Null;
        self.status = "Preparing recording…".into();
        self.error = false;
        if self.engine.is_none() && self.loading.is_none() {
            self.prepare_engine(false);
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
            if after >= AfterRecording::Pause {
                self.desired_playing = false;
                if let Some(engine) = &self.engine {
                    engine.set_playing(false);
                }
            }
            if after == AfterRecording::Stop {
                self.stop();
            }
            return;
        }
        if let Some(engine) = &self.engine {
            engine.request_stop_recording();
            if after >= AfterRecording::Pause {
                engine.set_playing(false);
                self.desired_playing = false;
            }
        }
        self.status = "Finishing recording…".into();
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
                .is_some_and(|track| track.kind == TrackKind::Audio)
        });
        self.recording.monitors.retain(|id, _| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Audio)
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
        }
        let input_error = self
            .engine
            .as_mut()
            .and_then(|engine| engine.take_input_error());
        if let Some(error) = input_error {
            self.recording.failed_input = true;
            self.recording.last_error = Some(error.clone());
            self.finish_recording(AfterRecording::Pause);
            self.message(Err(anyhow::anyhow!(error)), "");
        }
        let completed = self
            .engine
            .as_mut()
            .and_then(|engine| engine.take_recording_result());
        if let Some(result) = completed {
            if let Some(session) = self.recording.session.as_mut() {
                session.finishing = true;
            }
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
        if let Some(receiver) = &self.recording.finishing {
            match receiver.try_recv() {
                Ok(done) => {
                    self.recording.finishing = None;
                    self.publish_recording(done);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.recording.finishing = None;
                    let directory = self
                        .recording
                        .session
                        .take()
                        .map(|session| session.directory);
                    self.message(
                        Err(anyhow::anyhow!(
                            "Recording finalization stopped; captured files remain in {}",
                            directory.unwrap_or_default().display()
                        )),
                        "",
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let wanted = self.desired_input();
        if self.recording.failed_input && !self.recording_busy() {
            if let Some(engine) = self.engine.as_mut() {
                let _ = engine.disable_input();
            }
            self.recording.signature = None;
        }
        let routes_mutable = self
            .recording
            .session
            .as_ref()
            .is_none_or(|session| !session.started);
        if !self.recording.failed_input && self.recording.finishing.is_none() && routes_mutable {
            if self.engine.is_none()
                && self.loading.is_none()
                && !wanted.routes.is_empty()
                && (self.audio_device_enabled || self.virtual_audio)
            {
                self.prepare_engine(false);
            }
            if let Some(engine) = self.engine.as_mut() {
                let id = engine.snapshot().engine_id;
                let signature = (id, wanted.clone());
                if self.recording.signature.as_ref() != Some(&signature) {
                    let result = if wanted.routes.is_empty() {
                        engine.disable_input()
                    } else {
                        engine.configure_input(&self.project, wanted)
                    };
                    match result {
                        Ok(()) => self.recording.signature = Some(signature),
                        Err(error) => {
                            self.recording.failed_input = true;
                            self.finish_recording(AfterRecording::Pause);
                            self.recording.last_error = Some(format!("{error:#}"));
                            self.message(Err(error), "");
                        }
                    }
                }
            }
        }
        let can_start = self
            .recording
            .session
            .as_ref()
            .is_some_and(|session| !session.started && !session.finishing)
            && self
                .engine
                .as_ref()
                .is_some_and(|engine| engine.recording_snapshot().input_ready);
        if can_start {
            let session = self.recording.session.as_ref().unwrap();
            let config = RecordingConfig {
                directory: session.directory.clone(),
                tracks: session.tracks.iter().map(|track| track.id).collect(),
            };
            match self.engine.as_mut().unwrap().start_recording(config) {
                Ok(()) => {
                    self.recording.session.as_mut().unwrap().started = true;
                    self.desired_playing = true;
                    self.status = "Recording".into();
                }
                Err(error) => {
                    self.recording.session = None;
                    self.recording.last_error = Some(format!("{error:#}"));
                    self.message(Err(error), "");
                }
            }
        }
        if self.recording_busy()
            || self
                .engine
                .as_ref()
                .is_some_and(|engine| engine.recording_snapshot().input_opening)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn publish_recording(&mut self, done: FinalizedRecording) {
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
            if !commits.is_empty() {
                project.validate()?;
                self.begin_edit();
                self.project = project;
                self.waveforms.extend(done.previews);
                self.edit_label = Some(
                    if session.placement == RecordingPlacement::Overdub {
                        "Record audio overdub"
                    } else {
                        "Record audio takes"
                    }
                    .into(),
                );
                self.changed();
                self.flush_pending_edit();
                if let Some(first) = commits.first() {
                    self.selected_track = Some(first.track);
                    self.selected_clip = Some(first.clip);
                }
            }
            Ok(commits)
        })();
        match result {
            Ok(commits) => {
                self.recording.last_commit = serde_json::json!({"tracks":commits.iter().map(|commit| serde_json::json!({"track":commit.track,"clip":commit.clip,"group":commit.group})).collect::<Vec<_>>(), "files":paths, "takes":done.result.takes.len()});
                if let Some(error) = done.result.error {
                    self.recording.failed_input = true;
                    self.recording.last_error = Some(error.clone());
                    self.message(
                        Err(anyhow::anyhow!(
                            "{error}\n\nRecovered {} recorded file(s) in {}",
                            paths.len(),
                            session.directory.display()
                        )),
                        "",
                    );
                } else if !self.recording.failed_input {
                    self.status = if commits.is_empty() {
                        "Recording cancelled · No input captured".into()
                    } else {
                        format!("Recorded {} take(s)", done.result.takes.len())
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
        if !session.started
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
        if self.input_devices.is_empty() {
            self.refresh_input_devices();
        }
        let track = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .filter(|track| track.kind == TrackKind::Audio)
            .cloned();
        let title = track
            .as_ref()
            .map(|track| format!("Recording · {}", track.name))
            .unwrap_or_else(|| "Recording".into());
        let mut open = true;
        daw_ui::popup(title, ctx).id(egui::Id::new("recording_popup")).open(&mut open).default_width(460.0).show(ctx, |ui| {
            let Some(track) = &track else { daw_ui::text::note(ui, "Select an audio track"); return; };
            let busy = self.recording_busy();
            ui.horizontal_wrapped(|ui| {
                daw_ui::text::note(ui, self.settings.input_device_name.as_deref().unwrap_or("System default input"));
                if daw_ui::button(ui, "Audio setup…").clicked() { self.setup_page = SetupPage::Audio; self.open_popup(PopupKind::Setup); }
            });
            ui.add_enabled_ui(!busy, |ui| {
                let channels = self.input_devices.iter().find(|device| self.settings.input_device_name.as_ref().map_or(device.is_default, |name| name == &device.name)).map_or(2, |device| device.channels.min(32));
                let mut input = track.audio_input;
                daw_ui::form_grid(ui, "recording_input", |ui| {
                    let label = daw_ui::text::label(ui, "Input");
                    daw_ui::select("track_audio_input", input_label(input)).show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut input, None, "No input");
                        for channel in 0..channels { let choice = Some(AudioInputChannels::Mono {channel}); daw_ui::selectable_value(ui, &mut input, choice, input_label(choice)); }
                        for left in (0..channels.saturating_sub(1)).step_by(2) { let choice = Some(AudioInputChannels::Stereo {left,right:left+1}); daw_ui::selectable_value(ui, &mut input, choice, input_label(choice)); }
                    }).response.labelled_by(label.id);
                    ui.end_row();
                });
                if input != track.audio_input {
                    self.begin_edit(); self.project.track_mut(track.id).unwrap().audio_input = input;
                    self.edit_label = Some("Choose recording input".into()); self.changed(); self.recording.failed_input = false;
                }
                let mut armed = self.recording.armed.contains(&track.id);
                if daw_ui::checkbox(ui, &mut armed, "Arm recording").changed() { self.toggle_record_arm(track.id); }
                let mut mode = self.recording.monitors.get(&track.id).copied().unwrap_or_default();
                daw_ui::form_grid(ui, "recording_monitor", |ui| {
                    let label = daw_ui::text::label(ui, "Input monitor (dry)");
                    daw_ui::select("input_monitor_mode", mode.label()).show_ui(ui, |ui| { for choice in [MonitorMode::Off,MonitorMode::Auto,MonitorMode::On] { daw_ui::selectable_value(ui, &mut mode, choice, choice.label()); } }).response.labelled_by(label.id);
                    ui.end_row();
                    let label = daw_ui::text::label(ui, "Record into");
                    daw_ui::select("recording_placement", if self.recording.placement == RecordingPlacement::Take {"New take"} else {"Overdub layer"}).show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut self.recording.placement, RecordingPlacement::Take, "New take");
                        daw_ui::selectable_value(ui, &mut self.recording.placement, RecordingPlacement::Overdub, "Overdub layer");
                    }).response.labelled_by(label.id);
                    ui.end_row();
                });
                if mode != self.recording.monitors.get(&track.id).copied().unwrap_or_default() {
                    self.recording.monitors.insert(track.id, mode); self.recording.failed_input = false;
                }
            });
            let peaks = self.engine.as_ref().map_or([0.0;2], |engine| engine.input_peaks(track.id));
            daw_ui::input_meter(ui, peaks);
            daw_ui::text::note(ui, "Dry monitoring follows channel and output-bus levels. Inserts and sends are bypassed; recorded audio stays unchanged.");
            if self.project.loop_enabled { daw_ui::text::note(ui, "Loop recording keeps each pass as a separate take."); }
            ui.horizontal_wrapped(|ui| {
                if daw_ui::record_button(ui, busy, !self.recording_finalizing()).clicked() {
                    if busy { self.finish_recording(AfterRecording::Continue); } else { self.begin_recording(); }
                }
                if self.recording.failed_input && daw_ui::button(ui, "Retry input").clicked() {
                    self.recording.failed_input = false; self.recording.signature = None;
                }
                if let Some(clip) = self.selected().cloned() {
                    if daw_ui::button(ui, "Edit recording").clicked() { self.close_popups(); self.view = WorkspaceView::Clip; }
                    if clip.take_group().is_some() && daw_ui::button(ui, "Takes…").clicked() { self.open_audio_takes(&clip, None); }
                }
            });
        });
        self.recording_open &= open;
    }

    pub(crate) fn recording_diagnostics(&self, ctx: &egui::Context) -> serde_json::Value {
        let snapshot = self
            .engine
            .as_ref()
            .map(|engine| engine.recording_snapshot())
            .unwrap_or_default();
        let phase = if self.recording_finalizing() {
            "finalizing"
        } else if self.recording_busy() {
            if snapshot.active {
                "recording"
            } else {
                "starting"
            }
        } else {
            "idle"
        };
        serde_json::json!({"phase":phase,"armed":self.recording.armed,"monitors":self.recording.monitors.iter().map(|(id,mode)| (id.to_string(),mode.label())).collect::<HashMap<_,_>>(),
            "input_ready":snapshot.input_ready,"input_opening":snapshot.input_opening,"input_channels":snapshot.input_channels,
            "frames":snapshot.frames,"completed_passes":snapshot.completed_passes,"dropped_frames":snapshot.dropped_frames,"input_peaks":snapshot.input_peaks,
            "dummy_device":{"input_channels":self.virtual_io.input_channels,"input_latency_frames":self.virtual_io.input_latency_frames,"output_latency_frames":self.virtual_io.output_latency_frames},
            "last_commit":self.recording.last_commit,"error":self.recording.last_error,"input_device_name":self.settings.input_device_name,
            "input_device_rect":ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("input_device_picker_rect"))).map(|rect| [rect.left(),rect.top(),rect.right(),rect.bottom()])})
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
