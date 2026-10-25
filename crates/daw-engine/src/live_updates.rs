//! Latest-wins preparation and reclamation, separate from the playback worker.
use crate::render::{
    Renderer,
    live_edit::{LiveEditCompatibility, LiveEditPreparer, PreparedLiveEdit},
};
use anyhow::{Result, ensure};
use daw_core::Project;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(test)]
type PreparationGate = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

struct Request {
    sequence: u64,
    revision: u64,
    project: Project,
}
struct Ready {
    sequence: u64,
    revision: u64,
    plan: PreparedLiveEdit,
    refining: bool,
}

#[derive(Default)]
pub(super) struct LiveMailbox {
    request: Mutex<Option<Request>>,
    ready: Mutex<Option<Ready>>,
    has_ready: AtomicBool,
    #[cfg(test)]
    prepare_gate: Mutex<Option<PreparationGate>>,
    #[cfg(test)]
    refinement_gate: Mutex<Option<PreparationGate>>,
    retired: Mutex<Option<PreparedLiveEdit>>,
    wake: Condvar,
    shutdown: AtomicBool,
    preparation_cancel: AtomicBool,
    sequence: AtomicU64,
    pub requested_revision: AtomicU64,
    pub applied_revision: AtomicU64,
    pub refining_audio: AtomicBool,
    pub errors: AtomicU64,
    error: Mutex<Option<(u64, String)>>,
}

impl LiveMailbox {
    /// Never waits for preparation or reclamation. The plan holds all retired
    /// allocations after application, so their destruction stays off playback.
    pub fn apply_pending(&self, renderer: &mut Renderer) -> bool {
        if !self.has_ready.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut retired) = self.retired.try_lock() else {
            return false;
        };
        if retired.is_some() {
            return false;
        }
        let Ok(mut ready) = self.ready.try_lock() else {
            return false;
        };
        let Some(mut update) = ready.take() else {
            return false;
        };
        self.has_ready.store(false, Ordering::Release);
        let mut applied = false;
        if update.sequence == self.sequence.load(Ordering::Acquire) {
            match renderer.apply_live_edit(&mut update.plan) {
                Ok(()) => {
                    self.applied_revision
                        .store(update.revision, Ordering::Release);
                    self.refining_audio
                        .store(update.refining, Ordering::Release);
                    applied = true;
                }
                Err(error) => {
                    if let Ok(mut slot) = self.error.try_lock()
                        && update.sequence == self.sequence.load(Ordering::Acquire)
                    {
                        self.errors.fetch_add(1, Ordering::Relaxed);
                        *slot = Some((
                            update.sequence,
                            format!("Live edit was not applied: {error:#}"),
                        ));
                    }
                }
            }
        }
        *retired = Some(update.plan);
        self.wake.notify_one();
        applied
    }

    fn fail(&self, sequence: u64, message: String) {
        let mut error = self.error.lock().unwrap_or_else(|error| error.into_inner());
        if sequence == self.sequence.load(Ordering::Acquire) {
            // A failed final pass can race the render worker consuming its
            // already-published preview. That preview remains usable, but must
            // not claim refinement is still running after the failure.
            if let Some(ready) = self
                .ready
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                && ready.sequence == sequence
            {
                ready.refining = false;
            }
            self.refining_audio.store(false, Ordering::Release);
            self.errors.fetch_add(1, Ordering::Relaxed);
            *error = Some((sequence, message));
        }
    }
}

pub(super) struct LiveUpdates {
    pub mailbox: Arc<LiveMailbox>,
    compatible: LiveEditCompatibility,
    worker: Option<JoinHandle<()>>,
}

impl LiveUpdates {
    pub fn start(preparer: LiveEditPreparer, mailbox: Arc<LiveMailbox>) -> Result<Self> {
        let compatible = preparer.compatibility();
        let shared = Arc::clone(&mailbox);
        let worker = thread::Builder::new()
            .name("daw-edit-prepare".into())
            .spawn(move || prepare_loop(preparer, shared))?;
        Ok(Self {
            mailbox,
            compatible,
            worker: Some(worker),
        })
    }

    pub fn submit(&self, project: &Project, revision: u64) -> Result<bool> {
        ensure!(
            !self.mailbox.shutdown.load(Ordering::Acquire),
            "Audio edit preparation has stopped"
        );
        ensure!(
            self.worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished()),
            "Audio edit preparation stopped unexpectedly"
        );
        if !self.compatible.compatible(project) {
            return Ok(false);
        }
        let project = project.clone();
        let mut slot = self
            .mailbox
            .request
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let sequence = self
            .mailbox
            .sequence
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        // Stop work on an obsolete stretch promptly rather than waiting for a
        // whole source before preparing the newest held-drag position.
        self.mailbox
            .preparation_cancel
            .store(true, Ordering::Release);
        self.mailbox
            .requested_revision
            .store(revision, Ordering::Release);
        *slot = Some(Request {
            sequence,
            revision,
            project,
        });
        self.mailbox.wake.notify_one();
        Ok(true)
    }

    pub fn acknowledge_parameter(
        &self,
        track: u64,
        effect: Option<u64>,
        parameter: u32,
        value: f64,
    ) {
        self.compatible
            .acknowledge_parameter(track, effect, parameter, value);
    }

    #[cfg(test)]
    pub fn gate_next_preparation(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (started, entered) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        *self
            .mailbox
            .prepare_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((started, released));
        (entered, release)
    }

    #[cfg(test)]
    pub fn gate_next_refinement(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (started, entered) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        *self
            .mailbox
            .refinement_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((started, released));
        (entered, release)
    }

    pub fn take_error(&self) -> Option<String> {
        let (sequence, message) = self.mailbox.error.try_lock().ok()?.take()?;
        (sequence == self.mailbox.sequence.load(Ordering::Acquire)).then_some(message)
    }
}

impl Drop for LiveUpdates {
    fn drop(&mut self) {
        self.mailbox.shutdown.store(true, Ordering::Release);
        self.mailbox
            .preparation_cancel
            .store(true, Ordering::Release);
        self.mailbox.wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn prepare_loop(mut preparer: LiveEditPreparer, shared: Arc<LiveMailbox>) {
    while !shared.shutdown.load(Ordering::Acquire) {
        let retired = shared
            .retired
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        drop(retired);
        let request = {
            let mut slot = shared.request.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() && !shared.shutdown.load(Ordering::Acquire) {
                slot = shared
                    .wake
                    .wait_timeout(slot, Duration::from_millis(10))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            let request = slot.take();
            // Reset under the submit lock: a later request cannot have its
            // cancellation signal accidentally cleared after it was submitted.
            if request.is_some() {
                shared.preparation_cancel.store(false, Ordering::Release);
            }
            request
        };
        let Some(request) = request else { continue };
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        #[cfg(test)]
        {
            let gate = shared
                .prepare_gate
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some((started, released)) = gate {
                let _ = started.send(());
                while matches!(
                    released.recv_timeout(Duration::from_millis(5)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) && !shared.shutdown.load(Ordering::Acquire)
                {
                    thread::yield_now();
                }
            }
        }
        for preview in [true, false] {
            // Reclaim a superseded plan before spending time on the next pass,
            // so a stale ready item cannot keep the preview from being applied.
            drop(
                shared
                    .retired
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take(),
            );
            #[cfg(test)]
            if !preview {
                let gate = shared
                    .refinement_gate
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some((started, released)) = gate {
                    let _ = started.send(());
                    while matches!(
                        released.recv_timeout(Duration::from_millis(5)),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) && !shared.shutdown.load(Ordering::Acquire)
                        && !shared.preparation_cancel.load(Ordering::Acquire)
                    {
                        thread::yield_now();
                    }
                }
            }
            let result =
                preparer.prepare_cancelled(&request.project, &shared.preparation_cancel, preview);
            if shared.shutdown.load(Ordering::Acquire)
                || request.sequence != shared.sequence.load(Ordering::Acquire)
            {
                break;
            }
            match result {
                Ok(plan) => {
                    let refining = plan.needs_refinement();
                    let next = Ready {
                        sequence: request.sequence,
                        revision: request.revision,
                        plan,
                        refining,
                    };
                    // Replacing an obsolete ready plan destroys it on this thread.
                    let mut ready = shared.ready.lock().unwrap_or_else(|e| e.into_inner());
                    *ready = Some(next);
                    shared.has_ready.store(true, Ordering::Release);
                    // A cache hit or short/neutral source was already prepared at
                    // final quality. Do not duplicate that work or publish twice.
                    if !refining {
                        break;
                    }
                }
                Err(error) => {
                    let message = if preview {
                        format!(
                            "Live edit preparation failed; playback continues with the previous audio: {error:#}"
                        )
                    } else {
                        format!(
                            "Audio refinement failed; playback continues with the interactive preview: {error:#}"
                        )
                    };
                    shared.fail(request.sequence, message);
                    break;
                }
            }
        }
    }
    shared
        .ready
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    shared
        .retired
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
}
