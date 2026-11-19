//! Selected-source visual analysis. The existing isolated decoder admits the
//! complete source, then shared waveform/FFT analysis runs off the UI thread.
//! Only small display envelopes are cached; PCM and staging files are temporary.
use daw_core::{AudioData, AudioWaveformPeaks};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, SystemTime},
};

const DECODE_BYTES: usize = 32 * 1024 * 1024;
const WAVEFORM_COLUMNS: usize = 256;
const CACHE_CAPACITY: usize = 32;

#[derive(Debug)]
pub(crate) struct SampleVisual {
    /// Exact extrema over uniform time ranges, combining the existing stereo
    /// display channels. Absolute amplitude is clipped to −1..1, never boosted.
    pub waveform: Vec<[f32; 2]>,
    /// Peak over time for each existing log-frequency spectral band. Values
    /// retain the core analysis' absolute −90..−6 dBFS scale, mapped to 0..1.
    pub spectrum: Vec<f32>,
    pub min_frequency: f32,
    pub max_frequency: f32,
    pub sample_rate: u32,
    pub source_frames: usize,
}

pub(crate) enum SampleVisualStatus<'a> {
    Idle,
    Loading,
    Ready(&'a SampleVisual),
    Failed(&'a str),
}

struct Request {
    path: PathBuf,
    scratch: PathBuf,
    generation: u64,
}

struct Pending {
    path: PathBuf,
    generation: u64,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<SampleVisual, String>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct SampleVisualState {
    selected: Option<PathBuf>,
    generation: u64,
    queued: Option<Request>,
    pending: Option<Pending>,
    cache: VecDeque<(PathBuf, Result<SampleVisual, String>)>,
}

impl SampleVisualState {
    /// Called only for the visible inspector's selected source. No filesystem
    /// access or audio work occurs on this thread, including cache hits.
    pub fn request(&mut self, path: PathBuf, scratch: PathBuf, ctx: &egui::Context) {
        if self.selected.as_ref() == Some(&path) {
            return;
        }
        self.clear_selection();
        self.selected = Some(path.clone());
        if let Some(index) = self.cache.iter().position(|(cached, _)| cached == &path) {
            let cached = self.cache.remove(index).unwrap();
            self.cache.push_back(cached);
            return;
        }
        self.queued = Some(Request {
            path,
            scratch,
            generation: self.generation,
        });
        self.start_queued(ctx);
    }

    pub fn retry(&mut self, path: PathBuf, scratch: PathBuf, ctx: &egui::Context) {
        self.cache.retain(|(cached, _)| cached != &path);
        if self.selected.as_ref() == Some(&path) {
            self.clear_selection();
        }
        self.request(path, scratch, ctx);
    }

    /// Leaving the inspector cancels work but retains bounded reusable results.
    pub fn clear_selection(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.selected = None;
        self.queued = None;
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
    }

    /// Explicit rescans and document changes invalidate source identities.
    pub fn invalidate(&mut self) {
        self.clear_selection();
        self.cache.clear();
    }

    pub fn selected(&self, path: &Path) -> SampleVisualStatus<'_> {
        if self.selected.as_deref() != Some(path) {
            return SampleVisualStatus::Idle;
        }
        if let Some((_, result)) = self.cache.iter().find(|(cached, _)| cached == path) {
            return match result {
                Ok(visual) => SampleVisualStatus::Ready(visual),
                Err(error) => SampleVisualStatus::Failed(error),
            };
        }
        if self
            .queued
            .as_ref()
            .is_some_and(|request| request.path == path)
            || self.pending.as_ref().is_some_and(|pending| {
                pending.path == path
                    && pending.generation == self.generation
                    && !pending.cancel.load(Ordering::Acquire)
            })
        {
            SampleVisualStatus::Loading
        } else {
            SampleVisualStatus::Idle
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let completed =
            self.pending
                .as_ref()
                .and_then(|pending| match pending.receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Disconnected) => {
                        Some(Err("Sample analysis worker stopped".into()))
                    }
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(result) = completed {
            let pending = self.pending.take().unwrap();
            if pending.generation == self.generation
                && self.selected.as_ref() == Some(&pending.path)
                && !pending.cancel.load(Ordering::Acquire)
            {
                self.store(pending.path.clone(), result);
            }
        }
        self.start_queued(ctx);
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }

    fn store(&mut self, path: PathBuf, result: Result<SampleVisual, String>) {
        self.cache.retain(|(cached, _)| *cached != path);
        self.cache.push_back((path, result));
        while self.cache.len() > CACHE_CAPACITY {
            self.cache.pop_front();
        }
    }

    fn start_queued(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let Some(request) = self.queued.take() else {
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = request.path.clone();
        let ctx = ctx.clone();
        let spawn = std::thread::Builder::new()
            .name("rill-sample-visual".into())
            .spawn(move || {
                daw_core::thread_qos::set_background_utility_qos();
                let result = std::panic::catch_unwind(|| {
                    analyse_source(&worker_path, &request.scratch, &worker_cancel)
                })
                .map_err(|_| "Sample analysis worker stopped".to_owned())
                .and_then(|result| result.map_err(|error| format!("{error:#}")));
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        match spawn {
            Ok(_) => {
                self.pending = Some(Pending {
                    path: request.path,
                    generation: request.generation,
                    cancel,
                    receiver,
                })
            }
            Err(error) => self.store(
                request.path,
                Err(format!("Could not start sample analysis: {error}")),
            ),
        }
    }
}

#[derive(PartialEq)]
struct SourceStamp {
    bytes: u64,
    modified: Option<SystemTime>,
}
impl SourceStamp {
    fn read(path: &Path) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        anyhow::ensure!(metadata.is_file(), "Select an audio file for analysis");
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

fn analyse_source(
    path: &Path,
    scratch: &Path,
    cancel: &AtomicBool,
) -> anyhow::Result<SampleVisual> {
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample analysis cancelled");
    let stamp = SourceStamp::read(path)?;
    // The decoder rejects complete sources beyond this limit; never depict a
    // truncated prefix as a whole recording. Temporary WAVs are removed by its RAII owner.
    let audio = crate::audio_import::decode_audio_preview(path, scratch, DECODE_BYTES, cancel)?;
    let result = analyse(&audio, cancel)?;
    anyhow::ensure!(
        stamp == SourceStamp::read(path)?,
        "Sample changed during analysis; retry its details"
    );
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample analysis cancelled");
    Ok(result)
}

fn analyse(audio: &AudioData, cancel: &AtomicBool) -> anyhow::Result<SampleVisual> {
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample analysis cancelled");
    anyhow::ensure!(
        audio.samples.len() <= DECODE_BYTES / size_of::<f32>(),
        "Sample analysis exceeds the 32 MiB decoded source limit"
    );
    let peaks = AudioWaveformPeaks::build_cancellable(audio, cancel)?;
    anyhow::ensure!(peaks.source_frames > 0, "Sample contains no audio frames");
    let columns = WAVEFORM_COLUMNS.min(audio.frames());
    let mut waveform = Vec::with_capacity(columns);
    for column in 0..columns {
        anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample analysis cancelled");
        let start = column * audio.frames() / columns;
        let end = (column + 1) * audio.frames() / columns;
        let mut range = [f32::INFINITY, f32::NEG_INFINITY];
        for channel in 0..peaks.channels {
            if let Some((min, max)) = peaks.range(audio, start, end, channel) {
                range[0] = range[0].min(min);
                range[1] = range[1].max(max);
            }
        }
        waveform.push([range[0].clamp(-1.0, 1.0), range[1].clamp(-1.0, 1.0)]);
    }
    let spectrogram = daw_core::compute_audio_spectrogram_cancellable(audio, cancel)?;
    let mut spectrum = vec![0.0_f32; spectrogram.bands];
    for column in spectrogram.levels.chunks_exact(spectrogram.bands) {
        for (peak, value) in spectrum.iter_mut().zip(column) {
            *peak = peak.max(*value as f32 / 255.0);
        }
    }
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample analysis cancelled");
    Ok(SampleVisual {
        waveform,
        spectrum,
        min_frequency: spectrogram.min_frequency,
        max_frequency: spectrogram.max_frequency,
        sample_rate: audio.sample_rate,
        source_frames: audio.frames(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(amplitude: f32, stereo: bool) -> AudioData {
        AudioData {
            sample_rate: 48_000,
            channels: if stereo { 2 } else { 1 },
            samples: (0..48_000)
                .flat_map(|frame| {
                    let value =
                        amplitude * (std::f32::consts::TAU * 997.0 * frame as f32 / 48_000.0).sin();
                    if stereo {
                        vec![value, -value]
                    } else {
                        vec![value]
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn true_frequency_amplitude_and_antiphase_stereo_survive_projection() {
        let cancel = AtomicBool::new(false);
        let mono = analyse(&tone(0.25, false), &cancel).unwrap();
        let stereo = analyse(&tone(0.25, true), &cancel).unwrap();
        assert_eq!(mono.spectrum, stereo.spectrum);
        assert_eq!(mono.waveform.len(), WAVEFORM_COLUMNS);
        assert!(
            stereo
                .waveform
                .iter()
                .all(|range| range[0] < -0.24 && range[1] > 0.24)
        );
        let (band, peak) = mono
            .spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let frequency = mono.min_frequency
            * (mono.max_frequency / mono.min_frequency)
                .powf((band as f32 + 0.5) / mono.spectrum.len() as f32);
        assert!(
            (900.0..1100.0).contains(&frequency),
            "Dominant band {frequency} Hz"
        );
        let quiet = analyse(&tone(0.0025, false), &cancel).unwrap();
        assert!(
            *peak - quiet.spectrum[band] > 0.4,
            "Quiet sources must not be peak-normalized"
        );
    }

    #[test]
    fn silence_stays_flat_and_a_one_frame_impulse_keeps_its_time_bin() {
        let mut audio = AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.0; 4096],
        };
        let silence = analyse(&audio, &AtomicBool::new(false)).unwrap();
        assert!(silence.waveform.iter().all(|range| *range == [0.0, 0.0]));
        assert!(silence.spectrum.iter().all(|level| *level == 0.0));
        audio.samples[4095] = 1.0;
        let impulse = analyse(&audio, &AtomicBool::new(false)).unwrap();
        assert_eq!(impulse.waveform.last(), Some(&[0.0, 1.0]));
        assert!(
            impulse.waveform[..WAVEFORM_COLUMNS - 1]
                .iter()
                .all(|range| *range == [0.0, 0.0])
        );
        assert!(analyse(&audio, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn cancelled_late_completion_cannot_replace_the_new_selection() {
        let ctx = egui::Context::default();
        let old = PathBuf::from("old.wav");
        let current = PathBuf::from("current.wav");
        let audio = tone(0.25, false);
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut state = SampleVisualState {
            selected: Some(old.clone()),
            pending: Some(Pending {
                path: old.clone(),
                generation: 0,
                cancel: cancel.clone(),
                receiver,
            }),
            ..Default::default()
        };
        state.store(
            current.clone(),
            Ok(analyse(&audio, &AtomicBool::new(false)).unwrap()),
        );
        state.request(current.clone(), PathBuf::new(), &ctx);
        assert!(cancel.load(Ordering::Acquire));
        sender
            .send(Ok(analyse(&audio, &AtomicBool::new(false)).unwrap()))
            .unwrap();
        state.poll(&ctx);
        assert!(matches!(
            state.selected(&current),
            SampleVisualStatus::Ready(_)
        ));
        assert!(matches!(state.selected(&old), SampleVisualStatus::Idle));
        assert_eq!(state.cache.len(), 1);
        state.clear_selection();
        assert!(matches!(state.selected(&current), SampleVisualStatus::Idle));
    }
}
