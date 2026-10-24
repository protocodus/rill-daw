//! Serialize device teardown and initialization without blocking the UI.
use std::sync::mpsc::Receiver;

pub(crate) fn retire<T>(
    previous: Option<T>,
    preparing: Option<Receiver<(u64, anyhow::Result<T>)>>,
    retiring: Option<Receiver<()>>,
) {
    drop(previous);
    if let Some(preparing) = preparing
        && let Ok((_, result)) = preparing.recv()
    {
        drop(result);
    }
    if let Some(retiring) = retiring {
        let _ = retiring.recv();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    struct Device(Arc<Mutex<Vec<&'static str>>>, &'static str);
    impl Drop for Device {
        fn drop(&mut self) {
            self.0.lock().unwrap().push(self.1);
        }
    }

    #[test]
    fn replacement_waits_for_active_pending_and_retiring_devices_to_close() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (pending_tx, pending_rx) = mpsc::channel();
        let (retired_tx, retired_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let old = Device(Arc::clone(&events), "active closed");
        let worker_events = Arc::clone(&events);
        let worker = std::thread::spawn(move || {
            retire(Some(old), Some(pending_rx), Some(retired_rx));
            worker_events.lock().unwrap().push("replacement started");
            done_tx.send(()).unwrap();
        });
        assert!(done_rx.try_recv().is_err());
        pending_tx
            .send((1, Ok(Device(Arc::clone(&events), "pending closed"))))
            .unwrap();
        assert!(done_rx.try_recv().is_err());
        retired_tx.send(()).unwrap();
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        worker.join().unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec!["active closed", "pending closed", "replacement started"]
        );
    }
}
