//! Sample only the operating system's CPU counters, outside the UI and audio threads.
use daw_ui::CpuLoad;
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub(crate) struct CpuMonitor {
    latest: Arc<Mutex<Arc<CpuLoad>>>,
    // Closing the channel on drop wakes and stops the sampler immediately.
    _stop: Option<mpsc::Sender<()>>,
}

impl Default for CpuMonitor {
    fn default() -> Self {
        let mut monitor = Self::fixed(CpuLoad::Measuring);
        let latest = monitor.latest.clone();
        let (stop, shutdown) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("system-cpu".into())
            .spawn(move || {
                let mut system = sysinfo::System::new();
                system.refresh_cpu_usage();
                while shutdown.recv_timeout(Duration::from_secs(1))
                    == Err(mpsc::RecvTimeoutError::Timeout)
                {
                    system.refresh_cpu_usage();
                    let percent = |value: f32| value.round().clamp(0.0, 100.0) as u8;
                    let cores: Vec<_> = system
                        .cpus()
                        .iter()
                        .map(|cpu| percent(cpu.cpu_usage()))
                        .collect();
                    let sample = if cores.is_empty() {
                        CpuLoad::Unavailable
                    } else {
                        CpuLoad::Sample {
                            total: percent(system.global_cpu_usage()),
                            cores,
                        }
                    };
                    *latest.lock().unwrap() = Arc::new(sample);
                }
            });
        if started.is_ok() {
            monitor._stop = Some(stop);
        } else {
            *monitor.latest.lock().unwrap() = Arc::new(CpuLoad::Unavailable);
        }
        monitor
    }
}

impl CpuMonitor {
    pub(crate) fn fixed(sample: CpuLoad) -> Self {
        Self {
            latest: Arc::new(Mutex::new(Arc::new(sample))),
            _stop: None,
        }
    }

    pub(crate) fn sample(&self) -> Arc<CpuLoad> {
        self.latest.lock().unwrap().clone()
    }
}
