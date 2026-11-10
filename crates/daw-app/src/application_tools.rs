//! Explicitly authorized application tools. Never expose this host to the
//! embedded current-song agent; the application MCP profile opts into it.
use crate::{app::DawApp, document_ui::DocumentTransition, settings::Settings};
use daw_core::{MAX_BEAT, TrackKind};
use daw_tools::{ToolError, parse_arguments};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
enum Patch<T> {
    #[default]
    Keep,
    Set(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Set)
    }
}

struct SecretInput(String);
impl<'de> Deserialize<'de> for SecretInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}
impl Drop for SecretInput {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentArgs {
    action: String,
    path: Option<String>,
    #[serde(default)]
    discard_unsaved: bool,
    beat: Option<f64>,
    track_id: Option<u64>,
    #[serde(default = "include_tails")]
    include_tails: bool,
    /// Export: "wav" (default), "flac", "ogg" or "mp3".
    format: Option<String>,
    /// Export: 16 or 24 for WAV and FLAC, 32 (float) for WAV only.
    bit_depth: Option<u16>,
    /// Export: Ogg Vorbis quality, 0 to 10.
    quality: Option<u8>,
    /// Export: MP3 bitrate in kbit/s.
    bitrate_kbps: Option<u16>,
    sample_rate: Option<u32>,
    /// Export: write one file per channel into the folder given as path.
    #[serde(default)]
    stems: bool,
    /// Export stems: channel ids; default every audio/instrument track.
    channels: Option<Vec<u64>>,
    #[serde(default)]
    include_mix: bool,
    #[serde(default)]
    through_master: bool,
    /// Export stems: replace files that already exist.
    #[serde(default)]
    replace_existing: bool,
}
fn include_tails() -> bool {
    true
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AudioPatch {
    device_name: Patch<Option<String>>,
    input_device_name: Patch<Option<String>>,
    midi_input_device: Patch<Option<String>>,
    sample_rate: Option<u32>,
    buffer_frames: Option<u32>,
    resample_quality: Option<daw_engine::ResampleQuality>,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AiPatch {
    provider: Option<daw_ai::Provider>,
    model: Option<String>,
    api_key: Option<SecretInput>,
    remove_api_key: bool,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SetupArgs {
    audio: Option<AudioPatch>,
    scratch_dir: Option<String>,
    vst_dirs: Option<Vec<String>>,
    ai: Option<AiPatch>,
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::new("invalid_arguments", message)
}
fn path(value: &str) -> Result<PathBuf, ToolError> {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        return Err(invalid("Provide a nonempty path of at most 4096 bytes"));
    }
    Ok(PathBuf::from(value))
}
fn required_path(value: Option<&str>) -> Result<PathBuf, ToolError> {
    path(value.ok_or_else(|| invalid("This action requires an explicit path"))?)
}
fn redacted_setup(settings: &Settings) -> Value {
    json!({
        "audio":{"device_name":settings.device_name,"input_device_name":settings.input_device_name,
            "midi_input_device":settings.midi_input_device,"sample_rate":settings.sample_rate,
            "buffer_frames":settings.buffer_frames,"resample_quality":settings.resample_quality},
        "scratch_dir":settings.scratch_dir,"vst_dirs":settings.vst_dirs,
        "ai":{"provider":settings.ai.provider,"model":settings.ai.model,
            "api_key_configured":settings.ai.api_key.is_some()}
    })
}

impl DawApp {
    pub(crate) fn application_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Option<Result<Value, ToolError>> {
        match name {
            "document_status" => Some((|| {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(arguments)?;
                Ok(self.application_document_status())
            })()),
            "document_action" => Some((|| {
                let args: DocumentArgs = parse_arguments(arguments)?;
                self.application_document_action(args)
            })()),
            "get_setup" => Some((|| {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(arguments)?;
                // Refresh only enumerates metadata; it never opens a stream.
                // The shared helper also refreshes audio inputs and MIDI ports.
                self.refresh_devices();
                let mut result = redacted_setup(&self.settings);
                let audio_device = |device: &daw_engine::DeviceInfo| {
                    json!({
                        "name":device.name,"is_default":device.is_default,
                        "channels":device.channels,"supported_sample_rates":device.supported_sample_rates
                    })
                };
                result["available_devices"] = json!({
                    "outputs":self.devices.iter().map(audio_device).collect::<Vec<_>>(),
                    "inputs":self.input_devices.iter().map(audio_device).collect::<Vec<_>>(),
                    "midi":self.recording.midi_ports.iter().map(|port| json!({
                        "id":port.id,"name":port.name,
                        "selected":self.settings.midi_input_device.as_deref()==Some(port.id.as_str())
                    })).collect::<Vec<_>>()
                });
                Ok(result)
            })()),
            "configure_setup" => Some((|| {
                // Bound parsing without leaving a serialized plaintext key copy.
                let bytes = Zeroizing::new(
                    serde_json::to_vec(&arguments)
                        .map_err(|_| invalid("Invalid setup arguments"))?,
                );
                if bytes.len() > daw_tools::MAX_ARGUMENT_BYTES {
                    return Err(invalid("Tool arguments exceed 1 MiB"));
                }
                let args: SetupArgs = serde_json::from_value(arguments)
                    .map_err(|_| invalid("Invalid setup arguments"))?;
                self.application_configure_setup(args)
            })()),
            _ => None,
        }
    }

    pub(crate) fn application_document_status(&self) -> Value {
        json!({
            "name":self.project.name,"path":self.project_path,"modified":self.modified,
            "session_id":self.song_session.id,"revision":self.song_session.revision,
            "pending":self.project_io_pending() || self.state_sync_pending()
                || self.export_job.is_some() || self.loading.is_some() || self.document_guard.is_some()
                || self.audio_import_busy() || self.freeze_busy(),
            "recording":self.recording_busy(),"status":self.status,"error":self.error,
            "exported_files":self.exported_files
        })
    }

    fn application_document_action(&mut self, args: DocumentArgs) -> Result<Value, ToolError> {
        if self.edit_dirty || self.drag.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active edit gesture first",
            ));
        }
        if self.document_guard.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active document operation first",
            ));
        }
        self.flush_pending_edit();
        self.error = false;
        let mut files = None;
        match args.action.as_str() {
            "new" => self
                .request_tool_document_transition(DocumentTransition::New, args.discard_unsaved)?,
            "open" | "recover" => {
                let recovery = args.action == "recover";
                let path = match args.path.as_deref() {
                    Some(value) => path(value)?,
                    None if recovery => self.settings.scratch_dir.join("recovery.daw.json"),
                    None => return Err(invalid("Opening a project requires an explicit path")),
                };
                self.request_tool_document_transition(
                    DocumentTransition::Open { path, recovery },
                    args.discard_unsaved,
                )?;
            }
            "save" | "save_as" => {
                let path = if args.action == "save" {
                    args.path
                        .as_deref()
                        .map(path)
                        .transpose()?
                        .or_else(|| self.project_path.clone())
                        .ok_or_else(|| invalid("The current project has no path; provide one"))?
                } else {
                    required_path(args.path.as_deref())?
                };
                self.save_to(path);
            }
            "export" => files = Some(self.application_export(&args)?),
            "import" => self.application_import(
                &required_path(args.path.as_deref())?,
                args.track_id,
                args.beat.unwrap_or(self.cursor),
            )?,
            _ => return Err(invalid("Unknown document action")),
        }
        if self.error {
            return Err(ToolError::new("application_failed", self.status.clone()));
        }
        let mut result = json!({"accepted":true,"document":self.application_document_status()});
        if let Some(files) = files {
            // The render continues in the background; document_status reports
            // it pending, then lists the files once they are complete.
            result["files"] = json!(files);
        }
        Ok(result)
    }

    /// Start a mix or stem export; returns the files it will write.
    fn application_export(&mut self, args: &DocumentArgs) -> Result<Vec<PathBuf>, ToolError> {
        use daw_engine::{ExportFormat, WavEncoding};
        let path = required_path(args.path.as_deref())?;
        let mut specification = self.legacy_export_specification(args.include_tails);
        if let Some(rate) = args.sample_rate {
            specification.sample_rate = rate;
        }
        let options = &mut specification.options;
        options.format = match args.format.as_deref().unwrap_or("wav") {
            "wav" => ExportFormat::Wav,
            "flac" => ExportFormat::Flac,
            "ogg" => args
                .quality
                .map_or(ExportFormat::VORBIS_DEFAULT, |quality| {
                    ExportFormat::Vorbis { quality }
                }),
            "mp3" => args
                .bitrate_kbps
                .map_or(ExportFormat::MP3_DEFAULT, |kbps| ExportFormat::Mp3 { kbps }),
            _ => return Err(invalid("format must be wav, flac, ogg or mp3")),
        };
        options.encoding = match (args.bit_depth, options.format) {
            (Some(16), _) => WavEncoding::Pcm16,
            (Some(24), _) => WavEncoding::Pcm24,
            (Some(32), ExportFormat::Wav) => WavEncoding::Float32,
            (Some(_), _) => return Err(invalid("bit_depth must be 16 or 24 (32 for wav)")),
            (None, ExportFormat::Flac) => WavEncoding::Pcm24,
            (None, _) => WavEncoding::Float32,
        };
        if (args.quality.is_some() && !matches!(options.format, ExportFormat::Vorbis { .. }))
            || (args.bitrate_kbps.is_some() && !matches!(options.format, ExportFormat::Mp3 { .. }))
        {
            return Err(invalid(
                "quality applies to ogg and bitrate_kbps to mp3 only",
            ));
        }
        options
            .format
            .validate(options.encoding, specification.sample_rate)
            .map_err(|error| invalid(error.to_string()))?;
        if !args.stems {
            if args.channels.is_some()
                || args.include_mix
                || args.through_master
                || args.replace_existing
            {
                return Err(invalid(
                    "channels, include_mix, through_master and replace_existing need stems=true",
                ));
            }
            self.export_to_with_specification(path.clone(), specification);
            return Ok(vec![path]);
        }
        let channels: Vec<u64> = match &args.channels {
            Some(channels) => channels.clone(),
            None => crate::export_ui::default_stem_channels(&self.project),
        };
        let plans = daw_engine::plan_stems(
            &self.project,
            &self.document_file_stem(),
            &channels,
            args.include_mix,
            options.format,
        )
        .map_err(|error| invalid(error.to_string()))?;
        let files: Vec<PathBuf> = plans
            .iter()
            .map(|plan| path.join(&plan.file_name))
            .collect();
        if !path.is_dir() {
            return Err(invalid("path must be an existing folder when stems=true"));
        }
        let existing: Vec<_> = files
            .iter()
            .filter(|file| std::fs::symlink_metadata(file).is_ok())
            .filter_map(|file| file.file_name()?.to_str())
            .collect();
        if !existing.is_empty() && !args.replace_existing {
            return Err(invalid(format!(
                "Files already exist: {}; set replace_existing to replace them",
                existing.join(", ")
            )));
        }
        specification.stems = Some(crate::export_ui::StemSelection {
            channels: self
                .project
                .tracks
                .iter()
                .filter(|track| channels.contains(&track.id))
                .map(|track| track.id)
                .collect(),
            include_mix: args.include_mix,
            through_master: args.through_master,
            song: self.document_file_stem(),
            replace_existing: args.replace_existing,
        });
        self.export_to_with_specification(path, specification);
        Ok(files)
    }

    fn application_import(
        &mut self,
        source: &Path,
        track_id: Option<u64>,
        beat: f64,
    ) -> Result<(), ToolError> {
        self.tool_ready()?;
        if !beat.is_finite() || !(0.0..=MAX_BEAT).contains(&beat) {
            return Err(invalid(
                "Import beat must be finite and between 0 and 1000000",
            ));
        }
        let extension = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("mid") && !extension.eq_ignore_ascii_case("midi") {
            if track_id.is_some_and(|id| {
                self.project
                    .track(id)
                    .is_none_or(|track| track.kind != TrackKind::Audio)
            }) {
                return Err(invalid("Choose an existing audio track"));
            }
            if !self.import_picker_ready() {
                return Err(ToolError::new("busy", self.status.clone()));
            }
            // Reuse the same isolated, bounded, managed-media pipeline as file
            // drops and File → Import. The application capability returns an
            // accepted job; document_status reports it pending until committed.
            self.queue_audio_import(vec![source.to_path_buf()], track_id, beat);
            return if self.error {
                Err(ToolError::new("import_failed", self.status.clone()))
            } else {
                Ok(())
            };
        }
        if track_id.is_some() {
            return Err(invalid("MIDI import creates tracks; omit track_id"));
        }
        let mut candidate = self.project.clone();
        candidate
            .import_midi(source, beat)
            .map_err(|error| ToolError::new("import_failed", error.to_string()))?;
        self.commit_song_candidate(candidate, "Import media")?;
        self.cache_audio();
        self.error = false;
        self.status = "Media imported".into();
        Ok(())
    }

    fn application_configure_setup(&mut self, args: SetupArgs) -> Result<Value, ToolError> {
        if self.edit_dirty || self.drag.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active edit gesture first",
            ));
        }
        if self.document_guard.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active document operation first",
            ));
        }
        let mut settings = self.settings.clone();
        if let Some(audio) = args.audio {
            if let Patch::Set(value) = audio.device_name {
                settings.device_name = value;
            }
            if let Patch::Set(value) = audio.input_device_name {
                settings.input_device_name = value;
            }
            if let Patch::Set(value) = audio.midi_input_device {
                settings.midi_input_device = value;
            }
            if let Some(value) = audio.sample_rate {
                settings.sample_rate = value;
            }
            if let Some(value) = audio.buffer_frames {
                settings.buffer_frames = value;
            }
            if let Some(value) = audio.resample_quality {
                settings.resample_quality = value;
            }
        }
        if let Some(value) = args.scratch_dir {
            settings.scratch_dir = path(&value)?;
        }
        if let Some(values) = args.vst_dirs {
            if values.len() > 128 {
                return Err(invalid("At most 128 VST directories are supported"));
            }
            settings.vst_dirs = values
                .iter()
                .map(|value| path(value))
                .collect::<Result<_, _>>()?;
        }
        if let Some(ai) = args.ai {
            if ai.api_key.is_some() && ai.remove_api_key {
                return Err(invalid("Provide api_key or remove_api_key, not both"));
            }
            if let Some(provider) = ai.provider {
                if provider != settings.ai.provider {
                    settings.ai.api_key = None;
                    if ai.model.is_none() {
                        settings.ai.model = provider.default_model().into();
                    }
                }
                settings.ai.provider = provider;
            }
            if let Some(model) = ai.model {
                settings.ai.model = model;
            }
            if let Some(key) = &ai.api_key {
                let value = key.0.trim();
                if value.is_empty() || value.len() > 4096 || value.contains(['\n', '\r']) {
                    return Err(invalid(
                        "Provide a nonempty API key of at most 4096 bytes without line breaks",
                    ));
                }
                settings.ai.api_key =
                    Some(daw_ai::encrypt_api_key(value).map_err(|_| {
                        ToolError::new("settings_failed", "Could not encrypt API key")
                    })?);
            }
            if ai.remove_api_key {
                settings.ai.api_key = None;
            }
        }
        if settings.ai.model.trim().is_empty() || settings.ai.model.len() > 200 {
            return Err(invalid("Choose a model of at most 200 bytes"));
        }
        settings
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        let desired = redacted_setup(&settings);
        self.apply_settings_snapshot(settings);
        if self.error {
            return Err(ToolError::new("settings_failed", self.status.clone()));
        }
        Ok(json!({"accepted":true,"setup":desired,"document":self.application_document_status()}))
    }
}
