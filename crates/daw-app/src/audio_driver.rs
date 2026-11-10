//! Audio driver (host) choice: which drivers exist, which devices each offers,
//! and keeping the saved choice usable when the machine changes.
use crate::{app::DawApp, settings::Settings};
use daw_engine::{DeviceInfo, HostInfo};

/// Headless stand-in for a driver. `fixed` is (sample rate, buffer frames) for
/// a server-dictated driver such as JACK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VirtualHost {
    pub name: String,
    pub fixed: Option<(u32, u32)>,
}

impl VirtualHost {
    /// `RILL_E2E_AUDIO_HOSTS`: comma-separated `Name` or `Name:rate:frames`;
    /// the first entry is the default driver.
    pub fn parse_list(spec: &str) -> Vec<Self> {
        spec.split(',')
            .filter_map(|entry| {
                let mut parts = entry.trim().split(':');
                let name = parts.next().filter(|name| !name.is_empty())?;
                let fixed = parts
                    .next()
                    .and_then(|rate| rate.parse().ok())
                    .zip(parts.next().and_then(|frames| frames.parse().ok()));
                Some(Self {
                    name: name.into(),
                    fixed,
                })
            })
            .collect()
    }
}

fn selected_device<'a>(devices: &'a [DeviceInfo], name: &Option<String>) -> Option<&'a DeviceInfo> {
    devices.iter().find(|device| {
        name.as_ref()
            .map_or(device.is_default, |name| *name == device.name)
    })
}

/// The driver decides what the device can do. Follow its single sample rate and
/// keep the buffer inside its range, so controls the user cannot change always
/// show what the driver will really use.
pub(crate) fn follow_driver_limits(devices: &[DeviceInfo], settings: &mut Settings) {
    let Some(device) = selected_device(devices, &settings.device_name) else {
        return;
    };
    if let [rate] = device.supported_sample_rates[..] {
        settings.sample_rate = rate;
    }
    if let Some((min, max)) = device.buffer_frames {
        settings.buffer_frames = settings.buffer_frames.clamp(min, max);
    }
}

/// True when the driver leaves no choice of buffer size.
pub(crate) fn buffer_is_fixed(device: Option<&DeviceInfo>) -> bool {
    device.is_some_and(|device| device.buffer_frames.is_some_and(|(min, max)| min == max))
}

/// Buffer sizes to offer: the usual ladder inside the driver's range.
pub(crate) fn buffer_choices(device: Option<&DeviceInfo>) -> Vec<u32> {
    let Some((min, max)) = device.and_then(|device| device.buffer_frames) else {
        return vec![64, 128, 256, 512, 1024, 2048];
    };
    let mut sizes: Vec<_> = [64, 128, 256, 512, 1024, 2048]
        .into_iter()
        .filter(|size| (min..=max).contains(size))
        .collect();
    if sizes.is_empty() || min == max {
        sizes = vec![min];
    }
    sizes
}

pub(crate) fn driver_name(hosts: &[HostInfo], chosen: Option<&str>) -> String {
    chosen
        .map(str::to_owned)
        .or_else(|| {
            hosts
                .iter()
                .find(|host| host.is_default)
                .map(|host| host.name.clone())
        })
        .unwrap_or_default()
}

impl DawApp {
    pub(crate) fn refresh_hosts(&mut self) {
        self.audio_hosts = if self.virtual_audio {
            self.virtual_hosts
                .iter()
                .enumerate()
                .map(|(index, host)| HostInfo {
                    name: host.name.clone(),
                    is_default: index == 0,
                })
                .collect()
        } else if self.audio_device_enabled {
            daw_engine::list_hosts()
        } else {
            Vec::new()
        };
    }

    /// A saved driver this machine no longer offers (JACK uninstalled, ASIO build
    /// not installed) falls back to the platform default with its devices.
    /// The saved file is left alone, so the choice returns with the driver.
    pub fn reconcile_audio_host(&mut self) {
        self.refresh_hosts();
        let Some(saved) = self.settings.audio_host.clone() else {
            return;
        };
        if self.audio_hosts.is_empty() {
            return;
        }
        if self.audio_hosts.iter().any(|host| host.name == saved) {
            self.follow_saved_driver_limits();
            return;
        }
        self.settings.audio_host = None;
        self.settings.device_name = None;
        self.settings.input_device_name = None;
        self.setup_draft = self.settings.clone();
        self.message(
            Err(anyhow::anyhow!(
                "Audio driver '{saved}' is unavailable; using the default driver"
            )),
            "",
        );
    }

    /// Drivers that dictate the rate or buffer overrule the saved values for
    /// this session. Run after the device list is known.
    fn follow_saved_driver_limits(&mut self) {
        self.refresh_devices();
        follow_driver_limits(&self.devices, &mut self.settings);
        self.setup_draft.sample_rate = self.settings.sample_rate;
        self.setup_draft.buffer_frames = self.settings.buffer_frames;
    }

    /// One line under the Audio form when the chosen driver needs explaining.
    pub(crate) fn driver_note(&self) -> Option<String> {
        let host = self.setup_draft.audio_host.as_deref()?;
        if self.devices.is_empty() {
            return Some(if host == "JACK" {
                "No JACK server is running. Start JACK (or PipeWire's JACK support), then press Refresh devices.".into()
            } else {
                format!("{host} reports no devices. Check the driver, then press Refresh devices.")
            });
        }
        let fixed = self.devices.iter().any(|device| {
            device.supported_sample_rates.len() == 1 || buffer_is_fixed(Some(device))
        });
        fixed.then(|| {
            format!("{host} sets the sample rate and buffer size; change them in its own settings.")
        })
    }

    pub(crate) fn virtual_device(&self, host: Option<&str>, input: bool) -> DeviceInfo {
        let fixed = self
            .virtual_hosts
            .iter()
            .skip(1)
            .find(|candidate| Some(candidate.name.as_str()) == host)
            .and_then(|candidate| candidate.fixed);
        DeviceInfo {
            name: if input {
                "Rill virtual input"
            } else {
                "Rill virtual output"
            }
            .into(),
            is_default: true,
            supported_sample_rates: fixed
                .map_or_else(|| vec![44100, 48000, 88200, 96000], |(rate, _)| vec![rate]),
            channels: if input {
                self.virtual_io.input_channels
            } else {
                2
            },
            buffer_frames: fixed.map(|(_, frames)| (frames, frames)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(rates: &[u32], buffers: Option<(u32, u32)>) -> DeviceInfo {
        DeviceInfo {
            name: "Fixture".into(),
            is_default: true,
            supported_sample_rates: rates.to_vec(),
            channels: 2,
            buffer_frames: buffers,
        }
    }

    #[test]
    fn virtual_host_list_parses_names_and_fixed_server_values() {
        assert_eq!(
            VirtualHost::parse_list("ALSA, JACK:44100:128,,Odd:x:y"),
            vec![
                VirtualHost {
                    name: "ALSA".into(),
                    fixed: None
                },
                VirtualHost {
                    name: "JACK".into(),
                    fixed: Some((44100, 128))
                },
                VirtualHost {
                    name: "Odd".into(),
                    fixed: None
                },
            ]
        );
    }

    #[test]
    fn a_server_dictated_driver_overrides_rate_and_buffer() {
        let mut settings = Settings::default();
        follow_driver_limits(&[device(&[44100], Some((128, 128)))], &mut settings);
        assert_eq!((settings.sample_rate, settings.buffer_frames), (44100, 128));
    }

    #[test]
    fn a_buffer_range_clamps_without_touching_the_rate() {
        let mut settings = Settings {
            buffer_frames: 2048,
            ..Settings::default()
        };
        follow_driver_limits(&[device(&[44100, 48000], Some((64, 512)))], &mut settings);
        assert_eq!((settings.sample_rate, settings.buffer_frames), (48000, 512));
        assert_eq!(
            buffer_choices(Some(&device(&[], Some((64, 512))))),
            [64, 128, 256, 512]
        );
        assert_eq!(
            buffer_choices(Some(&device(&[], Some((4800, 4800))))),
            [4800]
        );
        assert!(buffer_is_fixed(Some(&device(&[], Some((4800, 4800))))));
        assert!(!buffer_is_fixed(None));
    }

    #[test]
    fn missing_saved_driver_falls_back_with_feedback_and_keeps_the_file() {
        let ctx = eframe::egui::Context::default();
        let mut app = DawApp::with_context(&ctx, daw_core::Project::new(), None);
        app.virtual_audio = true;
        app.virtual_hosts = VirtualHost::parse_list("ALSA,JACK");
        app.settings.audio_host = Some("ASIO".into());
        app.settings.device_name = Some("Old interface".into());
        app.reconcile_audio_host();
        assert_eq!(app.settings.audio_host, None);
        assert_eq!(app.settings.device_name, None);
        assert!(app.error);
        assert!(app.status.contains("'ASIO' is unavailable"));
        // An available driver is left alone.
        app.settings.audio_host = Some("JACK".into());
        app.error = false;
        app.reconcile_audio_host();
        assert_eq!(app.settings.audio_host.as_deref(), Some("JACK"));
        assert!(!app.error);
    }
}
