//! On-demand channel analysis. Only the render coordinator copies selected
//! samples; filtering, loudness windows and FFTs live on an independent worker.
use anyhow::{Context, Result};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

const FFT_SIZE: usize = 4096;
const PHASE_POINTS: usize = 256;
const GATE_BINS: usize = 87_001; // -70 to +800 LUFS, including any finite f32 input.

#[path = "signal_true_peak.rs"]
mod true_peak;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SignalMonitorTarget {
    Channel(u64),
    /// Final master processing, before device monitoring, click and clipping.
    Master,
}

#[derive(Clone, Debug, Serialize)]
pub struct SignalMonitorSnapshot {
    pub target: SignalMonitorTarget,
    pub sample_rate: u32,
    pub fft_size: usize,
    /// Single-sided stereo-power amplitude, bin frequency = index * rate / size.
    pub spectrum_db: Vec<f32>,
    pub lufs_momentary: Option<f32>,
    pub lufs_short_term: Option<f32>,
    /// Approximate BS.1770 stereo gating with a bounded 0.01 LU gate histogram.
    /// Gate quantization may affect populations near the relative threshold.
    pub lufs_integrated: Option<f32>,
    /// Peak and RMS over the most recent FFT window, not true-peak estimates.
    pub peak_dbfs: Option<f32>,
    pub rms_dbfs: Option<f32>,
    /// Exponentially averaged RMS, with a 300 ms mean-square time constant.
    pub rms_300ms_dbfs: Option<f32>,
    pub channel_rms_300ms_dbfs: [Option<f32>; 2],
    pub channel_peak_dbfs: [Option<f32>; 2],
    pub sample_peak_hold_dbfs: Option<f32>,
    /// Streaming FIR inter-sample peak estimates. The analyzer oversamples to
    /// at least 192 kHz; this is not a certified conformance measurement.
    pub true_peak_dbfs: Option<f32>,
    pub true_peak_hold_dbfs: Option<f32>,
    pub true_peak_oversampling: u8,
    /// Individual input samples whose absolute value exceeded full scale.
    pub overload_samples: [u64; 2],
    pub correlation: Option<f32>,
    pub phase: Vec<[f32; 2]>,
    pub observed_frames: u64,
    pub generation: u64,
    pub dropped_blocks: u64,
    #[serde(skip)]
    epoch: u64,
}

#[derive(Clone, Copy, Default)]
struct Request {
    target: Option<SignalMonitorTarget>,
    epoch: u64,
}

struct Block {
    request: Request,
    transport: u64,
    dropped: u64,
    count: usize,
    frames: [[f32; 2]; crate::render::BLOCK],
}

pub(crate) struct SignalMonitor {
    sample_rate: u32,
    enabled: AtomicBool,
    request: Mutex<Request>,
    epoch: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    sender: OnceLock<mpsc::SyncSender<Block>>,
    latest: Arc<Mutex<Option<Arc<SignalMonitorSnapshot>>>>,
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SignalMonitor {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            enabled: AtomicBool::new(false),
            request: Mutex::new(Request::default()),
            epoch: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            sender: OnceLock::new(),
            latest: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
        }
    }

    pub(crate) fn set_target(&self, target: Option<SignalMonitorTarget>) -> Result<()> {
        if target.is_some() && self.sender.get().is_none() {
            let mut worker = self
                .worker
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if self.sender.get().is_none() {
                let (sender, receiver) = mpsc::sync_channel::<Block>(32);
                let latest = Arc::clone(&self.latest);
                let shutdown = Arc::clone(&self.shutdown);
                let epoch = Arc::clone(&self.epoch);
                let dropped = Arc::clone(&self.dropped);
                let sample_rate = self.sample_rate;
                *worker = Some(
                    thread::Builder::new()
                        .name("rill-signal-analysis".into())
                        .spawn(move || {
                            let mut analysis = Analysis::new(sample_rate);
                            let mut identity = None;
                            let mut generation = 0;
                            while !shutdown.load(Ordering::Acquire) {
                                let block = match receiver.recv() {
                                    Ok(block) => block,
                                    Err(_) => break,
                                };
                                if block.request.epoch != epoch.load(Ordering::Acquire)
                                    || block.dropped != dropped.load(Ordering::Acquire)
                                {
                                    continue;
                                }
                                let key = (block.request.epoch, block.transport, block.dropped);
                                if identity != Some(key) {
                                    analysis.reset();
                                    identity = Some(key);
                                    generation += 1;
                                }
                                if analysis.push(&block.frames[..block.count]) {
                                    let mut snapshot = analysis
                                        .snapshot(block.request.target.expect("enabled monitor"));
                                    snapshot.generation = generation;
                                    snapshot.dropped_blocks = block.dropped;
                                    snapshot.epoch = block.request.epoch;
                                    if block.request.epoch == epoch.load(Ordering::Acquire) {
                                        *latest.lock().unwrap_or_else(|error| error.into_inner()) =
                                            Some(Arc::new(snapshot));
                                    }
                                }
                            }
                        })
                        .context("Starting signal-analysis worker")?,
                );
                let _ = self.sender.set(sender);
            }
        }
        let mut request = self
            .request
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if request.target != target {
            self.enabled.store(false, Ordering::Release);
            request.target = target;
            request.epoch = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
            self.dropped.store(0, Ordering::Relaxed);
            self.enabled.store(target.is_some(), Ordering::Release);
        }
        Ok(())
    }

    pub(crate) fn reset(&self) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        let mut request = self
            .request
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        request.epoch = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        self.dropped.store(0, Ordering::Relaxed);
    }

    /// One atomic load when disabled. The coordinator never waits for the UI.
    pub(crate) fn target(&self) -> Option<(SignalMonitorTarget, u64)> {
        if !self.enabled.load(Ordering::Acquire) {
            return None;
        }
        let request = self.request.try_lock().ok()?;
        request.target.map(|target| (target, request.epoch))
    }

    pub(crate) fn capture(
        &self,
        target: SignalMonitorTarget,
        epoch: u64,
        transport: u64,
        frames: &[[f32; 2]],
    ) {
        let Some(sender) = self.sender.get() else {
            return;
        };
        debug_assert!(frames.len() <= crate::render::BLOCK);
        let mut block = Block {
            request: Request {
                target: Some(target),
                epoch,
            },
            transport,
            dropped: self.dropped.load(Ordering::Relaxed),
            count: frames.len(),
            frames: [[0.0; 2]; crate::render::BLOCK],
        };
        block.frames[..frames.len()].copy_from_slice(frames);
        if sender.try_send(block).is_err() {
            // A gap must reset loudness timing, never masquerade as continuous audio.
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Arc<SignalMonitorSnapshot>> {
        if !self.enabled.load(Ordering::Acquire) {
            return None;
        }
        let snapshot = self.latest.try_lock().ok()?.clone()?;
        (snapshot.epoch == self.epoch.load(Ordering::Acquire)
            && snapshot.dropped_blocks == self.dropped.load(Ordering::Acquire))
        .then_some(snapshot)
    }
}

impl Drop for SignalMonitor {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.sender.take(); // Wake a sleeping receiver before joining its worker.
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    state: [f64; 2],
}
impl Biquad {
    fn process(&mut self, value: f64) -> f64 {
        let output = value * self.b[0] + self.state[0];
        self.state[0] = value * self.b[1] - output * self.a[0] + self.state[1];
        self.state[1] = value * self.b[2] - output * self.a[1];
        output
    }
}

fn k_weighting(rate: u32) -> [Biquad; 2] {
    let k = (std::f64::consts::PI * 1681.974450955533 / f64::from(rate)).tan();
    let q = 0.7071752369554196;
    let vh = 10.0_f64.powf(3.999843853973347 / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let denominator = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / denominator,
            2.0 * (k * k - vh) / denominator,
            (vh - vb * k / q + k * k) / denominator,
        ],
        a: [
            2.0 * (k * k - 1.0) / denominator,
            (1.0 - k / q + k * k) / denominator,
        ],
        ..Default::default()
    };
    let k = (std::f64::consts::PI * 38.13547087602444 / f64::from(rate)).tan();
    let q = 0.5003270373238773;
    let denominator = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        // BS.1770 RLB numerator is deliberately not divided by denominator.
        b: [1.0, -2.0, 1.0],
        a: [
            2.0 * (k * k - 1.0) / denominator,
            (1.0 - k / q + k * k) / denominator,
        ],
        ..Default::default()
    };
    [shelf, high_pass]
}

#[derive(Clone, Copy, Default)]
struct GateBin {
    energy: f64,
    count: u64,
}

struct Analysis {
    rate: u32,
    filters: [[Biquad; 2]; 2],
    energy: Vec<f64>,
    energy_cursor: usize,
    momentary_frames: usize,
    momentary_sum: f64,
    short_sum: f64,
    observed: u64,
    next_publish: u64,
    hop: u64,
    gates: Vec<GateBin>,
    absolute_energy: f64,
    absolute_count: u64,
    recent: Vec<[f32; 2]>,
    recent_true_peaks: Vec<[f64; 2]>,
    recent_cursor: usize,
    true_peak: true_peak::TruePeak,
    sample_peak_hold: [f64; 2],
    true_peak_hold: [f64; 2],
    overload_samples: [u64; 2],
    rms_energy: [f64; 2],
    rms_decay: f64,
    window: Vec<f64>,
    fft: Arc<dyn Fft<f64>>,
    fft_input: Vec<Complex<f64>>,
    fft_scratch: Vec<Complex<f64>>,
}

impl Analysis {
    fn new(rate: u32) -> Self {
        let rate = rate.clamp(8_000, 192_000);
        let fft = FftPlanner::<f64>::new().plan_fft_forward(FFT_SIZE);
        let hop = (u64::from(rate) / 10).max(1);
        Self {
            rate,
            filters: [k_weighting(rate); 2],
            energy: vec![0.0; rate as usize * 3],
            energy_cursor: 0,
            momentary_frames: (f64::from(rate) * 0.4).round() as usize,
            momentary_sum: 0.0,
            short_sum: 0.0,
            observed: 0,
            next_publish: hop,
            hop,
            gates: vec![GateBin::default(); GATE_BINS],
            absolute_energy: 0.0,
            absolute_count: 0,
            recent: vec![[0.0; 2]; FFT_SIZE],
            recent_true_peaks: vec![[0.0; 2]; FFT_SIZE],
            recent_cursor: 0,
            true_peak: true_peak::TruePeak::new(rate),
            sample_peak_hold: [0.0; 2],
            true_peak_hold: [0.0; 2],
            overload_samples: [0; 2],
            rms_energy: [0.0; 2],
            rms_decay: (-1.0 / (f64::from(rate) * 0.3)).exp(),
            window: (0..FFT_SIZE)
                .map(|index| {
                    0.5 - 0.5 * (2.0 * std::f64::consts::PI * index as f64 / FFT_SIZE as f64).cos()
                })
                .collect(),
            fft_scratch: vec![Complex::default(); fft.get_inplace_scratch_len()],
            fft,
            fft_input: vec![Complex::default(); FFT_SIZE],
        }
    }

    fn reset(&mut self) {
        self.filters = [k_weighting(self.rate); 2];
        self.energy.fill(0.0);
        self.energy_cursor = 0;
        self.momentary_sum = 0.0;
        self.short_sum = 0.0;
        self.observed = 0;
        self.next_publish = self.hop;
        self.gates.fill(GateBin::default());
        self.absolute_energy = 0.0;
        self.absolute_count = 0;
        self.recent.fill([0.0; 2]);
        self.recent_true_peaks.fill([0.0; 2]);
        self.recent_cursor = 0;
        self.true_peak.reset();
        self.sample_peak_hold = [0.0; 2];
        self.true_peak_hold = [0.0; 2];
        self.overload_samples = [0; 2];
        self.rms_energy = [0.0; 2];
    }

    fn push(&mut self, frames: &[[f32; 2]]) -> bool {
        let mut publish = false;
        for frame in frames {
            let frame = frame.map(|value| if value.is_finite() { value } else { 0.0 });
            let true_peaks = self.true_peak.process(frame);
            self.recent[self.recent_cursor] = frame;
            self.recent_true_peaks[self.recent_cursor] = true_peaks;
            self.recent_cursor = (self.recent_cursor + 1) % FFT_SIZE;
            let mut energy = 0.0;
            for (channel, sample) in frame.into_iter().enumerate() {
                let raw = f64::from(sample);
                self.sample_peak_hold[channel] = self.sample_peak_hold[channel].max(raw.abs());
                self.true_peak_hold[channel] =
                    self.true_peak_hold[channel].max(true_peaks[channel]);
                if sample.abs() > 1.0 {
                    self.overload_samples[channel] =
                        self.overload_samples[channel].saturating_add(1);
                }
                self.rms_energy[channel] =
                    self.rms_decay * self.rms_energy[channel] + (1.0 - self.rms_decay) * raw * raw;
                let weighted = self.filters[channel][0].process(f64::from(sample));
                let weighted = self.filters[channel][1].process(weighted);
                energy += weighted * weighted;
            }
            let old_momentary = if self.observed >= self.momentary_frames as u64 {
                self.energy[(self.energy_cursor + self.energy.len() - self.momentary_frames)
                    % self.energy.len()]
            } else {
                0.0
            };
            self.momentary_sum += energy - old_momentary;
            self.short_sum += energy - self.energy[self.energy_cursor];
            self.energy[self.energy_cursor] = energy;
            self.energy_cursor = (self.energy_cursor + 1) % self.energy.len();
            self.observed += 1;
            if self.observed >= self.next_publish {
                self.next_publish += self.hop;
                publish = true;
                // Rebase positive window energies at 10 Hz on this analysis
                // worker. Subtracting a very hot plug-in transient from rolling
                // floating sums can otherwise permanently erase quiet audio or
                // retain cancellation residue after the transient has left.
                self.momentary_sum = self.window_energy(self.momentary_frames);
                if self.observed >= self.momentary_frames as u64 {
                    let energy = self.momentary_sum.max(0.0) / self.momentary_frames as f64;
                    if let Some(loudness) = lufs(energy)
                        && loudness > -70.0
                    {
                        let index = (((f64::from(loudness) + 70.0) * 100.0).floor() as usize)
                            .min(GATE_BINS - 1);
                        self.gates[index].energy += energy;
                        self.gates[index].count += 1;
                        self.absolute_energy += energy;
                        self.absolute_count += 1;
                    }
                }
            }
        }
        publish
    }

    fn window_energy(&self, frames: usize) -> f64 {
        let count = (frames as u64).min(self.observed) as usize;
        let count = count.min(self.energy.len());
        if count == 0 {
            return 0.0;
        }
        let start = (self.energy_cursor + self.energy.len() - count) % self.energy.len();
        if start < self.energy_cursor {
            self.energy[start..self.energy_cursor].iter().sum()
        } else {
            self.energy[start..]
                .iter()
                .chain(&self.energy[..self.energy_cursor])
                .sum()
        }
    }

    fn snapshot(&mut self, target: SignalMonitorTarget) -> SignalMonitorSnapshot {
        self.momentary_sum = self.window_energy(self.momentary_frames);
        self.short_sum = self.energy.iter().sum();
        let count = self.observed.min(FFT_SIZE as u64) as usize;
        let mut left_energy = 0.0;
        let mut right_energy = 0.0;
        let mut product = 0.0;
        let mut peak = 0.0_f64;
        let mut channel_peaks = [0.0_f64; 2];
        let mut true_peaks = [0.0_f64; 2];
        let mut phase = Vec::with_capacity(PHASE_POINTS);
        for index in 0..count {
            let cursor = (self.recent_cursor + FFT_SIZE - count + index) % FFT_SIZE;
            let frame = self.recent[cursor];
            let [left, right] = frame.map(f64::from);
            left_energy += left * left;
            right_energy += right * right;
            product += left * right;
            peak = peak.max(left.abs()).max(right.abs());
            channel_peaks[0] = channel_peaks[0].max(left.abs());
            channel_peaks[1] = channel_peaks[1].max(right.abs());
            for (channel, value) in self.recent_true_peaks[cursor].iter().enumerate() {
                true_peaks[channel] = true_peaks[channel].max(*value);
            }
        }
        // One real sample from each time bucket, with deterministic within-
        // bucket dither. A fixed stride of 16 aliases a 1 kHz / 48 kHz tone to
        // only three phase positions, making a stereo ellipse look triangular.
        let points = count.min(PHASE_POINTS);
        for point in 0..points {
            let start = point * count / points;
            let end = (point + 1) * count / points;
            let mixed = (point as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let offset = (mixed ^ (mixed >> 32)) as usize % (end - start).max(1);
            let index = start + offset;
            phase.push(self.recent[(self.recent_cursor + FFT_SIZE - count + index) % FFT_SIZE]);
        }
        let denominator = (left_energy * right_energy).sqrt();
        let correlation =
            (denominator > 1e-20).then(|| (product / denominator).clamp(-1.0, 1.0) as f32);
        let spectrum_db = if self.observed >= FFT_SIZE as u64 {
            let mut powers = vec![0.0_f64; FFT_SIZE / 2 + 1];
            for channel in 0..2 {
                for index in 0..FFT_SIZE {
                    self.fft_input[index] = Complex::new(
                        f64::from(self.recent[(self.recent_cursor + index) % FFT_SIZE][channel])
                            * self.window[index],
                        0.0,
                    );
                }
                self.fft
                    .process_with_scratch(&mut self.fft_input, &mut self.fft_scratch);
                for (power, value) in powers.iter_mut().zip(&self.fft_input) {
                    *power += value.norm_sqr() * 0.5;
                }
            }
            powers
                .into_iter()
                .enumerate()
                .map(|(index, power)| {
                    let scale = if index == 0 || index == FFT_SIZE / 2 {
                        2.0
                    } else {
                        4.0
                    } / FFT_SIZE as f64;
                    (10.0 * (power * scale * scale).max(1e-16).log10()).clamp(-160.0, 800.0) as f32
                })
                .collect()
        } else {
            Vec::new()
        };
        let relative_gate = if self.absolute_count > 0 {
            lufs(self.absolute_energy / self.absolute_count as f64).map(|value| value - 10.0)
        } else {
            None
        };
        let integrated = relative_gate.and_then(|gate| {
            let mut energy = 0.0;
            let mut count = 0;
            for (index, bin) in self.gates.iter().enumerate() {
                if index as f32 * 0.01 - 70.0 >= gate {
                    energy += bin.energy;
                    count += bin.count;
                }
            }
            (count > 0).then(|| lufs(energy / count as f64)).flatten()
        });
        SignalMonitorSnapshot {
            target,
            sample_rate: self.rate,
            fft_size: FFT_SIZE,
            spectrum_db,
            lufs_momentary: (self.observed >= self.momentary_frames as u64)
                .then(|| lufs(self.momentary_sum.max(0.0) / self.momentary_frames as f64))
                .flatten(),
            lufs_short_term: (self.observed >= self.energy.len() as u64)
                .then(|| lufs(self.short_sum.max(0.0) / self.energy.len() as f64))
                .flatten(),
            lufs_integrated: integrated,
            peak_dbfs: dbfs(peak),
            rms_dbfs: (count > 0)
                .then(|| dbfs(((left_energy + right_energy) / (2.0 * count as f64)).sqrt()))
                .flatten(),
            rms_300ms_dbfs: dbfs(((self.rms_energy[0] + self.rms_energy[1]) * 0.5).sqrt()),
            channel_rms_300ms_dbfs: self.rms_energy.map(|energy| dbfs(energy.sqrt())),
            channel_peak_dbfs: channel_peaks.map(dbfs),
            sample_peak_hold_dbfs: dbfs(self.sample_peak_hold[0].max(self.sample_peak_hold[1])),
            true_peak_dbfs: dbfs(true_peaks[0].max(true_peaks[1])),
            true_peak_hold_dbfs: dbfs(self.true_peak_hold[0].max(self.true_peak_hold[1])),
            true_peak_oversampling: self.true_peak.factor(),
            overload_samples: self.overload_samples,
            correlation,
            phase,
            observed_frames: self.observed,
            generation: 0,
            dropped_blocks: 0,
            epoch: 0,
        }
    }
}

fn lufs(energy: f64) -> Option<f32> {
    (energy.is_finite() && energy > 1e-30).then(|| (-0.691 + 10.0 * energy.log10()) as f32)
}
fn dbfs(amplitude: f64) -> Option<f32> {
    (amplitude.is_finite() && amplitude > 1e-15).then(|| (20.0 * amplitude.log10()) as f32)
}

#[cfg(test)]
#[path = "signal_monitor_reference_tests.rs"]
mod reference_tests;

#[cfg(test)]
#[path = "signal_monitor_control_tests.rs"]
mod control_tests;
