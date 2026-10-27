//! Nonvisual recording operations over the same capture lifecycle as the UI.
//! A whole group of routes can be configured atomically without selecting tracks.
use crate::{
    app::DawApp,
    recording_ui::{AfterRecording, MonitorMode},
};
use daw_core::{
    AudioInputChannels, EditBatch, MidiInputChannel, RecordingPlacement, SongEdit, TrackKind,
};
use daw_tools::{SongToolHost, ToolError, parse_arguments};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

const MAX_ROUTES: usize = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionArgs {
    session_id: u64,
    expected_revision: u64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Monitor {
    Off,
    Auto,
    On,
}

impl From<Monitor> for MonitorMode {
    fn from(value: Monitor) -> Self {
        match value {
            Monitor::Off => Self::Off,
            Monitor::Auto => Self::Auto,
            Monitor::On => Self::On,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    None,
    AudioMono { channel: u16 },
    AudioStereo { left: u16, right: u16 },
    MidiAll,
    MidiChannel { channel: u8 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackConfiguration {
    track_id: u64,
    #[serde(default)]
    input: Option<Input>,
    #[serde(default)]
    armed: Option<bool>,
    #[serde(default)]
    monitor: Option<Monitor>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Placement {
    Take,
    Overdub,
}

impl From<Placement> for RecordingPlacement {
    fn from(value: Placement) -> Self {
        match value {
            Placement::Take => Self::Take,
            Placement::Overdub => Self::Overdub,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timing {
    precount_bars: u8,
    click_enabled: bool,
    click_gain: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    session_id: u64,
    expected_revision: u64,
    #[serde(default)]
    tracks: Vec<TrackConfiguration>,
    #[serde(default)]
    placement: Option<Placement>,
    #[serde(default)]
    timing: Option<Timing>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum After {
    #[default]
    Continue,
    Pause,
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stop {
    #[serde(default)]
    after: After,
}

impl DawApp {
    pub(crate) fn recording_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Option<Result<Value, ToolError>> {
        Some(match name {
            "get_recording" => {
                parse_arguments::<Empty>(arguments).map(|_| self.recording_tool_status())
            }
            "configure_recording" => parse_arguments::<Configuration>(arguments)
                .and_then(|configuration| self.configure_recording_tool(configuration)),
            "start_recording" => parse_arguments::<SessionArgs>(arguments)
                .and_then(|session| self.start_recording_tool(session)),
            "stop_recording" => parse_arguments::<Stop>(arguments).map(|request| {
                self.finish_recording(match request.after {
                    After::Continue => AfterRecording::Continue,
                    After::Pause => AfterRecording::Pause,
                    After::Stop => AfterRecording::Stop,
                });
                self.recording_tool_status()
            }),
            "retry_recording_input" => parse_arguments::<Empty>(arguments).and_then(|_| {
                self.recording_tool_idle()?;
                self.retry_recording_input();
                Ok(self.recording_tool_status())
            }),
            _ => return None,
        })
    }

    fn recording_tool_status(&self) -> Value {
        let mut state = self.recording_state();
        if let Some(object) = state.as_object_mut() {
            object.remove("dummy_device");
        }
        if let Some(commit) = state.get_mut("last_commit").and_then(Value::as_object_mut) {
            commit.remove("files");
        }
        state["session_id"] = json!(self.song_session.id);
        state["revision"] = json!(self.song_session.revision);
        state["precount_bars"] = json!(self.project.precount_bars);
        state["click_enabled"] = json!(self.project.metronome_enabled);
        state["click_gain"] = json!(self.project.metronome_gain);
        state["available_input_channels"] = json!(if self.virtual_audio {
            Some(self.virtual_io.input_channels)
        } else {
            self.input_devices
                .iter()
                .find(|device| {
                    self.settings
                        .input_device_name
                        .as_ref()
                        .map_or(device.is_default, |name| name == &device.name)
                })
                .map(|device| device.channels)
        });
        state["recordable_track_count"] = json!(
            self.project
                .tracks
                .iter()
                .filter(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
                .count()
        );
        state["tracks"] = json!(self.project.tracks.iter().filter(|track| {
            matches!(track.kind, TrackKind::Audio | TrackKind::Midi)
        }).take(256).map(|track| {
            let input = match track.kind {
                TrackKind::Audio => match track.audio_input {
                    None => json!({"kind":"none"}),
                    Some(AudioInputChannels::Mono { channel }) => json!({"kind":"audio_mono","channel":channel}),
                    Some(AudioInputChannels::Stereo { left, right }) => json!({"kind":"audio_stereo","left":left,"right":right}),
                },
                _ => match track.midi_input {
                    None => json!({"kind":"none"}),
                    Some(MidiInputChannel::All) => json!({"kind":"midi_all"}),
                    Some(MidiInputChannel::Channel { channel }) => json!({"kind":"midi_channel","channel":channel}),
                }
            };
            let monitor = state["monitors"][track.id.to_string()].as_str().unwrap_or("Off").to_ascii_lowercase();
            json!({"track_id":track.id,"name":track.name,"kind":track.kind,"input":input,"armed":self.recording.armed.contains(&track.id),"monitor":monitor})
        }).collect::<Vec<_>>());
        state["channel_indexing"] = json!("Audio and MIDI channel indices are zero-based.");
        state
    }

    fn recording_tool_idle(&self) -> Result<(), ToolError> {
        if self.recording_busy()
            || self.project_io_pending()
            || self.state_sync_pending()
            || self.document_guard.is_some()
            || self.drag.is_some()
        {
            return Err(ToolError::new(
                "busy",
                "Wait for recording or the current document operation to finish",
            ));
        }
        Ok(())
    }

    fn recording_tool_session(&self, id: u64, revision: u64) -> Result<(), ToolError> {
        if self.song_session.id != id || self.song_session.revision != revision {
            return Err(ToolError::new(
                "conflict",
                "Song changed; read fresh context before configuring recording",
            ));
        }
        Ok(())
    }

    fn configure_recording_tool(
        &mut self,
        configuration: Configuration,
    ) -> Result<Value, ToolError> {
        self.recording_tool_idle()?;
        self.flush_pending_edit();
        self.recording_tool_session(configuration.session_id, configuration.expected_revision)?;
        if configuration.tracks.len() > MAX_ROUTES {
            return Err(ToolError::new(
                "invalid_arguments",
                "Configure at most 32 tracks in one call",
            ));
        }
        let mut seen = HashSet::new();
        let mut edits = Vec::new();
        let mut controls = Vec::new();
        let mut armed = self.recording.armed.clone();
        let input_channels = if self.virtual_audio {
            Some(self.virtual_io.input_channels)
        } else {
            self.input_devices
                .iter()
                .find(|device| {
                    self.settings
                        .input_device_name
                        .as_ref()
                        .map_or(device.is_default, |name| name == &device.name)
                })
                .map(|device| device.channels)
        };
        for configuration in configuration.tracks {
            if !seen.insert(configuration.track_id) {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Configure each track at most once per call",
                ));
            }
            let track = self
                .project
                .track(configuration.track_id)
                .ok_or_else(|| ToolError::new("not_found", "Recording track not found"))?;
            if !matches!(track.kind, TrackKind::Audio | TrackKind::Midi) {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Only audio and MIDI tracks can record or monitor input",
                ));
            }
            if let Some(input) = configuration.input {
                let edit = match (track.kind, input) {
                    (_, Input::None) => SongEdit::ClearTrackInput {
                        track: track.id.into(),
                    },
                    (TrackKind::Audio, Input::AudioMono { channel }) => {
                        let input = AudioInputChannels::Mono { channel };
                        input
                            .validate(input_channels.unwrap_or(32).min(32))
                            .map_err(|error| {
                                ToolError::new("invalid_arguments", error.to_string())
                            })?;
                        SongEdit::SetAudioInput {
                            track: track.id.into(),
                            input,
                        }
                    }
                    (TrackKind::Audio, Input::AudioStereo { left, right }) => {
                        let input = AudioInputChannels::Stereo { left, right };
                        input
                            .validate(input_channels.unwrap_or(32).min(32))
                            .map_err(|error| {
                                ToolError::new("invalid_arguments", error.to_string())
                            })?;
                        SongEdit::SetAudioInput {
                            track: track.id.into(),
                            input,
                        }
                    }
                    (TrackKind::Midi, Input::MidiAll) => SongEdit::SetMidiInput {
                        track: track.id.into(),
                        input: MidiInputChannel::All,
                    },
                    (TrackKind::Midi, Input::MidiChannel { channel }) if channel < 16 => {
                        SongEdit::SetMidiInput {
                            track: track.id.into(),
                            input: MidiInputChannel::Channel { channel },
                        }
                    }
                    _ => {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "Input kind or channel does not match the recording track",
                        ));
                    }
                };
                edits.push(edit);
            }
            if let Some(enable) = configuration.armed {
                if enable {
                    armed.insert(track.id);
                } else {
                    armed.remove(&track.id);
                }
            }
            controls.push((
                track.id,
                configuration.armed,
                configuration.monitor.map(Into::into),
            ));
        }
        let audio_armed = self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio && armed.contains(&track.id))
            .count();
        let midi_armed = self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Midi && armed.contains(&track.id))
            .count();
        if audio_armed > MAX_ROUTES || midi_armed > MAX_ROUTES {
            return Err(ToolError::new(
                "invalid_arguments",
                "At most 32 audio and 32 MIDI tracks can be armed at once",
            ));
        }
        if let Some(timing) = configuration.timing {
            edits.push(SongEdit::SetMetronome {
                enabled: timing.click_enabled,
                gain: timing.click_gain,
                precount_bars: timing.precount_bars,
            });
        }
        if !edits.is_empty() {
            SongToolHost::apply_edits(
                self,
                EditBatch {
                    label: "Configure recording".into(),
                    edits,
                },
            )?;
        }
        self.configure_recording_controls(&controls, configuration.placement.map(Into::into));
        Ok(self.recording_tool_status())
    }

    fn start_recording_tool(&mut self, request: SessionArgs) -> Result<Value, ToolError> {
        self.recording_tool_idle()?;
        self.flush_pending_edit();
        self.recording_tool_session(request.session_id, request.expected_revision)?;
        let targets: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| self.recording.armed.contains(&track.id))
            .collect();
        if targets.is_empty() {
            return Err(ToolError::new(
                "invalid_arguments",
                "Arm an audio or MIDI track before starting recording",
            ));
        }
        if targets.iter().any(|track| match track.kind {
            TrackKind::Audio => track.audio_input.is_none(),
            TrackKind::Midi => {
                track.midi_input.is_none() || self.settings.midi_input_device.is_none()
            }
            _ => true,
        }) {
            return Err(ToolError::new(
                "invalid_arguments",
                "Choose an input for every armed track; MIDI also needs a selected input device",
            ));
        }
        if !self.audio_device_enabled && !self.virtual_audio {
            return Err(ToolError::new(
                "unavailable",
                "An audio input/output device is required for recording",
            ));
        }
        self.begin_recording_from_tool();
        if !self.recording_busy() {
            return Err(ToolError::new("execution_failed", self.status.clone()));
        }
        Ok(self.recording_tool_status())
    }
}
