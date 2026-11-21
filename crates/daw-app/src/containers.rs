//! R1 Split and R1 Chain in the app: creating them, what their panel shows
//! of their lanes, the lane edits it asks for, and presets that carry lanes.
use crate::app::DawApp;
use anyhow::{Context, Result, ensure};
use daw_core::{Effect, EffectLane, EffectSlot, MAX_EFFECT_LANES, PluginParameterValue, Project};
use daw_devices::devices::container;
use daw_devices::{DeviceDescriptor, Preset, PresetEffect, PresetLane, PresetPlugin};
use daw_devices_ui::{LaneAction, LaneEffect};

/// A new built-in effect. Containers start with the lanes of their default
/// preset: R1 Split with its three empty bands, R1 Chain with one lane.
pub(crate) fn new_effect(descriptor: &DeviceDescriptor) -> Effect {
    if !container::is_container(descriptor.id) {
        return Effect::Builtin {
            device: descriptor.id.into(),
        };
    }
    let lanes = daw_devices::factory_presets(descriptor)
        .first()
        .map_or(1, |preset| preset.lanes.len().max(1));
    Effect::Container {
        device: descriptor.id.into(),
        lanes: vec![EffectLane::default(); lanes],
    }
}

/// The effect a preset lane entry describes, with fresh IDs throughout.
/// Unknown built-in devices are refused rather than silently dropped.
fn slot_from_preset(effect: &PresetEffect) -> Result<EffectSlot> {
    let mut slot = if let Some(plugin) = &effect.plugin {
        let mut slot = EffectSlot::new(
            plugin_name(effect, &plugin.path),
            Effect::Vst3 {
                path: plugin.path.clone(),
                class_id: plugin.class_id.clone(),
            },
        );
        slot.state = decode_hex(&plugin.state).context("Invalid plug-in state in preset")?;
        for (key, value) in &effect.parameters {
            let id = key.parse().context("Plug-in parameters are keyed by ID")?;
            let value = value.as_f64().context("Plug-in parameters are numbers")?;
            ensure!(
                (0.0..=1.0).contains(&value),
                "Plug-in parameter out of range"
            );
            slot.parameters.push(PluginParameterValue { id, value });
        }
        slot
    } else {
        let descriptor = daw_devices::descriptor(&effect.device)
            .with_context(|| format!("Unknown device {} in preset", effect.device))?;
        let resolved = Preset::nested(effect)
            .resolve(descriptor)
            .map_err(anyhow::Error::msg)?;
        let kind = if container::is_container(descriptor.id) {
            let lanes = effect
                .lanes
                .iter()
                .map(|lane| {
                    Ok(EffectLane {
                        inserts: lane
                            .effects
                            .iter()
                            .map(slot_from_preset)
                            .collect::<Result<_>>()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Effect::Container {
                device: descriptor.id.into(),
                lanes: if lanes.is_empty() {
                    match new_effect(descriptor) {
                        Effect::Container { lanes, .. } => lanes,
                        _ => unreachable!(),
                    }
                } else {
                    lanes
                },
            }
        } else {
            Effect::Builtin {
                device: descriptor.id.into(),
            }
        };
        let mut slot = EffectSlot::new(
            if effect.name.is_empty() {
                descriptor.name
            } else {
                &effect.name
            },
            kind,
        );
        slot.parameters = resolved
            .values
            .iter()
            .map(|(id, value)| PluginParameterValue {
                id: *id,
                value: f64::from(*value),
            })
            .collect();
        slot
    };
    slot.enabled = effect.enabled;
    Ok(slot)
}

fn plugin_name(effect: &PresetEffect, path: &str) -> String {
    if effect.name.is_empty() {
        std::path::Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Plug-in".into())
    } else {
        effect.name.clone()
    }
}

/// What a container's lanes hold, as a preset stores it.
pub(crate) fn preset_lanes(lanes: &[EffectLane]) -> Vec<PresetLane> {
    lanes
        .iter()
        .map(|lane| PresetLane {
            effects: lane.inserts.iter().map(preset_effect).collect(),
        })
        .collect()
}

fn preset_effect(slot: &EffectSlot) -> PresetEffect {
    match &slot.effect {
        Effect::Vst3 { path, class_id } => PresetEffect {
            device: String::new(),
            name: slot.name.clone(),
            enabled: slot.enabled,
            parameters: slot
                .parameters
                .iter()
                .map(|parameter| (parameter.id.to_string(), parameter.value.into()))
                .collect(),
            lanes: Vec::new(),
            plugin: Some(PresetPlugin {
                path: path.clone(),
                class_id: class_id.clone(),
                state: encode_hex(&slot.state),
            }),
        },
        Effect::Builtin { device } | Effect::Container { device, .. } => {
            let parameters = daw_devices::descriptor(device)
                .map(|descriptor| {
                    Preset::capture(descriptor, &slot.name, |id| {
                        crate::builtin_devices::value(descriptor, &slot.parameters, id) as f32
                    })
                    .parameters
                })
                .unwrap_or_default();
            PresetEffect {
                device: device.clone(),
                name: slot.name.clone(),
                enabled: slot.enabled,
                parameters,
                lanes: preset_lanes(slot.effect.lanes()),
                plugin: None,
            }
        }
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len().is_multiple_of(2), "Odd hexadecimal length");
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).map_err(Into::into))
        .collect()
}

/// The lanes of the container `effect` on `track`, as its panel lists them.
pub(crate) fn lanes_view(app: &DawApp, track: u64, effect: u64) -> Option<Vec<Vec<LaneEffect>>> {
    let slot = app.project.track(track)?.effect(effect)?;
    Some(
        slot.effect
            .lanes()
            .iter()
            .map(|lane| {
                lane.inserts
                    .iter()
                    .map(|nested| LaneEffect {
                        id: nested.id,
                        name: nested.name.clone(),
                        model: crate::library::effect_model(&app.vst_catalog, &nested.effect),
                        enabled: nested.enabled,
                    })
                    .collect()
            })
            .collect(),
    )
}

/// The container holding `effect` and the lane it is in, if it is nested.
pub(crate) fn parent(track: &daw_core::Track, effect: u64) -> Option<(u64, usize)> {
    daw_core::effect_slots(&track.inserts).find_map(|slot| {
        slot.effect
            .lanes()
            .iter()
            .position(|lane| lane.inserts.iter().any(|nested| nested.id == effect))
            .map(|lane| (slot.id, lane))
    })
}

fn lanes_mut(project: &mut Project, track: u64, container: u64) -> Result<&mut Vec<EffectLane>> {
    project
        .track_mut(track)
        .context("Track not found")?
        .effect_mut(container)
        .context("Container not found")?
        .effect
        .lanes_mut()
        .context("Not a container")
}

/// Moves a lane's gain, mute and solo with it when lanes before it go.
fn shift_lane_parameters(parameters: &mut Vec<PluginParameterValue>, removed: usize) {
    let lane_of = |id: u32| {
        [
            container::LANE_GAIN,
            container::LANE_MUTE,
            container::LANE_SOLO,
        ]
        .into_iter()
        .find(|base| (*base..*base + MAX_EFFECT_LANES as u32).contains(&id))
        .map(|base| (base, (id - base) as usize))
    };
    parameters.retain(|parameter| lane_of(parameter.id).is_none_or(|(_, lane)| lane != removed));
    for parameter in parameters.iter_mut() {
        if let Some((base, lane)) = lane_of(parameter.id)
            && lane > removed
        {
            parameter.id = base + lane as u32 - 1;
        }
    }
}

/// Applies a lane edit to `project` and names it for Undo. The caller
/// rolls the project back on an error: a failed move has already taken the
/// effect out of its list.
pub(crate) fn edit_lanes(
    project: &mut Project,
    track: u64,
    container: u64,
    action: LaneAction,
) -> Result<&'static str> {
    let label = match action {
        LaneAction::AddLane => {
            let lanes = lanes_mut(project, track, container)?;
            ensure!(
                lanes.len() < MAX_EFFECT_LANES,
                "A Split holds at most 8 lanes"
            );
            lanes.push(EffectLane::default());
            "Add lane"
        }
        LaneAction::RemoveLane { lane } => {
            let lanes = lanes_mut(project, track, container)?;
            ensure!(lane < lanes.len(), "Lane not found");
            ensure!(lanes.len() > 1, "A container keeps at least one lane");
            lanes.remove(lane);
            let slot = project
                .track_mut(track)
                .and_then(|track| track.effect_mut(container))
                .context("Container not found")?;
            shift_lane_parameters(&mut slot.parameters, lane);
            "Remove lane"
        }
        LaneAction::SetEnabled { effect, enabled } => {
            project
                .track_mut(track)
                .and_then(|track| track.effect_mut(effect))
                .context("Effect not found")?
                .enabled = enabled;
            if enabled {
                "Enable effect"
            } else {
                "Bypass effect"
            }
        }
        LaneAction::Move {
            effect,
            lane,
            index,
        } => {
            ensure!(lane < MAX_EFFECT_LANES, "Lane not found");
            let inserts = &mut project.track_mut(track).context("Track not found")?.inserts;
            let (list, from) =
                daw_core::effect_slot_list_mut(inserts, effect).context("Effect not found")?;
            let slot = list.remove(from);
            // Moving a container into its own lane removed the target with it.
            let lanes = lanes_mut(project, track, container)?;
            if lanes.len() <= lane {
                lanes.resize(lane + 1, EffectLane::default());
            }
            let target = &mut lanes[lane].inserts;
            target.insert(index.min(target.len()), slot);
            "Move effect"
        }
        LaneAction::Remove { effect } => {
            project.remove_insert(track, effect)?;
            "Remove effect"
        }
        LaneAction::Open { .. } | LaneAction::AddEffect { .. } => {
            anyhow::bail!("Not an edit")
        }
    };
    project.validate()?;
    Ok(label)
}

/// Adds `slot` to the end of a container lane, creating empty lanes up to
/// it where the container has fewer.
pub(crate) fn insert_into_lane(
    project: &mut Project,
    track: u64,
    container: u64,
    lane: usize,
    slot: EffectSlot,
) -> Result<()> {
    ensure!(lane < MAX_EFFECT_LANES, "Lane not found");
    let lanes = lanes_mut(project, track, container)?;
    if lanes.len() <= lane {
        lanes.resize(lane + 1, EffectLane::default());
    }
    lanes[lane].inserts.push(slot);
    project.validate()
}

/// Loads a container preset into `project`: its parameters and, where the
/// preset has lanes, every lane's effects. The caller rolls back on error.
pub(crate) fn apply_preset(
    project: &mut Project,
    track: u64,
    container: u64,
    preset: &Preset,
) -> Result<daw_devices::ResolvedPreset> {
    let descriptor = daw_devices::descriptor(&preset.device).context("Unknown device in preset")?;
    let resolved = preset.resolve(descriptor).map_err(anyhow::Error::msg)?;
    let lanes = preset
        .lanes
        .iter()
        .map(|lane| {
            Ok(EffectLane {
                inserts: lane
                    .effects
                    .iter()
                    .map(slot_from_preset)
                    .collect::<Result<_>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let slot = project
        .track_mut(track)
        .and_then(|track| track.effect_mut(container))
        .context("Container not found")?;
    let Effect::Container {
        device,
        lanes: held,
    } = &mut slot.effect
    else {
        anyhow::bail!("Not a container");
    };
    ensure!(*device == preset.device, "Preset is for another device");
    if !lanes.is_empty() {
        *held = lanes;
    }
    slot.parameters = resolved
        .values
        .iter()
        .map(|(id, value)| PluginParameterValue {
            id: *id,
            value: f64::from(*value),
        })
        .collect();
    project.validate()?;
    Ok(resolved)
}

impl DawApp {
    /// Applies what a container panel asked for, as one Undo step.
    pub(crate) fn apply_lane_action(&mut self, track: u64, container: u64, action: LaneAction) {
        match action {
            LaneAction::Open { effect } => self.open_effect_editor(track, effect),
            LaneAction::AddEffect { lane } => {
                self.open_effect_chooser(crate::library::LibraryTarget::Insert {
                    track,
                    anchor: None,
                    lane: Some((container, lane)),
                });
            }
            action => {
                let mut candidate = self.project.clone();
                match edit_lanes(&mut candidate, track, container, action) {
                    Ok(label) => self.commit_candidate(candidate, label),
                    Err(error) => self.message(Err(error), ""),
                }
            }
        }
    }

    /// Loads a container preset as one Undo step.
    pub(crate) fn load_container_preset(
        &mut self,
        track: u64,
        container: u64,
        preset: &Preset,
    ) -> Result<()> {
        let mut candidate = self.project.clone();
        apply_preset(&mut candidate, track, container, preset)?;
        self.commit_candidate(candidate, "Load preset");
        Ok(())
    }

    /// Replaces the project with an edited copy as one named Undo step.
    pub(crate) fn commit_candidate(&mut self, candidate: Project, label: &str) {
        self.begin_edit();
        self.project = candidate;
        self.edit_label = Some(label.into());
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_presets_build_valid_lanes() {
        for descriptor in [&container::SPLIT, &container::CHAIN] {
            for preset in daw_devices::factory_presets(descriptor) {
                let lanes: Vec<_> = preset
                    .lanes
                    .iter()
                    .map(|lane| EffectLane {
                        inserts: lane
                            .effects
                            .iter()
                            .map(|effect| slot_from_preset(effect).unwrap())
                            .collect(),
                    })
                    .collect();
                let saved = preset_lanes(&lanes);
                assert_eq!(saved.len(), preset.lanes.len(), "{}", preset.name);
                let mut project = daw_core::Project::new();
                let track = project.add_audio_track("Audio");
                project
                    .track_mut(track)
                    .unwrap()
                    .inserts
                    .push(EffectSlot::new(
                        descriptor.name,
                        Effect::Container {
                            device: descriptor.id.into(),
                            lanes,
                        },
                    ));
                project.validate().unwrap();
            }
        }
    }

    #[test]
    fn plugin_state_survives_a_preset() {
        let mut slot = EffectSlot::new(
            "Plug",
            Effect::Vst3 {
                path: "/plugins/Plug.vst3".into(),
                class_id: Some("ABCD".into()),
            },
        );
        slot.state = vec![0, 1, 254, 255];
        slot.parameters = vec![PluginParameterValue { id: 7, value: 0.25 }];
        let restored = slot_from_preset(&preset_effect(&slot)).unwrap();
        assert_eq!(restored.state, slot.state);
        assert_eq!(restored.parameters, slot.parameters);
        assert_eq!(restored.effect, slot.effect);
        assert_ne!(restored.id, slot.id);
    }

    #[test]
    fn removing_a_lane_moves_the_later_lanes_settings() {
        let mut parameters = vec![
            PluginParameterValue {
                id: container::LANE_GAIN,
                value: 0.1,
            },
            PluginParameterValue {
                id: container::LANE_GAIN + 1,
                value: 0.2,
            },
            PluginParameterValue {
                id: container::LANE_SOLO + 2,
                value: 1.0,
            },
            PluginParameterValue {
                id: container::MIX,
                value: 0.5,
            },
        ];
        shift_lane_parameters(&mut parameters, 1);
        assert_eq!(
            parameters,
            vec![
                PluginParameterValue {
                    id: container::LANE_GAIN,
                    value: 0.1,
                },
                PluginParameterValue {
                    id: container::LANE_SOLO + 1,
                    value: 1.0,
                },
                PluginParameterValue {
                    id: container::MIX,
                    value: 0.5,
                },
            ]
        );
    }
}
