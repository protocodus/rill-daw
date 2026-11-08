//! Persistent bounded workers execute dependency-ready tracks. One owner holds
//! each track; the coordinator gathers inputs in canonical reduction order.
use super::{BlockContext, Processor, RenderTrack, finish_processing, schedule::Schedule};
use crate::performance::{RenderPerformanceSnapshot, TrackRenderPerformance, nanoseconds};
use anyhow::{Context, Result};
use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

struct Work {
    index: usize,
    track: Box<RenderTrack>,
    context: BlockContext,
    ready_at: Instant,
}
struct Completed {
    worker: Option<usize>,
    index: usize,
    track: Box<RenderTrack>,
    result: Result<()>,
    processing_ns: u64,
    ready_wait_ns: u64,
}

impl Work {
    fn process(mut self, worker: Option<usize>) -> Completed {
        let started = Instant::now();
        let ready_wait_ns = nanoseconds(started.duration_since(self.ready_at));
        let result = catch_unwind(AssertUnwindSafe(|| self.track.process(self.context)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Graph processor panicked")));
        Completed {
            worker,
            index: self.index,
            track: self.track,
            result,
            processing_ns: nanoseconds(started.elapsed()),
            ready_wait_ns,
        }
    }
}
struct Ready {
    index: usize,
    priority: u64,
    rank: usize,
    since: Instant,
}
impl PartialEq for Ready {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.rank == other.rank
    }
}
impl Eq for Ready {}
impl PartialOrd for Ready {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ready {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.rank.cmp(&self.rank))
    }
}

pub(super) struct WorkerPool {
    commands: Vec<mpsc::SyncSender<Work>>,
    completed: mpsc::Receiver<Completed>,
    threads: Vec<JoinHandle<()>>,
    schedule: Schedule,
    pending: Vec<usize>,
    ready: BinaryHeap<Ready>,
    idle: Vec<usize>,
    blocks: u64,
}

impl WorkerPool {
    #[cfg(test)]
    pub fn prime_completion_wait_for_test(&self) {
        assert!(matches!(
            self.completed
                .recv_timeout(std::time::Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }

    #[cfg(test)]
    pub fn disconnect_for_test(&mut self, worker: usize) {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        self.commands[worker] = sender;
        self.threads.swap_remove(worker).join().unwrap();
    }

    pub fn new(
        count: usize,
        tracks: &[RenderTrack],
        levels: &[Vec<usize>],
        timings: &mut [TrackRenderPerformance],
    ) -> Result<Self> {
        let (results, completed) = mpsc::sync_channel(count);
        let schedule = Schedule::new(tracks, levels);
        schedule.initialize_costs(tracks, timings);
        let mut pool = Self {
            commands: Vec::with_capacity(count),
            completed,
            threads: Vec::with_capacity(count),
            pending: vec![0; tracks.len()],
            ready: BinaryHeap::with_capacity(tracks.len()),
            idle: Vec::with_capacity(count),
            schedule,
            blocks: 0,
        };
        for worker in 0..count {
            let (commands, receiver) = mpsc::sync_channel::<Work>(1);
            let results = results.clone();
            let thread = thread::Builder::new()
                .name(format!("rill-graph-{worker}"))
                .spawn(move || {
                    while let Ok(work) = receiver.recv() {
                        if results.send(work.process(Some(worker))).is_err() {
                            break;
                        }
                    }
                })
                .context("Starting persistent audio graph worker")?;
            pool.commands.push(commands);
            pool.threads.push(thread);
        }
        Ok(pool)
    }

    pub fn process(
        &mut self,
        tracks: &mut [Option<Box<RenderTrack>>],
        context: BlockContext,
        tolerate: bool,
        output: &mut [[f64; 2]],
        timings: &mut [TrackRenderPerformance],
    ) -> Result<(u64, RenderPerformanceSnapshot)> {
        self.blocks = self.blocks.wrapping_add(1);
        if self.blocks.is_multiple_of(32) {
            self.schedule.update_priorities(timings);
        }
        self.pending.copy_from_slice(&self.schedule.predecessors);
        self.ready.clear();
        self.idle.clear();
        self.idle.extend((0..self.commands.len()).rev());
        let started = Instant::now();
        for &index in &self.schedule.roots {
            self.ready.push(Ready {
                index,
                priority: timings[index].critical_path_ns,
                rank: self.schedule.ranks[index],
                since: started,
            });
        }
        let mut stats = RenderPerformanceSnapshot {
            worker_count: self.commands.len(),
            block_frames: context.count,
            track_timings_measured: true,
            ..Default::default()
        };
        let mut in_flight = 0;
        let mut completed = 0;
        let mut failures = 0;
        let mut first_error = None;
        while completed < self.schedule.order.len() || in_flight > 0 {
            let mut inline = None;
            while first_error.is_none() && !self.idle.is_empty() && !self.ready.is_empty() {
                let ready = self.ready.pop().expect("ready work");
                let mut track = tracks[ready.index].take().expect("one graph node owner");
                self.schedule
                    .gather(ready.index, &mut track, tracks, context.count);
                // A lone cheap continuation (usually the final master) cannot
                // overlap other work. Avoid waking a helper just to sum it.
                let local = in_flight == 0
                    && self.ready.is_empty()
                    && matches!(track.processor, Processor::None)
                    && track.inserts.is_empty();
                let work = Work {
                    index: ready.index,
                    track,
                    context,
                    ready_at: ready.since,
                };
                if local {
                    inline = Some(work.process(None));
                    break;
                }
                let worker = self.idle.pop().expect("idle worker");
                match self.commands[worker].send(work) {
                    Ok(()) => {
                        in_flight += 1;
                        stats.peak_active_workers = stats.peak_active_workers.max(in_flight);
                    }
                    Err(error) => {
                        tracks[error.0.index] = Some(error.0.track);
                        first_error = Some(anyhow::anyhow!(
                            "Persistent graph worker unexpectedly exited"
                        ));
                    }
                }
            }
            if in_flight == 0 && inline.is_none() {
                if first_error.is_none() && completed < self.schedule.order.len() {
                    first_error = Some(anyhow::anyhow!("Audio graph has unresolved dependencies"));
                }
                break;
            }
            let done = if let Some(done) = inline {
                done
            } else {
                in_flight -= 1;
                match self.completed.try_recv() {
                    Ok(done) => done,
                    Err(mpsc::TryRecvError::Empty) => {
                        let waiting = Instant::now();
                        let result = self
                            .completed
                            .recv()
                            .context("Persistent graph workers exited");
                        stats.coordinator_wait_ns = stats
                            .coordinator_wait_ns
                            .saturating_add(nanoseconds(waiting.elapsed()));
                        result?
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err(anyhow::anyhow!("Persistent graph workers exited"));
                    }
                }
            };
            completed += 1;
            let Completed {
                worker,
                index,
                mut track,
                result,
                processing_ns,
                ready_wait_ns,
            } = done;
            match finish_processing(&mut track, result, tolerate) {
                Ok(count) => failures += count,
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
            tracks[index] = Some(track);
            if let Some(worker) = worker {
                self.idle.push(worker);
            }
            timings[index].record(processing_ns, ready_wait_ns);
            stats.nodes += 1;
            stats.processing_ns = stats.processing_ns.saturating_add(processing_ns);
            stats.ready_wait_ns = stats.ready_wait_ns.saturating_add(ready_wait_ns);
            stats.max_track_ns = stats.max_track_ns.max(processing_ns);
            if first_error.is_none() {
                let ready_at = Instant::now();
                for &target in &self.schedule.successors[index] {
                    self.pending[target] -= 1;
                    if self.pending[target] == 0 {
                        self.ready.push(Ready {
                            index: target,
                            priority: timings[target].critical_path_ns,
                            rank: self.schedule.ranks[target],
                            since: ready_at,
                        });
                    }
                }
            }
        }
        // Drain all sent owners before propagating a fatal error. Unsent work
        // remains owned by tracks, including after a disconnected worker.
        if let Some(error) = first_error {
            return Err(error);
        }
        self.schedule.gather_terminal(tracks, output);
        stats.wall_ns = nanoseconds(started.elapsed());
        Ok((failures, stats))
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.commands.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_queue_prioritizes_longer_paths_with_stable_ties() {
        let mut ready = BinaryHeap::new();
        let since = Instant::now();
        for (index, priority, rank) in [(0, 10, 0), (1, 30, 2), (2, 30, 1), (3, 20, 3)] {
            ready.push(Ready {
                index,
                priority,
                rank,
                since,
            });
        }
        let order: Vec<_> = std::iter::from_fn(|| ready.pop())
            .map(|node| node.index)
            .collect();
        assert_eq!(order, [2, 1, 3, 0]);
    }
}
