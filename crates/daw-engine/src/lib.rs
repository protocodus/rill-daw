//! Modular stereo renderer and device output. The device callback does no heap allocation,
//! locking, file access, or plugin processing. A bounded atomic queue connects it to a
//! render worker, so plugin work cannot block the hardware callback.
mod device;
mod render;

pub use device::{AudioEngine, DeviceInfo, DeviceSettings, EngineSnapshot, list_output_devices};
pub use render::{CompensationDelay, export_project_wav, export_wav, render, render_with_workers};

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
