//! Resident source metadata and optional small PCM. All admission reads happen
//! on one background worker; a waveform cache miss never removes source bounds.
use super::DawApp;
use daw_core::{AudioData, AudioFile, AudioFileInfo, ClipContent};
use eframe::egui;
use std::{collections::HashSet, sync::mpsc};

pub(crate) type AdmittedSource = (String, anyhow::Result<(AudioFile, Option<AudioData>)>);

pub(crate) struct Admission {
    session: u64,
    sources: Vec<AdmittedSource>,
}

pub(crate) fn load_paths(paths: Vec<String>, mut remaining: usize) -> Vec<AdmittedSource> {
    paths
        .into_iter()
        .map(|path| {
            let loaded = (|| {
                let file = AudioFile::open(&path)?;
                let info = file.info();
                let pcm = if !info.should_stream() && info.decoded_bytes <= remaining as u64 {
                    let data = AudioData::load_wav_with_limit(
                        &path,
                        crate::project_io::WAVEFORM_FILE_BYTES,
                    )?;
                    file.check_unchanged()?;
                    remaining = remaining.saturating_sub(data.samples.len() * size_of::<f32>());
                    Some(data)
                } else {
                    None
                };
                Ok((file, pcm))
            })();
            (path, loaded)
        })
        .collect()
}

impl DawApp {
    pub(crate) fn audio_source_info(&self, path: &str) -> Option<AudioFileInfo> {
        self.audio_files.get(path).map(AudioFile::info).or_else(|| {
            self.waveforms.get(path).map(|data| AudioFileInfo {
                sample_rate: data.sample_rate,
                channels: data.channels,
                frames: data.frames(),
                decoded_bytes: (data.samples.len() * size_of::<f32>()) as u64,
            })
        })
    }

    pub(crate) fn clip_is_disk_streamed(&self, track: u64, clip: &daw_core::Clip) -> bool {
        matches!(&clip.content,ClipContent::Audio{path,..} if self.audio_source_info(path).is_some_and(|info|info.should_stream()))
            || self
                .engine
                .as_ref()
                .and_then(|engine| engine.is_clip_streamed(track, clip.id))
                .unwrap_or(false)
    }

    pub(crate) fn audio_source_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"resident_pcm_bytes":self.waveforms.values().map(|data|data.samples.len()*size_of::<f32>()).sum::<usize>(),"metadata_sources":self.audio_files.len(),"streamed_sources":self.audio_files.values().filter(|file|file.info().should_stream()).count(),"pending":self.audio_admission.is_some()})
    }

    pub fn cache_audio(&mut self) {
        if self.audio_admission.is_some() {
            return;
        }
        self.audio_source_revision = self.revision;
        let wanted: HashSet<_> = self
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter_map(|clip| match &clip.content {
                ClipContent::Audio { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        self.audio_files
            .retain(|path, _| wanted.contains(path.as_str()));
        self.audio_admission_failed
            .retain(|path| wanted.contains(path.as_str()));
        let stale: Vec<_> = self
            .waveforms
            .keys()
            .filter(|path| !wanted.contains(path.as_str()))
            .cloned()
            .collect();
        let retired: Vec<_> = stale
            .iter()
            .filter_map(|path| self.waveforms.remove(path))
            .collect();
        if !retired.is_empty() {
            std::thread::spawn(move || drop(retired));
        }
        let paths: HashSet<_> = self
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter_map(|clip| match &clip.content {
                ClipContent::Audio { path, .. }
                    if !self.audio_files.contains_key(path)
                        && !self.audio_admission_failed.contains(path) =>
                {
                    Some(path.clone())
                }
                _ => None,
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let remaining = crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(
            self.waveforms
                .values()
                .map(|data| data.samples.len() * size_of::<f32>())
                .sum::<usize>(),
        );
        let (sender, receiver) = mpsc::sync_channel(1);
        let session = self.song_session.id;
        let ctx = self.render_context.clone();
        if std::thread::Builder::new()
            .name("rill-audio-sources".into())
            .spawn(move || {
                let fallback: Vec<_> = paths.iter().cloned().collect();
                let sources =
                    std::panic::catch_unwind(|| load_paths(paths.into_iter().collect(), remaining))
                        .unwrap_or_else(|_| {
                            fallback
                                .into_iter()
                                .map(|path| {
                                    (
                                        path,
                                        Err(anyhow::anyhow!(
                                            "Audio metadata worker stopped unexpectedly"
                                        )),
                                    )
                                })
                                .collect()
                        });
                let _ = sender.send(Admission { session, sources });
                ctx.request_repaint();
            })
            .is_ok()
        {
            self.audio_admission = Some(receiver);
        }
    }

    pub(crate) fn poll_audio_sources(&mut self, ctx: &egui::Context) {
        let result = self.audio_admission.as_ref().map(mpsc::Receiver::try_recv);
        match result {
            Some(Ok(admission)) => {
                self.audio_admission = None;
                if admission.session == self.song_session.id {
                    let wanted: HashSet<_> = self
                        .project
                        .tracks
                        .iter()
                        .flat_map(|track| &track.clips)
                        .filter_map(|clip| match &clip.content {
                            ClipContent::Audio { path, .. } => Some(path.as_str()),
                            _ => None,
                        })
                        .collect();
                    let mut remaining = crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(
                        self.waveforms
                            .values()
                            .map(|data| data.samples.len() * size_of::<f32>())
                            .sum::<usize>(),
                    );
                    let mut retired = Vec::new();
                    for (path, result) in admission.sources {
                        if !wanted.contains(path.as_str()) {
                            if let Ok((_, Some(pcm))) = result {
                                retired.push(pcm);
                            }
                            continue;
                        }
                        match result {
                            Ok((file, pcm)) => {
                                self.audio_files.insert(path.clone(), file);
                                if let Some(pcm) = pcm {
                                    let bytes = pcm.samples.len() * size_of::<f32>();
                                    if bytes <= remaining && !self.waveforms.contains_key(&path) {
                                        remaining -= bytes;
                                        self.waveforms.insert(path, pcm);
                                    } else {
                                        retired.push(pcm);
                                    }
                                }
                            }
                            Err(_) => {
                                self.audio_admission_failed.insert(path);
                            }
                        }
                    }
                    if !retired.is_empty() {
                        std::thread::spawn(move || drop(retired));
                    }
                } else {
                    std::thread::spawn(move || drop(admission));
                }
                self.cache_audio();
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.audio_admission = None;
            }
            _ => {}
        }
        if self.audio_admission.is_none() && self.audio_source_revision != self.revision {
            self.cache_audio();
        }
        if self.audio_admission.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}
