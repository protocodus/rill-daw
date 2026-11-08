//! Bounded, reusable PCM transport between an isolated host and its helper.
//!
//! The control pipe carries a mapping descriptor once and a generation/frame
//! pair per block. PCM remains native IEEE-754 data in an OS shared mapping.
//! A private temporary file supplies the mapping on Unix, macOS and Windows;
//! it is never resized after creation and the owner retains it until drop.
//! Atomic ownership handoffs separate host writes, helper processing and host
//! reads. A failed/timed-out exchange must poison the host: that mapping can
//! never be reused, even if its old helper is still finishing a block.

use crate::audio::{AudioBusBuffer, AudioBusConfig, AudioBusLayout};
use memmap2::{MmapMut, MmapOptions};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{align_of, size_of};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use tempfile::NamedTempFile;

const MAX_BUSES: usize = 64;
const MAX_CHANNELS: usize = 256;
const MAX_FRAMES: usize = 65_536;
const MAX_PCM_BYTES: usize = 64 * 1024 * 1024;
const MAGIC: u64 = u64::from_le_bytes(*b"RILLPCM1");
const IDLE: u32 = 0;
const INPUT_READY: u32 = 1;
const PROCESSING: u32 = 2;
const OUTPUT_READY: u32 = 3;
const POISONED: u32 = 4;

/// Private mapping identity and fixed bus layout, exchanged once at setup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedAudioDescriptor {
    /// Exclusive temporary file owned by the host for this helper instance.
    pub path: PathBuf,
    /// Identity checked against the mapped header before admitting the file.
    pub mapping_id: u64,
    /// Native input/output bus indices, channel counts and activation flags.
    pub layout: AudioBusLayout,
    /// Preallocated sample capacity of each channel.
    pub max_frames: u32,
}

/// Exact block identity to echo in the helper's successful control response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedAudioBlock {
    /// Monotonically increasing, nonzero sequence within one mapping.
    pub generation: u64,
    /// Number of valid frames in every channel for this exchange.
    pub frames: u32,
}

#[repr(C, align(64))]
struct Header {
    magic: u64,
    mapping_id: u64,
    layout_signature: u64,
    generation: AtomicU64,
    frames: AtomicU32,
    state: AtomicU32,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Validate dimensions before allocation, mapping, or channel-offset arithmetic.
/// Limits are 64 buses per direction, 256 channels in total and 64 MiB of PCM.
pub fn validate_layout(layout: &AudioBusLayout, max_frames: usize) -> io::Result<()> {
    if !(1..=MAX_FRAMES).contains(&max_frames) {
        return Err(invalid("Shared audio frame capacity must be 1..=65536"));
    }
    if layout.inputs.len() > MAX_BUSES || layout.outputs.len() > MAX_BUSES {
        return Err(invalid("Shared audio exceeds 64 buses per direction"));
    }
    let channels = layout
        .inputs
        .iter()
        .chain(&layout.outputs)
        .try_fold(0usize, |count, bus| {
            if bus.channel_count > MAX_CHANNELS {
                return Err(invalid("Shared audio bus exceeds 256 channels"));
            }
            count
                .checked_add(bus.channel_count)
                .ok_or_else(|| invalid("Shared audio channel count overflow"))
        })?;
    if channels > MAX_CHANNELS {
        return Err(invalid("Shared audio exceeds 256 aggregate channels"));
    }
    let bytes = channels
        .checked_mul(max_frames)
        .and_then(|samples| samples.checked_mul(size_of::<f32>()))
        .ok_or_else(|| invalid("Shared audio buffer size overflow"))?;
    if bytes > MAX_PCM_BYTES {
        return Err(invalid("Shared audio exceeds 64 MiB of PCM"));
    }
    Ok(())
}

fn channel_count(buses: &[AudioBusConfig]) -> usize {
    buses.iter().map(|bus| bus.channel_count).sum()
}

fn mapping_size(descriptor: &SharedAudioDescriptor) -> io::Result<usize> {
    validate_layout(&descriptor.layout, descriptor.max_frames as usize)?;
    Ok(size_of::<Header>()
        + (channel_count(&descriptor.layout.inputs) + channel_count(&descriptor.layout.outputs))
            * descriptor.max_frames as usize
            * size_of::<f32>())
}

fn hash_bytes(mut hash: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    for byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn layout_signature(descriptor: &SharedAudioDescriptor) -> u64 {
    let mut hash = hash_bytes(0xcbf2_9ce4_8422_2325, descriptor.max_frames.to_le_bytes());
    for buses in [&descriptor.layout.inputs, &descriptor.layout.outputs] {
        hash = hash_bytes(hash, (buses.len() as u64).to_le_bytes());
        for bus in buses {
            hash = hash_bytes(hash, (bus.channel_count as u64).to_le_bytes());
            hash = hash_bytes(hash, [u8::from(bus.active)]);
        }
    }
    hash
}

fn validate_buffers(
    buses: &[AudioBusBuffer],
    layout: &[AudioBusConfig],
    frames: usize,
) -> io::Result<()> {
    if buses.len() != layout.len() {
        return Err(invalid("Shared audio bus count changed"));
    }
    for (bus, expected) in buses.iter().zip(layout) {
        if bus.active != expected.active || bus.channels.len() != expected.channel_count {
            return Err(invalid("Shared audio channel layout or activation changed"));
        }
        if bus.channels.iter().any(|channel| channel.len() < frames) {
            return Err(invalid("Shared audio channel is shorter than the block"));
        }
    }
    Ok(())
}

fn clear_buffers(buses: &mut [AudioBusBuffer]) {
    for bus in buses {
        for channel in &mut bus.channels {
            channel.fill(0.0);
        }
    }
}

struct Mapping {
    data: MmapMut,
    max_frames: usize,
    input_channels: usize,
    output_channels: usize,
}

impl Mapping {
    fn map(file: &File, descriptor: &SharedAudioDescriptor) -> io::Result<Self> {
        let length = mapping_size(descriptor)?;
        let metadata = file.metadata()?;
        if metadata.len() != length as u64 || !metadata.is_file() {
            return Err(invalid(
                "Shared audio file size or type does not match its layout",
            ));
        }
        // SAFETY: The host exclusively creates the backing file, sets its size
        // once, and retains it. The helper only admits this exact bounded size.
        // Neither endpoint truncates it. PCM access is protected by the atomic
        // ownership protocol below; no mapped references reach native plug-ins.
        let data = unsafe { MmapOptions::new().len(length).map_mut(file)? };
        if data.as_ptr() as usize % align_of::<Header>() != 0 {
            return Err(invalid("Shared audio mapping is not correctly aligned"));
        }
        Ok(Self {
            data,
            max_frames: descriptor.max_frames as usize,
            input_channels: channel_count(&descriptor.layout.inputs),
            output_channels: channel_count(&descriptor.layout.outputs),
        })
    }

    fn header(&self) -> &Header {
        // SAFETY: map() checked length and alignment. Only the owner initializes
        // this header, before publishing its descriptor; mutable fields are
        // atomic and immutable fields stay fixed for the mapping's lifetime.
        unsafe { &*self.data.as_ptr().cast::<Header>() }
    }

    fn initialize(&mut self, descriptor: &SharedAudioDescriptor) {
        // SAFETY: The file is newly created and has not been shared with a helper.
        unsafe {
            self.data.as_mut_ptr().cast::<Header>().write(Header {
                magic: MAGIC,
                mapping_id: descriptor.mapping_id,
                layout_signature: layout_signature(descriptor),
                generation: AtomicU64::new(0),
                frames: AtomicU32::new(0),
                state: AtomicU32::new(IDLE),
            });
        }
        // Touch PCM pages during setup instead of first audio processing.
        self.clear_region(
            0,
            self.input_channels + self.output_channels,
            self.max_frames,
        );
    }

    fn valid_block(&self, block: SharedAudioBlock) -> io::Result<()> {
        if block.generation == 0 || block.frames == 0 || block.frames as usize > self.max_frames {
            return Err(invalid(
                "Invalid shared audio block identity or frame count",
            ));
        }
        Ok(())
    }

    fn matches_block(&self, block: SharedAudioBlock) -> bool {
        self.header().generation.load(Ordering::Relaxed) == block.generation
            && self.header().frames.load(Ordering::Relaxed) == block.frames
    }

    fn clear_region(&mut self, first_channel: usize, channels: usize, frames: usize) {
        for channel in first_channel..first_channel + channels {
            // SAFETY: Private callers use validated layout channels and frames
            // <= capacity; the endpoint owns PCM for this handoff phase.
            unsafe {
                self.data
                    .as_mut_ptr()
                    .add(size_of::<Header>() + channel * self.max_frames * size_of::<f32>())
                    .write_bytes(0, frames * size_of::<f32>());
            }
        }
    }

    fn write_buses(&mut self, first_channel: usize, buses: &[AudioBusBuffer], frames: usize) {
        let mut index = first_channel;
        for bus in buses {
            for channel in &bus.channels {
                // SAFETY: Shape/length validation precedes every call. Private
                // channel offsets are bounded by mapping_size. Host and helper
                // never copy the same PCM region concurrently under the state
                // handoff; native plug-ins see separate local sample vectors.
                unsafe {
                    let destination = self
                        .data
                        .as_mut_ptr()
                        .add(size_of::<Header>())
                        .cast::<f32>()
                        .add(index * self.max_frames);
                    if bus.active {
                        std::ptr::copy_nonoverlapping(channel.as_ptr(), destination, frames);
                    } else {
                        destination.write_bytes(0, frames);
                    }
                }
                index += 1;
            }
        }
    }

    fn read_buses(&self, first_channel: usize, buses: &mut [AudioBusBuffer], frames: usize) {
        let mut index = first_channel;
        for bus in buses {
            for channel in &mut bus.channels {
                if bus.active {
                    // SAFETY: Same shape, offset and exclusive ownership
                    // guarantees as write_buses. Every f32 bit pattern is valid;
                    // copying never changes NaNs, signed zeros or infinities.
                    unsafe {
                        let source = self
                            .data
                            .as_ptr()
                            .add(size_of::<Header>())
                            .cast::<f32>()
                            .add(index * self.max_frames);
                        std::ptr::copy_nonoverlapping(source, channel.as_mut_ptr(), frames);
                    }
                }
                index += 1;
            }
        }
    }
}

/// Host-owned reusable mapping. Keep it with the child that admitted its descriptor.
pub struct SharedAudioHost {
    // Unmap before tempfile cleanup, especially on Windows.
    mapping: Mapping,
    _file: NamedTempFile,
    descriptor: SharedAudioDescriptor,
    generation: u64,
    pending: Option<SharedAudioBlock>,
    poisoned: bool,
}

impl SharedAudioHost {
    /// Allocate and touch a private fixed-capacity mapping outside audio processing.
    pub fn new(layout: AudioBusLayout, max_frames: usize) -> io::Result<Self> {
        validate_layout(&layout, max_frames)?;
        let file = tempfile::Builder::new()
            .prefix("rill-pcm-")
            .rand_bytes(16)
            .tempfile()?;
        let descriptor = SharedAudioDescriptor {
            mapping_id: hash_bytes(
                0xcbf2_9ce4_8422_2325,
                file.path().as_os_str().as_encoded_bytes().iter().copied(),
            ),
            path: file.path().to_owned(),
            layout,
            max_frames: max_frames as u32,
        };
        file.as_file().set_len(mapping_size(&descriptor)? as u64)?;
        let mut mapping = Mapping::map(file.as_file(), &descriptor)?;
        mapping.initialize(&descriptor);
        Ok(Self {
            mapping,
            _file: file,
            descriptor,
            generation: 0,
            pending: None,
            poisoned: false,
        })
    }

    /// The descriptor to send once to this mapping's isolated helper.
    pub fn descriptor(&self) -> &SharedAudioDescriptor {
        &self.descriptor
    }

    /// Publish inputs for exactly one exchange without allocating or encoding PCM.
    /// A second request is rejected until the prior response has been consumed.
    pub fn begin(
        &mut self,
        inputs: &[AudioBusBuffer],
        frames: usize,
    ) -> io::Result<SharedAudioBlock> {
        if self.poisoned
            || self.pending.is_some()
            || self.mapping.header().state.load(Ordering::Acquire) != IDLE
        {
            return Err(invalid("Shared audio mapping is busy or poisoned"));
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("Shared audio generation exhausted"))?;
        let block = SharedAudioBlock {
            generation,
            frames: u32::try_from(frames)
                .map_err(|_| invalid("Shared audio frame count overflow"))?,
        };
        self.mapping.valid_block(block)?;
        validate_buffers(inputs, &self.descriptor.layout.inputs, frames)?;
        self.mapping.write_buses(0, inputs, frames);
        self.mapping.clear_region(
            self.mapping.input_channels,
            self.mapping.output_channels,
            frames,
        );
        self.generation = generation;
        self.pending = Some(block);
        self.mapping
            .header()
            .generation
            .store(generation, Ordering::Relaxed);
        self.mapping
            .header()
            .frames
            .store(block.frames, Ordering::Relaxed);
        self.mapping
            .header()
            .state
            .store(INPUT_READY, Ordering::Release);
        Ok(block)
    }

    /// Consume only a matching completed generation. All errors silence output
    /// and permanently poison this mapping; a late child cannot make it reusable.
    pub fn finish(
        &mut self,
        block: SharedAudioBlock,
        outputs: &mut [AudioBusBuffer],
    ) -> io::Result<()> {
        clear_buffers(outputs);
        let result = (|| {
            if self.poisoned || self.pending != Some(block) {
                return Err(invalid("Stale or unexpected shared audio completion"));
            }
            self.mapping.valid_block(block)?;
            validate_buffers(
                outputs,
                &self.descriptor.layout.outputs,
                block.frames as usize,
            )?;
            if self.mapping.header().state.load(Ordering::Acquire) != OUTPUT_READY
                || !self.mapping.matches_block(block)
            {
                return Err(invalid(
                    "Shared audio output is not ready for this generation",
                ));
            }
            self.mapping
                .read_buses(self.mapping.input_channels, outputs, block.frames as usize);
            self.pending = None;
            self.mapping.header().state.store(IDLE, Ordering::Release);
            Ok(())
        })();
        if result.is_err() {
            self.poison();
        }
        result
    }

    /// Abandon a failed exchange before replacing or terminating its helper.
    /// This touches only the atomic header, so it is safe while the child is
    /// processing or publishing PCM. Never retry processing on this mapping.
    pub fn poison(&mut self) {
        self.poisoned = true;
        self.pending = None;
        self.mapping.header().state.swap(POISONED, Ordering::AcqRel);
    }
}

/// Helper-side mapping. Native plug-ins receive local buffers, never mapped pointers.
pub struct SharedAudioWorker {
    mapping: Mapping,
    _file: File,
    descriptor: SharedAudioDescriptor,
    pending: Option<SharedAudioBlock>,
    last_generation: u64,
}

impl SharedAudioWorker {
    /// Admit a host-owned, bounded, private mapping and verify its identity/layout.
    /// Open this once per helper lifetime; recreate it when the bus layout changes.
    pub fn open(descriptor: SharedAudioDescriptor) -> io::Result<Self> {
        mapping_size(&descriptor)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&descriptor.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata()?;
            // SAFETY: geteuid has no preconditions or writable pointer arguments.
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(invalid(
                    "Shared audio file is not private to the current user",
                ));
            }
        }
        let mapping = Mapping::map(&file, &descriptor)?;
        if mapping.header().magic != MAGIC
            || mapping.header().mapping_id != descriptor.mapping_id
            || mapping.header().layout_signature != layout_signature(&descriptor)
            || mapping.header().state.load(Ordering::Acquire) > OUTPUT_READY
        {
            return Err(invalid("Shared audio mapping identity or layout mismatch"));
        }
        Ok(Self {
            mapping,
            _file: file,
            descriptor,
            pending: None,
            last_generation: 0,
        })
    }

    /// The fixed native bus layout and capacity accepted for this worker.
    pub fn descriptor(&self) -> &SharedAudioDescriptor {
        &self.descriptor
    }

    /// Copy a matching request into preallocated local input buffers. Silence
    /// inactive buses and unused tails, rejecting replays or overlapping work.
    pub fn read_inputs(
        &mut self,
        block: SharedAudioBlock,
        inputs: &mut [AudioBusBuffer],
    ) -> io::Result<()> {
        clear_buffers(inputs);
        self.mapping.valid_block(block)?;
        validate_buffers(
            inputs,
            &self.descriptor.layout.inputs,
            block.frames as usize,
        )?;
        if self.pending.is_some() || block.generation <= self.last_generation {
            return Err(invalid(
                "Shared audio request is stale or already processing",
            ));
        }
        if self.mapping.header().state.load(Ordering::Acquire) != INPUT_READY
            || !self.mapping.matches_block(block)
            || self
                .mapping
                .header()
                .state
                .compare_exchange(INPUT_READY, PROCESSING, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(invalid(
                "Shared audio inputs are not ready for this generation",
            ));
        }
        self.mapping.read_buses(0, inputs, block.frames as usize);
        self.pending = Some(block);
        self.last_generation = block.generation;
        Ok(())
    }

    /// Publish local plug-in outputs, preserving every sample bit. Clear local
    /// output buffers before invoking the native processor so omitted writes are
    /// silence. A poisoned or mismatched generation can never publish success.
    pub fn write_outputs(
        &mut self,
        block: SharedAudioBlock,
        outputs: &[AudioBusBuffer],
    ) -> io::Result<()> {
        self.mapping.valid_block(block)?;
        validate_buffers(
            outputs,
            &self.descriptor.layout.outputs,
            block.frames as usize,
        )?;
        if self.pending != Some(block)
            || self.mapping.header().state.load(Ordering::Acquire) != PROCESSING
            || !self.mapping.matches_block(block)
        {
            return Err(invalid(
                "Shared audio output generation was abandoned or changed",
            ));
        }
        self.mapping
            .write_buses(self.mapping.input_channels, outputs, block.frames as usize);
        if self
            .mapping
            .header()
            .state
            .compare_exchange(
                PROCESSING,
                OUTPUT_READY,
                Ordering::Release,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Err(invalid(
                "Shared audio processing was abandoned during publication",
            ));
        }
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct CountingAllocator;

    thread_local! {
        static COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    fn count_allocation() {
        if COUNT_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    }

    // SAFETY: Every allocation operation delegates unchanged to the system
    // allocator. Thread-local counters do not allocate or affect other tests.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count_allocation();
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count_allocation();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            count_allocation();
            unsafe { System.realloc(pointer, layout, size) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    fn layout() -> AudioBusLayout {
        AudioBusLayout {
            inputs: vec![
                AudioBusConfig {
                    channel_count: 2,
                    active: true,
                },
                AudioBusConfig {
                    channel_count: 1,
                    active: false,
                },
            ],
            outputs: vec![
                AudioBusConfig {
                    channel_count: 2,
                    active: true,
                },
                AudioBusConfig {
                    channel_count: 1,
                    active: false,
                },
            ],
        }
    }

    fn buffers(config: &[AudioBusConfig], frames: usize) -> Vec<AudioBusBuffer> {
        config
            .iter()
            .map(|bus| AudioBusBuffer::new(bus.channel_count, frames, bus.active))
            .collect()
    }

    #[test]
    fn exact_pcm_bits_round_trip_and_inactive_channels_and_tails_are_silent() {
        let layout = layout();
        let mut host = SharedAudioHost::new(layout.clone(), 16).unwrap();
        let mut worker = SharedAudioWorker::open(host.descriptor().clone()).unwrap();
        let mut input = buffers(&layout.inputs, 16);
        let bits = [
            0,
            0x8000_0000,
            0x3f80_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            1,
            0x7f7f_ffff,
        ];
        for channel in &mut input[0].channels {
            for (sample, bits) in channel.iter_mut().zip(bits) {
                *sample = f32::from_bits(bits);
            }
        }
        input[1].channels[0].fill(123.0);
        let block = host.begin(&input, bits.len()).unwrap();
        assert!(host.begin(&input, bits.len()).is_err());
        let mut local = buffers(&layout.inputs, 16);
        worker.read_inputs(block, &mut local).unwrap();
        assert_eq!(
            local[0].channels[0][..bits.len()]
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            bits
        );
        assert!(local[0].channels[0][bits.len()..].iter().all(|&s| s == 0.0));
        assert!(local[1].channels[0].iter().all(|&s| s == 0.0));
        worker.write_outputs(block, &local).unwrap();
        let mut output = buffers(&layout.outputs, 16);
        for bus in &mut output {
            for channel in &mut bus.channels {
                channel.fill(99.0);
            }
        }
        host.finish(block, &mut output).unwrap();
        assert_eq!(
            output[0].channels[0][..bits.len()]
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            bits
        );
        assert!(output[0].channels[0][bits.len()..]
            .iter()
            .all(|&s| s == 0.0));
        assert!(output[1].channels[0].iter().all(|&s| s == 0.0));
        let next = host.begin(&input, 1).unwrap();
        assert_eq!(next.generation, block.generation + 1);
        assert!(worker.read_inputs(block, &mut local).is_err());
        worker.read_inputs(next, &mut local).unwrap();
        worker.write_outputs(next, &local).unwrap();
        host.finish(next, &mut output).unwrap();
        assert!(output[0].channels[0][1..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn repeated_blocks_use_preallocated_storage_without_allocations() {
        let layout = layout();
        let mut host = SharedAudioHost::new(layout.clone(), 256).unwrap();
        let mut worker = SharedAudioWorker::open(host.descriptor().clone()).unwrap();
        let input = buffers(&layout.inputs, 256);
        let mut local = buffers(&layout.inputs, 256);
        let mut output = buffers(&layout.outputs, 256);
        struct StopCounting;
        impl Drop for StopCounting {
            fn drop(&mut self) {
                COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
            }
        }
        ALLOCATIONS.with(|count| count.set(0));
        COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
        let guard = StopCounting;
        for index in 0..1_000 {
            let block = host.begin(&input, 1 + index % 256).unwrap();
            worker.read_inputs(block, &mut local).unwrap();
            worker.write_outputs(block, &local).unwrap();
            host.finish(block, &mut output).unwrap();
        }
        drop(guard);
        assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    }

    #[test]
    fn event_only_layout_can_process_at_the_frame_capacity_limit() {
        let layout = AudioBusLayout {
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let mut host = SharedAudioHost::new(layout, MAX_FRAMES).unwrap();
        let mut worker = SharedAudioWorker::open(host.descriptor().clone()).unwrap();
        let block = host.begin(&[], MAX_FRAMES).unwrap();
        worker.read_inputs(block, &mut []).unwrap();
        worker.write_outputs(block, &[]).unwrap();
        host.finish(block, &mut []).unwrap();
    }

    #[test]
    fn invalid_capacities_shapes_and_descriptor_identity_are_rejected() {
        for frames in [0, MAX_FRAMES + 1, usize::MAX] {
            assert!(SharedAudioHost::new(layout(), frames).is_err());
        }
        let mut too_many = layout();
        too_many.inputs[0].channel_count = usize::MAX;
        assert!(SharedAudioHost::new(too_many, 1).is_err());
        let mut too_many = layout();
        too_many.inputs[0].channel_count = MAX_CHANNELS;
        assert!(SharedAudioHost::new(too_many, 1).is_err());
        let mut too_many = layout();
        too_many.inputs = vec![
            AudioBusConfig {
                channel_count: 0,
                active: false
            };
            MAX_BUSES + 1
        ];
        assert!(SharedAudioHost::new(too_many, 1).is_err());
        let mut host = SharedAudioHost::new(layout(), 16).unwrap();
        let mut wrong = host.descriptor().clone();
        wrong.mapping_id ^= 1;
        assert!(SharedAudioWorker::open(wrong).is_err());
        let mut wrong = host.descriptor().clone();
        wrong.layout.inputs[0].active = false;
        assert!(SharedAudioWorker::open(wrong).is_err());
        let mut wrong = host.descriptor().clone();
        wrong.max_frames += 1;
        assert!(SharedAudioWorker::open(wrong).is_err());
        let mut input = buffers(&layout().inputs, 16);
        assert!(host.begin(&input, 0).is_err());
        assert!(host.begin(&input, 17).is_err());
        input[0].channels[0].truncate(2);
        assert!(host.begin(&input, 3).is_err());
        input[0].active = false;
        assert!(host.begin(&input, 1).is_err());
    }

    #[test]
    fn timeout_poison_rejects_late_output_and_never_recycles_mapping() {
        let layout = layout();
        let mut host = SharedAudioHost::new(layout.clone(), 8).unwrap();
        let mut worker = SharedAudioWorker::open(host.descriptor().clone()).unwrap();
        let input = buffers(&layout.inputs, 8);
        let block = host.begin(&input, 8).unwrap();
        let mut local = buffers(&layout.inputs, 8);
        worker.read_inputs(block, &mut local).unwrap();
        host.poison();
        assert!(worker.write_outputs(block, &local).is_err());
        let mut output = buffers(&layout.outputs, 8);
        output[0].channels[0].fill(1.0);
        assert!(host.finish(block, &mut output).is_err());
        assert!(output
            .iter()
            .flat_map(|bus| &bus.channels)
            .flatten()
            .all(|&s| s == 0.0));
        assert!(host.begin(&input, 8).is_err());
        // The old child keeps a valid mapping even after the owner drops it;
        // neither unmapping nor deleting the file invalidates its mapped pages.
        let path = host.descriptor().path.clone();
        drop(host);
        assert!(worker.write_outputs(block, &local).is_err());
        drop(worker);
        // Windows can defer/unallow deletion while the peer still maps the
        // file. Production reaps/releases the peer first; this lifetime test
        // deliberately reverses that order and cleans up after both unmap.
        if let Err(error) = std::fs::remove_file(path) {
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }
    }

    #[test]
    fn stale_completion_silences_output_and_poisoning_is_permanent() {
        let layout = layout();
        let mut host = SharedAudioHost::new(layout.clone(), 8).unwrap();
        let mut worker = SharedAudioWorker::open(host.descriptor().clone()).unwrap();
        let input = buffers(&layout.inputs, 8);
        let block = host.begin(&input, 8).unwrap();
        let mut local = buffers(&layout.inputs, 8);
        worker.read_inputs(block, &mut local).unwrap();
        worker.write_outputs(block, &local).unwrap();
        let mut output = buffers(&layout.outputs, 8);
        assert!(host
            .finish(
                SharedAudioBlock {
                    generation: block.generation + 1,
                    ..block
                },
                &mut output
            )
            .is_err());
        assert!(host.finish(block, &mut output).is_err());
        assert!(host.begin(&input, 8).is_err());
    }

    #[test]
    fn shared_mapping_round_trips_through_a_separate_process() {
        let layout = layout();
        let mut host = SharedAudioHost::new(layout.clone(), 8).unwrap();
        let mut input = buffers(&layout.inputs, 8);
        input[0].channels[0].copy_from_slice(&[0.25, -0.5, 0.75, 1.0, -1.0, 0.0, 0.125, -0.25]);
        let block = host.begin(&input, 8).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shared_audio::tests::subprocess_endpoint",
                "--nocapture",
            ])
            .env(
                "RILL_SHARED_AUDIO_TEST_DESCRIPTOR",
                serde_json::to_string(host.descriptor()).unwrap(),
            )
            .env(
                "RILL_SHARED_AUDIO_TEST_BLOCK",
                serde_json::to_string(&block).unwrap(),
            )
            .status()
            .unwrap();
        assert!(status.success());
        let mut output = buffers(&layout.outputs, 8);
        host.finish(block, &mut output).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn subprocess_endpoint() {
        let Ok(descriptor) = std::env::var("RILL_SHARED_AUDIO_TEST_DESCRIPTOR") else {
            return;
        };
        let descriptor: SharedAudioDescriptor = serde_json::from_str(&descriptor).unwrap();
        let block =
            serde_json::from_str(&std::env::var("RILL_SHARED_AUDIO_TEST_BLOCK").unwrap()).unwrap();
        let mut input = buffers(&descriptor.layout.inputs, descriptor.max_frames as usize);
        let mut worker = SharedAudioWorker::open(descriptor).unwrap();
        worker.read_inputs(block, &mut input).unwrap();
        worker.write_outputs(block, &input).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn worker_rejects_symlinks_and_nonprivate_files() {
        use std::os::unix::fs::PermissionsExt;
        let host = SharedAudioHost::new(layout(), 8).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&host.descriptor().path, &alias).unwrap();
        let mut descriptor = host.descriptor().clone();
        descriptor.path = alias;
        assert!(SharedAudioWorker::open(descriptor).is_err());
        std::fs::set_permissions(
            &host.descriptor().path,
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(SharedAudioWorker::open(host.descriptor().clone()).is_err());
    }
}
