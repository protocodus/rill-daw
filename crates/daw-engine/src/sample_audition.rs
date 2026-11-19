//! A bounded audition lane on the existing render worker and output stream.
//! Its clock and generation are independent of song playback and MIDI preview.
use super::{AudioQueue, BLOCK};
use daw_devices::SampleData;
use daw_devices::dsp::{SincBank, Step};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub(super) struct Shared {
    queue: AudioQueue,
    command: Mutex<Option<(u64, Option<Arc<SampleData>>)>>,
    epoch: AtomicU64,
    active: AtomicU64,
    finished: AtomicU64,
    // Only the output callback writes these. Atomic storage keeps the shared
    // controls safe without putting a lock or an allocation in that callback.
    output_epoch: AtomicU64,
    last_output: AtomicU64,
    fade_from: AtomicU64,
    fade_left: AtomicU64,
}

impl Shared {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            queue: AudioQueue::new(capacity),
            command: Mutex::new(None),
            epoch: AtomicU64::new(0),
            active: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            output_epoch: AtomicU64::new(0),
            last_output: AtomicU64::new(0),
            fade_from: AtomicU64::new(0),
            fade_left: AtomicU64::new(0),
        }
    }

    pub(super) fn request(&self, sample: Option<Arc<SampleData>>) {
        let mut command = self
            .command
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        self.active
            .store(if sample.is_some() { epoch } else { 0 }, Ordering::Release);
        *command = Some((epoch, sample));
    }

    pub(super) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire) != 0
    }

    /// Fatal output failure and engine shutdown invalidate pending work without
    /// waiting for the control thread or dropping source buffers in a callback.
    pub(super) fn abort(&self) {
        self.active.store(0, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.last_output.store(0, Ordering::Relaxed);
        self.fade_from.store(0, Ordering::Relaxed);
        self.fade_left.store(0, Ordering::Relaxed);
    }

    pub(super) fn ready_for(&self, count: usize) -> bool {
        let epoch = self.active.load(Ordering::Acquire);
        if epoch == 0 {
            return true;
        }
        self.queue.discard_before(epoch);
        self.queue.slots.len() - self.queue.free() >= count
            || self.finished.load(Ordering::Acquire) == epoch
    }

    /// A short replacement/stop ramp avoids discontinuities while preserving
    /// the source onset. No source objects are read or destroyed here.
    pub(super) fn consume(&self, sample_rate: u32) -> [f32; 2] {
        let epoch = self.epoch.load(Ordering::Acquire);
        if epoch == 0 {
            return [0.0; 2];
        }
        if self.output_epoch.load(Ordering::Relaxed) != epoch {
            self.output_epoch.store(epoch, Ordering::Relaxed);
            self.fade_from
                .store(self.last_output.load(Ordering::Relaxed), Ordering::Relaxed);
            self.fade_left
                .store(u64::from(sample_rate.max(1000)) / 1000, Ordering::Relaxed);
        }
        // A worker block already in flight can arrive after the epoch change.
        // Stale frames are discarded by the sole consumer before every read.
        self.queue.discard_before(epoch);
        let mut output = [0.0; 2];
        if self.active.load(Ordering::Acquire) == epoch
            && let Some((sample, final_frame, generation)) = self.queue.pop_generation(epoch)
            && generation == epoch
        {
            output = sample;
            if final_frame > 0.0 {
                let _ = self
                    .active
                    .compare_exchange(epoch, 0, Ordering::AcqRel, Ordering::Relaxed);
            }
        }
        let remaining = self.fade_left.load(Ordering::Relaxed);
        if remaining > 0 {
            let from = unpack(self.fade_from.load(Ordering::Relaxed));
            let old = remaining as f32 / (sample_rate.max(1000) as f32 / 1000.0);
            for channel in 0..2 {
                output[channel] = from[channel] * old + output[channel] * (1.0 - old);
            }
            self.fade_left.store(remaining - 1, Ordering::Relaxed);
        }
        self.last_output.store(pack(output), Ordering::Relaxed);
        output
    }
}

fn pack(sample: [f32; 2]) -> u64 {
    u64::from(sample[0].to_bits()) | (u64::from(sample[1].to_bits()) << 32)
}
fn unpack(value: u64) -> [f32; 2] {
    [
        f32::from_bits(value as u32),
        f32::from_bits((value >> 32) as u32),
    ]
}

struct Playing {
    sample: Arc<SampleData>,
    epoch: u64,
    position: f64,
    step: f64,
    reader: Step,
}

pub(super) struct Worker {
    playing: Option<Playing>,
    bank: &'static SincBank,
    sample_rate: u32,
}

impl Worker {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            playing: None,
            bank: SincBank::shared(),
            sample_rate,
        }
    }

    /// Runs even when the song's queue is full or its transport is paused.
    pub(super) fn fill(&mut self, shared: &Shared) {
        if let Ok(mut command) = shared.command.try_lock()
            && let Some((epoch, sample)) = command.take()
        {
            self.playing = sample
                .filter(|_| shared.active.load(Ordering::Acquire) == epoch)
                .map(|sample| {
                    let step = f64::from(sample.sample_rate()) / f64::from(self.sample_rate);
                    Playing {
                        sample,
                        epoch,
                        position: 0.0,
                        step,
                        reader: Step::new(step as f32),
                    }
                });
        }
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| shared.active.load(Ordering::Acquire) != playing.epoch)
        {
            self.playing = None;
        }
        let Some(playing) = &mut self.playing else {
            return;
        };
        if shared.queue.free() < BLOCK {
            return;
        }
        let mut frames = [[0.0; 2]; BLOCK];
        let mut final_frames = [0.0; BLOCK];
        let mut count = 0;
        while count < BLOCK && playing.position < playing.sample.frames() as f64 {
            let left = playing.sample.channel(0);
            let tap = left.tap(self.bank, &playing.reader, playing.position);
            frames[count][0] = left.read(&tap);
            frames[count][1] = if playing.sample.channel_count() > 1 {
                playing.sample.channel(1).read(&tap)
            } else {
                frames[count][0]
            };
            let remaining = (playing.sample.frames() as f64 - playing.position) / playing.step;
            let fade = (remaining / (f64::from(self.sample_rate) * 0.001)).min(1.0) as f32;
            for sample in &mut frames[count] {
                *sample *= fade;
            }
            playing.position += playing.step;
            count += 1;
        }
        let finished = playing.position >= playing.sample.frames() as f64;
        if finished && count > 0 {
            final_frames[count - 1] = 1.0;
        }
        shared
            .queue
            .push_block(&frames[..count], &final_frames[..count], playing.epoch);
        if finished {
            shared.finished.store(playing.epoch, Ordering::Release);
            self.playing = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AudioEngine, Controls, DeviceSettings, fill_output, output_stream_failed};
    use super::*;
    use daw_core::{AudioData, Clip, Project};
    use std::sync::atomic::AtomicBool;

    fn source(rate: u32, frames: usize, value: impl Fn(usize) -> f32) -> Arc<SampleData> {
        let samples: Vec<_> = (0..frames).map(value).collect();
        Arc::new(SampleData::new(rate as f32, &[&samples]))
    }

    fn engine(project: &Project) -> AudioEngine {
        AudioEngine::start_virtual_cancellable(
            project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    #[test]
    fn sample_audition_preserves_replacement_frames_for_a_stale_consumer_epoch() {
        let shared = Shared::new(8);
        shared.epoch.store(4, Ordering::Release);
        shared.active.store(4, Ordering::Release);
        shared.output_epoch.store(4, Ordering::Relaxed);
        shared.queue.push_block(&[[0.1; 2]], &[0.0], 3);
        // Model replacement publication between the consumer's epoch snapshot
        // and its queue read. The old pass may discard 3, but must preserve 5.
        shared.queue.push_block(&[[0.5; 2], [0.6; 2]], &[0.0; 2], 5);
        assert!(shared.ready_for(2));
        assert_eq!(shared.queue.free(), 6);
        assert_eq!(shared.consume(48_000), [0.0; 2]);
        assert_eq!(shared.queue.free(), 6);

        shared.epoch.store(5, Ordering::Release);
        shared.active.store(5, Ordering::Release);
        shared.output_epoch.store(5, Ordering::Relaxed);
        assert_eq!(shared.consume(48_000), [0.5; 2]);
        assert_eq!(shared.consume(48_000), [0.6; 2]);
    }

    #[test]
    fn sample_audition_queue_generation_order_wraps_without_dropping_future_frames() {
        let queue = AudioQueue::new(8);
        queue.push_block(&[[0.1; 2]], &[0.0], u64::MAX);
        queue.push_block(&[[0.2; 2]], &[0.0], 0);
        queue.discard_before(u64::MAX);
        assert_eq!(queue.pop_generation(u64::MAX).unwrap().0, [0.1; 2]);
        queue.discard_before(u64::MAX);
        assert!(queue.pop_generation(u64::MAX).is_none());
        assert_eq!(queue.pop_generation(0).unwrap().0, [0.2; 2]);
    }

    #[test]
    fn sample_audition_plays_to_completion_at_source_pitch_without_moving_paused_transport() {
        let mut engine = engine(&Project::new());
        engine.seek(7.0);
        let sample = source(24_000, 2400, |frame| {
            0.4 * (std::f32::consts::TAU * 400.0 * frame as f32 / 24_000.0).sin()
        });
        engine.audition_sample(Some(sample)).unwrap();
        assert!(engine.sample_audition_active());
        let output = engine.pump_virtual(6000).unwrap();
        assert_eq!(engine.snapshot().position_beats, 7.0);
        assert!(!engine.snapshot().playing);
        assert!(!engine.sample_audition_active());
        let crossings = output[480..4320]
            .windows(2)
            .filter(|pair| pair[0][0] < 0.0 && pair[1][0] >= 0.0)
            .count();
        assert!(
            (crossings as i32 - 32).abs() <= 1,
            "source pitch survives rate conversion"
        );
        assert!(output[5000..].iter().all(|frame| *frame == [0.0; 2]));
        assert!(engine.output_peak_hold()[0] > 0.39);
        assert_eq!(
            engine.snapshot().consumed_frames,
            0,
            "song queue is untouched"
        );
    }

    #[test]
    fn sample_audition_replaces_and_stops_without_stale_audio_or_transport_changes() {
        let mut engine = engine(&Project::new());
        engine
            .audition_sample(Some(source(48_000, 48_000, |_| 0.2)))
            .unwrap();
        let first = engine.pump_virtual(512).unwrap();
        assert!((first[300][0] - 0.2).abs() < 1e-5);
        engine
            .audition_sample(Some(source(48_000, 48_000, |_| -0.3)))
            .unwrap();
        let replacement = engine.pump_virtual(512).unwrap();
        assert!((replacement[300][0] + 0.3).abs() < 1e-5);
        assert!(
            replacement
                .windows(2)
                .all(|pair| (pair[1][0] - pair[0][0]).abs() < 0.02)
        );
        engine.audition_sample(None).unwrap();
        assert!(!engine.sample_audition_active());
        let stopped = engine.pump_virtual(512).unwrap();
        assert!(stopped[64..].iter().all(|frame| *frame == [0.0; 2]));
        assert!(!engine.snapshot().playing);
        assert_eq!(engine.snapshot().position_beats, 0.0);
    }

    #[test]
    fn sample_audition_mixes_with_playback_but_not_the_project_master_meter() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("song.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.2; 48_000],
        }
        .write_wav(&path)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Song");
        project.track_mut(track).unwrap().gain = 1.0;
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Song",
            0.0,
            2.0,
            path.to_string_lossy(),
        ));
        let mut engine = engine(&project);
        engine.set_playing(true);
        engine.pump_virtual(512).unwrap();
        let before = engine.snapshot();
        engine
            .audition_sample(Some(source(48_000, 4800, |_| 0.3)))
            .unwrap();
        let mixed = engine.pump_virtual(1024).unwrap();
        assert!((mixed[500][0] - 0.5).abs() < 1e-5);
        assert_eq!(engine.snapshot().generation, before.generation);
        assert!(engine.snapshot().position_beats > before.position_beats);
        assert!((engine.channel_peaks(track)[0] - 0.2).abs() < 1e-5);
        assert!(engine.output_peak_hold()[0] > 0.49);
        engine.set_playing(false);
        let position = engine.snapshot().position_beats;
        let paused = engine.pump_virtual(512).unwrap();
        assert!(
            (paused[300][0] - 0.3).abs() < 1e-5,
            "pausing the song leaves audition running"
        );
        assert_eq!(engine.snapshot().position_beats, position);
    }

    #[test]
    fn sample_audition_callback_allocates_nothing_and_preserves_note_preview() {
        let controls = Controls::new();
        controls.render_preview.store(true, Ordering::Release);
        controls.auditioning.store(true, Ordering::Release);
        let queue = AudioQueue::new(BLOCK * 2);
        queue.push_block(&[[0.2, 0.1]; BLOCK], &[0.0; BLOCK], 0);
        controls
            .sample_audition
            .request(Some(source(48_000, 4096, |_| 0.3)));
        let mut worker = Worker::new(48_000);
        worker.fill(&controls.sample_audition);
        let mut output = [0.0_f32; BLOCK * 2];
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| fill_output(
                &mut output,
                2,
                &queue,
                &controls
            )),
            0
        );
        assert!((output[400] - 0.5).abs() < 1e-5);
        assert!(controls.auditioning.load(Ordering::Acquire));
        assert_eq!(controls.position.load(Ordering::Acquire), 0);
    }

    #[test]
    fn sample_audition_cannot_resume_after_output_failure_or_engine_stop() {
        let mut engine = engine(&Project::new());
        engine
            .audition_sample(Some(source(48_000, 48_000, |_| 0.3)))
            .unwrap();
        engine.pump_virtual(512).unwrap();
        output_stream_failed(
            &engine.controls,
            &cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable),
        );
        assert!(!engine.sample_audition_active());
        assert!(
            engine
                .pump_virtual(512)
                .unwrap()
                .iter()
                .all(|frame| *frame == [0.0; 2])
        );
        assert!(
            engine
                .audition_sample(Some(source(48_000, 10, |_| 0.3)))
                .is_err()
        );
        engine.stop();
        assert!(!engine.sample_audition_active());
    }
}
