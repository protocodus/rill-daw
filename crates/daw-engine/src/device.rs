use crate::meter::ChannelMeters;
use crate::render::{BLOCK, Renderer};
#[path = "live_updates.rs"]
mod live_updates;
use live_updates::{LiveMailbox, LiveUpdates};
#[cfg(test)]
#[path = "live_device_tests.rs"]
mod live_device_tests;
use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use daw_core::Project;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct DeviceSettings {
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub resample_quality: crate::ResampleQuality,
}
impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            device_name: None,
            sample_rate: 48_000,
            buffer_frames: 256,
            resample_quality: crate::ResampleQuality::High,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub supported_sample_rates: Vec<u32>,
    pub channels: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EngineSnapshot {
    pub playing: bool,
    pub position_beats: f64,
    pub peak_left: f32,
    pub peak_right: f32,
    pub underruns: u64,
    pub render_errors: u64,
    pub engine_id: u64,
    pub generation: u64,
    pub requested_revision: u64,
    pub applied_revision: u64,
    /// The current live edit uses a band-limited proxy while the selected
    /// algorithm prepares the original at full rate. Export never uses proxies.
    pub refining_audio: bool,
    pub edit_errors: u64,
    pub rendered_frames: u64,
    pub consumed_frames: u64,
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|device| device.description().ok())
        .map(|description| description.name().to_owned());
    let mut output = Vec::new();
    for device in host.output_devices()? {
        let name = device.description()?.name().to_owned();
        let mut rates = Vec::new();
        let mut channels = 0;
        if let Ok(configs) = device.supported_output_configs() {
            for config in configs {
                channels = channels.max(config.channels());
                for rate in [
                    22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
                ] {
                    if rate >= config.min_sample_rate() && rate <= config.max_sample_rate() {
                        rates.push(rate);
                    }
                }
            }
        }
        rates.sort_unstable();
        rates.dedup();
        output.push(DeviceInfo {
            is_default: default_name.as_deref() == Some(&name),
            name,
            supported_sample_rates: rates,
            channels,
        });
    }
    Ok(output)
}

static NEXT_ENGINE_ID: AtomicU64 = AtomicU64::new(1);

struct Controls {
    engine_id: u64,
    rendered_frames: AtomicU64,
    consumed_frames: AtomicU64,
    playing: AtomicBool,
    auditioning: AtomicBool,
    shutdown: AtomicBool,
    generation: AtomicU64,
    seek_beat: AtomicU64,
    position: AtomicU64,
    peak_left: AtomicU32,
    peak_right: AtomicU32,
    underruns: AtomicU64,
    render_errors: AtomicU64,
    render_error: Mutex<Option<String>>,
    finished_generation: AtomicU64,
}
impl Controls {
    fn new() -> Self {
        Self {
            engine_id: NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed),
            rendered_frames: AtomicU64::new(0),
            consumed_frames: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            auditioning: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            seek_beat: AtomicU64::new(0),
            position: AtomicU64::new(0),
            peak_left: AtomicU32::new(0),
            peak_right: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            render_errors: AtomicU64::new(0),
            render_error: Mutex::new(None),
            finished_generation: AtomicU64::new(u64::MAX),
        }
    }

    /// Only the background render worker publishes text. The device callback
    /// continues using the atomic counter and never formats or locks diagnostics.
    fn set_render_error(&self, mut error: String) {
        const MAX_BYTES: usize = 64 * 1024;
        if error.len() > MAX_BYTES {
            let mut end = MAX_BYTES;
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            error.truncate(end);
            error.push_str("\n[Further diagnostic text omitted]");
        }
        *self
            .render_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
    }

    fn take_render_error(&self) -> Option<String> {
        self.render_error.try_lock().ok()?.take()
    }
}

struct Slot {
    stereo: AtomicU64,
    beat: AtomicU64,
    generation: AtomicU64,
}
struct AudioQueue {
    slots: Vec<Slot>,
    read: AtomicUsize,
    write: AtomicUsize,
}
impl AudioQueue {
    fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity)
                .map(|_| Slot {
                    stereo: AtomicU64::new(0),
                    beat: AtomicU64::new(0),
                    generation: AtomicU64::new(0),
                })
                .collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }
    fn free(&self) -> usize {
        self.slots.len()
            - self
                .write
                .load(Ordering::Relaxed)
                .wrapping_sub(self.read.load(Ordering::Acquire))
    }
    fn push(&self, frame: [f32; 2], beat: f64, generation: u64) {
        let write = self.write.load(Ordering::Relaxed);
        let slot = &self.slots[write % self.slots.len()];
        slot.stereo.store(
            frame[0].to_bits() as u64 | ((frame[1].to_bits() as u64) << 32),
            Ordering::Relaxed,
        );
        slot.beat.store(beat.to_bits(), Ordering::Relaxed);
        slot.generation.store(generation, Ordering::Relaxed);
        self.write.store(write.wrapping_add(1), Ordering::Release);
    }
    fn pop(&self) -> Option<([f32; 2], f64, u64)> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.write.load(Ordering::Acquire) {
            return None;
        }
        let slot = &self.slots[read % self.slots.len()];
        let stereo = slot.stereo.load(Ordering::Relaxed);
        let beat = f64::from_bits(slot.beat.load(Ordering::Relaxed));
        let generation = slot.generation.load(Ordering::Relaxed);
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some((
            [
                f32::from_bits(stereo as u32),
                f32::from_bits((stereo >> 32) as u32),
            ],
            beat,
            generation,
        ))
    }
    fn discard_stale(&self, generation: u64) {
        for _ in 0..self.slots.len() {
            let read = self.read.load(Ordering::Relaxed);
            if read == self.write.load(Ordering::Acquire) {
                break;
            }
            if self.slots[read % self.slots.len()]
                .generation
                .load(Ordering::Relaxed)
                == generation
            {
                break;
            }
            self.read.store(read.wrapping_add(1), Ordering::Release);
        }
    }
}

pub struct AudioEngine {
    live_updates: Option<LiveUpdates>,
    virtual_queue: Option<Arc<AudioQueue>>,
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<()>>,
    controls: Arc<Controls>,
    sample_rate: u32,
    parameters: mpsc::SyncSender<ParameterChange>,
    audition: Arc<Mutex<Option<(u64, u8, u8)>>>,
    channel_meters: Arc<ChannelMeters>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParameterChange {
    track: u64,
    effect: Option<u64>,
    parameter: u32,
    value: f64,
}

fn collect_parameter_changes(
    receiver: &mpsc::Receiver<ParameterChange>,
    pending: &mut Vec<ParameterChange>,
) {
    pending.clear();
    for _ in 0..128 {
        let Ok(change) = receiver.try_recv() else {
            break;
        };
        if let Some(previous) = pending.iter_mut().find(|previous| {
            previous.track == change.track
                && previous.effect == change.effect
                && previous.parameter == change.parameter
        }) {
            *previous = change;
        } else {
            pending.push(change);
        }
    }
}

/// A fatal graph error must not be retried on every worker tick. A new transport
/// generation permits one retry; replacing the graph creates a fresh latch.
#[derive(Default)]
struct RenderFailureLatch {
    failed_generation: Option<u64>,
}

impl RenderFailureLatch {
    fn attempt(
        &mut self,
        controls: &Controls,
        generation: u64,
        operation: impl FnOnce() -> Result<()>,
    ) -> bool {
        if self.failed_generation == Some(generation) {
            return false;
        }
        let Err(error) = operation() else { return true };
        self.failed_generation = Some(generation);
        controls.set_render_error(format!("Audio rendering stopped\n\n{error:#}"));
        controls.render_errors.fetch_add(1, Ordering::Relaxed);
        controls
            .finished_generation
            .store(generation, Ordering::Release);
        controls.playing.store(false, Ordering::Release);
        controls.auditioning.store(false, Ordering::Release);
        false
    }
}

fn wait_for_renderer(
    ready: mpsc::Receiver<Result<()>>,
    controls: &Controls,
    worker: JoinHandle<()>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<JoinHandle<()>> {
    let deadline = Instant::now() + timeout;
    let error = loop {
        if cancel.load(Ordering::Acquire) {
            break anyhow::anyhow!("Audio preparation cancelled");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break anyhow::anyhow!("Audio renderer initialization timed out");
        }
        match ready.recv_timeout(remaining.min(Duration::from_millis(25))) {
            Ok(Ok(())) if !cancel.load(Ordering::Acquire) => return Ok(worker),
            Ok(Ok(())) => break anyhow::anyhow!("Audio preparation cancelled"),
            Ok(Err(error)) => break error,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break anyhow::anyhow!("Audio renderer initialization stopped unexpectedly");
            }
        }
    };
    cancel.store(true, Ordering::Release);
    controls.shutdown.store(true, Ordering::Release);
    // A failed/timed-out preparation cannot release ownership while a plugin
    // host is still loading. Individual native calls have bounded IPC deadlines.
    let _ = worker.join();
    Err(error)
}

impl AudioEngine {
    /// Opens the selected output and prepares all clips and instruments before returning.
    pub fn start(project: &Project, settings: DeviceSettings) -> Result<Self> {
        Self::start_cancellable(project, settings, Arc::new(AtomicBool::new(false)))
    }

    /// Cancel preparation between assets and isolated plugin loads. The caller
    /// still owns this operation until all resources have been released.
    pub fn start_cancellable(
        project: &Project,
        settings: DeviceSettings,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_internal(project, settings, cancel, false)
    }

    /// Device-free deterministic output for headless application verification.
    /// Uses the production renderer, queue and callback consumer, but never opens
    /// an output device. Call `pump_virtual` to consume audio explicitly.
    pub fn start_virtual_cancellable(
        project: &Project,
        settings: DeviceSettings,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_internal(project, settings, cancel, true)
    }

    fn start_internal(
        project: &Project,
        settings: DeviceSettings,
        cancel: Arc<AtomicBool>,
        virtual_output: bool,
    ) -> Result<Self> {
        ensure!(
            !cancel.load(Ordering::Acquire),
            "Audio preparation cancelled"
        );
        ensure!(
            (16..=8192).contains(&settings.buffer_frames),
            "Buffer size must be between 16 and 8192 frames"
        );
        let output = if virtual_output {
            None
        } else {
            let host = cpal::default_host();
            let device = if let Some(name) = &settings.device_name {
                host.output_devices()?
                    .find(|device| {
                        device
                            .description()
                            .is_ok_and(|description| description.name() == name)
                    })
                    .with_context(|| format!("Output device '{name}' is unavailable"))?
            } else {
                host.default_output_device()
                    .context("No default audio output device is available")?
            };
            let mut supported: Vec<_> = device
                .supported_output_configs()?
                .filter(|range| {
                    range.channels() > 0
                        && range.min_sample_rate() <= settings.sample_rate
                        && range.max_sample_rate() >= settings.sample_rate
                })
                .collect();
            supported.sort_by_key(|range| {
                (
                    range.channels() != 2,
                    range.sample_format() != cpal::SampleFormat::F32,
                )
            });
            let range = supported
                .first()
                .context("Device does not support the requested sample rate")?;
            let supported_config = range.with_sample_rate(settings.sample_rate);
            let format = supported_config.sample_format();
            let mut config = supported_config.config();
            config.buffer_size = match range.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => {
                    ensure!(
                        settings.buffer_frames >= *min && settings.buffer_frames <= *max,
                        "Device buffer size must be between {min} and {max} frames"
                    );
                    cpal::BufferSize::Fixed(settings.buffer_frames)
                }
                cpal::SupportedBufferSize::Unknown => {
                    cpal::BufferSize::Fixed(settings.buffer_frames)
                }
            };
            Some((device, config, format))
        };
        let controls = Arc::new(Controls::new());
        let channel_meters = Arc::new(ChannelMeters::new(project));
        let worker_meters = Arc::clone(&channel_meters);
        let queue = Arc::new(AudioQueue::new(
            (settings.buffer_frames as usize * 2).max(BLOCK * 2),
        ));
        let project = project.clone();
        let worker_controls = Arc::clone(&controls);
        let worker_queue = Arc::clone(&queue);
        let sample_rate = settings.sample_rate;
        let render_options = crate::RenderOptions {
            resample_quality: settings.resample_quality,
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (preparer_tx, preparer_rx) = mpsc::sync_channel(1);
        let live_mailbox = Arc::new(LiveMailbox::default());
        let worker_edits = Arc::clone(&live_mailbox);
        let (parameter_tx, parameter_rx) = mpsc::sync_channel::<ParameterChange>(128);
        let audition = Arc::new(Mutex::new(None));
        let worker_audition = Arc::clone(&audition);
        let worker_cancel = Arc::clone(&cancel);
        let worker = thread::Builder::new()
            .name("daw-render".into())
            .spawn(move || {
                // Plugins are created, processed, and destroyed on this owning thread.
                let mut renderer = match Renderer::new_with_options_cancelled(
                    &project,
                    sample_rate,
                    true,
                    render_options,
                    &worker_cancel,
                ) {
                    Ok(mut renderer) => {
                        renderer.attach_channel_meters(&worker_meters);
                        let _ = preparer_tx.send(renderer.live_edit_preparer());
                        let _ = ready_tx.send(Ok(()));
                        renderer
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let mut generation = worker_controls.generation.load(Ordering::Acquire);
                let mut frames = [[0.0; 2]; BLOCK];
                let mut positions = [0.0; BLOCK];
                let mut pending_parameters: Vec<ParameterChange> = Vec::with_capacity(128);
                let mut render_failure = RenderFailureLatch::default();
                let mut preview = false;
                while !worker_controls.shutdown.load(Ordering::Acquire) {
                    if worker_edits.apply_pending(&mut renderer) {
                        worker_controls
                            .finished_generation
                            .store(u64::MAX, Ordering::Release);
                    }
                    // UI edits never touch the callback. Coalesce rapid drags before
                    // issuing bounded IPC on the render worker, including while paused.
                    collect_parameter_changes(&parameter_rx, &mut pending_parameters);
                    for change in &pending_parameters {
                        if worker_controls.shutdown.load(Ordering::Acquire) {
                            break;
                        }
                        if let Err(error) = renderer.set_plugin_parameter(
                            change.track,
                            change.effect,
                            change.parameter,
                            change.value,
                        ) {
                            worker_controls.set_render_error(format!(
                                "Plugin parameter update failed\n\nUpdating plugin parameter {} on channel ID {}: {error:#}",
                                change.parameter, change.track
                            ));
                            worker_controls
                                .render_errors
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if worker_controls.shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    let held = *worker_audition
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let next_preview =
                        !worker_controls.playing.load(Ordering::Acquire) && held.is_some();
                    if preview != next_preview {
                        worker_controls.auditioning.store(false, Ordering::Release);
                        preview = next_preview;
                        generation = worker_controls
                            .generation
                            .fetch_add(1, Ordering::AcqRel)
                            .wrapping_add(1);
                        let beat = f64::from_bits(worker_controls.position.load(Ordering::Acquire));
                        if !render_failure.attempt(&worker_controls, generation, || {
                            renderer.set_preview(preview, beat)
                        }) {
                            thread::sleep(Duration::from_micros(250));
                            continue;
                        }
                    }
                    let next_generation = worker_controls.generation.load(Ordering::Acquire);
                    if generation != next_generation {
                        generation = next_generation;
                        let beat =
                            f64::from_bits(worker_controls.seek_beat.load(Ordering::Acquire));
                        if !render_failure
                            .attempt(&worker_controls, generation, || renderer.seek(beat))
                        {
                            thread::sleep(Duration::from_micros(250));
                            continue;
                        }
                        worker_controls
                            .finished_generation
                            .store(u64::MAX, Ordering::Release);
                    }
                    if !render_failure
                        .attempt(&worker_controls, generation, || renderer.set_audition(held))
                    {
                        worker_controls.auditioning.store(false, Ordering::Release);
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    {
                        let current = worker_audition
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        worker_controls.auditioning.store(
                            preview && current.is_some() && *current == held,
                            Ordering::Release,
                        );
                    }
                    if worker_queue.free() < BLOCK {
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    if !preview && renderer.finished() {
                        worker_controls
                            .finished_generation
                            .store(generation, Ordering::Release);
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    if !render_failure.attempt(&worker_controls, generation, || {
                        if preview {
                            renderer.render_preview(&mut frames, &mut positions)
                        } else {
                            renderer.render(&mut frames, &mut positions)
                        }
                    }) {
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    if renderer.plugin_errors > 0 {
                        while let Some(error) = renderer.take_plugin_error() {
                            worker_controls.set_render_error(error);
                        }
                        worker_controls
                            .render_errors
                            .fetch_add(renderer.plugin_errors, Ordering::Relaxed);
                        renderer.plugin_errors = 0;
                    }
                    for (frame, beat) in frames.iter().zip(positions) {
                        worker_queue.push(*frame, beat, generation);
                    }
                    worker_controls
                        .rendered_frames
                        .fetch_add(BLOCK as u64, Ordering::Relaxed);
                }
            })?;
        let worker = wait_for_renderer(
            ready_rx,
            &controls,
            worker,
            &cancel,
            Duration::from_secs(35),
        )?;
        let preparer = match preparer_rx.recv() {
            Ok(preparer) => preparer,
            Err(error) => {
                controls.shutdown.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(error.into());
            }
        };
        let live_updates = match LiveUpdates::start(preparer, live_mailbox) {
            Ok(updates) => updates,
            Err(error) => {
                controls.shutdown.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(error);
            }
        };
        let mut engine = Self {
            live_updates: Some(live_updates),
            virtual_queue: virtual_output.then(|| Arc::clone(&queue)),
            stream: None,
            worker: Some(worker),
            controls,
            sample_rate,
            parameters: parameter_tx,
            audition,
            channel_meters,
        };
        if let Some((device, config, format)) = output {
            let controls = Arc::clone(&engine.controls);
            let stream_result = match format {
                cpal::SampleFormat::F32 => {
                    build_stream::<f32>(&device, &config, Arc::clone(&queue), Arc::clone(&controls))
                }
                cpal::SampleFormat::F64 => {
                    build_stream::<f64>(&device, &config, Arc::clone(&queue), Arc::clone(&controls))
                }
                cpal::SampleFormat::I16 => {
                    build_stream::<i16>(&device, &config, Arc::clone(&queue), Arc::clone(&controls))
                }
                cpal::SampleFormat::I32 => {
                    build_stream::<i32>(&device, &config, Arc::clone(&queue), Arc::clone(&controls))
                }
                cpal::SampleFormat::U16 => {
                    build_stream::<u16>(&device, &config, Arc::clone(&queue), Arc::clone(&controls))
                }
                _ => Err(anyhow::anyhow!(
                    "Unsupported output sample format: {format:?}"
                )),
            };
            engine.stream = Some(stream_result?);
            if let Err(error) = engine.stream.as_ref().unwrap().play() {
                engine.stop();
                return Err(error.into());
            }
        }
        Ok(engine)
    }

    /// Prepare a compatible edit while the existing graph keeps rendering.
    /// `false` means a structural change requires normal graph preparation.
    pub fn try_update_project(&self, project: &Project, revision: u64) -> Result<bool> {
        ensure!(
            !self.controls.shutdown.load(Ordering::Acquire),
            "Audio engine has stopped"
        );
        self.live_updates
            .as_ref()
            .context("Live edits are unavailable")?
            .submit(project, revision)
    }

    /// Retrieve asynchronous preparation errors without waiting on playback.
    pub fn take_edit_error(&self) -> Option<String> {
        self.live_updates.as_ref()?.take_error()
    }

    /// Retrieve the latest detailed render/plugin failure once, without waiting
    /// for the render worker. At most one bounded message is retained.
    pub fn take_render_error(&self) -> Option<String> {
        self.controls.take_render_error()
    }

    /// Consume the production output queue without physical audio hardware.
    /// Waiting for each bounded chunk makes this a sample-continuity oracle,
    /// rather than an assertion about hardware scheduling or driver deadlines.
    pub fn pump_virtual(&mut self, frames: usize) -> Result<Vec<[f32; 2]>> {
        ensure!(frames <= 480_000, "Virtual output request is too large");
        let queue = self
            .virtual_queue
            .as_ref()
            .context("Not a virtual audio engine")?;
        let mut output = Vec::with_capacity(frames);
        let mut samples = [0.0_f32; BLOCK * 2];
        while output.len() < frames {
            let count = (frames - output.len()).min(BLOCK);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                ensure!(
                    !self.controls.shutdown.load(Ordering::Acquire),
                    "Audio engine has stopped"
                );
                let active = self.controls.playing.load(Ordering::Acquire)
                    || self.controls.auditioning.load(Ordering::Acquire);
                let generation = self.controls.generation.load(Ordering::Acquire);
                queue.discard_stale(generation);
                if !active
                    || queue.slots.len() - queue.free() >= count
                    || self.controls.finished_generation.load(Ordering::Acquire) == generation
                {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Virtual output timed out waiting for rendered audio"
                );
                thread::sleep(Duration::from_micros(250));
            }
            fill_output(&mut samples[..count * 2], 2, queue, &self.controls);
            output.extend_from_slice(samples[..count * 2].as_chunks::<2>().0);
        }
        Ok(output)
    }

    /// Replace the held preview note. This bounded shared command is read only by
    /// the render worker; neither the UI nor the device callback touches plugins.
    pub fn audition_note(&self, note: Option<(u64, u8, u8)>) -> Result<()> {
        if let Some((_, pitch, velocity)) = note {
            ensure!(
                pitch < 128 && velocity > 0 && velocity < 128,
                "Invalid audition note"
            );
        }
        *self
            .audition
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = note;
        if note.is_none() {
            self.controls.auditioning.store(false, Ordering::Release);
        }
        Ok(())
    }

    pub fn set_playing(&self, playing: bool) {
        if playing
            && self.controls.finished_generation.load(Ordering::Acquire)
                == self.controls.generation.load(Ordering::Acquire)
        {
            self.seek(0.0);
        }
        self.controls.playing.store(playing, Ordering::Release);
    }
    pub fn seek(&self, beat: f64) {
        if !beat.is_finite() {
            return;
        }
        let beat = beat.clamp(0.0, 1_000_000.0);
        self.controls
            .seek_beat
            .store(beat.to_bits(), Ordering::Release);
        self.controls
            .position
            .store(beat.to_bits(), Ordering::Release);
        self.controls.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn position_beats(&self) -> f64 {
        f64::from_bits(self.controls.position.load(Ordering::Relaxed))
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    /// Queue a normalized instrument edit for the render worker. This is
    /// nonblocking and leaves the hardware callback and transport untouched.
    pub fn set_instrument_parameter(&self, track: u64, parameter: u32, value: f64) -> Result<()> {
        self.set_plugin_parameter(track, None, parameter, value)
    }
    /// Queue a normalized parameter edit for an instrument (`None`) or insert
    /// (`Some` stable slot ID). The bounded render-worker queue never blocks the UI
    /// or touches the hardware callback. Opaque presets require graph preparation
    /// because they can change plug-in latency and bus layout.
    pub fn set_plugin_parameter(
        &self,
        track: u64,
        effect: Option<u64>,
        parameter: u32,
        value: f64,
    ) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Parameter value must be normalized to 0..=1"
        );
        self.parameters
            .try_send(ParameterChange {
                track,
                effect,
                parameter,
                value,
            })
            .map_err(|error| anyhow::anyhow!("Plug-in parameter queue unavailable: {error}"))?;
        if let Some(updates) = &self.live_updates {
            updates.acknowledge_parameter(track, effect, parameter, value);
        }
        Ok(())
    }
    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            playing: self.controls.playing.load(Ordering::Relaxed),
            position_beats: self.position_beats(),
            peak_left: f32::from_bits(self.controls.peak_left.load(Ordering::Relaxed)),
            peak_right: f32::from_bits(self.controls.peak_right.load(Ordering::Relaxed)),
            underruns: self.controls.underruns.load(Ordering::Relaxed),
            render_errors: self.controls.render_errors.load(Ordering::Relaxed),
            engine_id: self.controls.engine_id,
            generation: self.controls.generation.load(Ordering::Relaxed),
            requested_revision: self.live_updates.as_ref().map_or(0, |updates| {
                updates.mailbox.requested_revision.load(Ordering::Acquire)
            }),
            applied_revision: self.live_updates.as_ref().map_or(0, |updates| {
                updates.mailbox.applied_revision.load(Ordering::Acquire)
            }),
            refining_audio: self
                .live_updates
                .as_ref()
                .is_some_and(|updates| updates.mailbox.refining_audio.load(Ordering::Acquire)),
            edit_errors: self
                .live_updates
                .as_ref()
                .map_or(0, |updates| updates.mailbox.errors.load(Ordering::Relaxed)),
            rendered_frames: self.controls.rendered_frames.load(Ordering::Relaxed),
            consumed_frames: self.controls.consumed_frames.load(Ordering::Relaxed),
        }
    }
    /// Post-insert, post-fader/pan stereo channel peaks (including buses), with
    /// instant attack and a 24 dB/second release. This nonconsuming atomic read
    /// never locks, allocates, or touches a plugin. Stopped/missing channels read
    /// as silence. The render queue can lead the audible output by its bounded
    /// prebuffer, just as other transport telemetry can.
    pub fn channel_peaks(&self, track: u64) -> [f32; 2] {
        if !self.controls.playing.load(Ordering::Relaxed)
            && !self.controls.auditioning.load(Ordering::Relaxed)
        {
            [0.0; 2]
        } else {
            self.channel_meters.peaks(track)
        }
    }
    pub fn stop(&mut self) {
        self.controls.playing.store(false, Ordering::Release);
        self.controls.auditioning.store(false, Ordering::Release);
        self.controls.shutdown.store(true, Ordering::Release);
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.live_updates.take();
    }
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<AudioQueue>,
    controls: Arc<Controls>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let errors = Arc::clone(&controls);
    Ok(device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            fill_output(data, channels, &queue, &controls)
        },
        move |_| {
            errors.render_errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )?)
}

fn fill_output<T: cpal::Sample + cpal::FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    queue: &AudioQueue,
    controls: &Controls,
) {
    let playing = controls.playing.load(Ordering::Acquire);
    let auditioning = controls.auditioning.load(Ordering::Acquire);
    let generation = controls.generation.load(Ordering::Acquire);
    queue.discard_stale(generation);
    let mut peak = [0.0_f32; 2];
    let mut underrun = false;
    let mut consumed = 0;
    for frame in data.chunks_mut(channels) {
        let mut stereo = [0.0; 2];
        if playing || auditioning {
            // At most one queue's worth of stale samples can exist following a seek.
            for _ in 0..=queue.slots.len() {
                match queue.pop() {
                    Some((samples, beat, epoch)) if epoch == generation => {
                        stereo = samples;
                        consumed += 1;
                        if playing {
                            controls.position.store(beat.to_bits(), Ordering::Relaxed);
                        }
                        break;
                    }
                    Some(_) => continue,
                    None => {
                        if controls.finished_generation.load(Ordering::Acquire) == generation {
                            controls.playing.store(false, Ordering::Release);
                        } else {
                            underrun = true;
                        }
                        break;
                    }
                }
            }
        }
        for channel in 0..2 {
            stereo[channel] = if stereo[channel].is_finite() {
                stereo[channel].clamp(-1.0, 1.0)
            } else {
                0.0
            };
            peak[channel] = peak[channel].max(stereo[channel].abs());
        }
        if channels == 1 {
            frame[0] = T::from_sample((stereo[0] + stereo[1]) * 0.5);
        } else {
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = T::from_sample(if channel < 2 { stereo[channel] } else { 0.0 });
            }
        }
    }
    controls
        .consumed_frames
        .fetch_add(consumed, Ordering::Relaxed);
    controls
        .peak_left
        .store(peak[0].to_bits(), Ordering::Relaxed);
    controls
        .peak_right
        .store(peak[1].to_bits(), Ordering::Relaxed);
    if underrun {
        controls.underruns.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    #[test]
    fn channel_peak_reads_are_nonblocking_and_silent_when_transport_is_stopped() {
        let mut project = Project::new();
        let channel = project.add_audio_track("Meter");
        let meters = Arc::new(ChannelMeters::new(&project));
        meters.channel(channel).unwrap().publish([0.3, 0.7], 0.0);
        let (parameters, _receiver) = mpsc::sync_channel(1);
        let engine = AudioEngine {
            live_updates: None,
            virtual_queue: None,
            stream: None,
            worker: None,
            controls: Arc::new(Controls::new()),
            sample_rate: 48_000,
            parameters,
            audition: Arc::new(Mutex::new(None)),
            channel_meters: meters,
        };
        assert_eq!(engine.channel_peaks(channel), [0.0; 2]);
        engine.set_playing(true);
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                assert_eq!(engine.channel_peaks(channel), [0.3, 0.7]);
                assert_eq!(engine.channel_peaks(channel), [0.3, 0.7]);
                assert_eq!(engine.channel_peaks(u64::MAX), [0.0; 2]);
            }),
            0
        );
        engine.set_playing(false);
        assert_eq!(engine.channel_peaks(channel), [0.0; 2]);
        engine.controls.auditioning.store(true, Ordering::Relaxed);
        assert_eq!(engine.channel_peaks(channel), [0.3, 0.7]);
    }

    #[test]
    fn cancelled_or_timed_out_initialization_keeps_ownership_until_worker_cleanup() {
        for timed_out in [false, true] {
            let controls = Arc::new(Controls::new());
            let cancel = Arc::new(AtomicBool::new(!timed_out));
            let (release, gate) = mpsc::channel();
            let (ready_tx, ready) = mpsc::channel();
            let cleaned = Arc::new(AtomicBool::new(false));
            let worker_cleaned = Arc::clone(&cleaned);
            let worker = thread::spawn(move || {
                // Represents a native call already in progress when cancelled.
                gate.recv().unwrap();
                drop(ready_tx);
                worker_cleaned.store(true, Ordering::Release);
            });
            let (done, completion) = mpsc::channel();
            let waiter_controls = Arc::clone(&controls);
            let waiter_cancel = Arc::clone(&cancel);
            let waiter = thread::spawn(move || {
                let result = wait_for_renderer(
                    ready,
                    &waiter_controls,
                    worker,
                    &waiter_cancel,
                    if timed_out {
                        Duration::ZERO
                    } else {
                        Duration::from_secs(5)
                    },
                );
                done.send(result.is_err()).unwrap();
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while !controls.shutdown.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }
            assert!(cancel.load(Ordering::Acquire));
            assert!(
                matches!(completion.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "Failed initialization returned while its worker still owned resources"
            );
            release.send(()).unwrap();
            assert!(completion.recv_timeout(Duration::from_secs(5)).unwrap());
            assert!(cleaned.load(Ordering::Acquire));
            waiter.join().unwrap();
        }
    }

    #[test]
    fn audition_callback_plays_without_moving_the_transport_and_stops_on_release() {
        let queue = AudioQueue::new(8);
        let controls = Controls::new();
        controls
            .position
            .store(7.0_f64.to_bits(), Ordering::Relaxed);
        controls.auditioning.store(true, Ordering::Release);
        queue.push([0.25, -0.25], 9.0, 0);
        let mut output = [0.0_f32; 2];
        fill_output(&mut output, 2, &queue, &controls);
        assert_eq!(output, [0.25, -0.25]);
        assert_eq!(
            f64::from_bits(controls.position.load(Ordering::Relaxed)),
            7.0
        );
        assert!(!controls.playing.load(Ordering::Relaxed));
        controls.auditioning.store(false, Ordering::Release);
        queue.push([0.25, 0.25], 9.0, 0);
        fill_output(&mut output, 2, &queue, &controls);
        assert_eq!(output, [0.0, 0.0]);
    }

    #[test]
    fn live_parameter_queue_keeps_instruments_and_stable_effect_slots_separate() {
        let (parameters, receiver) = mpsc::sync_channel(128);
        let engine = AudioEngine {
            live_updates: None,
            virtual_queue: None,
            stream: None,
            worker: None,
            controls: Arc::new(Controls::new()),
            sample_rate: 48_000,
            parameters,
            audition: Arc::new(Mutex::new(None)),
            channel_meters: Arc::new(ChannelMeters::default()),
        };
        engine.set_instrument_parameter(1, 7, 0.1).unwrap();
        engine.set_plugin_parameter(1, Some(10), 7, 0.2).unwrap();
        engine.set_plugin_parameter(1, Some(11), 7, 0.3).unwrap();
        engine.set_plugin_parameter(2, Some(12), 7, 0.4).unwrap();
        engine.set_plugin_parameter(1, Some(10), 7, 0.9).unwrap();
        assert!(
            engine
                .set_plugin_parameter(1, Some(10), 7, f64::NAN)
                .is_err()
        );
        let mut pending = Vec::with_capacity(128);
        collect_parameter_changes(&receiver, &mut pending);
        assert_eq!(
            pending,
            vec![
                ParameterChange {
                    track: 1,
                    effect: None,
                    parameter: 7,
                    value: 0.1
                },
                ParameterChange {
                    track: 1,
                    effect: Some(10),
                    parameter: 7,
                    value: 0.9
                },
                ParameterChange {
                    track: 1,
                    effect: Some(11),
                    parameter: 7,
                    value: 0.3
                },
                ParameterChange {
                    track: 2,
                    effect: Some(12),
                    parameter: 7,
                    value: 0.4
                },
            ]
        );
        for _ in 0..128 {
            engine.set_plugin_parameter(1, Some(10), 7, 0.5).unwrap();
        }
        assert!(engine.set_plugin_parameter(1, Some(10), 7, 0.6).is_err());
    }

    #[test]
    fn fatal_graph_error_is_not_retried_until_an_explicit_transport_reset() {
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Release);
        let mut latch = RenderFailureLatch::default();
        let mut attempts = 0;
        // Simulate subsequent worker ticks after a graph worker has disconnected.
        // The same production gate wraps both render calls and seek prefill.
        for _ in 0..1000 {
            assert!(!latch.attempt(&controls, 0, || {
                attempts += 1;
                bail!("Persistent graph worker unexpectedly exited")
            }));
        }
        assert_eq!(attempts, 1);
        assert_eq!(controls.render_errors.load(Ordering::Relaxed), 1);
        assert_eq!(controls.finished_generation.load(Ordering::Acquire), 0);
        assert!(!controls.playing.load(Ordering::Acquire));
        assert!(
            controls
                .take_render_error()
                .unwrap()
                .contains("Audio rendering stopped\n\nPersistent graph worker unexpectedly exited")
        );
        assert!(controls.take_render_error().is_none());
        assert!(!latch.attempt(&controls, 0, || bail!("must stay latched")));
        assert!(controls.take_render_error().is_none());

        // A deliberate seek/new transport generation can retry a transient fault.
        controls.generation.store(1, Ordering::Release);
        controls.playing.store(true, Ordering::Release);
        assert!(latch.attempt(&controls, 1, || {
            attempts += 1;
            Ok(())
        }));
        assert_eq!(attempts, 2);
        assert!(controls.playing.load(Ordering::Acquire));
        assert_eq!(controls.render_errors.load(Ordering::Relaxed), 1);

        // A persistent fault on the next explicit retry is also reported once.
        for _ in 0..1000 {
            assert!(!latch.attempt(&controls, 1, || {
                attempts += 1;
                bail!("Seek prefill failed")
            }));
        }
        assert_eq!(attempts, 3);
        assert_eq!(controls.render_errors.load(Ordering::Relaxed), 2);
        assert_eq!(controls.finished_generation.load(Ordering::Acquire), 1);
        assert!(
            controls
                .take_render_error()
                .unwrap()
                .contains("Seek prefill failed")
        );
    }

    #[test]
    fn render_error_slot_is_bounded_unicode_safe_and_does_not_wait_for_the_worker() {
        let controls = Controls::new();
        controls.set_render_error("old failure".into());
        controls.set_render_error(format!("Plugin 診断 🎹: {}", "é".repeat(40_000)));
        let guard = controls.render_error.lock().unwrap();
        assert!(controls.take_render_error().is_none());
        drop(guard);
        let error = controls.take_render_error().unwrap();
        assert!(error.starts_with("Plugin 診断 🎹:"));
        assert!(error.ends_with("[Further diagnostic text omitted]"));
        assert!(error.len() <= 64 * 1024 + 40);
        assert!(controls.take_render_error().is_none());
    }

    #[test]
    fn queue_wrap_and_generation_flush() {
        let queue = AudioQueue::new(4);
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Relaxed);
        queue.push([0.1, 0.2], 1.0, 0);
        queue.push([0.3, 0.4], 2.0, 1);
        controls.generation.store(1, Ordering::Relaxed);
        let mut output = [0.0_f32; 2];
        fill_output(&mut output, 2, &queue, &controls);
        assert_eq!(output, [0.3, 0.4]);
        assert_eq!(
            f64::from_bits(controls.position.load(Ordering::Relaxed)),
            2.0
        );
        for _ in 0..10 {
            queue.push([0.5, 0.6], 3.0, 1);
            assert_eq!(queue.pop().unwrap().0, [0.5, 0.6]);
        }
        assert_eq!(queue.free(), 4);
    }
    #[test]
    fn callback_silences_nonfinite_and_extra_channels() {
        let queue = AudioQueue::new(4);
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Relaxed);
        queue.push([f32::NAN, 3.0], 1.0, 0);
        let mut output = [9.0_f32; 4];
        fill_output(&mut output, 4, &queue, &controls);
        assert_eq!(output, [0.0, 1.0, 0.0, 0.0]);
    }
    #[test]
    fn hardware_callback_allocates_nothing() {
        let queue = AudioQueue::new(512);
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Relaxed);
        for _ in 0..512 {
            queue.push([0.1, -0.1], 1.0, 0);
        }
        let mut output = [0.0_f32; 1024];
        let count = crate::allocation_test_support::count_allocations(|| {
            fill_output(&mut output, 2, &queue, &controls)
        });
        assert_eq!(count, 0);
    }
    #[test]
    fn engine_can_be_initialized_on_a_background_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<AudioEngine>();
    }
}
