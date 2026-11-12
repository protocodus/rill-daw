use crate::{ParamSpec, Telemetry};
use daw_dsp::MipMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Instrument,
    Effect,
}

/// Everything the DAW needs to know about a device without creating it.
pub struct DeviceDescriptor {
    /// Persisted in projects and presets; never renamed.
    pub id: &'static str,
    pub name: &'static str,
    pub kind: DeviceKind,
    /// Library category, such as "Filter" or "Synth".
    pub category: &'static str,
    pub parameters: &'static [ParamSpec],
    /// 0, 1 (Sampler) or 16 (Drums).
    pub sample_slots: u32,
    pub sidechain_input: bool,
    /// Factory `.rillpreset` documents, compiled in.
    pub factory_presets: &'static [&'static str],
    /// Choice labels that older versions wrote into presets, as
    /// `(parameter key, old label, current label)`. Presets store labels, so
    /// renaming an option without an entry here breaks every older preset.
    pub legacy_labels: &'static [(&'static str, &'static str, &'static str)],
    /// The last parts of parameter keys a random roll never moves, besides
    /// the output level and volume every device keeps: where a roll would
    /// change what the device is rather than how it sounds (which part of a
    /// drum's recording plays, which pads cut each other).
    pub fixed_on_roll: &'static [&'static str],
    /// Latency in samples at the given normalized parameter values (by ID)
    /// and sample rate: what [`Device::latency_samples`] reports for them.
    /// The engine compares it across an edit to see whether delay
    /// compensation has to be rebuilt.
    pub latency: fn(values: &dyn Fn(u32) -> f32, sample_rate: f32) -> u32,
    pub create: fn() -> Box<dyn Device>,
}

impl DeviceDescriptor {
    pub fn parameter(&self, id: u32) -> Option<&'static ParamSpec> {
        self.parameters.iter().find(|parameter| parameter.id == id)
    }

    pub fn parameter_by_key(&self, key: &str) -> Option<&'static ParamSpec> {
        self.parameters
            .iter()
            .find(|parameter| parameter.key == key)
    }
}

impl std::fmt::Debug for DeviceDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id)
    }
}

/// Transport state for synced delays and LFOs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessContext {
    pub tempo: f64,
    /// Beat position of the block's first frame.
    pub beat: f64,
    pub playing: bool,
}

impl Default for ProcessContext {
    fn default() -> Self {
        Self {
            tempo: 120.0,
            beat: 0.0,
            playing: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Frame offset inside the block.
    pub offset: u32,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    /// `pitch` is a MIDI note number as a float, so fine tuning survives.
    NoteOn {
        note: u32,
        pitch: f32,
        velocity: f32,
    },
    NoteOff {
        note: u32,
    },
    /// Per-note pressure, 0–1.
    Pressure {
        note: u32,
        value: f32,
    },
    /// Channel pitch bend, −1–1.
    PitchBend(f32),
    /// Mod wheel, 0–1.
    ModWheel(f32),
    Sustain(bool),
}

/// Decoded audio at its source rate, one or two channels, kept as the
/// mip-maps a band-limited read at any pitch needs. The engine supplies it;
/// devices never read files.
pub struct SampleData {
    sample_rate: f32,
    channels: Vec<MipMap>,
}

impl SampleData {
    /// The largest recording a device takes, decoded to 32-bit samples:
    /// resident in memory, never streamed.
    pub const MAX_BYTES: usize = 32 * 1024 * 1024;

    /// One or two channels of equal length; more are dropped. Allocates and
    /// filters about twice the recording, so call it off the audio thread.
    pub fn new(sample_rate: f32, channels: &[&[f32]]) -> Self {
        let mut maps: Vec<MipMap> = channels
            .iter()
            .take(2)
            .map(|samples| MipMap::new(samples))
            .collect();
        if maps.is_empty() {
            maps.push(MipMap::new(&[]));
        }
        Self {
            sample_rate,
            channels: maps,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, MipMap::frames)
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// A channel; the last one for an index past it, so a mono recording
    /// plays on both sides.
    pub fn channel(&self, index: usize) -> &MipMap {
        &self.channels[index.min(self.channels.len().saturating_sub(1))]
    }
}

/// One block of stereo audio. Effects transform it in place; instruments
/// overwrite it.
pub struct StereoIo<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
    /// External sidechain, for devices that declare `sidechain_input`.
    pub sidechain: Option<(&'a [f32], &'a [f32])>,
}

impl StereoIo<'_> {
    pub fn frames(&self) -> usize {
        self.left.len().min(self.right.len())
    }
}

/// A built-in instrument or effect, as the engine runs it.
///
/// `process` must not allocate, lock, do I/O or loop without bound; the
/// conformance tests check every registered device for this.
pub trait Device: Send {
    fn descriptor(&self) -> &'static DeviceDescriptor;
    /// The only place that may allocate. `max_block` bounds later blocks.
    fn prepare(&mut self, sample_rate: f32, max_block: usize);
    /// Unknown IDs are ignored; values are clamped to 0–1.
    fn set_parameter(&mut self, id: u32, normalized: f32);
    fn set_sample(&mut self, _slot: u32, _sample: Option<Arc<SampleData>>) {}
    fn process(&mut self, context: &ProcessContext, events: &[Event], io: StereoIo<'_>);
    /// Clears audio state (delay lines, filter memories, voices) but keeps
    /// parameters. Also restarts the seeded random generators.
    fn reset(&mut self);
    fn latency_samples(&self) -> u32 {
        0
    }
    fn tail_samples(&self) -> u32 {
        0
    }
    fn telemetry(&self) -> Arc<Telemetry>;
}
