//! Modular stereo renderer and device output. The device callback does no heap allocation,
//! locking, file access, or plugin processing. A bounded atomic queue connects it to a
//! render worker, so plugin work cannot block the hardware callback.
mod device;
mod join;
mod meter;
mod quality;
mod render;
mod stretch;

pub use join::{export_audio_join, export_audio_join_cancelled, render_audio_join};
pub use quality::{RenderOptions, ResampleQuality};

pub use device::{
    AudioEngine, DeviceInfo, DeviceSettings, EngineSnapshot, InputRoute, InputSettings,
    MidiPortInfo, MidiRecordingResult, MidiRoute, MidiSettings, MidiSnapshot, RecordedMidiTrack,
    RecordingConfig, RecordingResult, RecordingSnapshot, RecordingTake, RecordingTiming,
    RecordingTimingSnapshot, VirtualIoSettings, list_input_devices, list_midi_inputs,
    list_output_devices,
};
pub use render::{
    CompensationDelay, ExportOptions, export_project_wav, export_project_wav_with_options,
    export_wav, export_wav_with_options, render, render_range_cancellable, render_with_options,
    render_with_workers,
};

/// Control-thread query using the same folder, mute, solo and bus eligibility as
/// playback/export. It does not create a renderer, plug-in or audio device.
pub fn audible_track_ids(
    project: &daw_core::Project,
) -> anyhow::Result<std::collections::HashSet<u64>> {
    let order = project.routing_order()?;
    Ok(render::audible_tracks(project, &order))
}

#[cfg(test)]
mod allocation_test_support {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! {
        static TRACKING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }
    struct CountingAllocator;
    // The test allocator forwards every allocation/deallocation to System unchanged.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = TRACKING.try_with(|tracking| {
                if tracking.get() {
                    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let _ = TRACKING.try_with(|tracking| {
                if tracking.get() {
                    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.realloc(pointer, layout, size) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;
    pub fn count_allocations(action: impl FnOnce()) -> usize {
        ALLOCATIONS.with(|count| count.set(0));
        TRACKING.with(|tracking| tracking.set(true));
        action();
        TRACKING.with(|tracking| tracking.set(false));
        ALLOCATIONS.with(Cell::get)
    }
}
