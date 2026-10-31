//! Input capture is clocked by the output consumer, never by render-ahead.
//! Both device callbacks use bounded atomic queues; WAV I/O and finalization
//! belong to one writer thread. Monitoring is deliberately dry, before export.
use super::{AudioEngine, Controls, DeviceInfo};
use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use daw_core::{AudioInputChannels, Project, TrackKind};
use std::{
    fmt::Write as _,
    fs::{self, File},
    io::BufWriter,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_CHANNELS: usize = 32;
const INPUT_CAPACITY: usize = 8192;
const RECORD_CAPACITY: usize = 16384;
const IDLE: usize = 0;
const PREPARING: usize = 1;
const ACTIVE: usize = 2;
const FINALIZING: usize = 3;
const COMPLETE: usize = 4;
const FAULT_WRITING: usize = usize::MAX;
const CLOCK_ORIGIN_NS: u64 = 10_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputRoute {
    pub track: u64,
    pub channels: AudioInputChannels,
    pub monitor: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputSettings {
    pub device_name: Option<String>,
    pub routes: Vec<InputRoute>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualIoSettings {
    pub input_channels: u16,
    pub input_latency_frames: u32,
    pub output_latency_frames: u32,
}
impl Default for VirtualIoSettings {
    fn default() -> Self {
        Self {
            input_channels: 2,
            input_latency_frames: 0,
            output_latency_frames: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct RecordingConfig {
    pub directory: PathBuf,
    pub tracks: Vec<u64>,
}
#[derive(Clone, Debug)]
pub struct RecordingTake {
    pub track: u64,
    pub path: PathBuf,
    pub start_beat: f64,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub complete: bool,
}
#[derive(Debug)]
pub struct RecordingResult {
    pub takes: Vec<RecordingTake>,
    pub error: Option<String>,
    pub dropped_frames: u64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordingSnapshot {
    pub active: bool,
    pub preparing: bool,
    pub finalizing: bool,
    pub frames: u64,
    pub completed_passes: u64,
    pub dropped_frames: u64,
    pub input_peaks: [f32; 2],
    pub input_ready: bool,
    pub input_opening: bool,
    pub input_channels: u16,
}

pub fn list_input_devices() -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|device| device.description().ok())
        .map(|description| description.name().to_owned());
    let mut devices = Vec::new();
    for device in host.input_devices()? {
        let name = device.description()?.name().to_owned();
        let mut rates = Vec::new();
        let mut channels = 0;
        if let Ok(configs) = device.supported_input_configs() {
            for config in configs {
                channels = channels.max(config.channels());
                for rate in [
                    8000, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
                ] {
                    if (config.min_sample_rate()..=config.max_sample_rate()).contains(&rate) {
                        rates.push(rate);
                    }
                }
            }
        }
        rates.sort_unstable();
        rates.dedup();
        devices.push(DeviceInfo {
            is_default: default.as_deref() == Some(name.as_str()),
            name,
            supported_sample_rates: rates,
            channels,
        });
    }
    Ok(devices)
}

#[derive(Clone, Copy)]
struct InputFrame {
    samples: [f32; MAX_CHANNELS],
    position: u64,
    epoch: u64,
}
struct FrameSlot {
    samples: [AtomicU32; MAX_CHANNELS],
    position: AtomicU64,
    epoch: AtomicU64,
}
struct FrameQueue {
    slots: Box<[FrameSlot]>,
    read: AtomicUsize,
    write: AtomicUsize,
}

fn pack_stereo(values: [f32; 2]) -> u64 {
    u64::from(values[0].to_bits()) | (u64::from(values[1].to_bits()) << 32)
}
fn unpack_stereo(value: u64) -> [f32; 2] {
    [
        f32::from_bits(value as u32),
        f32::from_bits((value >> 32) as u32),
    ]
}
#[derive(Default)]
struct MonitorGain {
    target: AtomicU64,
    previous: AtomicU64,
    current: AtomicU64,
    remaining: AtomicU32,
}
impl MonitorGain {
    // The output callback is the sole writer of interpolation state. Atomics
    // avoid shared interior mutability and let the UI publish coherent stereo.
    fn next(&self, sample_rate: u32) -> [f32; 2] {
        let target = self.target.load(Ordering::Acquire);
        let mut left = self.remaining.load(Ordering::Relaxed);
        if target != self.previous.load(Ordering::Relaxed) {
            self.previous.store(target, Ordering::Relaxed);
            left = (sample_rate / 200).max(1);
        }
        if left == 0 {
            return unpack_stereo(target);
        }
        let target = unpack_stereo(target);
        let current = unpack_stereo(self.current.load(Ordering::Relaxed));
        let values = std::array::from_fn(|side| {
            current[side] + (target[side] - current[side]) / left as f32
        });
        self.current.store(pack_stereo(values), Ordering::Relaxed);
        self.remaining.store(left - 1, Ordering::Relaxed);
        values
    }
}
impl FrameQueue {
    fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity)
                .map(|_| FrameSlot {
                    samples: std::array::from_fn(|_| AtomicU32::new(0)),
                    position: AtomicU64::new(0),
                    epoch: AtomicU64::new(0),
                })
                .collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }
    fn len(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }
    fn push(&self, frame: InputFrame) -> bool {
        let write = self.write.load(Ordering::Relaxed);
        if write.wrapping_sub(self.read.load(Ordering::Acquire)) >= self.slots.len() {
            return false;
        }
        let slot = &self.slots[write % self.slots.len()];
        for (sample, value) in slot.samples.iter().zip(frame.samples) {
            sample.store(value.to_bits(), Ordering::Relaxed);
        }
        slot.position.store(frame.position, Ordering::Relaxed);
        slot.epoch.store(frame.epoch, Ordering::Relaxed);
        self.write.store(write.wrapping_add(1), Ordering::Release);
        true
    }
    fn pop(&self) -> Option<InputFrame> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.write.load(Ordering::Acquire) {
            return None;
        }
        let slot = &self.slots[read % self.slots.len()];
        let result = InputFrame {
            samples: std::array::from_fn(|i| {
                f32::from_bits(slot.samples[i].load(Ordering::Relaxed))
            }),
            position: slot.position.load(Ordering::Relaxed),
            epoch: slot.epoch.load(Ordering::Relaxed),
        };
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some(result)
    }
}

pub(super) struct InputShared {
    clock_origin: Instant,
    virtual_clock: AtomicBool,
    sample_rate: AtomicU32,
    input_clock_frames: AtomicU64,
    output_clock_frames: AtomicU64,
    virtual_channels: AtomicUsize,
    input_latency_frames: AtomicU32,
    output_latency_frames: AtomicU32,
    record_anchor_time: AtomicU64,
    record_anchor_position: AtomicU64,
    record_next_position: AtomicU64,
    record_started: AtomicBool,
    last_output_end: AtomicU64,
    last_input_end: AtomicU64,
    stop_at: AtomicU64,
    tail_complete: AtomicBool,
    input: FrameQueue,
    recording: FrameQueue,
    epoch: AtomicU64,
    ready: AtomicBool,
    opening: AtomicBool,
    channels: AtomicUsize,
    phase: AtomicUsize,
    stop: AtomicBool,
    fault: AtomicUsize,
    fault_details: [AtomicU8; 1024],
    fault_details_len: AtomicUsize,
    reported_fault: AtomicUsize,
    frames: AtomicU64,
    passes: AtomicU64,
    dropped: AtomicU64,
    primed: AtomicBool,
    producer_busy: AtomicBool,
    consumer_busy: AtomicBool,
    peaks: [AtomicU32; MAX_CHANNELS],
    monitor: [MonitorGain; MAX_CHANNELS],
    pub(super) tempo: AtomicU64,
    loop_enabled: AtomicBool,
    loop_start: AtomicU64,
    loop_end: AtomicU64,
}
impl InputShared {
    pub(super) fn new() -> Self {
        Self {
            clock_origin: Instant::now(),
            virtual_clock: AtomicBool::new(false),
            sample_rate: AtomicU32::new(48000),
            input_clock_frames: AtomicU64::new(0),
            output_clock_frames: AtomicU64::new(0),
            virtual_channels: AtomicUsize::new(2),
            input_latency_frames: AtomicU32::new(0),
            output_latency_frames: AtomicU32::new(0),
            record_anchor_time: AtomicU64::new(0),
            record_anchor_position: AtomicU64::new(0),
            record_next_position: AtomicU64::new(0),
            record_started: AtomicBool::new(false),
            last_output_end: AtomicU64::new(0),
            last_input_end: AtomicU64::new(0),
            stop_at: AtomicU64::new(u64::MAX),
            tail_complete: AtomicBool::new(false),
            input: FrameQueue::new(INPUT_CAPACITY),
            recording: FrameQueue::new(RECORD_CAPACITY),
            epoch: AtomicU64::new(0),
            ready: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            channels: AtomicUsize::new(0),
            phase: AtomicUsize::new(IDLE),
            stop: AtomicBool::new(false),
            fault: AtomicUsize::new(0),
            fault_details: std::array::from_fn(|_| AtomicU8::new(0)),
            fault_details_len: AtomicUsize::new(0),
            reported_fault: AtomicUsize::new(0),
            frames: AtomicU64::new(0),
            passes: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            primed: AtomicBool::new(false),
            producer_busy: AtomicBool::new(false),
            consumer_busy: AtomicBool::new(false),
            peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            monitor: std::array::from_fn(|_| MonitorGain::default()),
            tempo: AtomicU64::new(120_f64.to_bits()),
            loop_enabled: AtomicBool::new(false),
            loop_start: AtomicU64::new(0),
            loop_end: AtomicU64::new(0),
        }
    }
    pub(super) fn capture_requested(&self) -> bool {
        matches!(self.phase.load(Ordering::Acquire), PREPARING | ACTIVE)
    }
    pub(super) fn active(&self) -> bool {
        self.phase.load(Ordering::Acquire) == ACTIVE && !self.stop.load(Ordering::Acquire)
    }
    fn fail(&self, code: usize) {
        let _ = self
            .fault
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Relaxed);
        if self.fault.load(Ordering::Acquire) != FAULT_WRITING {
            self.stop.store(true, Ordering::Release);
        }
    }
    // CPAL also sends advisory notifications through its error callback.
    // Error text is copied to bounded atomic storage; formatting the public
    // diagnostic and allocating its String belongs to the UI/writer thread.
    pub(super) fn device_failed(
        &self,
        code: usize,
        error: &cpal::Error,
        epoch: Option<u64>,
    ) -> bool {
        if error.kind() == cpal::ErrorKind::RealtimeDenied
            || (error.kind() == cpal::ErrorKind::DeviceChanged && !self.capture_requested())
            || epoch.is_some_and(|epoch| epoch != self.epoch.load(Ordering::Acquire))
        {
            return false;
        }
        if self
            .fault
            .compare_exchange(0, FAULT_WRITING, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            let mut writer = DiagnosticWriter {
                buffer: &self.fault_details,
                len: 0,
            };
            let _ = write!(&mut writer, "{error}");
            if epoch.is_some_and(|epoch| epoch != self.epoch.load(Ordering::Acquire)) {
                self.fault.store(0, Ordering::Release);
                return false;
            }
            self.fault_details_len.store(writer.len, Ordering::Release);
            self.fault.store(
                if error.kind() == cpal::ErrorKind::DeviceChanged {
                    8
                } else {
                    code
                },
                Ordering::Release,
            );
            self.stop.store(true, Ordering::Release);
        }
        true
    }
    fn input_failed(&self, epoch: u64, error: &cpal::Error) {
        if self.device_failed(4, error, Some(epoch)) && self.epoch.load(Ordering::Acquire) == epoch
        {
            self.ready.store(false, Ordering::Release);
        }
    }
    fn fault_description(&self) -> Option<String> {
        let message = self.fault_message()?;
        let len = self
            .fault_details_len
            .load(Ordering::Acquire)
            .min(self.fault_details.len());
        if len == 0 {
            return Some(message.to_owned());
        }
        let bytes: Vec<_> = self.fault_details[..len]
            .iter()
            .map(|byte| byte.load(Ordering::Relaxed))
            .collect();
        Some(format!(
            "{message}\nBackend: {}",
            String::from_utf8_lossy(&bytes)
        ))
    }
    pub(super) fn request_stop(&self) {
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        let end = self.last_output_end.load(Ordering::Acquire);
        self.stop_at.store(end, Ordering::Release);
        if self.record_anchor_time.load(Ordering::Acquire) == 0
            || self
                .last_input_end
                .load(Ordering::Acquire)
                .saturating_add(self.frame_ns() / 2)
                >= end
        {
            self.tail_complete.store(true, Ordering::Release);
        }
        self.stop.store(true, Ordering::Release);
    }
    fn frame_ns(&self) -> u64 {
        1_000_000_000 / u64::from(self.sample_rate.load(Ordering::Relaxed).max(1))
    }
    pub(super) fn now_ns(&self) -> u64 {
        CLOCK_ORIGIN_NS.saturating_add(
            self.clock_origin
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX - CLOCK_ORIGIN_NS)) as u64,
        )
    }
    pub(super) fn output_time(&self, info: Option<&cpal::OutputCallbackInfo>) -> u64 {
        if self.virtual_clock.load(Ordering::Relaxed) {
            CLOCK_ORIGIN_NS
                + ((u128::from(self.output_clock_frames.load(Ordering::Relaxed))
                    + u128::from(self.output_latency_frames.load(Ordering::Relaxed)))
                    * 1_000_000_000
                    / u128::from(self.sample_rate.load(Ordering::Relaxed))) as u64
        } else {
            // CPAL stream timestamp origins need not match. Translate each
            // stream's capture/callback or callback/playback interval onto our
            // monotonic clock. Accuracy follows the driver's reported timing.
            let delay = info
                .map(|info| {
                    let timestamp = info.timestamp();
                    timestamp
                        .playback
                        .saturating_duration_since(timestamp.callback)
                })
                .unwrap_or_default();
            self.now_ns()
                .saturating_add(delay.as_nanos().min(u128::from(u64::MAX)) as u64)
        }
    }
    pub(super) fn advance_output_clock(&self, frames: usize) {
        self.output_clock_frames
            .fetch_add(frames as u64, Ordering::Relaxed);
    }
    fn feed_virtual(&self, samples: &[f32], channels: usize) {
        let frames = samples.len() / channels;
        let first = self
            .input_clock_frames
            .fetch_add(frames as u64, Ordering::Relaxed);
        let time = i128::from(CLOCK_ORIGIN_NS)
            + (i128::from(first) - i128::from(self.input_latency_frames.load(Ordering::Relaxed)))
                * 1_000_000_000
                / i128::from(self.sample_rate.load(Ordering::Relaxed));
        self.feed_at(
            samples,
            channels,
            self.epoch.load(Ordering::Acquire),
            time.max(0) as u64,
        );
    }
    fn feed_at<T: cpal::Sample>(
        &self,
        data: &[T],
        channels: usize,
        epoch: u64,
        first_capture_ns: u64,
    ) where
        f32: cpal::FromSample<T>,
    {
        if !self.ready.load(Ordering::Acquire)
            || epoch != self.epoch.load(Ordering::Acquire)
            || channels == 0
        {
            return;
        }
        if self
            .producer_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            if epoch == self.epoch.load(Ordering::Acquire) && self.capture_requested() {
                self.dropped
                    .fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
                self.fail(1);
            }
            return;
        }
        let mut peak = [0_f32; MAX_CHANNELS];
        for (index, frame) in data.chunks_exact(channels).enumerate() {
            if epoch != self.epoch.load(Ordering::Acquire) {
                break;
            }
            let mut values = [0_f32; MAX_CHANNELS];
            for (index, sample) in frame.iter().take(MAX_CHANNELS).enumerate() {
                let value = <f32 as cpal::FromSample<T>>::from_sample_(*sample);
                if !value.is_finite() {
                    self.fail(5);
                    continue;
                }
                values[index] = value;
                peak[index] = peak[index].max(value.abs());
            }
            if !self.input.push(InputFrame {
                samples: values,
                position: first_capture_ns.saturating_add(
                    (index as u128 * 1_000_000_000
                        / u128::from(self.sample_rate.load(Ordering::Relaxed)))
                        as u64,
                ),
                epoch,
            }) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                if self.capture_requested() {
                    self.fail(1);
                }
            } else {
                self.primed.store(true, Ordering::Release);
            }
        }
        for (cell, value) in self.peaks.iter().zip(peak) {
            cell.store(value.to_bits(), Ordering::Relaxed);
        }
        self.producer_busy.store(false, Ordering::Release);
    }
    /// Called only by the output consumer, exactly once per hardware frame.
    #[cfg(test)]
    fn consume(&self, position: Option<u64>, sample_rate: u32, playback_ns: u64) -> [f32; 2] {
        self.consume_gated(position, sample_rate, playback_ns, true)
    }
    pub(super) fn consume_gated(
        &self,
        position: Option<u64>,
        sample_rate: u32,
        playback_ns: u64,
        capture_allowed: bool,
    ) -> [f32; 2] {
        // Preserve an already-scheduled ADC/DAC tail after the common gate closes.
        let capture_allowed = capture_allowed
            || (self.stop.load(Ordering::Acquire)
                && self.record_anchor_time.load(Ordering::Acquire) > 0);
        self.consumer_busy.store(true, Ordering::SeqCst);
        let epoch = self.epoch.load(Ordering::Acquire);
        let phase = self.phase.load(Ordering::SeqCst);
        let frame_ns = self.frame_ns();
        self.last_output_end
            .store(playback_ns.saturating_add(frame_ns), Ordering::Release);
        if phase == PREPARING {
            self.consumer_busy.store(false, Ordering::SeqCst);
            return [0.0; 2];
        }
        if phase == ACTIVE
            && capture_allowed
            && self.record_anchor_time.load(Ordering::Acquire) == 0
            && !self.stop.load(Ordering::Acquire)
            && let Some(position) = position
        {
            self.record_anchor_position
                .store(position, Ordering::Relaxed);
            self.record_anchor_time
                .store(playback_ns, Ordering::Release);
        }
        if phase == ACTIVE
            && capture_allowed
            && self.record_anchor_time.load(Ordering::Acquire) > 0
            && position.is_none()
            && !self.stop.load(Ordering::Acquire)
        {
            self.fail(7);
        }
        let mut input = None;
        for _ in 0..INPUT_CAPACITY {
            let Some(frame) = self.input.pop() else {
                break;
            };
            if frame.epoch == epoch {
                input = Some(frame);
                break;
            }
        }
        let mut output = [0.0_f32; 2];
        if self.ready.load(Ordering::Acquire)
            && let Some(mut frame) = input
        {
            for (channel, value) in frame.samples.iter().enumerate() {
                let gain = self.monitor[channel].next(sample_rate);
                output[0] += *value * gain[0];
                output[1] += *value * gain[1];
            }
            let capture_ns = frame.position;
            let capture_end = capture_ns.saturating_add(frame_ns);
            self.last_input_end.store(capture_end, Ordering::Release);
            let anchor = self.record_anchor_time.load(Ordering::Acquire);
            let stop_at = self.stop_at.load(Ordering::Acquire);
            let stopping = self.stop.load(Ordering::Acquire);
            if phase == ACTIVE
                && capture_allowed
                && anchor > 0
                && capture_ns.saturating_add(frame_ns / 2) >= anchor
                && (!stopping || capture_ns.saturating_add(frame_ns / 2) < stop_at)
                && self.fault.load(Ordering::Acquire) == 0
            {
                let next = if !self.record_started.swap(true, Ordering::AcqRel) {
                    let offset = ((u128::from(capture_ns.saturating_sub(anchor))
                        * u128::from(sample_rate)
                        + 500_000_000)
                        / 1_000_000_000) as u64;
                    self.record_anchor_position
                        .load(Ordering::Relaxed)
                        .saturating_add(offset)
                } else {
                    self.record_next_position.load(Ordering::Relaxed)
                };
                self.record_next_position
                    .store(next.saturating_add(1), Ordering::Relaxed);
                let tempo = f64::from_bits(self.tempo.load(Ordering::Relaxed));
                let loop_start = (f64::from_bits(self.loop_start.load(Ordering::Relaxed))
                    * sample_rate as f64
                    * 60.0
                    / tempo)
                    .round() as u64;
                let loop_end = (f64::from_bits(self.loop_end.load(Ordering::Relaxed))
                    * sample_rate as f64
                    * 60.0
                    / tempo)
                    .round() as u64;
                frame.position = if self.loop_enabled.load(Ordering::Relaxed)
                    && loop_end > loop_start
                    && next >= loop_end
                {
                    loop_start + (next - loop_end) % (loop_end - loop_start)
                } else {
                    next
                };
                if self.recording.push(frame) {
                    self.frames.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    self.fail(2);
                }
            }
            if stopping && capture_end.saturating_add(frame_ns / 2) >= stop_at {
                self.tail_complete.store(true, Ordering::Release);
            }
        } else if phase == ACTIVE && capture_allowed && !self.stop.load(Ordering::Acquire) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.fail(3);
        }
        self.consumer_busy.store(false, Ordering::SeqCst);
        output
    }
    fn fault_message(&self) -> Option<&'static str> {
        match self.fault.load(Ordering::Acquire) {
            1 => Some(
                "Audio input overflowed before capture; check the audio buffer size and device clock settings",
            ),
            2 => Some(
                "Recording disk queue overflowed; try a faster scratch disk or larger audio buffer",
            ),
            3 => Some(
                "Audio input did not keep up with output; check device sample rates and buffer settings",
            ),
            4 => Some("Audio input stream failed or disconnected"),
            5 => Some("Audio input produced non-finite samples"),
            6 => Some(
                "Audio output stream failed or disconnected\n\nCheck the output device in Audio setup, then choose Retry playback or press Play to reconnect.",
            ),
            7 => Some(
                "Audio output ran out of playback samples; recording stopped to preserve overdub alignment",
            ),
            8 => Some(
                "The audio device changed during recording; capture stopped to preserve overdub alignment",
            ),
            9 => Some(
                "Audio output dropout interrupted recording\n\nCapture stopped to preserve timing and retain the recorded prefix. Choose Retry playback or press Play to reconnect before recording again.",
            ),
            _ => None,
        }
    }
}

struct DiagnosticWriter<'a> {
    buffer: &'a [AtomicU8],
    len: usize,
}
impl std::fmt::Write for DiagnosticWriter<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let mut len = value.len().min(self.buffer.len() - self.len);
        while !value.is_char_boundary(len) {
            len -= 1;
        }
        for byte in &value.as_bytes()[..len] {
            self.buffer[self.len].store(*byte, Ordering::Relaxed);
            self.len += 1;
        }
        Ok(())
    }
}

struct OpeningInput {
    receiver: mpsc::Receiver<Result<cpal::Stream>>,
    cancel: Arc<AtomicBool>,
}
struct RecordingJob {
    receiver: mpsc::Receiver<RecordingResult>,
    worker: JoinHandle<()>,
}
pub(super) struct InputController {
    controls: Arc<Controls>,
    sample_rate: u32,
    buffer_frames: u32,
    virtual_input: bool,
    settings: InputSettings,
    stream: Option<cpal::Stream>,
    opening: Option<OpeningInput>,
    next_open: Option<(Project, InputSettings)>,
    recording: Option<RecordingJob>,
    error: Option<String>,
}
impl InputController {
    pub(super) fn new(
        controls: Arc<Controls>,
        sample_rate: u32,
        buffer_frames: u32,
        virtual_input: bool,
    ) -> Self {
        controls
            .input
            .virtual_clock
            .store(virtual_input, Ordering::Release);
        controls
            .input
            .sample_rate
            .store(sample_rate, Ordering::Release);
        Self {
            controls,
            sample_rate,
            buffer_frames,
            virtual_input,
            settings: InputSettings::default(),
            stream: None,
            opening: None,
            next_open: None,
            recording: None,
            error: None,
        }
    }
    fn shared(&self) -> &Arc<InputShared> {
        &self.controls.input
    }
    pub(super) fn sync_mix(&self, project: &Project) -> Result<()> {
        if self.shared().capture_requested() {
            ensure!(
                project.tempo.to_bits() == self.shared().tempo.load(Ordering::Acquire)
                    && project.loop_enabled == self.shared().loop_enabled.load(Ordering::Acquire)
                    && project.loop_start.to_bits()
                        == self.shared().loop_start.load(Ordering::Acquire)
                    && project.loop_end.to_bits() == self.shared().loop_end.load(Ordering::Acquire),
                "Finish recording before changing tempo or loop boundaries"
            );
        }
        let order = project.routing_order()?;
        let audible = crate::render::audible_tracks(project, &order);
        let mut matrix = [[0.0_f32; 2]; MAX_CHANNELS];
        for route in self.settings.routes.iter().filter(|route| route.monitor) {
            let mut factors = [1.0_f32; 2];
            let mut current = Some(route.track);
            while let Some(track) = current.and_then(|id| project.track(id)) {
                if !audible.contains(&track.id) {
                    factors = [0.0; 2];
                    break;
                }
                factors[0] *= track.gain * (1.0 - track.pan.max(0.0));
                factors[1] *= track.gain * (1.0 + track.pan.min(0.0));
                current = track.output_bus;
            }
            match route.channels {
                AudioInputChannels::Mono { channel } => {
                    matrix[channel as usize][0] += factors[0];
                    matrix[channel as usize][1] += factors[1];
                }
                AudioInputChannels::Stereo { left, right } => {
                    matrix[left as usize][0] += factors[0];
                    matrix[right as usize][1] += factors[1];
                }
            }
        }
        for (target, values) in self.shared().monitor.iter().zip(matrix) {
            target.target.store(pack_stereo(values), Ordering::Release);
        }
        self.shared()
            .tempo
            .store(project.tempo.to_bits(), Ordering::Release);
        self.shared()
            .loop_enabled
            .store(project.loop_enabled, Ordering::Release);
        self.shared()
            .loop_start
            .store(project.loop_start.to_bits(), Ordering::Release);
        self.shared()
            .loop_end
            .store(project.loop_end.to_bits(), Ordering::Release);
        Ok(())
    }
    fn configure(&mut self, project: &Project, settings: InputSettings) -> Result<()> {
        self.poll();
        ensure!(
            settings.routes.len() <= 32,
            "At most 32 input routes are supported"
        );
        let mut tracks = std::collections::HashSet::new();
        for route in &settings.routes {
            ensure!(tracks.insert(route.track), "Duplicate input route");
            ensure!(
                project
                    .track(route.track)
                    .is_some_and(|track| track.kind == TrackKind::Audio),
                "Input route requires an audio track"
            );
            route.channels.validate(MAX_CHANNELS as u16)?;
        }
        let same_device = self.settings.device_name == settings.device_name;
        let same_routes = self
            .settings
            .routes
            .iter()
            .map(|route| (route.track, route.channels))
            .eq(settings
                .routes
                .iter()
                .map(|route| (route.track, route.channels)));
        ensure!(
            self.recording.is_none() || (same_device && same_routes),
            "Finish recording before changing inputs"
        );
        if let Some(opening) = &self.opening {
            if self.settings == settings && self.next_open.is_none() {
                return self.sync_mix(project);
            }
            // Exactly one native open may be outstanding, even if a driver is
            // slow. Keep only the newest request and invalidate old callbacks.
            self.next_open = Some((project.clone(), settings));
            opening.cancel.store(true, Ordering::Release);
            self.shared().epoch.fetch_add(1, Ordering::AcqRel);
            self.shared().ready.store(false, Ordering::Release);
            return Ok(());
        }
        if same_device && self.shared().ready.load(Ordering::Acquire) && !settings.routes.is_empty()
        {
            let count = self.shared().channels.load(Ordering::Acquire) as u16;
            for route in &settings.routes {
                route.channels.validate(count)?;
            }
            self.settings = settings;
            return self.sync_mix(project);
        }
        ensure!(
            self.recording.is_none(),
            "Finish recording and retrieve captured takes before changing inputs"
        );
        if let Some(opening) = self.opening.take() {
            opening.cancel.store(true, Ordering::Release);
        }
        let previous = self.stream.take();
        self.shared().ready.store(false, Ordering::Release);
        self.shared().channels.store(0, Ordering::Release);
        self.shared().primed.store(false, Ordering::Release);
        self.shared().fault_details_len.store(0, Ordering::Release);
        self.shared().fault.store(0, Ordering::Release);
        self.shared().reported_fault.store(0, Ordering::Release);
        let epoch = self
            .shared()
            .epoch
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.settings = settings;
        self.sync_mix(project)?;
        if self.settings.routes.is_empty() {
            self.shared().opening.store(false, Ordering::Release);
            if previous.is_some() {
                thread::spawn(move || drop(previous));
            }
            return Ok(());
        }
        if self.virtual_input {
            ensure!(
                self.settings
                    .device_name
                    .as_deref()
                    .is_none_or(|name| name == "Rill virtual input"),
                "Selected input device is unavailable"
            );
            for route in &self.settings.routes {
                route
                    .channels
                    .validate(self.shared().virtual_channels.load(Ordering::Relaxed) as u16)?;
            }
            self.shared().channels.store(
                self.shared().virtual_channels.load(Ordering::Relaxed),
                Ordering::Release,
            );
            self.shared().input_clock_frames.store(
                self.shared().output_clock_frames.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            self.shared().ready.store(true, Ordering::Release);
            self.shared().opening.store(false, Ordering::Release);
            self.shared().primed.store(true, Ordering::Release);
            return Ok(());
        }
        let settings = self.settings.clone();
        let shared = Arc::clone(self.shared());
        let rate = self.sample_rate;
        let buffer = self.buffer_frames;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, receiver) = mpsc::channel();
        shared.opening.store(true, Ordering::Release);
        thread::spawn(move || {
            drop(previous);
            let result = open_input(
                settings,
                rate,
                buffer,
                Arc::clone(&shared),
                epoch,
                &worker_cancel,
            );
            if shared.epoch.load(Ordering::Acquire) == epoch {
                shared.opening.store(false, Ordering::Release);
                if result.is_err() {
                    shared.ready.store(false, Ordering::Release);
                }
            }
            let _ = tx.send(result);
        });
        self.opening = Some(OpeningInput { receiver, cancel });
        Ok(())
    }
    fn poll(&mut self) {
        if let Some(opening) = &self.opening {
            match opening.receiver.try_recv() {
                Ok(result) => {
                    self.opening = None;
                    if let Some((project, settings)) = self.next_open.take() {
                        // Stream teardown can wait for a backend callback.
                        thread::spawn(move || drop(result));
                        if let Err(error) = self.configure(&project, settings) {
                            self.error = Some(format!("Open audio input: {error:#}"));
                        }
                        return;
                    }
                    match result {
                        Ok(stream) => self.stream = Some(stream),
                        Err(error) => self.error = Some(format!("Open audio input: {error:#}")),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.opening = None;
                    self.shared().opening.store(false, Ordering::Release);
                    if let Some((project, settings)) = self.next_open.take() {
                        if let Err(error) = self.configure(&project, settings) {
                            self.error = Some(format!("Open audio input: {error:#}"));
                        }
                    } else {
                        self.error = Some("Audio input preparation stopped unexpectedly".into());
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    pub(super) fn shutdown(&mut self) {
        self.next_open = None;
        self.shared().request_stop();
        self.shared().ready.store(false, Ordering::Release);
        self.shared().epoch.fetch_add(1, Ordering::AcqRel);
        if let Some(opening) = self.opening.take() {
            opening.cancel.store(true, Ordering::Release);
        }
        self.stream.take();
        if let Some(job) = self.recording.take() {
            let _ = job.worker.join();
        }
    }
}

fn open_input(
    settings: InputSettings,
    sample_rate: u32,
    buffer_frames: u32,
    shared: Arc<InputShared>,
    epoch: u64,
    cancel: &AtomicBool,
) -> Result<cpal::Stream> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        "Input preparation cancelled"
    );
    let host = cpal::default_host();
    let device = if let Some(name) = settings.device_name {
        host.input_devices()?
            .find(|device| {
                device
                    .description()
                    .is_ok_and(|description| description.name() == name)
            })
            .with_context(|| format!("Input device '{name}' is unavailable"))?
    } else {
        host.default_input_device()
            .context("No default audio input device is available")?
    };
    let required = settings
        .routes
        .iter()
        .map(|route| match route.channels {
            AudioInputChannels::Mono { channel } => channel + 1,
            AudioInputChannels::Stereo { left, right } => left.max(right) + 1,
        })
        .max()
        .unwrap_or(1);
    let mut configs: Vec<_> = device
        .supported_input_configs()?
        .filter(|range| {
            range.channels() >= required
                && range.min_sample_rate() <= sample_rate
                && range.max_sample_rate() >= sample_rate
        })
        .collect();
    configs.sort_by_key(|range| {
        (
            range.sample_format() != cpal::SampleFormat::F32,
            range.channels(),
        )
    });
    let range = configs
        .first()
        .context("Input does not support the selected channels at the project sample rate")?;
    let format = range.sample_format();
    let mut config = range.with_sample_rate(sample_rate).config();
    config.buffer_size = match range.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed(buffer_frames.clamp(*min, *max))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    };
    let stream = match format {
        cpal::SampleFormat::F32 => build_input::<f32>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::F64 => build_input::<f64>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::I16 => build_input::<i16>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::I32 => build_input::<i32>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::U16 => build_input::<u16>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::U32 => build_input::<u32>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::I8 => build_input::<i8>(&device, config, Arc::clone(&shared), epoch),
        cpal::SampleFormat::U8 => build_input::<u8>(&device, config, Arc::clone(&shared), epoch),
        _ => Err(anyhow::anyhow!(
            "Unsupported input sample format: {format:?}"
        )),
    }?;
    ensure!(
        !cancel.load(Ordering::Acquire) && shared.epoch.load(Ordering::Acquire) == epoch,
        "Input preparation cancelled"
    );
    shared
        .channels
        .store(config.channels as usize, Ordering::Release);
    shared.ready.store(true, Ordering::Release);
    stream.play()?;
    Ok(stream)
}
fn build_input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<InputShared>,
    epoch: u64,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let errors = Arc::clone(&shared);
    let channels = config.channels as usize;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            let timestamp = info.timestamp();
            let delay = timestamp
                .callback
                .saturating_duration_since(timestamp.capture);
            let capture = shared
                .now_ns()
                .saturating_sub(delay.as_nanos().min(u128::from(u64::MAX)) as u64);
            shared.feed_at(data, channels, epoch, capture);
        },
        move |error| {
            errors.input_failed(epoch, &error);
        },
        None,
    )?)
}

impl AudioEngine {
    pub fn configure_input(&mut self, project: &Project, settings: InputSettings) -> Result<()> {
        self.input.configure(project, settings)
    }
    pub fn disable_input(&mut self) -> Result<()> {
        ensure!(
            self.input.recording.is_none(),
            "Finish recording and retrieve captured takes before closing inputs"
        );
        self.input.settings.routes.clear();
        self.input.shutdown();
        Ok(())
    }
    pub fn poll_input(&mut self) {
        self.input.poll();
    }
    pub fn recording_snapshot(&self) -> RecordingSnapshot {
        let shared = &self.controls.input;
        let phase = shared.phase.load(Ordering::Acquire);
        RecordingSnapshot {
            active: phase == ACTIVE,
            preparing: phase == PREPARING,
            finalizing: phase == FINALIZING || phase == COMPLETE,
            frames: shared.frames.load(Ordering::Relaxed),
            completed_passes: shared.passes.load(Ordering::Relaxed),
            dropped_frames: shared.dropped.load(Ordering::Relaxed),
            input_peaks: [
                f32::from_bits(shared.peaks[0].load(Ordering::Relaxed)),
                f32::from_bits(shared.peaks[1].load(Ordering::Relaxed)),
            ],
            input_ready: shared.ready.load(Ordering::Acquire),
            input_opening: shared.opening.load(Ordering::Acquire),
            input_channels: shared.channels.load(Ordering::Acquire) as u16,
        }
    }
    pub fn input_peaks(&self, track: u64) -> [f32; 2] {
        if !self.controls.input.ready.load(Ordering::Acquire) {
            return [0.0; 2];
        }
        let Some(route) = self
            .input
            .settings
            .routes
            .iter()
            .find(|route| route.track == track)
        else {
            return [0.0; 2];
        };
        let read = |channel: u16| {
            f32::from_bits(self.controls.input.peaks[channel as usize].load(Ordering::Relaxed))
        };
        match route.channels {
            AudioInputChannels::Mono { channel } => [read(channel); 2],
            AudioInputChannels::Stereo { left, right } => [read(left), read(right)],
        }
    }
    pub fn take_input_error(&mut self) -> Option<String> {
        self.input.poll();
        if let Some(error) = self.input.error.take() {
            return Some(error);
        }
        let shared = &self.controls.input;
        let fault = shared.fault.load(Ordering::Acquire);
        if fault != 0
            && fault != FAULT_WRITING
            && shared.reported_fault.swap(fault, Ordering::AcqRel) != fault
        {
            shared.fault_description()
        } else {
            None
        }
    }
    pub fn start_recording(&mut self, config: RecordingConfig) -> Result<()> {
        self.ensure_output_ready()?;
        self.input.poll();
        let shared = &self.controls.input;
        ensure!(
            shared.ready.load(Ordering::Acquire)
                && shared.fault.load(Ordering::Acquire) != FAULT_WRITING,
            "Select an available input and wait until it is ready"
        );
        ensure!(
            self.input.recording.is_none() && shared.phase.load(Ordering::Acquire) == IDLE,
            "A recording is already active or finalizing"
        );
        ensure!(
            !config.tracks.is_empty() && config.tracks.len() <= 32,
            "Arm between one and 32 audio tracks"
        );
        let mut tracks = std::collections::HashSet::new();
        let routes = config
            .tracks
            .iter()
            .map(|id| {
                ensure!(tracks.insert(*id), "Track is armed more than once");
                self.input
                    .settings
                    .routes
                    .iter()
                    .find(|route| route.track == *id)
                    .cloned()
                    .with_context(|| format!("Armed track {id} has no configured input"))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !config.directory.as_os_str().is_empty(),
            "Choose a recording scratch directory"
        );
        shared.stop.store(false, Ordering::Release);
        shared.stop_at.store(u64::MAX, Ordering::Release);
        shared.tail_complete.store(false, Ordering::Release);
        shared.record_anchor_time.store(0, Ordering::Release);
        shared.record_started.store(false, Ordering::Release);
        shared.fault_details_len.store(0, Ordering::Release);
        shared.fault.store(0, Ordering::Release);
        shared.reported_fault.store(0, Ordering::Release);
        shared.frames.store(0, Ordering::Release);
        shared.passes.store(0, Ordering::Release);
        shared.dropped.store(0, Ordering::Release);
        while shared.recording.pop().is_some() {}
        shared.phase.store(PREPARING, Ordering::Release);
        self.controls
            .finished_generation
            .store(u64::MAX, Ordering::Release);
        let controls = Arc::clone(&self.controls);
        let sample_rate = self.sample_rate;
        let buffer = self.input.buffer_frames as usize;
        let virtual_input = self.input.virtual_input;
        let (tx, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("daw-record-writer".into())
            .spawn(move || {
                let result = record_to_disk(
                    &controls,
                    config,
                    routes,
                    sample_rate,
                    buffer,
                    virtual_input,
                );
                controls.input.phase.store(COMPLETE, Ordering::Release);
                let _ = tx.send(result);
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                shared.phase.store(IDLE, Ordering::Release);
                return Err(error.into());
            }
        };
        self.input.recording = Some(RecordingJob { receiver, worker });
        if let Err(error) = self.ensure_output_ready() {
            // A device fault can race preparation after the entry check.
            self.controls.input.request_stop();
            return Err(error);
        }
        Ok(())
    }
    pub fn request_stop_recording(&self) {
        self.controls.input.request_stop();
    }
    pub fn take_recording_result(&mut self) -> Option<RecordingResult> {
        self.input.poll();
        let job = self.input.recording.as_ref()?;
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => RecordingResult {
                takes: Vec::new(),
                error: Some("Recording writer stopped unexpectedly".into()),
                dropped_frames: self.controls.input.dropped.load(Ordering::Relaxed),
            },
        };
        let job = self.input.recording.take().unwrap();
        // The result is sent as the thread's last operation. Join only when the
        // runtime confirms completion, otherwise reclaim this handle off UI.
        if job.worker.is_finished() {
            let _ = job.worker.join();
        } else {
            thread::spawn(move || {
                let _ = job.worker.join();
            });
        }
        self.controls.input.phase.store(IDLE, Ordering::Release);
        Some(result)
    }
    /// Configure the device-free test backend. Input and output maintain independent
    /// sample clocks; latency settings model the driver's capture/playback delays.
    pub fn configure_virtual_io(&mut self, settings: VirtualIoSettings) -> Result<()> {
        ensure!(
            self.input.virtual_input,
            "Dummy I/O requires a virtual audio engine"
        );
        ensure!(
            self.input.recording.is_none(),
            "Finish recording before changing dummy I/O"
        );
        ensure!(
            (1..=MAX_CHANNELS as u16).contains(&settings.input_channels),
            "Dummy input must have between 1 and 32 channels"
        );
        ensure!(
            settings.input_latency_frames <= self.sample_rate
                && settings.output_latency_frames <= self.sample_rate,
            "Dummy device latency must not exceed one second per device"
        );
        for route in &self.input.settings.routes {
            route.channels.validate(settings.input_channels)?;
        }
        let shared = &self.controls.input;
        if shared.virtual_channels.load(Ordering::Relaxed) == usize::from(settings.input_channels)
            && shared.input_latency_frames.load(Ordering::Relaxed) == settings.input_latency_frames
            && shared.output_latency_frames.load(Ordering::Relaxed)
                == settings.output_latency_frames
        {
            return Ok(());
        }
        let ready = shared.ready.swap(false, Ordering::AcqRel);
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        shared
            .virtual_channels
            .store(usize::from(settings.input_channels), Ordering::Release);
        shared
            .input_latency_frames
            .store(settings.input_latency_frames, Ordering::Release);
        shared
            .output_latency_frames
            .store(settings.output_latency_frames, Ordering::Release);
        shared.input_clock_frames.store(
            shared.output_clock_frames.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        if ready {
            shared
                .channels
                .store(usize::from(settings.input_channels), Ordering::Release);
            shared.ready.store(true, Ordering::Release);
        }
        Ok(())
    }
    /// Deliver only input callbacks. Use pump_virtual independently to exercise
    /// scheduling, clock skew and bounded overflow without touching audio hardware.
    pub fn feed_virtual_input(&mut self, samples: &[f32], channels: u16) -> Result<()> {
        ensure!(
            self.input.virtual_input,
            "Virtual input requires a virtual audio engine"
        );
        ensure!(
            self.controls.input.ready.load(Ordering::Acquire),
            "Select and configure a virtual input before feeding audio"
        );
        ensure!(
            channels > 0 && samples.len().is_multiple_of(usize::from(channels)),
            "Input data must contain complete channel frames"
        );
        ensure!(
            samples.len() / usize::from(channels) <= 480_000,
            "Virtual input request is too large"
        );
        ensure!(
            usize::from(channels) == self.controls.input.channels.load(Ordering::Acquire),
            "Virtual input channel count differs from the selected device"
        );
        self.controls
            .input
            .feed_virtual(samples, usize::from(channels));
        Ok(())
    }
    /// Run the same input and output callbacks in bounded, deterministic chunks.
    pub fn pump_virtual_input(&mut self, samples: &[f32], channels: u16) -> Result<Vec<[f32; 2]>> {
        ensure!(
            self.input.virtual_input,
            "Virtual input requires a virtual audio engine"
        );
        ensure!(
            channels > 0 && samples.len().is_multiple_of(channels as usize),
            "Input data must contain complete channel frames"
        );
        ensure!(
            samples.len() / channels as usize <= 480_000,
            "Virtual input request is too large"
        );
        ensure!(
            channels as usize == self.controls.input.channels.load(Ordering::Acquire),
            "Virtual input channel count differs from the selected device"
        );
        let mut output = Vec::with_capacity(samples.len() / channels as usize);
        for chunk in samples.chunks(super::BLOCK * channels as usize) {
            self.feed_virtual_input(chunk, channels)?;
            output.extend(self.pump_virtual(chunk.len() / channels as usize)?);
            // Offline test driving must not manufacture disk overflow merely
            // because it runs faster than the simulated device's wall clock.
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.controls.input.recording.len() > RECORD_CAPACITY / 2 {
                ensure!(Instant::now() < deadline, "Recording writer timed out");
                thread::sleep(Duration::from_micros(100));
            }
        }
        Ok(output)
    }
    pub fn fail_virtual_output(&self) -> Result<()> {
        ensure!(
            self.input.virtual_input,
            "Virtual output failure requires a virtual audio engine"
        );
        super::output_stream_failed(
            &self.controls,
            &cpal::Error::with_message(
                cpal::ErrorKind::DeviceNotAvailable,
                "Virtual output disconnected",
            ),
        );
        Ok(())
    }
    pub fn xrun_virtual_output(&self) -> Result<()> {
        ensure!(
            self.input.virtual_input,
            "Virtual output dropout requires a virtual audio engine"
        );
        super::output_stream_failed(&self.controls, &cpal::Error::new(cpal::ErrorKind::Xrun));
        Ok(())
    }
    pub fn fail_virtual_input(&self) -> Result<()> {
        ensure!(
            self.input.virtual_input,
            "Virtual input failure requires a virtual audio engine"
        );
        self.controls.input.input_failed(
            self.controls.input.epoch.load(Ordering::Acquire),
            &cpal::Error::with_message(
                cpal::ErrorKind::DeviceNotAvailable,
                "Virtual input disconnected",
            ),
        );
        Ok(())
    }
}

type WavWriter = hound::WavWriter<BufWriter<File>>;
struct OpenTake {
    route: InputRoute,
    path: PathBuf,
    writer: WavWriter,
    start: u64,
    frames: u64,
}
fn finish_takes(
    open: &mut Vec<OpenTake>,
    result: &mut RecordingResult,
    sample_rate: u32,
    tempo: f64,
    complete: bool,
) {
    for take in open.drain(..) {
        match take.writer.finalize() {
            Ok(()) if take.frames > 0 => result.takes.push(RecordingTake {
                track: take.route.track,
                path: take.path,
                start_beat: take.start as f64 * tempo / (sample_rate as f64 * 60.0),
                frames: take.frames,
                sample_rate,
                channels: take.route.channels.channel_count(),
                complete,
            }),
            Ok(()) => {
                let _ = fs::remove_file(take.path);
            }
            Err(error) => {
                result.error.get_or_insert_with(|| {
                    format!("Finalize recording {}: {error}", take.path.display())
                });
            }
        }
    }
}
fn record_to_disk(
    controls: &Controls,
    config: RecordingConfig,
    routes: Vec<InputRoute>,
    sample_rate: u32,
    buffer: usize,
    virtual_input: bool,
) -> RecordingResult {
    let shared = &controls.input;
    let tempo = f64::from_bits(shared.tempo.load(Ordering::Acquire));
    let loop_range = shared.loop_enabled.load(Ordering::Acquire).then(|| {
        (
            (f64::from_bits(shared.loop_start.load(Ordering::Acquire)) * sample_rate as f64 * 60.0
                / tempo)
                .round() as u64,
            (f64::from_bits(shared.loop_end.load(Ordering::Acquire)) * sample_rate as f64 * 60.0
                / tempo)
                .round() as u64,
        )
    });
    let mut result = RecordingResult {
        takes: Vec::new(),
        error: None,
        dropped_frames: 0,
    };
    let mut open = Vec::with_capacity(routes.len());
    let mut last_position = None;
    let mut pass = 0_u64;
    let outcome = (|| -> Result<()> {
        fs::create_dir_all(&config.directory).with_context(|| {
            format!("Create recording directory {}", config.directory.display())
        })?;
        let token = format!(
            "recording-{}-{}-{}",
            std::process::id(),
            controls.engine_id,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let directory = config.directory.join(token);
        fs::create_dir(&directory)
            .with_context(|| format!("Create recording session {}", directory.display()))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !virtual_input && shared.input.len() < buffer.min(INPUT_CAPACITY / 4) {
            if shared.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Input device produced no audio within five seconds"
            );
            thread::sleep(Duration::from_millis(1));
        }
        if shared.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        shared.phase.store(ACTIVE, Ordering::Release);
        controls
            .finished_generation
            .store(u64::MAX, Ordering::Release);

        let mut stopping_since = None;
        loop {
            while let Some(frame) = shared.recording.pop() {
                ensure!(
                    frame.epoch == shared.epoch.load(Ordering::Acquire),
                    "Input changed during recording"
                );
                if let Some(previous) = last_position {
                    ensure!(
                        frame.position == previous + 1
                            || loop_range.is_some_and(
                                |(start, end)| previous + 1 == end && frame.position == start
                            ),
                        "Transport moved during recording; captured audio was recovered"
                    );
                }
                if open.is_empty() {
                    ensure!(
                        pass < daw_core::MAX_AUDIO_TAKES as u64,
                        "Recording reached the 64-take session limit; completed takes were retained"
                    );
                    for route in &routes {
                        let path =
                            directory.join(format!("track-{}-take-{}.wav", route.track, pass + 1));
                        let file = File::options()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .with_context(|| format!("Create recording {}", path.display()))?;
                        let spec = hound::WavSpec {
                            channels: route.channels.channel_count(),
                            sample_rate,
                            bits_per_sample: 32,
                            sample_format: hound::SampleFormat::Float,
                        };
                        let writer =
                            hound::WavWriter::new(BufWriter::with_capacity(64 * 1024, file), spec)?;
                        open.push(OpenTake {
                            route: route.clone(),
                            path,
                            writer,
                            start: frame.position,
                            frames: 0,
                        });
                    }
                }
                for take in &open {
                    ensure!(
                        (take.frames + 1) * u64::from(take.route.channels.channel_count()) * 4
                            <= 512 * 1024 * 1024,
                        "Recording reached the 512 MiB per-take playback limit; captured audio was retained"
                    );
                }
                for take in &mut open {
                    match take.route.channels {
                        AudioInputChannels::Mono { channel } => {
                            take.writer.write_sample(frame.samples[channel as usize])?
                        }
                        AudioInputChannels::Stereo { left, right } => {
                            take.writer.write_sample(frame.samples[left as usize])?;
                            take.writer.write_sample(frame.samples[right as usize])?;
                        }
                    }
                    take.frames += 1;
                    // Keep every WAV below RIFF's 4 GiB limit, retaining a
                    // recoverable file and an explicit error before overflow.
                    ensure!(
                        take.frames * u64::from(take.route.channels.channel_count()) * 4
                            < u64::from(u32::MAX) - 1024,
                        "Recording reached WAV size limit"
                    );
                }
                last_position = Some(frame.position);
                if loop_range.is_some_and(|(_, end)| frame.position + 1 == end) {
                    let complete = loop_range.is_some_and(|(start, end)| {
                        open.first()
                            .is_some_and(|take| take.start == start && take.frames == end - start)
                    });
                    finish_takes(&mut open, &mut result, sample_rate, tempo, complete);
                    ensure!(
                        result.error.is_none(),
                        "Recording file could not be finalized; completed takes were retained"
                    );
                    pass += 1;
                    shared.passes.store(pass, Ordering::Release);
                }
            }
            if shared.stop.load(Ordering::Acquire) {
                let stopping_since = stopping_since.get_or_insert_with(Instant::now);
                let tail_ready = shared.tail_complete.load(Ordering::Acquire)
                    || shared.record_anchor_time.load(Ordering::Acquire) == 0
                    || shared.fault.load(Ordering::Acquire) != 0;
                ensure!(
                    tail_ready || stopping_since.elapsed() < Duration::from_secs(2),
                    "Input did not deliver the recording tail within two seconds; available audio was recovered"
                );
                if !tail_ready {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                shared.phase.store(FINALIZING, Ordering::SeqCst);
                // A callback which observed ACTIVE must finish its bounded push
                // before the queue can be declared drained.
                if !shared.consumer_busy.load(Ordering::SeqCst) && shared.recording.len() == 0 {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    })();
    shared.phase.store(FINALIZING, Ordering::SeqCst);
    shared.stop.store(true, Ordering::Release);
    // Error exits close the producer too: an in-flight callback must not append
    // an old-session frame after this result is reaped and a new session begins.
    while shared.consumer_busy.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    if let Err(error) = outcome {
        result
            .error
            .get_or_insert_with(|| format!("Record audio: {error:#}"));
    }
    while shared.fault.load(Ordering::Acquire) == FAULT_WRITING {
        thread::yield_now();
    }
    if let Some(error) = shared.fault_description() {
        match &mut result.error {
            Some(existing) => {
                existing.push('\n');
                existing.push_str(&error);
            }
            None => result.error = Some(error),
        }
    }
    let complete = loop_range.is_none() && result.error.is_none();
    finish_takes(&mut open, &mut result, sample_rate, tempo, complete);
    result.dropped_frames = shared.dropped.load(Ordering::Relaxed);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceSettings;
    use daw_core::AudioData;

    fn fixture(loop_range: Option<(f64, f64)>) -> (AudioEngine, Project, u64) {
        let mut project = Project::new();
        project.tempo = 120.0;
        let track = project.add_audio_track("Recorded audio");
        project.track_mut(track).unwrap().gain = 1.0;
        if let Some((start, end)) = loop_range {
            project.loop_enabled = true;
            project.loop_start = start;
            project.loop_end = end;
        }
        let mut engine = AudioEngine::start_virtual_cancellable(
            &project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
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
        (engine, project, track)
    }
    fn active(engine: &AudioEngine) {
        let end = Instant::now() + Duration::from_secs(5);
        while !engine.recording_snapshot().active {
            assert!(
                Instant::now() < end,
                "recording never activated: {:?}",
                engine.recording_snapshot()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn finish(engine: &mut AudioEngine) -> RecordingResult {
        engine.request_stop_recording();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = engine.take_recording_result() {
                return result;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn stereo(value: f32, frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|_| [value, -value * 0.5]).collect()
    }
    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn recording_uses_consumed_clock_and_exact_loop_files_including_partial() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, _, track) = fixture(Some((0.0, 1.0)));
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        engine.pump_virtual_input(&stereo(0.1, 24000), 2).unwrap();
        engine.pump_virtual_input(&stereo(0.2, 24000), 2).unwrap();
        engine.pump_virtual_input(&stereo(0.3, 12000), 2).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.dropped_frames, 0);
        assert_eq!(result.takes.len(), 3);
        for (take, (frames, value, complete)) in
            result
                .takes
                .iter()
                .zip([(24000, 0.1, true), (24000, 0.2, true), (12000, 0.3, false)])
        {
            assert_eq!(take.frames, frames);
            assert_eq!(take.start_beat, 0.0);
            assert_eq!(take.complete, complete);
            let data = AudioData::load_wav(&take.path).unwrap();
            assert_eq!(data.channels, 1);
            assert_eq!(data.frames() as u64, frames);
            assert!(data.samples.iter().all(|sample| *sample == value));
        }
        assert!(
            engine.snapshot().playing,
            "Stopping capture must keep backing playback running"
        );
    }

    #[test]
    fn blank_project_records_past_end_and_raw_channels_ignore_monitor_fader() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, mut project, track) = fixture(None);
        project.track_mut(track).unwrap().gain = 0.25;
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: None,
                    routes: vec![InputRoute {
                        track,
                        channels: AudioInputChannels::Mono { channel: 1 },
                        monitor: true,
                    }],
                },
            )
            .unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        let output = engine.pump_virtual_input(&stereo(0.4, 48000), 2).unwrap();
        for frame in &output[256..] {
            near(frame[0], -0.05);
            near(frame[1], -0.05);
        }
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.takes.len(), 1);
        assert_eq!(result.takes[0].frames, 48000);
        assert!(result.takes[0].complete);
        let data = AudioData::load_wav(&result.takes[0].path).unwrap();
        assert!(data.samples.iter().all(|sample| *sample == -0.2));
        assert!(engine.position_beats() >= 2.0 - 1e-8);
    }

    #[test]
    fn dry_monitor_obeys_live_channel_bus_pan_mute_solo_and_smoothing() {
        let (mut engine, mut project, track) = fixture(None);
        let bus = project.add_bus("Bus");
        project.set_output(track, Some(bus)).unwrap();
        project.track_mut(track).unwrap().gain = 0.5;
        project.track_mut(track).unwrap().pan = -0.5;
        project.track_mut(bus).unwrap().gain = 0.5;
        project.track_mut(bus).unwrap().pan = 0.25;
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
        let output = engine.pump_virtual_input(&stereo(0.2, 512), 2).unwrap();
        near(output[511][0], 0.0375);
        near(output[511][1], 0.025);
        assert!(output[0][0] > 0.0 && output[0][0] < 0.001);
        project.track_mut(bus).unwrap().mute = true;
        // The input mix follows even a structural project update rejected by
        // the playback graph; normal same-graph faders use this same path.
        let _ = engine.try_update_project(&project, 1).unwrap();
        let output = engine.pump_virtual_input(&stereo(0.2, 512), 2).unwrap();
        near(output[511][0], 0.0);
        near(output[511][1], 0.0);
        project.track_mut(bus).unwrap().mute = false;
        let other = project.add_audio_track("Solo other");
        project.track_mut(other).unwrap().solo = true;
        engine.input.sync_mix(&project).unwrap();
        let output = engine.pump_virtual_input(&stereo(0.2, 512), 2).unwrap();
        near(output[511][0], 0.0);
        project.track_mut(other).unwrap().solo = false;
        project.track_mut(bus).unwrap().solo = true;
        engine.input.sync_mix(&project).unwrap();
        let output = engine.pump_virtual_input(&stereo(0.2, 512), 2).unwrap();
        near(output[511][0], 0.0375);
    }

    #[test]
    fn two_armed_tracks_stay_sample_aligned_and_stereo_order_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, mut project, first) = fixture(None);
        let second = project.add_audio_track("Stereo reversed");
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: None,
                    routes: vec![
                        InputRoute {
                            track: first,
                            channels: AudioInputChannels::Mono { channel: 0 },
                            monitor: false,
                        },
                        InputRoute {
                            track: second,
                            channels: AudioInputChannels::Stereo { left: 1, right: 0 },
                            monitor: false,
                        },
                    ],
                },
            )
            .unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![first, second],
            })
            .unwrap();
        active(&engine);
        engine.pump_virtual_input(&stereo(0.4, 12345), 2).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.takes.len(), 2);
        for take in &result.takes {
            assert_eq!(take.frames, 12345);
            assert_eq!(take.start_beat, 0.0);
        }
        let stereo = AudioData::load_wav(&result.takes[1].path).unwrap();
        assert_eq!(stereo.channels, 2);
        assert!(
            stereo
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| *frame == [-0.2, 0.4])
        );
    }

    #[test]
    fn disconnect_recovers_recorded_prefix_and_input_can_be_reopened() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, project, track) = fixture(None);
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        engine.pump_virtual_input(&stereo(0.25, 1024), 2).unwrap();
        engine.fail_virtual_input().unwrap();
        let result = finish(&mut engine);
        assert!(result.error.as_deref().unwrap().contains("disconnected"));
        assert_eq!(result.takes[0].frames, 1024);
        assert_eq!(
            AudioData::load_wav(&result.takes[0].path).unwrap().frames(),
            1024
        );
        assert!(engine.take_input_error().unwrap().contains("disconnected"));
        assert!(engine.take_input_error().is_none());
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
        assert!(engine.recording_snapshot().input_ready);
    }

    #[test]
    fn callback_capture_monitor_and_overflow_paths_allocate_nothing() {
        let shared = InputShared::new();
        shared.ready.store(true, Ordering::Release);
        shared.channels.store(2, Ordering::Release);
        shared.phase.store(ACTIVE, Ordering::Release);
        let samples = [0.25_f32; 512];
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                shared.feed_at(&samples, 2, 0, CLOCK_ORIGIN_NS);
                for position in 0..256 {
                    shared.consume(
                        Some(position),
                        48000,
                        CLOCK_ORIGIN_NS + position * 1_000_000_000 / 48000,
                    );
                }
            }),
            0
        );
        for position in 256..RECORD_CAPACITY + 1 {
            shared.feed_at(
                &[0.1_f32, 0.2],
                2,
                0,
                CLOCK_ORIGIN_NS + position as u64 * 1_000_000_000 / 48000,
            );
            shared.consume(
                Some(position as u64),
                48000,
                CLOCK_ORIGIN_NS + position as u64 * 1_000_000_000 / 48000,
            );
        }
        assert!(shared.stop.load(Ordering::Acquire));
        assert_eq!(shared.fault.load(Ordering::Acquire), 2);
        assert_eq!(shared.recording.len(), RECORD_CAPACITY);
        assert_eq!(shared.recording.pop().unwrap().samples[0], 0.25);
    }

    #[test]
    fn partial_first_loop_pass_is_not_marked_complete_and_capture_retains_seek_offset() {
        let directory = tempfile::tempdir().unwrap();
        let (release, gate) = mpsc::channel();
        super::super::RENDER_READY_TEST_GATE.with(|slot| *slot.borrow_mut() = Some(gate));
        let (mut engine, _, track) = fixture(Some((0.0, 1.0)));
        // Seek after readiness is published but before the worker's first loop.
        // This must remain pending even if the render thread is descheduled.
        engine.seek(0.5);
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        release.send(()).unwrap();
        engine.pump_virtual_input(&stereo(0.1, 12000), 2).unwrap();
        engine.pump_virtual_input(&stereo(0.2, 24000), 2).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.takes.len(), 2);
        assert_eq!(result.takes[0].start_beat, 0.5);
        assert_eq!(result.takes[0].frames, 12000);
        assert!(!result.takes[0].complete);
        assert_eq!(result.takes[1].start_beat, 0.0);
        assert_eq!(result.takes[1].frames, 24000);
        assert!(result.takes[1].complete);
    }

    #[test]
    fn unavailable_input_and_unwritable_directory_fail_without_starting_capture() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, project, track) = fixture(None);
        assert!(
            engine
                .configure_input(
                    &project,
                    InputSettings {
                        device_name: Some("Missing".into()),
                        routes: vec![InputRoute {
                            track,
                            channels: AudioInputChannels::Mono { channel: 0 },
                            monitor: false
                        }]
                    }
                )
                .is_err()
        );
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
        let path = directory.path().join("file");
        fs::write(&path, b"not a directory").unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: path,
                tracks: vec![track],
            })
            .unwrap();
        let result = finish(&mut engine);
        assert!(result.error.as_deref().unwrap().contains("directory"));
        assert!(result.takes.is_empty());
        assert!(!engine.snapshot().playing);
    }
    #[test]
    fn virtual_capture_compensates_input_and_output_latency_before_loop_splitting() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, _, track) = fixture(Some((0.0, 0.01)));
        engine
            .configure_virtual_io(VirtualIoSettings {
                input_channels: 2,
                input_latency_frames: 5,
                output_latency_frames: 7,
            })
            .unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        let samples: Vec<f32> = stereo(-0.9, 12)
            .into_iter()
            .chain(stereo(0.1, 240))
            .chain(stereo(0.2, 240))
            .chain(stereo(0.3, 120))
            .collect();
        engine.pump_virtual_input(&samples[..600 * 2], 2).unwrap();
        assert_eq!(engine.recording_snapshot().frames, 588);
        engine.request_stop_recording();
        assert!(
            engine.take_recording_result().is_none(),
            "The scheduled playback tail still awaits input"
        );
        engine.pump_virtual_input(&samples[600 * 2..], 2).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.takes.len(), 3);
        for (take, (frames, value, complete)) in
            result
                .takes
                .iter()
                .zip([(240, 0.1, true), (240, 0.2, true), (120, 0.3, false)])
        {
            assert_eq!(take.frames, frames);
            assert_eq!(take.start_beat, 0.0);
            assert_eq!(take.complete, complete);
            assert!(
                AudioData::load_wav(&take.path)
                    .unwrap()
                    .samples
                    .iter()
                    .all(|sample| *sample == value)
            );
        }
    }

    #[test]
    fn independent_dummy_feeds_use_one_contiguous_clock_and_configurable_channels() {
        let directory = tempfile::tempdir().unwrap();
        let (mut engine, project, track) = fixture(None);
        engine
            .configure_virtual_io(VirtualIoSettings {
                input_channels: 4,
                ..VirtualIoSettings::default()
            })
            .unwrap();
        engine
            .configure_input(
                &project,
                InputSettings {
                    device_name: Some("Rill virtual input".into()),
                    routes: vec![InputRoute {
                        track,
                        channels: AudioInputChannels::Mono { channel: 3 },
                        monitor: false,
                    }],
                },
            )
            .unwrap();
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![track],
            })
            .unwrap();
        active(&engine);
        engine
            .feed_virtual_input(&[0.0, 0.1, 0.2, 0.3].repeat(333), 4)
            .unwrap();
        engine
            .feed_virtual_input(&[0.0, 0.1, 0.2, 0.4].repeat(517), 4)
            .unwrap();
        assert_eq!(engine.recording_snapshot().frames, 0);
        assert_eq!(
            engine
                .controls
                .input
                .input_clock_frames
                .load(Ordering::Relaxed),
            850
        );
        engine.pump_virtual(850).unwrap();
        let result = finish(&mut engine);
        assert!(result.error.is_none(), "{:?}", result.error);
        let samples = AudioData::load_wav(&result.takes[0].path).unwrap().samples;
        assert_eq!(samples.len(), 850);
        assert!(samples[..333].iter().all(|x| *x == 0.3));
        assert!(samples[333..].iter().all(|x| *x == 0.4));
    }

    #[test]
    fn output_disconnect_and_input_overflow_recover_playable_prefixes() {
        for output_failure in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut engine, _, track) = fixture(None);
            engine
                .start_recording(RecordingConfig {
                    directory: directory.path().into(),
                    tracks: vec![track],
                })
                .unwrap();
            active(&engine);
            engine.pump_virtual_input(&stereo(0.2, 512), 2).unwrap();
            if output_failure {
                engine.fail_virtual_output().unwrap();
            } else {
                engine
                    .feed_virtual_input(&stereo(0.3, INPUT_CAPACITY + 32), 2)
                    .unwrap();
            }
            let result = finish(&mut engine);
            assert!(
                result
                    .error
                    .as_deref()
                    .unwrap()
                    .contains(if output_failure {
                        "output stream"
                    } else {
                        "overflowed"
                    })
            );
            assert_eq!(result.takes[0].frames, 512);
            assert!(
                AudioData::load_wav(&result.takes[0].path)
                    .unwrap()
                    .samples
                    .iter()
                    .all(|sample| *sample == 0.2)
            );
        }
    }

    #[test]
    fn timestamp_rounding_after_nonzero_playback_does_not_discard_first_sample() {
        let shared = InputShared::new();
        shared.sample_rate.store(44100, Ordering::Relaxed);
        shared.ready.store(true, Ordering::Release);
        shared.channels.store(2, Ordering::Release);
        shared.phase.store(ACTIVE, Ordering::Release);
        // Split timestamp arithmetic can put an exactly aligned sample 1ns
        // before the output anchor. It must still map to the same audio frame.
        let anchor = CLOCK_ORIGIN_NS + 5_283_832;
        shared.feed_at(&[0.1, 0.2, 0.3, 0.4], 2, 0, anchor - 1);
        shared.consume(Some(233), 44100, anchor);
        shared.consume(Some(234), 44100, anchor + 1_000_000_000 / 44100);
        let first = shared.recording.pop().unwrap();
        let second = shared.recording.pop().unwrap();
        assert_eq!(first.position, 233);
        assert_eq!(first.samples[0], 0.1);
        assert_eq!(second.position, 234);
        shared.request_stop();
        assert!(shared.tail_complete.load(Ordering::Acquire));
    }
    #[test]
    fn cpal_advisories_are_not_fatal_and_backend_errors_retain_unicode_details_without_allocating()
    {
        let shared = InputShared::new();
        shared.ready.store(true, Ordering::Release);
        let scheduling = cpal::Error::new(cpal::ErrorKind::RealtimeDenied);
        let route = cpal::Error::new(cpal::ErrorKind::DeviceChanged);
        shared.input_failed(0, &scheduling);
        shared.input_failed(0, &route);
        assert!(shared.ready.load(Ordering::Acquire));
        assert_eq!(shared.fault.load(Ordering::Acquire), 0);
        shared.phase.store(ACTIVE, Ordering::Release);
        let failure = cpal::Error::with_message(
            cpal::ErrorKind::DeviceNotAvailable,
            "Input Ω disconnected: driver code 42 🎙️",
        );
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| shared.input_failed(0, &failure)),
            0
        );
        assert!(!shared.ready.load(Ordering::Acquire));
        assert!(shared.stop.load(Ordering::Acquire));
        let description = shared.fault_description().unwrap();
        assert!(description.contains("Input Ω disconnected: driver code 42 🎙️"));
        let shared = InputShared::new();
        shared.phase.store(ACTIVE, Ordering::Release);
        shared.input_failed(0, &route);
        assert_eq!(shared.fault.load(Ordering::Acquire), 8);
        assert!(
            shared
                .fault_description()
                .unwrap()
                .contains("changed during recording")
        );
        let shared = InputShared::new();
        let long = cpal::Error::with_message(cpal::ErrorKind::BackendError, "音🎛".repeat(1000));
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                shared.device_failed(6, &long, None);
            }),
            0
        );
        let description = shared.fault_description().unwrap();
        assert!(description.len() < 1200);
        assert!(!description.contains('�'));
    }
    #[test]
    fn cancelling_native_preparation_is_quiet_and_needs_no_hardware() {
        let directory = tempfile::tempdir().unwrap();
        let controls = Controls::new();
        controls.input.stop.store(true, Ordering::Release);
        let result = record_to_disk(
            &controls,
            RecordingConfig {
                directory: directory.path().into(),
                tracks: vec![1],
            },
            vec![InputRoute {
                track: 1,
                channels: AudioInputChannels::Mono { channel: 0 },
                monitor: false,
            }],
            48000,
            256,
            false,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.takes.is_empty());
    }
}
