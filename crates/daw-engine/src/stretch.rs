//! Fixed-rate, pitch-preserving source preparation. No DSP runs on playback.
//! Signalsmith's documented seek/process/flush latency contract is applied in
//! bounded chunks so cancellation does not wait for an entire source file.
use crate::quality::MAX_ASSET_BYTES;
use anyhow::{Context, Result, ensure};
use daw_core::{AudioData, MAX_SOURCE_RATE, MIN_SOURCE_RATE};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
};

unsafe extern "C" {
    fn rill_stretch_create(channels: u32, sample_rate: u32) -> *mut c_void;
    fn rill_stretch_destroy(handle: *mut c_void);
    fn rill_stretch_input_latency(handle: *mut c_void) -> u32;
    fn rill_stretch_output_latency(handle: *mut c_void) -> u32;
    fn rill_stretch_seek(handle: *mut c_void, input: *const f32, frames: u32, rate: f64) -> i32;
    fn rill_stretch_process(
        handle: *mut c_void,
        input: *const f32,
        input_frames: u32,
        output: *mut f32,
        output_frames: u32,
    ) -> i32;
    fn rill_stretch_flush(handle: *mut c_void, output: *mut f32, frames: u32) -> i32;
}

struct Stretch(NonNull<c_void>);
impl Drop for Stretch {
    fn drop(&mut self) {
        // SAFETY: this owner receives the only non-null pointer from create.
        unsafe { rill_stretch_destroy(self.0.as_ptr()) };
    }
}

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<()> {
    ensure!(
        !cancel.is_some_and(|flag| flag.load(Ordering::Acquire)),
        "Audio stretch cancelled"
    );
    Ok(())
}

pub(crate) fn prepare_stretched(
    source: &AudioData,
    rate: f64,
    cancel: Option<&AtomicBool>,
) -> Result<AudioData> {
    check_cancelled(cancel)?;
    source.validate()?;
    ensure!(
        rate.is_finite() && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate),
        "Invalid stretch rate"
    );
    ensure!(
        (1..=2).contains(&source.channels) && (8000..=384000).contains(&source.sample_rate),
        "Invalid stretch source format"
    );
    if rate == 1.0 || source.frames() == 0 {
        return Ok(source.clone());
    }
    let frames = (source.frames() as f64 / rate).ceil() as usize;
    let channels = source.channels;
    let samples = frames
        .checked_mul(channels)
        .context("Stretch size overflow")?;
    ensure!(
        samples <= MAX_ASSET_BYTES / size_of::<f32>(),
        "Stretched audio exceeds the 512 MiB asset limit"
    );
    let mut output = Vec::new();
    output
        .try_reserve_exact(samples)
        .context("Allocate stretched source")?;
    output.resize(samples, 0.0);
    // SAFETY: validated mono/stereo and supported sample rate; C++ contains all exceptions.
    let state = Stretch(
        NonNull::new(unsafe { rill_stretch_create(channels as u32, source.sample_rate) })
            .context("Initialize audio stretcher")?,
    );
    let handle = state.0.as_ptr();
    // SAFETY: handle is alive and uniquely owned for this entire preparation.
    let (input_latency, output_latency) = unsafe {
        (
            rill_stretch_input_latency(handle) as usize,
            rill_stretch_output_latency(handle) as usize,
        )
    };
    ensure!(
        input_latency <= 131072 && output_latency <= 131072,
        "Stretch latency exceeds preparation bounds"
    );
    let mut input = vec![0.0; input_latency.max(16384) * channels];
    let preload = source.frames().min(input_latency) * channels;
    input[..preload].copy_from_slice(&source.samples[..preload]);
    // SAFETY: input contains input_latency complete interleaved frames.
    ensure!(
        unsafe { rill_stretch_seek(handle, input.as_ptr(), input_latency as u32, rate) } != 0,
        "Prepare audio stretch pre-roll"
    );
    let mut block = vec![0.0; output_latency.max(4096) * channels];
    let mut consumed = 0;
    let mut produced = 0;
    while produced < frames {
        check_cancelled(cancel)?;
        let count = (frames - produced).min(4096);
        let next_consumed =
            (((produced + count) as f64 * rate).round() as usize).min(source.frames());
        let input_frames = next_consumed - consumed;
        ensure!(
            input_frames <= 16384,
            "Stretch block exceeds preparation bounds"
        );
        input[..input_frames * channels].fill(0.0);
        let start = (input_latency + consumed).min(source.frames());
        let copy_frames = input_frames.min(source.frames() - start);
        input[..copy_frames * channels]
            .copy_from_slice(&source.samples[start * channels..(start + copy_frames) * channels]);
        // SAFETY: disjoint input/output allocations contain the stated complete
        // frames; the fixed capacities cover every ratio accepted above.
        ensure!(
            unsafe {
                rill_stretch_process(
                    handle,
                    input.as_ptr(),
                    input_frames as u32,
                    block.as_mut_ptr(),
                    count as u32,
                )
            } != 0,
            "Prepare stretched audio block"
        );
        copy_output(
            &block[..count * channels],
            produced,
            output_latency,
            channels,
            &mut output,
        );
        produced += count;
        consumed = next_consumed;
    }
    check_cancelled(cancel)?;
    // SAFETY: block has output_latency complete interleaved frames; handle lives.
    ensure!(
        unsafe { rill_stretch_flush(handle, block.as_mut_ptr(), output_latency as u32) } != 0,
        "Finish audio stretch tail"
    );
    copy_output(
        &block[..output_latency * channels],
        produced,
        output_latency,
        channels,
        &mut output,
    );
    let result = AudioData {
        sample_rate: source.sample_rate,
        channels,
        samples: output,
    };
    result.validate()?;
    check_cancelled(cancel)?;
    Ok(result)
}

fn copy_output(
    block: &[f32],
    produced: usize,
    latency: usize,
    channels: usize,
    output: &mut [f32],
) {
    let skip = latency.saturating_sub(produced).min(block.len() / channels);
    let destination = produced.saturating_sub(latency) * channels;
    if destination < output.len() {
        let count = (block.len() - skip * channels).min(output.len() - destination);
        output[destination..destination + count]
            .copy_from_slice(&block[skip * channels..skip * channels + count]);
    }
}

#[cfg(test)]
#[path = "stretch_tests.rs"]
mod tests;
