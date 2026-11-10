//! Audio driver (CPAL host) selection shared by output, input and enumeration.
use anyhow::{Result, anyhow};
use cpal::HostId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    pub name: String,
    pub is_default: bool,
}

/// The drivers this build and machine offer, platform default first.
pub fn list_hosts() -> Vec<HostInfo> {
    let default = cpal::default_host().id();
    let mut hosts: Vec<_> = cpal::available_hosts()
        .into_iter()
        .map(|id| HostInfo {
            name: id.name().to_owned(),
            is_default: id == default,
        })
        .collect();
    hosts.sort_by_key(|host| !host.is_default);
    hosts
}

/// ASIO drives one device for both directions, so capture follows the output choice.
pub fn is_single_device_host(name: &str) -> bool {
    name.eq_ignore_ascii_case("ASIO")
}

/// `None` is the platform default. A named driver must be available now.
fn pick_host(available: &[HostId], default: HostId, wanted: Option<&str>) -> Result<HostId> {
    let Some(name) = wanted else {
        return Ok(default);
    };
    available
        .iter()
        .copied()
        .find(|id| id.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow!("Audio driver '{name}' is unavailable"))
}

pub(crate) fn open_host(wanted: Option<&str>) -> Result<cpal::Host> {
    let Some(name) = wanted else {
        return Ok(cpal::default_host());
    };
    let id = pick_host(
        &cpal::available_hosts(),
        cpal::default_host().id(),
        Some(name),
    )?;
    cpal::host_from_id(id)
        .map_err(|error| anyhow!("Audio driver '{name}' failed to start: {error}"))
}

/// JACK creates no devices when its server is down, which reads as a missing
/// default device. Name the actual cause.
pub(crate) fn no_default_device(host: &cpal::Host, direction: &str) -> anyhow::Error {
    if host.id().name() == "JACK" {
        anyhow!(
            "No JACK server is running. Start JACK (or PipeWire's JACK support), then refresh devices"
        )
    } else {
        anyhow!("No default audio {direction} device is available")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_when_unset_and_named_hosts_resolve_case_insensitively() {
        let default = cpal::default_host().id();
        let available = cpal::available_hosts();
        assert_eq!(pick_host(&available, default, None).unwrap(), default);
        let name = default.name().to_ascii_lowercase();
        assert_eq!(
            pick_host(&available, default, Some(&name)).unwrap(),
            default
        );
    }

    #[test]
    fn unavailable_driver_is_an_error_not_a_silent_default() {
        let default = cpal::default_host().id();
        let error = pick_host(&cpal::available_hosts(), default, Some("No such driver"))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Audio driver 'No such driver' is unavailable");
        assert!(open_host(Some("No such driver")).is_err());
        assert!(open_host(None).is_ok());
    }

    #[test]
    fn host_list_starts_with_the_platform_default() {
        let hosts = list_hosts();
        assert!(hosts[0].is_default);
        assert_eq!(hosts.iter().filter(|host| host.is_default).count(), 1);
        assert_eq!(hosts[0].name, cpal::default_host().id().name());
    }

    #[test]
    fn only_asio_shares_one_device_between_directions() {
        assert!(is_single_device_host("ASIO"));
        assert!(!is_single_device_host("JACK"));
        assert!(!is_single_device_host("WASAPI"));
    }
}
