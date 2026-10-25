//! MIDI input callbacks publish fixed-size messages only. The output consumer
//! stamps capture against its actual playback clock; the render worker alone
//! sends normalized polyphonic notes to isolated plugins.
use super::{AudioEngine, Controls};
use crate::render::Renderer;
use anyhow::{Context, Result, ensure};
use daw_core::{Note, Project, TrackKind};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const CAPACITY: usize = 8192;
const MAX_EVENTS: usize = 131_072;
const VIRTUAL_PORT: &str = "rill-virtual-midi";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPortInfo {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiRoute {
    pub track: u64,
    pub channel: Option<u8>,
    pub monitor: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MidiSettings {
    pub port_id: Option<String>,
    pub routes: Vec<MidiRoute>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MidiSnapshot {
    pub input_ready: bool,
    pub input_opening: bool,
    /// Prepared and armed, including while the shared pre-count gate is closed.
    pub active: bool,
    /// At least one actual timeline sample has entered the recording.
    pub recording: bool,
    pub events: u64,
    pub held_notes: u32,
}
#[derive(Clone, Debug)]
pub struct RecordedMidiTrack {
    pub track: u64,
    pub start_beat: f64,
    pub length_beats: f64,
    pub notes: Vec<Note>,
}
#[derive(Clone, Debug, Default)]
pub struct MidiRecordingResult {
    pub tracks: Vec<RecordedMidiTrack>,
    pub error: Option<String>,
}

pub fn list_midi_inputs() -> Result<Vec<MidiPortInfo>> {
    let input =
        midir::MidiInput::new("RILL MIDI discovery").context("Initialize MIDI input discovery")?;
    input
        .ports()
        .iter()
        .map(|port| {
            Ok(MidiPortInfo {
                id: port.id(),
                name: input.port_name(port).context("Read MIDI input name")?,
            })
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Event {
    time: u64,
    data: u32,
    epoch: u64,
}
struct Slot {
    time: AtomicU64,
    data: AtomicU32,
    epoch: AtomicU64,
}
struct Queue {
    slots: Box<[Slot]>,
    read: AtomicUsize,
    write: AtomicUsize,
}
impl Queue {
    fn new() -> Self {
        Self {
            slots: (0..CAPACITY)
                .map(|_| Slot {
                    time: AtomicU64::new(0),
                    data: AtomicU32::new(0),
                    epoch: AtomicU64::new(0),
                })
                .collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }
    fn push(&self, event: Event) -> bool {
        let write = self.write.load(Ordering::Relaxed);
        if write.wrapping_sub(self.read.load(Ordering::Acquire)) >= self.slots.len() {
            return false;
        }
        let slot = &self.slots[write % self.slots.len()];
        slot.time.store(event.time, Ordering::Relaxed);
        slot.data.store(event.data, Ordering::Relaxed);
        slot.epoch.store(event.epoch, Ordering::Relaxed);
        self.write.store(write.wrapping_add(1), Ordering::Release);
        true
    }
    fn peek(&self) -> Option<Event> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.write.load(Ordering::Acquire) {
            return None;
        }
        let slot = &self.slots[read % self.slots.len()];
        Some(Event {
            time: slot.time.load(Ordering::Relaxed),
            data: slot.data.load(Ordering::Relaxed),
            epoch: slot.epoch.load(Ordering::Relaxed),
        })
    }
    fn pop(&self) -> Option<Event> {
        let value = self.peek()?;
        self.read.fetch_add(1, Ordering::Release);
        Some(value)
    }
    // Caller must own the consumer gate so no inspected slot can be recycled.
    // The producer may keep publishing into slots beyond this bounded snapshot.
    fn pending(&self) -> impl Iterator<Item = Event> + '_ {
        let read = self.read.load(Ordering::Relaxed);
        let count = self.write.load(Ordering::Acquire).wrapping_sub(read);
        (0..count).map(move |offset| {
            let slot = &self.slots[read.wrapping_add(offset) % self.slots.len()];
            Event {
                time: slot.time.load(Ordering::Relaxed),
                data: slot.data.load(Ordering::Relaxed),
                epoch: slot.epoch.load(Ordering::Relaxed),
            }
        })
    }
}

/// The callback normally owns the input consumer. After Stop, finalization may
/// borrow it without waiting; a concurrent callback simply leaves MIDI queued.
struct ObserverGuard<'a>(&'a AtomicBool);
impl Drop for ObserverGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct MidiShared {
    incoming: Queue,
    monitoring: Queue,
    recorded: Queue,
    ready: AtomicBool,
    opening: AtomicBool,
    requested: AtomicBool,
    started: AtomicBool,
    producer: AtomicBool,
    observer: AtomicBool,
    monitor_requested: AtomicBool,
    epoch: AtomicU64,
    reset: AtomicU64,
    fault: AtomicUsize,
    reported_fault: AtomicUsize,
    anchor_ns: AtomicU64,
    start_sample: AtomicU64,
    frames: AtomicU64,
    next_output_ns: AtomicU64,
    sample_rate: AtomicU32,
    events: AtomicU64,
    held: AtomicU32,
    route_count: AtomicUsize,
    route_tracks: [AtomicU64; 32],
    route_channels: [AtomicU32; 32],
    route_monitor: [AtomicBool; 32],
}
impl MidiShared {
    pub(super) fn new() -> Self {
        Self {
            incoming: Queue::new(),
            monitoring: Queue::new(),
            recorded: Queue::new(),
            ready: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            requested: AtomicBool::new(false),
            started: AtomicBool::new(false),
            producer: AtomicBool::new(false),
            observer: AtomicBool::new(false),
            monitor_requested: AtomicBool::new(false),
            epoch: AtomicU64::new(1),
            reset: AtomicU64::new(1),
            fault: AtomicUsize::new(0),
            reported_fault: AtomicUsize::new(0),
            anchor_ns: AtomicU64::new(0),
            start_sample: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            next_output_ns: AtomicU64::new(0),
            sample_rate: AtomicU32::new(48000),
            events: AtomicU64::new(0),
            held: AtomicU32::new(0),
            route_count: AtomicUsize::new(0),
            route_tracks: std::array::from_fn(|_| AtomicU64::new(0)),
            route_channels: std::array::from_fn(|_| AtomicU32::new(16)),
            route_monitor: std::array::from_fn(|_| AtomicBool::new(false)),
        }
    }
    pub(super) fn capture_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
    pub(super) fn active(&self) -> bool {
        self.capture_requested()
    }
    pub(super) fn monitoring_requested(&self) -> bool {
        self.ready.load(Ordering::Acquire)
            && self.monitor_requested.load(Ordering::Acquire)
            && self.fault.load(Ordering::Acquire) == 0
    }
    fn claim_observer(&self) -> Option<ObserverGuard<'_>> {
        self.observer
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| ObserverGuard(&self.observer))
    }
    fn captured_event(&self, event: Event, frames: u64) -> Option<Event> {
        let sample_rate = u64::from(self.sample_rate.load(Ordering::Relaxed));
        let anchor = self.anchor_ns.load(Ordering::Relaxed);
        if !self.started.load(Ordering::Acquire)
            || event.epoch != self.epoch.load(Ordering::Acquire)
            || event.time.saturating_add(500_000_000 / sample_rate) < anchor
        {
            return None;
        }
        let nanos = event.time.saturating_sub(anchor);
        let frame =
            ((u128::from(nanos) * u128::from(sample_rate) + 500_000_000) / 1_000_000_000) as u64;
        (frame < frames).then_some(Event {
            time: frame,
            ..event
        })
    }
    fn finish_capture(&self, events: &mut Vec<Event>) -> Option<(u64, u64)> {
        if self.requested.load(Ordering::SeqCst) {
            return None;
        }
        let _observer = self.claim_observer()?;
        let frames = self.frames.load(Ordering::Acquire);
        let start = self.start_sample.load(Ordering::Acquire);
        let retain = |event, events: &mut Vec<Event>| {
            if events.len() < MAX_EVENTS {
                events.push(event);
            } else {
                self.fail(3);
            }
        };
        while let Some(event) = self.recorded.pop() {
            retain(event, events);
        }
        // No output callback is required after Stop. Inspect the bounded queue
        // under its consumer gate, including late input behind future events.
        // Leave it intact for monitoring; clearing started prevents recapture.
        for event in self.incoming.pending() {
            if let Some(event) = self.captured_event(event, frames) {
                retain(event, events);
                self.events.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.started.store(false, Ordering::Release);
        Some((frames, start))
    }
    fn fail(&self, code: usize) {
        let _ = self
            .fault
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Relaxed);
        self.requested.store(false, Ordering::SeqCst);
        self.ready.store(false, Ordering::Release);
        self.reset.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn output_failed(&self) {
        self.fail(5);
    }
    fn fault_text(&self) -> Option<String> {
        match self.fault.load(Ordering::Acquire) {
            0 => None,
            1 => Some(
                "MIDI input queue overflow; recording was stopped and captured notes were retained"
                    .into(),
            ),
            2 => Some("MIDI input disconnected; captured notes were retained".into()),
            3 => Some("MIDI recording event limit reached; captured notes were retained".into()),
            4 => Some(
                "Audio output underrun interrupted MIDI recording; captured notes were retained"
                    .into(),
            ),
            5 => Some(
                "Audio output failed during MIDI recording; captured notes were retained".into(),
            ),
            _ => Some("MIDI input failed; captured notes were retained".into()),
        }
    }
    fn enqueue(&self, data: u32, time: u64, epoch: u64) {
        if !self.ready.load(Ordering::Acquire) || epoch != self.epoch.load(Ordering::Acquire) {
            return;
        }
        if self
            .producer
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            self.fail(1);
            return;
        }
        if self.ready.load(Ordering::Acquire)
            && epoch == self.epoch.load(Ordering::Acquire)
            && !self.incoming.push(Event { time, data, epoch })
        {
            self.fail(1);
        }
        self.producer.store(false, Ordering::Release);
    }
    pub(super) fn observe_frame(
        &self,
        position: Option<u64>,
        sample_rate: u32,
        playback_ns: u64,
        capture_allowed: bool,
    ) {
        if !self.ready.load(Ordering::Acquire) && !self.requested.load(Ordering::Acquire) {
            return;
        }
        let Some(_observer) = self.claim_observer() else {
            return;
        };
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.next_output_ns.store(
            playback_ns + (1_000_000_000 / u64::from(sample_rate)),
            Ordering::Release,
        );
        if capture_allowed
            && position.is_none()
            && self.started.load(Ordering::Acquire)
            && self.capture_requested()
        {
            self.fail(4);
        }
        let capture =
            capture_allowed && position.is_some() && self.requested.load(Ordering::SeqCst);
        if capture && !self.started.load(Ordering::Relaxed) {
            self.anchor_ns.store(playback_ns, Ordering::Relaxed);
            self.start_sample
                .store(position.unwrap(), Ordering::Relaxed);
            self.frames.store(0, Ordering::Relaxed);
            self.started.store(true, Ordering::Release);
        }
        // Stop freezes the range, but callbacks can still deliver timestamps
        // inside it before finalization. Keep those events without extending it.
        let capture_frames = self.frames.load(Ordering::Relaxed) + u64::from(capture);
        let epoch = self.epoch.load(Ordering::Acquire);
        while let Some(event) = self.incoming.peek() {
            if event.epoch == epoch && event.time > playback_ns {
                break;
            }
            let event = self.incoming.pop().unwrap();
            if event.epoch != epoch || !self.ready.load(Ordering::Acquire) {
                continue;
            }
            if !self.monitoring.push(event) {
                self.fail(1);
                break;
            }
            if let Some(event) = self.captured_event(event, capture_frames) {
                if !self.recorded.push(event) {
                    self.fail(1);
                    break;
                }
                self.events.fetch_add(1, Ordering::Relaxed);
            }
        }
        if capture {
            self.frames.fetch_add(1, Ordering::Release);
        }
    }
}

#[derive(Default)]
struct MidiTimestampClock {
    anchor: Option<(u64, u64)>,
    previous: u64,
}
impl MidiTimestampClock {
    fn map(&mut self, driver_us: u64, receipt_ns: u64) -> u64 {
        let (driver_anchor, host_anchor) = *self.anchor.get_or_insert((driver_us, receipt_ns));
        let mapped = host_anchor
            .saturating_add(driver_us.saturating_sub(driver_anchor).saturating_mul(1000));
        // A restarted/wrapped backend clock gets a fresh epoch. Ordinary callback
        // delivery jitter does not alter the musical intervals between events.
        if driver_us < self.previous
            || mapped > receipt_ns.saturating_add(1_000_000_000)
            || receipt_ns > mapped.saturating_add(60_000_000_000)
        {
            self.anchor = Some((driver_us, receipt_ns));
            self.previous = driver_us;
            receipt_ns
        } else {
            self.previous = driver_us;
            mapped
        }
    }
}

fn message(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 3 || bytes[1] > 127 || bytes[2] > 127 {
        return None;
    }
    match bytes[0] & 0xf0 {
        0x80 | 0x90 => {
            Some(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
        }
        0xb0 if matches!(bytes[1], 64 | 120 | 123) => {
            Some(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
        }
        _ => None,
    }
}
#[derive(Clone, Copy)]
struct Voice {
    start: u64,
    velocity: u8,
    down: bool,
}
struct NoteState {
    voices: Vec<Option<Voice>>,
    sustain: [bool; 16],
}
impl NoteState {
    fn new() -> Self {
        Self {
            voices: vec![None; 2048],
            sustain: [false; 16],
        }
    }
    fn apply(&mut self, event: Event, mut finish: impl FnMut(u8, u8, Voice, u64)) {
        let status = (event.data & 255) as u8;
        let channel = status & 15;
        let pitch = ((event.data >> 8) & 127) as u8;
        let value = ((event.data >> 16) & 127) as u8;
        let index = usize::from(channel) * 128 + usize::from(pitch);
        match status & 0xf0 {
            0x90 if value > 0 => {
                if let Some(old) = self.voices[index].take() {
                    finish(channel, pitch, old, event.time);
                }
                self.voices[index] = Some(Voice {
                    start: event.time,
                    velocity: value,
                    down: true,
                });
            }
            0x80 | 0x90 => {
                if let Some(mut old) = self.voices[index].take() {
                    if self.sustain[usize::from(channel)] {
                        old.down = false;
                        self.voices[index] = Some(old);
                    } else {
                        finish(channel, pitch, old, event.time);
                    }
                }
            }
            0xb0 if pitch == 64 => {
                let down = value >= 64;
                self.sustain[usize::from(channel)] = down;
                if !down {
                    for pitch in 0..128 {
                        let index = usize::from(channel) * 128 + pitch;
                        if self.voices[index].is_some_and(|voice| !voice.down) {
                            finish(
                                channel,
                                pitch as u8,
                                self.voices[index].take().unwrap(),
                                event.time,
                            );
                        }
                    }
                }
            }
            0xb0 if matches!(pitch, 120 | 123) => {
                self.sustain[usize::from(channel)] = false;
                for pitch in 0..128 {
                    if let Some(old) = self.voices[usize::from(channel) * 128 + pitch].take() {
                        finish(channel, pitch as u8, old, event.time);
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) struct MidiMonitor {
    state: NoteState,
    reset: u64,
    notes: Vec<(u64, u8, u8)>,
    dirty: bool,
}
impl MidiMonitor {
    pub(super) fn new() -> Self {
        Self {
            state: NoteState::new(),
            reset: 0,
            notes: Vec::with_capacity(4096),
            dirty: true,
        }
    }
    fn refresh(&mut self, shared: &MidiShared, renderer: &mut Renderer, offset: u32) -> Result<()> {
        let epoch = shared.epoch.load(Ordering::Acquire);
        let reset = shared.reset.load(Ordering::Acquire);
        if reset != self.reset {
            self.state.voices.fill(None);
            self.state.sustain.fill(false);
            self.reset = reset;
        }
        self.notes.clear();
        if shared.monitoring_requested() {
            for route in 0..shared.route_count.load(Ordering::Acquire).min(32) {
                if !shared.route_monitor[route].load(Ordering::Relaxed) {
                    continue;
                }
                let track = shared.route_tracks[route].load(Ordering::Relaxed);
                let channel = shared.route_channels[route].load(Ordering::Relaxed);
                for pitch in 0..128 {
                    let velocity = (0..16)
                        .filter(|ch| channel == 16 || *ch == channel as usize)
                        .filter_map(|ch| {
                            self.state.voices[ch * 128 + pitch].map(|voice| voice.velocity)
                        })
                        .max();
                    if let Some(velocity) = velocity {
                        self.notes.push((track, pitch as u8, velocity));
                    }
                }
            }
        }
        if epoch != shared.epoch.load(Ordering::Acquire) || !shared.ready.load(Ordering::Acquire) {
            self.notes.clear();
        }
        self.notes.sort_unstable();
        shared
            .held
            .store(self.notes.len() as u32, Ordering::Relaxed);
        renderer.set_midi_input_notes(&self.notes, offset)
    }
    pub(super) fn update_suspended(&mut self, shared: &MidiShared) {
        let reset = shared.reset.load(Ordering::Acquire);
        if reset != self.reset {
            self.state.voices.fill(None);
            self.state.sustain.fill(false);
            self.reset = reset;
        }
        let epoch = shared.epoch.load(Ordering::Acquire);
        for _ in 0..256 {
            let Some(event) = shared.monitoring.pop() else {
                break;
            };
            if event.epoch == epoch && shared.ready.load(Ordering::Acquire) {
                self.state.apply(event, |_, _, _, _| {});
            }
        }
        self.dirty = true;
        shared.held.store(0, Ordering::Relaxed);
    }
    pub(super) fn update(&mut self, shared: &MidiShared, renderer: &mut Renderer) -> Result<()> {
        let reset = shared.reset.load(Ordering::Acquire);
        if reset != self.reset {
            self.state.voices.fill(None);
            self.state.sustain.fill(false);
            self.reset = reset;
            self.dirty = true;
        }
        if self.dirty {
            self.refresh(shared, renderer, 0)?;
            self.dirty = false;
        }
        // Restore held keys after an explicit transport seek or preview change;
        // unchanged snapshots return immediately without scanning all MIDI keys.
        renderer.set_midi_input_notes(&self.notes, 0)?;
        let epoch = shared.epoch.load(Ordering::Acquire);
        let mut first = None;
        for _ in 0..256 {
            let Some(event) = shared.monitoring.pop() else {
                break;
            };
            if event.epoch != epoch || !shared.ready.load(Ordering::Acquire) {
                continue;
            }
            let anchor = *first.get_or_insert(event.time);
            let frames = u128::from(event.time.saturating_sub(anchor))
                * u128::from(shared.sample_rate.load(Ordering::Relaxed))
                / 1_000_000_000;
            let offset = frames.min((crate::render::BLOCK - 1) as u128) as u32;
            let status = (event.data & 255) as u8;
            let pitch = ((event.data >> 8) & 127) as u8;
            let velocity = ((event.data >> 16) & 127) as u8;
            if status & 0xf0 == 0x90 && velocity > 0 {
                for route in 0..shared.route_count.load(Ordering::Acquire).min(32) {
                    let channel = shared.route_channels[route].load(Ordering::Relaxed);
                    if shared.route_monitor[route].load(Ordering::Relaxed)
                        && (channel == 16 || channel == u32::from(status & 15))
                    {
                        renderer.release_midi_input_note(
                            shared.route_tracks[route].load(Ordering::Relaxed),
                            pitch,
                            offset,
                        );
                    }
                }
            }
            self.state.apply(event, |_, _, _, _| {});
            // Apply every edge, not only the final chord. Short notes and repeated
            // presses keep their articulation even when one render poll sees both.
            self.refresh(shared, renderer, offset)?;
        }
        Ok(())
    }
}

struct NativeInput {
    _connection: midir::MidiInputConnection<()>,
    probe: midir::MidiInput,
    port_id: String,
}
struct Recording {
    routes: Vec<MidiRoute>,
    events: Vec<Event>,
    tempo: f64,
    loop_range: Option<(u64, u64)>,
}
pub(super) struct MidiController {
    controls: Arc<Controls>,
    virtual_input: bool,
    settings: MidiSettings,
    tempo: f64,
    loop_range: Option<(u64, u64)>,
    opening: Option<mpsc::Receiver<Result<NativeInput>>>,
    native: Option<NativeInput>,
    recording: Option<Recording>,
    error: Option<String>,
    probe_at: Instant,
}
impl MidiController {
    pub(super) fn new(controls: Arc<Controls>, virtual_input: bool) -> Self {
        Self {
            controls,
            virtual_input,
            settings: MidiSettings::default(),
            tempo: 120.0,
            loop_range: None,
            opening: None,
            native: None,
            recording: None,
            error: None,
            probe_at: Instant::now(),
        }
    }
    fn close(&mut self) {
        self.controls.midi.ready.store(false, Ordering::Release);
        self.controls.midi.opening.store(false, Ordering::Release);
        self.controls.midi.epoch.fetch_add(1, Ordering::AcqRel);
        self.controls.midi.reset.fetch_add(1, Ordering::AcqRel);
        self.controls
            .midi
            .monitor_requested
            .store(false, Ordering::Release);
        if let Some(native) = self.native.take() {
            thread::spawn(move || drop(native));
        }
        if let Some(opening) = self.opening.take() {
            thread::spawn(move || drop(opening.recv()));
        }
    }
    pub(super) fn shutdown(&mut self) {
        self.controls.midi.requested.store(false, Ordering::SeqCst);
        self.close();
    }
    pub(super) fn poll(&mut self) {
        let opened = self.opening.as_ref().map(|rx| rx.try_recv());
        match opened {
            Some(Ok(result)) => {
                self.opening = None;
                self.controls.midi.opening.store(false, Ordering::Release);
                match result {
                    Ok(native) => {
                        self.native = Some(native);
                        self.controls.midi.ready.store(true, Ordering::Release);
                    }
                    Err(error) => {
                        self.error = Some(format!("{error:#}"));
                        self.controls.midi.fail(2);
                    }
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.opening = None;
                self.controls.midi.opening.store(false, Ordering::Release);
                self.error = Some("MIDI input setup stopped unexpectedly".into());
                self.controls.midi.fail(2);
            }
            _ => {}
        }
        if self.probe_at.elapsed() > Duration::from_millis(500) {
            self.probe_at = Instant::now();
            if self
                .native
                .as_ref()
                .is_some_and(|native| native.probe.find_port_by_id(&native.port_id).is_none())
            {
                self.controls.midi.fail(2);
                self.close();
            }
        }
        if let Some(recording) = &mut self.recording {
            while let Some(event) = self.controls.midi.recorded.pop() {
                if recording.events.len() == MAX_EVENTS {
                    self.controls.midi.fail(3);
                    break;
                }
                recording.events.push(event);
            }
        }
    }
}
impl Drop for MidiController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl AudioEngine {
    pub fn configure_midi(&mut self, project: &Project, settings: MidiSettings) -> Result<()> {
        ensure!(
            self.midi.recording.is_none(),
            "Finish MIDI recording before changing its input"
        );
        ensure!(
            settings.routes.len() <= 32,
            "MIDI input supports at most 32 tracks"
        );
        let mut route_ids = std::collections::HashSet::new();
        for route in &settings.routes {
            ensure!(
                route_ids.insert(route.track),
                "MIDI track has more than one input route"
            );
            ensure!(
                project
                    .track(route.track)
                    .is_some_and(|track| track.kind == TrackKind::Midi),
                "MIDI input target is not a MIDI track"
            );
            ensure!(
                route.channel.is_none_or(|channel| channel < 16),
                "MIDI channel must be 1 through 16"
            );
        }
        self.midi.tempo = project.tempo;
        let samples_per_beat = f64::from(self.sample_rate) * 60.0 / project.tempo;
        self.midi.loop_range = project.loop_enabled.then(|| {
            (
                (project.loop_start * samples_per_beat).round() as u64,
                (project.loop_end * samples_per_beat).round() as u64,
            )
        });
        ensure!(
            self.midi
                .loop_range
                .is_none_or(|(left, right)| right > left),
            "MIDI recording loop must contain at least one sample"
        );
        if settings == self.midi.settings
            && (self.controls.midi.ready.load(Ordering::Acquire)
                || self.controls.midi.opening.load(Ordering::Acquire))
        {
            return Ok(());
        }
        self.midi.close();
        self.midi.error = None;
        self.midi.settings = settings.clone();
        let shared = &self.controls.midi;
        shared.fault.store(0, Ordering::Release);
        shared.reported_fault.store(0, Ordering::Release);
        shared
            .route_count
            .store(settings.routes.len(), Ordering::Release);
        for (index, route) in settings.routes.iter().enumerate() {
            shared.route_tracks[index].store(route.track, Ordering::Relaxed);
            shared.route_channels[index]
                .store(route.channel.map_or(16, u32::from), Ordering::Relaxed);
            shared.route_monitor[index].store(route.monitor, Ordering::Relaxed);
        }
        shared.monitor_requested.store(
            settings.routes.iter().any(|route| route.monitor),
            Ordering::Release,
        );
        shared
            .sample_rate
            .store(self.sample_rate, Ordering::Relaxed);
        shared
            .next_output_ns
            .store(self.controls.input.now_ns(), Ordering::Release);
        if settings.routes.is_empty() {
            return Ok(());
        }
        let port_id = settings.port_id.context("Choose a MIDI input device")?;
        if self.midi.virtual_input {
            ensure!(
                port_id == VIRTUAL_PORT,
                "Virtual MIDI input device is unavailable: {port_id}"
            );
            shared.ready.store(true, Ordering::Release);
            return Ok(());
        }
        shared.opening.store(true, Ordering::Release);
        let epoch = shared.epoch.load(Ordering::Acquire);
        let controls = Arc::clone(&self.controls);
        let (tx, rx) = mpsc::channel();
        self.midi.opening = Some(rx);
        thread::spawn(move || {
            let result = (|| -> Result<NativeInput> {
                let mut input =
                    midir::MidiInput::new("RILL MIDI input").context("Initialize MIDI input")?;
                input.ignore(
                    midir::Ignore::Time | midir::Ignore::ActiveSense | midir::Ignore::Sysex,
                );
                let port = input
                    .find_port_by_id(&port_id)
                    .with_context(|| format!("MIDI input is unavailable: {port_id}"))?;
                let mut clock = MidiTimestampClock::default();
                let connection = input
                    .connect(
                        &port,
                        "RILL MIDI input",
                        move |timestamp, bytes, _| {
                            if let Some(data) = message(bytes) {
                                let time = clock.map(timestamp, controls.input.now_ns());
                                controls.midi.enqueue(data, time, epoch);
                            }
                        },
                        (),
                    )
                    .map_err(|error| anyhow::anyhow!("Connect MIDI input {port_id}: {error}"))?;
                let probe = midir::MidiInput::new("RILL MIDI status")
                    .context("Initialize MIDI device status")?;
                Ok(NativeInput {
                    _connection: connection,
                    probe,
                    port_id,
                })
            })();
            let _ = tx.send(result);
        });
        Ok(())
    }
    pub fn disable_midi(&mut self) -> Result<()> {
        ensure!(
            self.midi.recording.is_none(),
            "Finish MIDI recording before closing input"
        );
        self.midi.close();
        self.midi.settings = MidiSettings::default();
        Ok(())
    }
    pub fn poll_midi(&mut self) {
        self.midi.poll();
    }
    pub fn midi_snapshot(&self) -> MidiSnapshot {
        let s = &self.controls.midi;
        MidiSnapshot {
            input_ready: s.ready.load(Ordering::Acquire),
            input_opening: s.opening.load(Ordering::Acquire),
            active: s.capture_requested(),
            recording: s.started.load(Ordering::Acquire) && s.capture_requested(),
            events: s.events.load(Ordering::Relaxed),
            held_notes: s.held.load(Ordering::Relaxed),
        }
    }
    pub fn take_midi_error(&mut self) -> Option<String> {
        self.midi.poll();
        if let Some(error) = self.midi.error.take() {
            self.controls.midi.reported_fault.store(
                self.controls.midi.fault.load(Ordering::Acquire),
                Ordering::Release,
            );
            return Some(error);
        }
        let shared = &self.controls.midi;
        let code = shared.fault.load(Ordering::Acquire);
        if code != 0 && shared.reported_fault.swap(code, Ordering::AcqRel) != code {
            shared.fault_text()
        } else {
            None
        }
    }
    pub fn start_midi_recording(&mut self, tracks: Vec<u64>) -> Result<()> {
        self.midi.poll();
        let shared = &self.controls.midi;
        ensure!(
            shared.ready.load(Ordering::Acquire),
            "Choose an available MIDI input device"
        );
        ensure!(
            self.midi.recording.is_none(),
            "MIDI recording is already active"
        );
        ensure!(
            !tracks.is_empty() && tracks.len() <= 32,
            "Arm between one and 32 MIDI tracks"
        );
        ensure!(
            tracks
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == tracks.len(),
            "MIDI track is armed more than once"
        );
        let routes = tracks
            .iter()
            .map(|id| {
                self.midi
                    .settings
                    .routes
                    .iter()
                    .find(|route| route.track == *id)
                    .cloned()
                    .with_context(|| format!("Armed MIDI track {id} has no input"))
            })
            .collect::<Result<Vec<_>>>()?;
        while shared.recorded.pop().is_some() {}
        shared.started.store(false, Ordering::Release);
        shared.frames.store(0, Ordering::Relaxed);
        shared.events.store(0, Ordering::Relaxed);
        self.midi.recording = Some(Recording {
            routes,
            events: Vec::new(),
            tempo: self.midi.tempo,
            loop_range: self.midi.loop_range,
        });
        shared.requested.store(true, Ordering::SeqCst);
        self.controls
            .finished_generation
            .store(u64::MAX, Ordering::Release);
        Ok(())
    }
    pub fn request_stop_midi_recording(&self) {
        self.controls.midi.requested.store(false, Ordering::SeqCst);
        self.controls.midi.reset.fetch_add(1, Ordering::AcqRel);
    }
    pub fn take_midi_recording_result(&mut self) -> Option<MidiRecordingResult> {
        self.midi.poll();
        let shared = &self.controls.midi;
        let (frames, start) = shared.finish_capture(&mut self.midi.recording.as_mut()?.events)?;
        let recording = self.midi.recording.take().unwrap();
        let mut result = assemble(recording, frames, start, self.sample_rate);
        if let Some(error) = shared.fault_text() {
            result.error = Some(error);
        }
        Some(result)
    }
    pub fn inject_virtual_midi(&mut self, bytes: &[u8], offset_frames: u32) -> Result<()> {
        self.inject_virtual_midi_offset(bytes, i64::from(offset_frames))
    }
    /// Simulate a driver callback arriving after its timestamped performance.
    pub fn inject_virtual_midi_delayed(&mut self, bytes: &[u8], delay_frames: u32) -> Result<()> {
        self.inject_virtual_midi_offset(bytes, -i64::from(delay_frames))
    }
    fn inject_virtual_midi_offset(&mut self, bytes: &[u8], offset_frames: i64) -> Result<()> {
        ensure!(
            self.midi.virtual_input,
            "MIDI injection requires virtual audio mode"
        );
        let data = message(bytes)
            .context("Inject a complete MIDI note or sustain/all-notes-off message")?;
        let shared = &self.controls.midi;
        ensure!(
            shared.ready.load(Ordering::Acquire),
            "Virtual MIDI input is not ready"
        );
        ensure!(
            offset_frames.unsigned_abs() <= u64::from(self.sample_rate) * 60,
            "MIDI offset exceeds one minute"
        );
        let delta = (u128::from(offset_frames.unsigned_abs()) * 1_000_000_000
            / u128::from(self.sample_rate)) as u64;
        let now = self.controls.input.output_time(None);
        let time = if offset_frames < 0 {
            now.checked_sub(delta)
                .context("MIDI delay precedes the virtual device clock")?
        } else {
            now.saturating_add(delta)
        };
        shared.enqueue(data, time, shared.epoch.load(Ordering::Acquire));
        Ok(())
    }
    pub fn fail_virtual_midi(&self) -> Result<()> {
        ensure!(
            self.midi.virtual_input,
            "MIDI fault injection requires virtual audio mode"
        );
        self.controls.midi.fail(2);
        Ok(())
    }
}

fn assemble(
    mut recording: Recording,
    frames: u64,
    start: u64,
    sample_rate: u32,
) -> MidiRecordingResult {
    if frames == 0 {
        return MidiRecordingResult::default();
    }
    recording.events.sort_by_key(|event| event.time);
    let mut state = NoteState::new();
    let mut completed = Vec::new();
    for event in recording
        .events
        .iter()
        .copied()
        .filter(|event| event.time < frames)
    {
        state.apply(event, |channel, pitch, voice, end| {
            if end > voice.start {
                completed.push((channel, pitch, voice, end));
            }
        });
    }
    for (index, voice) in state.voices.into_iter().enumerate() {
        if let Some(voice) = voice
            && frames > voice.start
        {
            completed.push(((index / 128) as u8, (index % 128) as u8, voice, frames));
        }
    }
    let factor = recording.tempo / 60.0 / f64::from(sample_rate);
    let map = |at: u64| -> (u64, u64) {
        if let Some((left, right)) = recording.loop_range {
            let first = right.saturating_sub(start);
            if at >= first {
                let within = (at - first) % (right - left);
                return (left + within, right - left - within);
            }
            (start + at, first - at)
        } else {
            (start + at, u64::MAX)
        }
    };
    let clip_start = if recording
        .loop_range
        .is_some_and(|(_, right)| frames > right.saturating_sub(start))
    {
        recording.loop_range.unwrap().0.min(start)
    } else {
        start
    };
    let end_position = if let Some((_, right)) = recording.loop_range {
        if frames >= right.saturating_sub(start) {
            right
        } else {
            start + frames
        }
    } else {
        start + frames
    };
    let mut result = MidiRecordingResult::default();
    for route in recording.routes {
        let mut notes = Vec::new();
        for &(channel, pitch, voice, end) in &completed {
            if notes.len() >= MAX_EVENTS {
                break;
            }
            if route.channel.is_some_and(|selected| selected != channel) {
                continue;
            }
            let mut at = voice.start;
            while at < end {
                let (position, remaining) = map(at);
                let length = (end - at).min(remaining);
                if length == 0 {
                    break;
                }
                if notes.len() == MAX_EVENTS {
                    result.error = Some("MIDI recording note limit reached; the bounded recorded prefix was retained".into());
                    break;
                }
                notes.push(Note::new(
                    pitch,
                    (position - clip_start) as f64 * factor,
                    length as f64 * factor,
                    voice.velocity,
                ));
                at += length;
            }
        }
        if !notes.is_empty() {
            notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
            result.tracks.push(RecordedMidiTrack {
                track: route.track,
                start_beat: clip_start as f64 * factor,
                length_beats: (end_position - clip_start) as f64 * factor,
                notes,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    const RATE: u32 = 48000;
    const ORIGIN: u64 = 1_000_000_000;
    fn at(frame: u64) -> u64 {
        ORIGIN + (u128::from(frame) * 1_000_000_000 / u128::from(RATE)) as u64
    }
    fn shared() -> MidiShared {
        let shared = MidiShared::new();
        shared.ready.store(true, Ordering::Release);
        shared.requested.store(true, Ordering::Release);
        shared
    }
    fn send(shared: &MidiShared, frame: u64, bytes: [u8; 3]) {
        shared.enqueue(message(&bytes).unwrap(), at(frame), 1);
    }
    fn pump(shared: &MidiShared, from: u64, to: u64, allowed: bool) {
        for frame in from..to {
            shared.observe_frame(Some(frame), RATE, at(frame), allowed);
        }
    }
    fn result(
        shared: &MidiShared,
        routes: Vec<MidiRoute>,
        loop_range: Option<(u64, u64)>,
    ) -> MidiRecordingResult {
        let mut events = Vec::new();
        shared.requested.store(false, Ordering::SeqCst);
        let (frames, start) = shared.finish_capture(&mut events).unwrap();
        assemble(
            Recording {
                routes,
                events,
                tempo: 120.0,
                loop_range,
            },
            frames,
            start,
            RATE,
        )
    }
    fn route(track: u64, channel: Option<u8>) -> MidiRoute {
        MidiRoute {
            track,
            channel,
            monitor: true,
        }
    }
    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }
    #[test]
    fn callback_clock_records_chords_velocity_zero_retrigger_sustain_and_channel_filters() {
        let shared = shared();
        for (frame, bytes) in [
            (0, [0x90, 60, 90]),
            (100, [0x91, 64, 70]),
            (200, [0x90, 60, 110]),
            (250, [0xb0, 64, 127]),
            (300, [0x90, 60, 0]),
            (400, [0x81, 64, 0]),
            (500, [0xb0, 64, 0]),
        ] {
            send(&shared, frame, bytes);
        }
        pump(&shared, 0, 600, true);
        let result = result(&shared, vec![route(1, None), route(2, Some(1))], None);
        assert_eq!(result.tracks.len(), 2);
        assert_eq!(result.tracks[0].notes.len(), 3);
        let notes = &result.tracks[0].notes;
        assert_eq!((notes[0].pitch, notes[0].velocity), (60, 90));
        near(notes[0].length, 200.0 / 24000.0);
        assert_eq!((notes[1].pitch, notes[1].velocity), (64, 70));
        near(notes[1].start, 100.0 / 24000.0);
        near(notes[1].length, 300.0 / 24000.0);
        assert_eq!(notes[2].velocity, 110);
        near(notes[2].start, 200.0 / 24000.0);
        near(notes[2].length, 300.0 / 24000.0);
        assert_eq!(result.tracks[1].notes.len(), 1);
        assert_eq!(result.tracks[1].notes[0].pitch, 64);
    }
    #[test]
    fn count_in_notes_are_excluded_and_exact_first_recording_frame_is_included() {
        let shared = shared();
        send(&shared, 0, [0x90, 48, 90]);
        send(&shared, 99, [0x90, 50, 90]);
        send(&shared, 100, [0x90, 60, 95]);
        send(&shared, 120, [0x80, 48, 0]);
        send(&shared, 140, [0x80, 50, 0]);
        send(&shared, 160, [0x80, 60, 0]);
        pump(&shared, 0, 100, false);
        pump(&shared, 100, 200, true);
        let result = result(&shared, vec![route(1, None)], None);
        assert_eq!(result.tracks[0].notes.len(), 1);
        assert_eq!(result.tracks[0].notes[0].pitch, 60);
        near(result.tracks[0].start_beat, 100.0 / 24000.0);
        near(result.tracks[0].notes[0].start, 0.0);
        near(result.tracks[0].notes[0].length, 60.0 / 24000.0);
    }
    #[test]
    fn late_callback_delivery_preserves_event_timestamps_and_stop_closes_held_notes() {
        let shared = shared();
        pump(&shared, 0, 200, true);
        send(&shared, 100, [0x90, 60, 100]);
        send(&shared, 300, [0x80, 60, 0]);
        send(&shared, 350, [0x90, 67, 80]);
        pump(&shared, 200, 500, true);
        let result = result(&shared, vec![route(1, None)], None);
        let notes = &result.tracks[0].notes;
        near(notes[0].start, 100.0 / 24000.0);
        near(notes[0].length, 200.0 / 24000.0);
        near(notes[1].length, 150.0 / 24000.0);
    }
    #[test]
    fn stop_drains_late_input_without_another_output_callback_and_excludes_the_boundary() {
        let shared = shared();
        pump(&shared, 100, 600, true);
        // Delivered after the last output callback, with their original clock.
        send(&shared, 50, [0x90, 48, 100]); // before capture
        send(&shared, 200, [0x90, 60, 100]);
        send(&shared, 300, [0x80, 60, 0]);
        send(&shared, 450, [0x90, 67, 80]); // held at Stop
        send(&shared, 600, [0x90, 72, 90]); // exactly at the excluded end
        let result = result(&shared, vec![route(1, None)], None);
        let track = &result.tracks[0];
        assert_eq!(track.notes.len(), 2);
        near(track.start_beat, 100.0 / 24000.0);
        near(track.length_beats, 500.0 / 24000.0);
        assert_eq!(track.notes[0].pitch, 60);
        near(track.notes[0].start, 100.0 / 24000.0);
        near(track.notes[0].length, 100.0 / 24000.0);
        assert_eq!(track.notes[1].pitch, 67);
        near(track.notes[1].start, 350.0 / 24000.0);
        near(track.notes[1].length, 150.0 / 24000.0);
        assert_eq!(shared.incoming.pending().count(), 5);
        assert_eq!(shared.events.load(Ordering::Relaxed), 3);
    }
    #[test]
    fn stop_retains_delayed_events_behind_a_future_event_without_reordering_monitor_input() {
        let shared = shared();
        pump(&shared, 0, 500, true);
        send(&shared, 700, [0x90, 72, 100]);
        send(&shared, 200, [0x90, 60, 90]);
        send(&shared, 300, [0x80, 60, 0]);
        let result = result(&shared, vec![route(1, None)], None);
        assert_eq!(result.tracks[0].notes.len(), 1);
        near(result.tracks[0].notes[0].start, 200.0 / 24000.0);
        near(result.tracks[0].notes[0].length, 100.0 / 24000.0);
        assert_eq!(shared.incoming.peek().unwrap().time, at(700));
        shared.observe_frame(None, RATE, at(700), true);
        assert!(shared.recorded.pop().is_none());
        assert_eq!(shared.monitoring.pending().count(), 3);
    }
    #[test]
    fn callback_after_stop_retains_a_delayed_note_off_without_extending_capture() {
        let shared = shared();
        send(&shared, 10, [0x90, 60, 100]);
        pump(&shared, 0, 500, true);
        shared.requested.store(false, Ordering::SeqCst);
        send(&shared, 200, [0x80, 60, 0]);
        send(&shared, 600, [0x90, 72, 100]);
        shared.observe_frame(None, RATE, at(700), true);
        let result = result(&shared, vec![route(1, None)], None);
        let track = &result.tracks[0];
        assert_eq!(track.notes.len(), 1);
        near(track.length_beats, 500.0 / 24000.0);
        near(track.notes[0].start, 10.0 / 24000.0);
        near(track.notes[0].length, 190.0 / 24000.0);
        assert_eq!(shared.frames.load(Ordering::Acquire), 500);
    }
    #[test]
    fn stop_consumer_handoff_never_waits_or_pops_concurrently_with_a_callback() {
        let shared = shared();
        pump(&shared, 0, 500, true);
        send(&shared, 100, [0x90, 60, 100]);
        shared.requested.store(false, Ordering::SeqCst);
        let callback = shared.claim_observer().unwrap();
        let mut events = Vec::new();
        assert!(shared.finish_capture(&mut events).is_none());
        assert_eq!(shared.incoming.read.load(Ordering::Acquire), 0);
        drop(callback);
        // The converse: an output callback never blocks on UI finalization.
        let finalizer = shared.claim_observer().unwrap();
        let allocations = crate::allocation_test_support::count_allocations(|| {
            shared.observe_frame(None, RATE, at(600), true);
        });
        assert_eq!(allocations, 0);
        assert_eq!(shared.incoming.read.load(Ordering::Acquire), 0);
        drop(finalizer);
        let result = result(&shared, vec![route(1, None)], None);
        assert_eq!(result.tracks[0].notes.len(), 1);
        near(result.tracks[0].notes[0].length, 400.0 / 24000.0);
    }
    #[test]
    fn held_notes_split_at_loop_boundaries_and_every_pass_is_retained() {
        let shared = shared();
        send(&shared, 100, [0x90, 60, 100]);
        send(&shared, 1400, [0x80, 60, 0]);
        pump(&shared, 0, 1500, true);
        let result = result(&shared, vec![route(1, None)], Some((0, 600)));
        let recorded = &result.tracks[0];
        assert_eq!(recorded.notes.len(), 3);
        near(recorded.start_beat, 0.0);
        near(recorded.length_beats, 600.0 / 24000.0);
        let total: f64 = recorded.notes.iter().map(|note| note.length).sum();
        near(total, 1300.0 / 24000.0);
        assert!(
            recorded
                .notes
                .iter()
                .all(|note| note.start + note.length <= recorded.length_beats + 1e-10)
        );
    }
    #[test]
    fn callback_capture_and_overflow_paths_allocate_nothing() {
        let shared = shared();
        let data = message(&[0x90, 60, 100]).unwrap();
        let allocations = crate::allocation_test_support::count_allocations(|| {
            shared.enqueue(data, at(0), 1);
            shared.observe_frame(Some(0), RATE, at(0), true);
            for _ in 0..=CAPACITY {
                shared.enqueue(data, at(1), 1);
            }
        });
        assert_eq!(allocations, 0);
        assert_eq!(shared.fault.load(Ordering::Acquire), 1);
        assert!(!shared.capture_requested());
        assert!(shared.recorded.pop().is_some());
    }
    #[test]
    fn driver_timestamps_preserve_intervals_through_callback_jitter_and_recover_clock_resets() {
        let mut clock = MidiTimestampClock::default();
        assert_eq!(clock.map(1000, 1_000_000_000), 1_000_000_000);
        assert_eq!(clock.map(2000, 1_005_000_000), 1_001_000_000);
        assert_eq!(clock.map(100, 1_010_000_000), 1_010_000_000);
        assert_eq!(clock.map(1100, 1_014_000_000), 1_011_000_000);
        assert_eq!(clock.map(100_000_000, 1_020_000_000), 1_020_000_000);
    }
    #[test]
    fn disabled_input_observer_is_idle_and_output_failure_preserves_capture() {
        let disabled = MidiShared::new();
        disabled.observe_frame(Some(0), RATE, at(0), true);
        assert_eq!(disabled.next_output_ns.load(Ordering::Relaxed), 0);
        let shared = shared();
        send(&shared, 0, [0x90, 60, 100]);
        pump(&shared, 0, 100, true);
        shared.output_failed();
        assert!(!shared.capture_requested());
        assert!(shared.fault_text().unwrap().contains("output"));
        assert_eq!(
            result(&shared, vec![route(1, None)], None).tracks[0]
                .notes
                .len(),
            1
        );
    }
    #[test]
    fn unsupported_messages_do_not_turn_into_notes_and_epoch_discards_retired_device_events() {
        assert!(message(&[0x90, 60, 128]).is_none());
        assert!(message(&[0xf8]).is_none());
        assert!(message(&[0xb0, 7, 100]).is_none());
        assert!(message(&[0xe0, 0, 64]).is_none());
        let shared = shared();
        send(&shared, 0, [0x90, 60, 100]);
        shared.epoch.store(2, Ordering::Release);
        pump(&shared, 0, 10, true);
        assert!(
            result(&shared, vec![route(1, None)], None)
                .tracks
                .is_empty()
        );
    }
}
