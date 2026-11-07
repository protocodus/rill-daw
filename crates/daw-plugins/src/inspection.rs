//! On-demand discovery of a plug-in's ports and controller catalogue.
//! All native code runs in a disposable helper, with one bounded deadline.
use crate::{PluginInfo, helper_path};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use vst3_host::process_isolation::{HostCommand, HostResponse, PluginHostProcess};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginAudioBus {
    pub channels: usize,
    pub active: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub main: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginParameterMetadata {
    pub id: u32,
    pub name: String,
    pub unit: String,
    pub steps: i32,
    pub read_only: bool,
    pub can_automate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginInspection {
    pub inputs: Vec<PluginAudioBus>,
    pub outputs: Vec<PluginAudioBus>,
    pub midi_input: bool,
    pub midi_output: bool,
    pub parameters: Vec<PluginParameterMetadata>,
}

/// Call from a worker thread. This neither opens an editor nor changes any
/// project instance. Channel counts describe the plug-in's initial bus layout.
pub fn inspect_plugin(plugin: &PluginInfo) -> Result<PluginInspection> {
    inspect_plugin_with_state(plugin, &[])
}

/// Inspect the bus layout after restoring this instance's captured state. A preset may
/// expose different ports from the factory default; the project is never mutated here.
pub fn inspect_plugin_with_state(plugin: &PluginInfo, state: &[u8]) -> Result<PluginInspection> {
    inspect_plugin_with_configuration(plugin, state, &[])
}

/// Inspect saved state plus normalized overrides in playback preparation order.
pub fn inspect_plugin_with_configuration(
    plugin: &PluginInfo,
    state: &[u8],
    parameters: &[(u32, f64)],
) -> Result<PluginInspection> {
    ensure!(
        state.len() <= 64 * 1024 * 1024,
        "Plug-in state exceeds 64 MiB"
    );
    ensure!(
        parameters.len() <= 4096,
        "Too many plug-in parameter overrides for inspection"
    );
    ensure!(
        parameters
            .iter()
            .all(|(_, value)| value.is_finite() && (0.0..=1.0).contains(value)),
        "Invalid normalized plug-in parameter override"
    );
    let budget = Duration::from_secs(30);
    let deadline = Instant::now() + budget;
    let mut worker = PluginHostProcess::new(Some(helper_path()?), budget)
        .map_err(anyhow::Error::msg)
        .context("Starting isolated plug-in inspection")?;
    let mut send = |command| -> Result<HostResponse> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "Plug-in inspection exceeded 30 seconds"
        );
        worker.set_timeout(remaining);
        worker.set_slow_command_timeout(remaining);
        let response = worker.send_command(command).map_err(anyhow::Error::msg)?;
        if let HostResponse::Error { message } = response {
            bail!("{message}");
        }
        Ok(response)
    };
    let loaded = send(HostCommand::LoadPlugin {
        path: plugin.path.to_string_lossy().into_owned(),
        sample_rate: 48_000.0,
        block_size: 512,
        tempo: 120.0,
        time_sig_numerator: 4,
        time_sig_denominator: 4,
        class_id: plugin.class_id.clone(),
    })
    .context("Loading plug-in details")?;
    let HostResponse::PluginInfo {
        has_midi_input,
        has_midi_output,
        ..
    } = loaded
    else {
        bail!("Unexpected plug-in inspection response");
    };
    if !state.is_empty() {
        let response = send(HostCommand::LoadState {
            data: state.to_vec(),
            context: vst3_host::plugin::StateContext::Project,
        })?;
        ensure!(
            matches!(response, HostResponse::Success { .. }),
            "Plug-in state restore failed during inspection"
        );
    }
    for &(id, value) in parameters {
        let response = send(HostCommand::SetParameter { id, value })?;
        ensure!(
            matches!(response, HostResponse::Success { .. }),
            "Plug-in parameter override failed during inspection"
        );
    }
    if !state.is_empty() || !parameters.is_empty() {
        ensure!(
            matches!(
                send(HostCommand::ServiceHostRequests)?,
                HostResponse::RestartFlags { .. }
            ),
            "Unexpected plug-in restart response during inspection"
        );
    }
    let HostResponse::AudioBusLayout { layout } = send(HostCommand::AudioBusLayout)? else {
        bail!("Plug-in did not report an audio bus layout");
    };
    crate::validate_audio_bus_layout(&layout, 512)?;
    let HostResponse::AudioBusDetails { details } = send(HostCommand::AudioBusDetails)? else {
        bail!("Plug-in did not report native audio bus details");
    };
    ensure!(
        details.inputs.len() == layout.inputs.len()
            && details.outputs.len() == layout.outputs.len(),
        "Plug-in audio bus layout changed during inspection"
    );
    ensure!(
        details
            .inputs
            .iter()
            .chain(&details.outputs)
            .all(|bus| bus.name.len() <= 1024),
        "Plug-in audio bus name exceeds the metadata limit"
    );
    let HostResponse::Parameters { params } = send(HostCommand::GetAllParameters)? else {
        bail!("Plug-in did not report a parameter catalogue");
    };
    Ok(PluginInspection {
        inputs: layout
            .inputs
            .into_iter()
            .zip(details.inputs)
            .map(|(bus, details)| PluginAudioBus {
                channels: bus.channel_count,
                active: bus.active,
                name: details.name,
                main: details.main,
            })
            .collect(),
        outputs: layout
            .outputs
            .into_iter()
            .zip(details.outputs)
            .map(|(bus, details)| PluginAudioBus {
                channels: bus.channel_count,
                active: bus.active,
                name: details.name,
                main: details.main,
            })
            .collect(),
        midi_input: has_midi_input,
        midi_output: has_midi_output,
        parameters: params
            .into_iter()
            .map(|parameter| PluginParameterMetadata {
                id: parameter.id,
                name: parameter.name,
                unit: parameter.unit,
                steps: parameter.step_count,
                read_only: parameter.is_read_only,
                can_automate: parameter.can_automate,
            })
            .collect(),
    })
}
