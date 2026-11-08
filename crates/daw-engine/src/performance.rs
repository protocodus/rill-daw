//! Render-worker timings. These describe prepared block execution, not device
//! callback latency; output queue underruns remain the actual deadline signal.
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RenderPerformanceSnapshot {
    pub sequence: u64,
    pub block_frames: usize,
    /// Persistent graph workers, excluding the coordinator (zero for serial).
    pub worker_count: usize,
    pub nodes: usize,
    /// Largest number of dispatched, not-yet-collected tasks. This is not a
    /// measurement of simultaneous CPU execution inside native plug-ins.
    pub peak_active_workers: usize,
    pub track_timings_measured: bool,
    pub wall_ns: u64,
    /// Sum of track processing times, potentially overlapping across workers.
    /// Includes cheap sole continuations executed by the coordinator.
    pub processing_ns: u64,
    /// Sum of time between dependency readiness and actual processing start.
    pub ready_wait_ns: u64,
    pub coordinator_wait_ns: u64,
    pub max_track_ns: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct TrackRenderPerformance {
    pub track_id: u64,
    pub last_process_ns: u64,
    pub last_ready_wait_ns: u64,
    pub estimated_process_ns: u64,
    pub critical_path_ns: u64,
}

pub(crate) fn nanoseconds(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

impl TrackRenderPerformance {
    pub(crate) fn record(&mut self, processing: u64, ready_wait: u64) {
        self.last_process_ns = processing;
        self.last_ready_wait_ns = ready_wait;
        // Bounded integer EWMA: changing costs influence subsequent blocks,
        // while one delayed IPC call does not reorder every following block.
        self.estimated_process_ns = if self.estimated_process_ns == 0 {
            processing.max(1)
        } else {
            self.estimated_process_ns
                .saturating_sub(self.estimated_process_ns / 8)
                .saturating_add(processing / 8)
                .max(1)
        };
    }
}
