//! Disk-backed preparation uses the same continuous DSP history as the resident
//! path. Only fixed-size input windows and output blocks live in memory.
use super::{Stretch, TimeMap, check_cancelled};
use crate::audio_asset::{AudioAsset, TempPathLease};
use anyhow::{Context, Result, ensure};
use daw_core::{
    AUDIO_FILE_READ_MAX_BYTES, AudioFile, AudioFileReader, AudioProcessingAlgorithm,
    MAX_SOURCE_RATE, MAX_STREAMED_AUDIO_BYTES, MIN_SOURCE_RATE,
};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io::BufWriter,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    sync::{Arc, atomic::AtomicBool},
};

// Float WAV adds a format extension/fact chunk. Keep room for all RIFF metadata,
// and never rely on Hound's final u32 header arithmetic to discover overflow.
const MAX_FLOAT_WAV_BYTES: u64 = MAX_STREAMED_AUDIO_BYTES;
const BLOCK_FRAMES: usize = 4096;
const MAX_NATIVE_GRAIN_FRAMES: usize = 1_048_576;

/// A private, leased file becomes an asset only after exact-length finalization.
/// Cancellation or any I/O/DSP error drops the lease and removes partial output.
pub(crate) struct StreamWriter<'a> {
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    lease: Arc<TempPathLease>,
    channels: usize,
    sample_rate: u32,
    frames: usize,
    written: usize,
    cancel: Option<&'a AtomicBool>,
}

impl<'a> StreamWriter<'a> {
    pub(crate) fn new(
        sample_rate: u32,
        channels: usize,
        frames: usize,
        cancel: Option<&'a AtomicBool>,
    ) -> Result<Self> {
        check_cancelled(cancel)?;
        ensure!(
            (8000..=384000).contains(&sample_rate) && (1..=2).contains(&channels),
            "Invalid prepared audio format"
        );
        let bytes = (frames as u128) * channels as u128 * size_of::<f32>() as u128;
        ensure!(
            bytes <= u128::from(MAX_FLOAT_WAV_BYTES),
            "Prepared float WAV exceeds the 4 GiB RIFF file limit"
        );
        let lease = TempPathLease::new()?;
        let writer = hound::WavWriter::new(
            BufWriter::new(File::create(lease.path()).context("Create prepared audio file")?),
            hound::WavSpec {
                channels: channels as u16,
                sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .context("Write prepared audio header")?;
        Ok(Self {
            writer: Some(writer),
            lease,
            channels,
            sample_rate,
            frames,
            written: 0,
            cancel,
        })
    }

    pub(crate) fn remaining_frames(&self) -> usize {
        self.frames - self.written
    }

    pub(crate) fn write(&mut self, samples: &[f32]) -> Result<()> {
        check_cancelled(self.cancel)?;
        ensure!(
            samples.len().is_multiple_of(self.channels),
            "Prepared audio block has an incomplete frame"
        );
        let frames = samples.len() / self.channels;
        ensure!(
            frames <= self.remaining_frames(),
            "Prepared audio exceeds its declared frame count"
        );
        ensure!(
            samples.iter().all(|sample| sample.is_finite()),
            "Prepared audio contains non-finite samples"
        );
        let writer = self
            .writer
            .as_mut()
            .context("Prepared audio writer is closed")?;
        for &sample in samples {
            writer
                .write_sample(sample)
                .context("Write prepared audio samples")?;
        }
        self.written += frames;
        check_cancelled(self.cancel)
    }

    pub(crate) fn finish(mut self) -> Result<Arc<AudioAsset>> {
        check_cancelled(self.cancel)?;
        ensure!(
            self.written == self.frames,
            "Prepared audio returned too few frames"
        );
        self.writer
            .take()
            .context("Prepared audio writer is closed")?
            .finalize()
            .context("Finalize prepared audio")?;
        OpenOptions::new()
            .write(true)
            .open(self.lease.path())?
            .sync_all()
            .context("Synchronize prepared audio")?;
        check_cancelled(self.cancel)?;
        let file = AudioFile::open(self.lease.path()).context("Validate prepared audio file")?;
        let info = file.info();
        ensure!(
            info.frames == self.frames
                && info.channels == self.channels
                && info.sample_rate == self.sample_rate,
            "Prepared audio metadata changed during finalization"
        );
        check_cancelled(self.cancel)?;
        Ok(AudioAsset::from_prepared_file(
            file,
            Arc::clone(&self.lease),
        ))
    }
}

#[cfg(test)]
pub(crate) fn prepare_file(
    source: &AudioFile,
    rate: f64,
    semitones: f64,
    algorithm: AudioProcessingAlgorithm,
    cancel: Option<&AtomicBool>,
) -> Result<Arc<AudioAsset>> {
    prepare_file_mapped(
        source,
        &TimeMap::constant(rate),
        semitones,
        algorithm,
        cancel,
    )
}

pub(crate) fn prepare_file_mapped(
    source: &AudioFile,
    map: &TimeMap,
    semitones: f64,
    algorithm: AudioProcessingAlgorithm,
    cancel: Option<&AtomicBool>,
) -> Result<Arc<AudioAsset>> {
    check_cancelled(cancel)?;
    map.validate()?;
    let rate = map.rate();
    ensure!(
        rate.is_finite() && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate),
        "Invalid stretch rate"
    );
    ensure!(
        semitones.is_finite() && (-24.0..=24.0).contains(&semitones),
        "Audio transpose must be between -24 and +24 semitones"
    );
    let info = source.info();
    let frames = map.output_frames(info.frames);
    let mut writer = StreamWriter::new(info.sample_rate, info.channels, frames, cancel)?;
    let mut reader = source.reader()?;
    if frames == 0 {
        source.check_unchanged()?;
        return writer.finish();
    }
    if map.is_identity() && semitones == 0.0 {
        let mut buffer = vec![0.0; BLOCK_FRAMES * info.channels];
        let mut offset = 0;
        while offset < info.frames {
            check_cancelled(cancel)?;
            let count = (info.frames - offset).min(BLOCK_FRAMES);
            read_exact(
                &mut reader,
                offset,
                &mut buffer[..count * info.channels],
                count,
            )?;
            writer.write(&buffer[..count * info.channels])?;
            offset += count;
        }
        source.check_unchanged()?;
        return writer.finish();
    }
    match algorithm {
        AudioProcessingAlgorithm::Signalsmith => prepare_signalsmith(
            &mut reader,
            info.frames,
            info.channels,
            info.sample_rate,
            frames,
            map,
            semitones,
            &mut writer,
            cancel,
        )?,
        AudioProcessingAlgorithm::Bungee => {
            let mut io = NativeIo {
                reader,
                writer,
                channels: info.channels,
                cancel,
                error: None,
            };
            // SAFETY: the synchronous bridge invokes callbacks sequentially with
            // this live context. All native windows have a fixed admitted bound.
            let knots = map.flat_knots();
            let status = unsafe {
                rill_bungee_process_stream(
                    info.frames,
                    frames,
                    info.channels as u32,
                    info.sample_rate,
                    rate,
                    knots.as_ptr(),
                    map.knots.len(),
                    semitones,
                    std::ptr::from_mut(&mut io).cast(),
                    native_read,
                    native_write,
                    native_cancelled,
                )
            };
            check_cancelled(cancel)?;
            if let Some(error) = io.error {
                return Err(error);
            }
            ensure!(status == 1, "Bungee disk audio preparation failed");
            source.check_unchanged()?;
            return io.writer.finish();
        }
    }
    source.check_unchanged()?;
    writer.finish()
}

fn read_exact(
    reader: &mut AudioFileReader,
    start: usize,
    buffer: &mut [f32],
    frames: usize,
) -> Result<()> {
    let channels = reader.info().channels;
    ensure!(
        buffer.len() == frames * channels,
        "Invalid stretch read frame count"
    );
    let window = AUDIO_FILE_READ_MAX_BYTES / (channels * size_of::<f32>());
    for (index, block) in buffer.chunks_mut(window * channels).enumerate() {
        ensure!(
            reader.read_frames(start + index * window, block)? == block.len() / channels,
            "Audio source ended during preparation"
        );
    }
    ensure!(
        buffer.iter().all(|sample| sample.is_finite()),
        "Audio source contains non-finite samples"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_signalsmith(
    reader: &mut AudioFileReader,
    source_frames: usize,
    channels: usize,
    sample_rate: u32,
    frames: usize,
    map: &TimeMap,
    semitones: f64,
    writer: &mut StreamWriter<'_>,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    // SAFETY: the writer has admitted mono/stereo and the sample rate; the
    // exception-contained bridge owns one deterministic native DSP instance.
    let state = Stretch(
        NonNull::new(unsafe {
            super::rill_stretch_create(channels as u32, sample_rate, semitones)
        })
        .context("Initialize disk audio stretcher")?,
    );
    let handle = state.0.as_ptr();
    // SAFETY: handle is uniquely owned and alive throughout this operation.
    let (input_latency, output_latency) = unsafe {
        (
            super::rill_stretch_input_latency(handle) as usize,
            super::rill_stretch_output_latency(handle) as usize,
        )
    };
    ensure!(
        input_latency <= 131072 && output_latency <= 131072,
        "Stretch latency exceeds preparation bounds"
    );
    let mut input = vec![0.0; input_latency.max(16384) * channels];
    let preload = source_frames.min(input_latency);
    read_exact(reader, 0, &mut input[..preload * channels], preload)?;
    check_cancelled(cancel)?;
    // SAFETY: complete interleaved frames, including zero padding to latency.
    ensure!(
        unsafe {
            super::rill_stretch_seek(
                handle,
                input.as_ptr(),
                input_latency as u32,
                map.initial_rate(),
            )
        } != 0,
        "Prepare disk audio stretch pre-roll"
    );
    let mut output = vec![0.0; output_latency.max(BLOCK_FRAMES) * channels];
    let (mut consumed, mut produced) = (0, 0);
    while produced < frames {
        check_cancelled(cancel)?;
        let count = (frames - produced).min(BLOCK_FRAMES);
        let next_consumed =
            (map.input_at((produced + count) as f64).round().max(0.0) as usize).min(source_frames);
        let input_frames = next_consumed - consumed;
        ensure!(
            input_frames <= 16384,
            "Stretch block exceeds preparation bounds"
        );
        input[..input_frames * channels].fill(0.0);
        let start = input_latency.saturating_add(consumed).min(source_frames);
        let copy = input_frames.min(source_frames - start);
        read_exact(reader, start, &mut input[..copy * channels], copy)?;
        // SAFETY: fixed bounded allocations contain every stated input/output
        // frame. Native processing sees the same block sequence as resident PCM.
        ensure!(
            unsafe {
                super::rill_stretch_process(
                    handle,
                    input.as_ptr(),
                    input_frames as u32,
                    output.as_mut_ptr(),
                    count as u32,
                )
            } != 0,
            "Prepare disk stretched audio block"
        );
        write_output(
            writer,
            &output[..count * channels],
            produced,
            output_latency,
            channels,
        )?;
        produced += count;
        consumed = next_consumed;
    }
    check_cancelled(cancel)?;
    // SAFETY: output has capacity for the admitted bounded native tail.
    ensure!(
        unsafe { super::rill_stretch_flush(handle, output.as_mut_ptr(), output_latency as u32) }
            != 0,
        "Finish disk audio stretch tail"
    );
    write_output(
        writer,
        &output[..output_latency * channels],
        produced,
        output_latency,
        channels,
    )
}

fn write_output(
    writer: &mut StreamWriter<'_>,
    block: &[f32],
    produced: usize,
    latency: usize,
    channels: usize,
) -> Result<()> {
    let skip = latency.saturating_sub(produced).min(block.len() / channels);
    let count = (block.len() / channels - skip).min(writer.remaining_frames());
    writer.write(&block[skip * channels..(skip + count) * channels])
}

struct NativeIo<'a> {
    reader: AudioFileReader,
    writer: StreamWriter<'a>,
    channels: usize,
    cancel: Option<&'a AtomicBool>,
    error: Option<anyhow::Error>,
}
impl NativeIo<'_> {
    fn callback(&mut self, operation: impl FnOnce(&mut Self) -> Result<()>) -> i32 {
        if self.error.is_some() {
            return 0;
        }
        match catch_unwind(AssertUnwindSafe(|| operation(self))) {
            Ok(Ok(())) => 1,
            Ok(Err(error)) => {
                self.error = Some(error);
                0
            }
            Err(_) => {
                self.error = Some(anyhow::anyhow!("Disk stretch I/O callback panicked"));
                0
            }
        }
    }
}
unsafe extern "C" fn native_read(
    context: *mut c_void,
    start: usize,
    frames: usize,
    output: *mut f32,
) -> i32 {
    // SAFETY: the synchronous native bridge supplies this live unique context.
    let io = unsafe { &mut *context.cast::<NativeIo<'_>>() };
    io.callback(|io| {
        check_cancelled(io.cancel)?;
        ensure!(
            frames <= MAX_NATIVE_GRAIN_FRAMES && !output.is_null(),
            "Invalid native stretch input window"
        );
        // SAFETY: bridge allocated the admitted interleaved window for this call.
        let samples = unsafe { std::slice::from_raw_parts_mut(output, frames * io.channels) };
        read_exact(&mut io.reader, start, samples, frames)
    })
}
unsafe extern "C" fn native_write(context: *mut c_void, input: *const f32, frames: usize) -> i32 {
    // SAFETY: the synchronous native bridge supplies this live unique context.
    let io = unsafe { &mut *context.cast::<NativeIo<'_>>() };
    io.callback(|io| {
        ensure!(
            frames <= MAX_NATIVE_GRAIN_FRAMES && !input.is_null(),
            "Invalid native stretch output block"
        );
        // SAFETY: bridge retains the complete admitted interleaved block.
        io.writer
            .write(unsafe { std::slice::from_raw_parts(input, frames * io.channels) })
    })
}
unsafe extern "C" fn native_cancelled(context: *mut c_void) -> i32 {
    // SAFETY: context is live throughout the synchronous native call.
    let io = unsafe { &*context.cast::<NativeIo<'_>>() };
    i32::from(check_cancelled(io.cancel).is_err() || io.error.is_some())
}
unsafe extern "C" {
    fn rill_bungee_process_stream(
        input_frames: usize,
        output_frames: usize,
        channels: u32,
        sample_rate: u32,
        rate: f64,
        knots: *const f64,
        knot_count: usize,
        semitones: f64,
        context: *mut c_void,
        read: unsafe extern "C" fn(*mut c_void, usize, usize, *mut f32) -> i32,
        write: unsafe extern "C" fn(*mut c_void, *const f32, usize) -> i32,
        cancelled: unsafe extern "C" fn(*mut c_void) -> i32,
    ) -> i32;
}

#[cfg(test)]
#[path = "stretch_file_tests.rs"]
mod tests;
