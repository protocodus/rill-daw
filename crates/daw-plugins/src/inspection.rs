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
    let HostResponse::AudioBusLayout { layout } = send(HostCommand::AudioBusLayout)? else {
        bail!("Plug-in did not report an audio bus layout");
    };
    let HostResponse::Parameters { params } = send(HostCommand::GetAllParameters)? else {
        bail!("Plug-in did not report a parameter catalogue");
    };
    Ok(PluginInspection {
        inputs: layout
            .inputs
            .into_iter()
            .map(|bus| PluginAudioBus {
                channels: bus.channel_count,
                active: bus.active,
            })
            .collect(),
        outputs: layout
            .outputs
            .into_iter()
            .map(|bus| PluginAudioBus {
                channels: bus.channel_count,
                active: bus.active,
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
