//! In-process hosting of Rill's built-in devices, with failure containment at
//! the device boundary. See `docs/R1_DEVICES.md#engine-daw-engine`.
use anyhow::{Context, Result, bail};
use daw_devices::{Device, Event, EventKind, ProcessContext, SampleData, StereoIo};
use daw_plugins::{
    ExpressiveController as Controller, ExpressiveMidiData as Data,
    ExpressiveMidiMessage as Message, ExpressiveVelocity as Velocity, MidiMessage,
};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::SystemTime;

/// The live telemetry of every built-in device of the sounding graph, by
/// track and effect slot (`None`: the track's instrument). The render worker
/// replaces the list when the graph changes; panels look their device up.
pub(crate) type TelemetryList = Vec<((u64, Option<u64>), Arc<daw_devices::Telemetry>)>;
pub(crate) type DeviceTelemetry = Mutex<TelemetryList>;

/// Bounds the per-block event list so translation never allocates.
const MAX_EVENTS: usize = 4096;

type SampleKey = (PathBuf, u64, Option<SystemTime>);

/// Decodes the recording at `path` for a device's sample slot, off the audio
/// thread. Graph preparations share one decode per file while any graph
/// holds it, so a parameter edit does not rebuild the mip-maps. Recordings
/// over [`SampleData::MAX_BYTES`] decoded are refused.
pub(crate) fn load_sample(path: &str) -> Result<Arc<SampleData>> {
    static CACHE: OnceLock<Mutex<HashMap<SampleKey, Weak<SampleData>>>> = OnceLock::new();
    let canonical = std::fs::canonicalize(path).context("The sample file is missing")?;
    let metadata = canonical.metadata()?;
    let key = (canonical, metadata.len(), metadata.modified().ok());
    let cache = CACHE.get_or_init(Default::default);
    let cached = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .and_then(Weak::upgrade);
    if let Some(sample) = cached {
        return Ok(sample);
    }
    let audio = daw_core::AudioData::load_wav_with_limit(&key.0, SampleData::MAX_BYTES)
        .with_context(|| {
            format!(
                "Samples are limited to {} MiB decoded",
                SampleData::MAX_BYTES / (1024 * 1024)
            )
        })?;
    let channels = audio.channels;
    let planar: Vec<Vec<f32>> = (0..channels.min(2))
        .map(|channel| {
            audio
                .samples
                .iter()
                .skip(channel)
                .step_by(channels)
                .copied()
                .collect()
        })
        .collect();
    let slices: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
    let sample = Arc::new(SampleData::new(audio.sample_rate as f32, &slices));
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|_, sample| sample.strong_count() > 0);
    cache.insert(key, Arc::downgrade(&sample));
    Ok(sample)
}

pub(crate) struct BuiltinDevice {
    device: Box<dyn Device>,
    events: Vec<Event>,
}

impl BuiltinDevice {
    /// `None` for a device ID this version does not know.
    pub(crate) fn create(
        id: &str,
        sample_rate: u32,
        max_block: usize,
        parameters: &[daw_core::PluginParameterValue],
    ) -> Option<Self> {
        let mut device = daw_devices::create(id)?;
        device.prepare(sample_rate as f32, max_block);
        for parameter in parameters {
            device.set_parameter(parameter.id, parameter.value as f32);
        }
        // Start at the saved values instead of ramping from the defaults.
        device.reset();
        Some(Self {
            device,
            events: Vec::with_capacity(MAX_EVENTS),
        })
    }

    /// Called during graph preparation only: a device may rebuild state.
    pub(crate) fn set_sample(&mut self, slot: u32, sample: Option<Arc<SampleData>>) {
        self.device.set_sample(slot, sample);
    }

    pub(crate) fn name(&self) -> &'static str {
        self.device.descriptor().name
    }

    pub(crate) fn latency_samples(&self) -> u32 {
        self.device.latency_samples()
    }

    pub(crate) fn telemetry(&self) -> Arc<daw_devices::Telemetry> {
        self.device.telemetry()
    }

    pub(crate) fn tail_samples(&self) -> u32 {
        self.device.tail_samples()
    }

    /// Clears audio state on a seek; parameters stay.
    pub(crate) fn reset(&mut self) {
        self.device.reset();
    }

    pub(crate) fn set_parameter(&mut self, id: u32, value: f64) {
        self.device.set_parameter(id, value as f32);
    }

    /// Effects: transforms the block in place. `sidechain` is the signal
    /// routed to the device's sidechain input, if any.
    pub(crate) fn process_effect(
        &mut self,
        context: &ProcessContext,
        left: &mut [f32],
        right: &mut [f32],
        sidechain: Option<(&[f32], &[f32])>,
    ) -> Result<()> {
        self.events.clear();
        self.run(context, left, right, sidechain)
    }

    /// Instruments: overwrites the block from note and controller events.
    pub(crate) fn process_instrument(
        &mut self,
        context: &ProcessContext,
        midi: &[MidiMessage],
        expressive: &[Message],
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        self.events.clear();
        for message in midi {
            let kind = if message.on {
                EventKind::NoteOn {
                    note: u32::from(message.pitch),
                    pitch: f32::from(message.pitch),
                    velocity: f32::from(message.velocity) / 127.0,
                }
            } else {
                EventKind::NoteOff {
                    note: u32::from(message.pitch),
                }
            };
            self.push(message.offset, kind);
        }
        for message in expressive {
            if let Some(kind) = expressive_event(message.data) {
                self.push(message.offset, kind);
            }
        }
        // Both inputs are offset-ordered; a stable sort merges them.
        self.events.sort_by_key(|event| event.offset);
        left.fill(0.0);
        right.fill(0.0);
        self.run(context, left, right, None)
    }

    fn push(&mut self, offset: u32, kind: EventKind) {
        if self.events.len() < MAX_EVENTS {
            self.events.push(Event { offset, kind });
        }
    }

    fn run(
        &mut self,
        context: &ProcessContext,
        left: &mut [f32],
        right: &mut [f32],
        sidechain: Option<(&[f32], &[f32])>,
    ) -> Result<()> {
        let processed = catch_unwind(AssertUnwindSafe(|| {
            self.device.process(
                context,
                &self.events,
                StereoIo {
                    left: &mut *left,
                    right: &mut *right,
                    sidechain,
                },
            );
        }));
        if processed.is_err() {
            left.fill(0.0);
            right.fill(0.0);
            bail!(
                "Built-in device \"{}\" stopped after an internal error",
                self.name()
            );
        }
        if !left
            .iter()
            .chain(right.iter())
            .all(|sample| sample.is_finite())
        {
            // Contained here: silence this block and start the device afresh.
            left.fill(0.0);
            right.fill(0.0);
            self.device.reset();
        }
        Ok(())
    }
}

/// The audio inputs a built-in insert offers to routing: its main input
/// and, where the device has one, a stereo sidechain as bus 1.
pub(crate) fn bus_layout(
    descriptor: &daw_devices::DeviceDescriptor,
) -> daw_plugins::AudioBusLayout {
    let stereo = daw_plugins::AudioBusConfig {
        channel_count: 2,
        active: true,
    };
    daw_plugins::AudioBusLayout {
        inputs: vec![stereo; if descriptor.sidechain_input { 2 } else { 1 }],
        outputs: vec![stereo],
    }
}

fn velocity(velocity: Velocity) -> f32 {
    match velocity {
        Velocity::Midi1(value) => f32::from(value) / 127.0,
        Velocity::Midi2(value) => f32::from(value) / 65535.0,
    }
}

/// Note IDs from precise notes stay distinct from plain MIDI pitches (0–127).
fn note_id(note_id: i32, pitch: u8) -> u32 {
    if note_id >= 0 {
        128 + note_id as u32
    } else {
        u32::from(pitch)
    }
}

fn expressive_event(data: Data) -> Option<EventKind> {
    Some(match data {
        Data::NoteOn {
            pitch,
            velocity: value,
            note_id: id,
            tuning,
            ..
        } => EventKind::NoteOn {
            note: note_id(id, pitch),
            pitch: f32::from(pitch) + tuning / 100.0,
            velocity: velocity(value),
        },
        Data::NoteOff {
            pitch, note_id: id, ..
        } => EventKind::NoteOff {
            note: note_id(id, pitch),
        },
        Data::PolyPressure {
            pitch,
            value,
            note_id: id,
            ..
        } => EventKind::Pressure {
            note: note_id(id, pitch),
            value: value as f32,
        },
        Data::Controller {
            controller, value, ..
        } => match controller {
            Controller::PitchBend => EventKind::PitchBend((2.0 * value - 1.0) as f32),
            Controller::ControlChange(1) => EventKind::ModWheel(value as f32),
            Controller::ControlChange(64) => EventKind::Sustain(value >= 0.5),
            _ => return None,
        },
        Data::NoteExpressionValue { .. } | Data::NoteExpressionInt { .. } => return None,
    })
}

/// Controllers every built-in instrument reacts to, with their rest values.
pub(crate) const INSTRUMENT_CONTROLLERS: [(Controller, f64); 3] = [
    (Controller::PitchBend, 0.5),
    (Controller::ControlChange(1), 0.0),
    (Controller::ControlChange(64), 0.0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use daw_devices::{DeviceDescriptor, Telemetry};
    use std::sync::Arc;

    /// Misbehaves on purpose: panics or writes NaN, and counts resets.
    struct Faulty {
        panic: bool,
        resets: usize,
    }

    impl Device for Faulty {
        fn descriptor(&self) -> &'static DeviceDescriptor {
            &daw_devices::devices::filter::DESCRIPTOR
        }
        fn prepare(&mut self, _: f32, _: usize) {}
        fn set_parameter(&mut self, _: u32, _: f32) {}
        fn process(&mut self, _: &ProcessContext, _: &[Event], io: StereoIo<'_>) {
            assert!(!self.panic, "device bug");
            io.left.fill(f32::NAN);
        }
        fn reset(&mut self) {
            self.resets += 1;
        }
        fn telemetry(&self) -> Arc<Telemetry> {
            Arc::default()
        }
    }

    fn faulty(panic: bool) -> BuiltinDevice {
        BuiltinDevice {
            device: Box::new(Faulty { panic, resets: 0 }),
            events: Vec::new(),
        }
    }

    #[test]
    fn a_panic_is_contained_and_reported() {
        let mut device = faulty(true);
        let mut left = vec![0.5; 64];
        let mut right = left.clone();
        let error = device
            .process_effect(&ProcessContext::default(), &mut left, &mut right, None)
            .unwrap_err();
        assert!(error.to_string().contains("R1 Filter"));
        assert!(left.iter().chain(&right).all(|sample| *sample == 0.0));
    }

    #[test]
    fn nonfinite_output_is_silenced_and_the_device_reset() {
        let mut device = faulty(false);
        let mut left = vec![0.5; 64];
        let mut right = left.clone();
        device
            .process_effect(&ProcessContext::default(), &mut left, &mut right, None)
            .unwrap();
        assert!(left.iter().chain(&right).all(|sample| *sample == 0.0));
    }

    #[test]
    fn filter_processes_in_place_and_reports_latency() {
        let mut device = BuiltinDevice::create(
            "r1-filter",
            48_000,
            256,
            &[daw_core::PluginParameterValue { id: 30, value: 0.0 }],
        )
        .unwrap();
        assert_eq!(device.latency_samples(), 31);
        let mut left = vec![0.0; 256];
        left[0] = 1.0;
        let mut right = left.clone();
        device
            .process_effect(&ProcessContext::default(), &mut left, &mut right, None)
            .unwrap();
        // Mix 0 from the saved values: the dry path, delayed by the latency.
        assert!((left[31] - 1.0).abs() < 1e-3);
        assert!(BuiltinDevice::create("r1-unknown", 48_000, 256, &[]).is_none());
    }

    #[test]
    fn controllers_become_device_events() {
        assert_eq!(
            expressive_event(Data::Controller {
                channel: 0,
                controller: Controller::PitchBend,
                value: 1.0,
            }),
            Some(EventKind::PitchBend(1.0))
        );
        assert_eq!(
            expressive_event(Data::Controller {
                channel: 0,
                controller: Controller::ControlChange(1),
                value: 0.25,
            }),
            Some(EventKind::ModWheel(0.25))
        );
        assert_eq!(
            expressive_event(Data::NoteOn {
                channel: 0,
                pitch: 60,
                velocity: Velocity::Midi1(127),
                note_id: 3,
                tuning: 50.0,
            }),
            Some(EventKind::NoteOn {
                note: 131,
                pitch: 60.5,
                velocity: 1.0,
            })
        );
    }
}
