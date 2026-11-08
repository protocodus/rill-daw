//! Bounded native bus storage and stereo mapping shared by the proxy and helper.
use anyhow::{Context, Result, ensure};
use vst3_host::{AudioBusConfig, AudioBusLayout, audio::AudioBusBuffer};

const MAX_AUDIO_BUSES: usize = 64;
pub(crate) const MAX_AUDIO_CHANNELS: usize = 256;
const MAX_BUS_BUFFER_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn validate_audio_bus_layout(layout: &AudioBusLayout, frames: usize) -> Result<()> {
    ensure!(
        layout.inputs.len() <= MAX_AUDIO_BUSES && layout.outputs.len() <= MAX_AUDIO_BUSES,
        "Plug-in exceeds the 64 audio buses per direction limit"
    );
    let channels = layout
        .inputs
        .iter()
        .chain(&layout.outputs)
        .try_fold(0usize, |sum, bus| {
            ensure!(
                bus.channel_count <= MAX_AUDIO_CHANNELS,
                "Plug-in audio bus has too many channels"
            );
            sum.checked_add(bus.channel_count)
                .context("Plug-in channel count overflow")
        })?;
    ensure!(
        channels <= MAX_AUDIO_CHANNELS,
        "Plug-in exceeds the 256 audio channels limit"
    );
    let bytes = channels
        .checked_mul(frames)
        .and_then(|count| count.checked_mul(std::mem::size_of::<f32>()))
        .context("Plug-in audio buffer size overflow")?;
    ensure!(
        bytes <= MAX_BUS_BUFFER_BYTES,
        "Plug-in audio buffers exceed 64 MiB"
    );
    Ok(())
}

pub(crate) fn copy_stereo_input(channels: &mut [Vec<f32>], left: &[f32], right: &[f32]) {
    if channels.len() == 1 {
        for ((output, left), right) in channels[0].iter_mut().zip(left).zip(right) {
            *output = (left + right) * 0.5;
        }
    } else if channels.len() >= 2 {
        channels[0].copy_from_slice(left);
        channels[1].copy_from_slice(right);
    }
}

pub(crate) fn validate_pair(
    buses: &[vst3_host::audio::AudioBusBuffer],
    bus: usize,
    first_channel: usize,
) -> Result<()> {
    ensure!(
        first_channel.is_multiple_of(2),
        "Plug-in stereo pair must begin at an even channel"
    );
    let config = buses
        .get(bus)
        .context("Plug-in audio bus index is out of range")?;
    ensure!(config.active, "Plug-in audio bus is inactive");
    ensure!(
        first_channel < config.channels.len(),
        "Plug-in audio channel index is out of range"
    );
    Ok(())
}

/// Resize existing sample storage and reset it before the next block. Shrinking
/// retains capacity so variable final blocks do not allocate when playback resumes.
pub(crate) fn prepare_channels(channels: &mut [Vec<f32>], frames: usize) {
    for channel in channels {
        channel.resize(frames, 0.0);
        channel.fill(0.0);
    }
}

pub(crate) fn prepare_outputs(outputs: &mut Vec<Vec<f32>>, channels: usize, frames: usize) {
    outputs.resize_with(channels, Vec::new);
    prepare_channels(outputs, frames);
}

pub(crate) fn prepare_bus_outputs(
    outputs: &mut Vec<AudioBusBuffer>,
    layout: &[AudioBusConfig],
    frames: usize,
) {
    outputs.resize_with(layout.len(), || AudioBusBuffer::new(0, 0, false));
    for (bus, config) in outputs.iter_mut().zip(layout) {
        bus.active = config.active;
        prepare_outputs(&mut bus.channels, config.channel_count, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_storage_is_reused_and_silenced_across_variable_blocks() {
        let layout = [
            AudioBusConfig {
                channel_count: 2,
                active: true,
            },
            AudioBusConfig {
                channel_count: 1,
                active: false,
            },
        ];
        let mut outputs = Vec::new();
        prepare_bus_outputs(&mut outputs, &layout, 64);
        let pointers: Vec<_> = outputs
            .iter()
            .flat_map(|bus| bus.channels.iter())
            .map(|channel| channel.as_ptr())
            .collect();
        for frames in [64, 8, 0, 64] {
            for bus in &mut outputs {
                for channel in &mut bus.channels {
                    channel.fill(1.0);
                }
            }
            prepare_bus_outputs(&mut outputs, &layout, frames);
            for (channel, pointer) in outputs
                .iter()
                .flat_map(|bus| bus.channels.iter())
                .zip(&pointers)
            {
                assert_eq!(
                    channel.as_ptr(),
                    *pointer,
                    "steady-state channels must not reallocate"
                );
                assert_eq!(channel.len(), frames);
                assert!(channel.iter().all(|sample| *sample == 0.0));
            }
            assert!(outputs[0].active);
            assert!(!outputs[1].active);
        }
    }

    #[test]
    fn recycled_outputs_follow_changed_native_layouts() {
        let mut outputs = vec![
            AudioBusBuffer::new(2, 64, true),
            AudioBusBuffer::new(5, 64, true),
        ];
        prepare_bus_outputs(
            &mut outputs,
            &[AudioBusConfig {
                channel_count: 1,
                active: false,
            }],
            8,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].channels, vec![vec![0.0; 8]]);
        assert!(!outputs[0].active);
        prepare_bus_outputs(
            &mut outputs,
            &[
                AudioBusConfig {
                    channel_count: 3,
                    active: true,
                },
                AudioBusConfig {
                    channel_count: 0,
                    active: false,
                },
            ],
            16,
        );
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].channels, vec![vec![0.0; 16]; 3]);
        assert!(outputs[0].active);
        assert!(outputs[1].channels.is_empty());
    }
}
