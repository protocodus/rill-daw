//! Reusable prepared offline renderer, including bounded block telemetry for
//! diagnostics and benchmarks. Preparation and latency priming happen in new().
use crate::{RenderPerformanceSnapshot, TrackRenderPerformance, render::Renderer};
use anyhow::Result;
use daw_core::Project;

pub struct OfflineRenderer {
    renderer: Renderer,
}

impl OfflineRenderer {
    /// None uses automatic worker selection; Some(1) selects serial rendering.
    pub fn new(project: &Project, sample_rate: u32, workers: Option<usize>) -> Result<Self> {
        Ok(Self {
            renderer: Renderer::new_profiled_offline(project, sample_rate, workers)?,
        })
    }

    pub fn render(&mut self, output: &mut [[f32; 2]], positions: &mut [f64]) -> Result<()> {
        self.renderer.render(output, positions)
    }

    /// Latest internal segment, at most 256 frames. To collect each block,
    /// render in 256-frame chunks. Empty calls do not advance the snapshot.
    pub fn performance_snapshot(&self) -> RenderPerformanceSnapshot {
        self.renderer.performance_snapshot()
    }

    /// Last-segment track timings and smoothed scheduling estimates. Silent or
    /// pruned tracks can have zero last-segment time; IDs remain stable.
    pub fn track_performance(&self) -> &[TrackRenderPerformance] {
        self.renderer.track_performance()
    }

    /// Diagnostic policy override for isolated native plug-ins. Set between
    /// render calls; Auto uses negotiated helper capabilities by default.
    pub fn set_block_transport(&mut self, policy: daw_plugins::BlockTransport) -> Result<()> {
        self.renderer.set_block_transport(policy)
    }

    /// Cumulative IPC traffic across native instruments and inserts, including
    /// preparation. Subtract snapshots to measure a prepared render interval.
    pub fn ipc_stats(&self) -> daw_plugins::IpcStats {
        self.renderer.ipc_stats()
    }
}
