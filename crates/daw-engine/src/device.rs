use crate::render::{BLOCK, Renderer};
use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use daw_core::Project;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct DeviceSettings {
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub buffer_frames: u32,
}
impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            device_name: None,
            sample_rate: 48_000,
            buffer_frames: 256,
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

struct Controls {
    playing: AtomicBool,
    shutdown: AtomicBool,
    generation: AtomicU64,
    seek_beat: AtomicU64,
    position: AtomicU64,
    peak_left: AtomicU32,
    peak_right: AtomicU32,
    underruns: AtomicU64,
    render_errors: AtomicU64,
    finished_generation: AtomicU64,
}
impl Controls {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            seek_beat: AtomicU64::new(0),
            position: AtomicU64::new(0),
            peak_left: AtomicU32::new(0),
            peak_right: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            render_errors: AtomicU64::new(0),
            finished_generation: AtomicU64::new(u64::MAX),
        }
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
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<()>>,
    controls: Arc<Controls>,
    sample_rate: u32,
}

impl AudioEngine {
    /// Opens the selected output and prepares all clips and instruments before returning.
    pub fn start(project: &Project, settings: DeviceSettings) -> Result<Self> {
        ensure!(
            (16..=8192).contains(&settings.buffer_frames),
            "Buffer size must be between 16 and 8192 frames"
        );
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
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Fixed(settings.buffer_frames),
        };
        let controls = Arc::new(Controls::new());
        let queue = Arc::new(AudioQueue::new(
            (settings.buffer_frames as usize * 2).max(BLOCK * 2),
        ));
        let project = project.clone();
        let worker_controls = Arc::clone(&controls);
        let worker_queue = Arc::clone(&queue);
        let sample_rate = settings.sample_rate;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("daw-render".into())
            .spawn(move || {
                // Plugins are created, processed, and destroyed on this owning thread.
                let mut renderer = match Renderer::new(&project, sample_rate, true) {
                    Ok(renderer) => {
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
                while !worker_controls.shutdown.load(Ordering::Acquire) {
                    let next_generation = worker_controls.generation.load(Ordering::Acquire);
                    if generation != next_generation {
                        generation = next_generation;
                        let beat =
                            f64::from_bits(worker_controls.seek_beat.load(Ordering::Acquire));
                        if renderer.seek(beat).is_err() {
                            worker_controls
                                .render_errors
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        worker_controls
                            .finished_generation
                            .store(u64::MAX, Ordering::Release);
                    }
                    if worker_queue.free() < BLOCK {
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    if renderer.finished() {
                        worker_controls
                            .finished_generation
                            .store(generation, Ordering::Release);
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    }
                    if renderer.render(&mut frames, &mut positions).is_err() {
                        frames.fill([0.0; 2]);
                        positions.fill(renderer.position_beats());
                        worker_controls
                            .render_errors
                            .fetch_add(1, Ordering::Relaxed);
                        // Bound failure handling. Stop rather than repeatedly executing a failed plugin.
                        worker_controls
                            .finished_generation
                            .store(generation, Ordering::Release);
                        worker_controls.playing.store(false, Ordering::Release);
                        continue;
                    }
                    if renderer.plugin_errors > 0 {
                        worker_controls
                            .render_errors
                            .fetch_add(renderer.plugin_errors, Ordering::Relaxed);
                        renderer.plugin_errors = 0;
                    }
                    for (frame, beat) in frames.iter().zip(positions) {
                        worker_queue.push(*frame, beat, generation);
                    }
                }
            })?;
        match ready_rx.recv_timeout(Duration::from_secs(35)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = worker.join();
                return Err(error);
            }
            Err(error) => {
                controls.shutdown.store(true, Ordering::Release);
                bail!("Audio renderer initialization failed: {error}");
            }
        }
        let stream_result = match format {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, queue, Arc::clone(&controls))
            }
            cpal::SampleFormat::F64 => {
                build_stream::<f64>(&device, &config, queue, Arc::clone(&controls))
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, queue, Arc::clone(&controls))
            }
            cpal::SampleFormat::I32 => {
                build_stream::<i32>(&device, &config, queue, Arc::clone(&controls))
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, queue, Arc::clone(&controls))
            }
            _ => Err(anyhow::anyhow!(
                "Unsupported output sample format: {format:?}"
            )),
        };
        let stream = match stream_result {
            Ok(stream) => stream,
            Err(error) => {
                controls.shutdown.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(error);
            }
        };
        let mut engine = Self {
            stream: Some(stream),
            worker: Some(worker),
            controls,
            sample_rate,
        };
        if let Err(error) = engine.stream.as_ref().unwrap().play() {
            engine.stop();
            return Err(error.into());
        }
        Ok(engine)
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
    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            playing: self.controls.playing.load(Ordering::Relaxed),
            position_beats: self.position_beats(),
            peak_left: f32::from_bits(self.controls.peak_left.load(Ordering::Relaxed)),
            peak_right: f32::from_bits(self.controls.peak_right.load(Ordering::Relaxed)),
            underruns: self.controls.underruns.load(Ordering::Relaxed),
            render_errors: self.controls.render_errors.load(Ordering::Relaxed),
        }
    }
    pub fn stop(&mut self) {
        self.controls.playing.store(false, Ordering::Release);
        self.controls.shutdown.store(true, Ordering::Release);
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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
    let generation = controls.generation.load(Ordering::Acquire);
    queue.discard_stale(generation);
    let mut peak = [0.0_f32; 2];
    let mut underrun = false;
    for frame in data.chunks_mut(channels) {
        let mut stereo = [0.0; 2];
        if playing {
            // At most one queue's worth of stale samples can exist following a seek.
            for _ in 0..=queue.slots.len() {
                match queue.pop() {
                    Some((samples, beat, epoch)) if epoch == generation => {
                        stereo = samples;
                        controls.position.store(beat.to_bits(), Ordering::Relaxed);
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
