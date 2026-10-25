//! Fixed worker threads and bounded queues; tracks carry their preallocated buffers.
//! Batch independent nodes to amortize dispatch. This never runs on the device callback.
use super::{BlockContext, RenderTrack, finish_processing};
use anyhow::{Context, Result};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread::{self, JoinHandle},
};

struct Node {
    index: usize,
    track: Box<RenderTrack>,
    result: Result<()>,
}

struct Work {
    nodes: Vec<Node>,
    context: BlockContext,
}

struct Completed {
    worker: usize,
    nodes: Vec<Node>,
}

pub(super) struct WorkerPool {
    commands: Vec<mpsc::SyncSender<Work>>,
    completed: mpsc::Receiver<Completed>,
    threads: Vec<JoinHandle<()>>,
    batches: Vec<Vec<Node>>,
}

impl WorkerPool {
    #[cfg(test)]
    pub fn prime_completion_wait_for_test(&self) {
        // std's bounded channels allocate their waiter list on the first
        // blocking receive, not necessarily the first receive. A warm-up render
        // may find every result ready and skip that path entirely. Exercise the
        // actual empty completion queue before measuring steady-state dispatch.
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

    pub fn new(count: usize, max_width: usize) -> Result<Self> {
        let (results, completed) = mpsc::sync_channel(count);
        let mut pool = Self {
            commands: Vec::with_capacity(count),
            completed,
            threads: Vec::with_capacity(count),
            // A narrow level may use fewer workers than the pool. Each retained
            // batch reserves the largest assignment it can receive at full width.
            batches: (0..count)
                .map(|_| Vec::with_capacity(max_width.div_ceil(count)))
                .collect(),
        };
        for worker in 0..count {
            let (commands, receiver) = mpsc::sync_channel::<Work>(1);
            let results = results.clone();
            let thread = thread::Builder::new()
                .name(format!("rill-graph-{worker}"))
                .spawn(move || {
                    while let Ok(Work { mut nodes, context }) = receiver.recv() {
                        for node in &mut nodes {
                            // A Rust-side panic must still return node ownership. Native
                            // crashes remain in the independently supervised VST helper.
                            node.result =
                                catch_unwind(AssertUnwindSafe(|| node.track.process(context)))
                                    .unwrap_or_else(|_| {
                                        Err(anyhow::anyhow!("Graph processor panicked"))
                                    });
                        }
                        if results.send(Completed { worker, nodes }).is_err() {
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
        level: &[usize],
        context: BlockContext,
        tolerate: bool,
    ) -> Result<u64> {
        let active = self.commands.len().min(level.len());
        for (offset, &index) in level.iter().enumerate() {
            self.batches[offset % active].push(Node {
                index,
                track: tracks[index].take().expect("graph node has one owner"),
                result: Ok(()),
            });
        }
        let mut pending = 0;
        let mut failures = 0;
        let mut first_error = None;
        for worker in 0..active {
            let nodes = std::mem::take(&mut self.batches[worker]);
            match self.commands[worker].send(Work { nodes, context }) {
                Ok(()) => pending += 1,
                Err(error) => {
                    let mut nodes = error.0.nodes;
                    for node in nodes.drain(..) {
                        tracks[node.index] = Some(node.track);
                    }
                    self.batches[worker] = nodes;
                    for batch in &mut self.batches[worker + 1..active] {
                        for node in batch.drain(..) {
                            tracks[node.index] = Some(node.track);
                        }
                    }
                    first_error = Some(anyhow::anyhow!(
                        "Persistent graph worker unexpectedly exited"
                    ));
                    break;
                }
            }
        }
        for _ in 0..pending {
            let Completed { worker, mut nodes } = self
                .completed
                .recv()
                .context("Persistent graph workers exited")?;
            for mut node in nodes.drain(..) {
                match finish_processing(&mut node.track, node.result, tolerate) {
                    Ok(count) => failures += count,
                    Err(error) if first_error.is_none() => first_error = Some(error),
                    Err(_) => {}
                }
                tracks[node.index] = Some(node.track);
            }
            self.batches[worker] = nodes;
        }
        // Drain every in-flight batch before propagating any failure.
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(failures)
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
