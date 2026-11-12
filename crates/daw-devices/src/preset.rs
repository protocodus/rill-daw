//! The `.rillpreset` format, shared by every device. See `docs/R1_DEVICES.md#presets`.
use crate::{DeviceDescriptor, ParamRange};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PRESET_FORMAT: u32 = 1;
pub const PRESET_EXTENSION: &str = "rillpreset";
const MAX_PRESET_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub format: u32,
    pub device: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Parameter key → plain value in the parameter's unit; choices by label,
    /// toggles as booleans.
    #[serde(default)]
    pub parameters: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<PresetSample>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetSample {
    pub slot: u32,
    /// Relative to the preset file.
    pub file: String,
}

/// A preset applied to a descriptor: every parameter's normalized value, in
/// descriptor order, plus what was skipped.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPreset {
    pub values: Vec<(u32, f32)>,
    /// Keys the device does not have, and values of the wrong kind; those
    /// parameters keep their default.
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetError(pub String);

impl std::fmt::Display for PresetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PresetError {}

impl Preset {
    pub fn parse(text: &str) -> Result<Self, PresetError> {
        if text.len() > MAX_PRESET_BYTES {
            return Err(PresetError("Preset is larger than 1 MiB".into()));
        }
        let preset: Self = serde_json::from_str(text)
            .map_err(|error| PresetError(format!("Invalid preset: {error}")))?;
        if preset.format != PRESET_FORMAT {
            return Err(PresetError(format!(
                "Preset format {} is not supported",
                preset.format
            )));
        }
        if preset.name.trim().is_empty() || preset.name.len() > 128 {
            return Err(PresetError("Preset name must be 1–128 characters".into()));
        }
        Ok(preset)
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("preset serializes");
        text.push('\n');
        text
    }

    /// Captures the current values (normalized, by ID) of every parameter.
    pub fn capture(descriptor: &DeviceDescriptor, name: &str, value: impl Fn(u32) -> f32) -> Self {
        let parameters = descriptor
            .parameters
            .iter()
            .map(|spec| {
                let normalized = value(spec.id);
                let plain = spec.to_plain(normalized);
                let value = match spec.range {
                    ParamRange::Choice(_) => {
                        Value::from(spec.choice_label(normalized).unwrap_or_default())
                    }
                    ParamRange::Toggle => Value::from(plain >= 0.5),
                    // Round away float noise so saved files stay readable.
                    _ => Value::from((f64::from(plain) * 1e4).round() / 1e4),
                };
                (spec.key.to_string(), value)
            })
            .collect();
        Self {
            format: PRESET_FORMAT,
            device: descriptor.id.into(),
            name: name.into(),
            author: String::new(),
            category: String::new(),
            tags: Vec::new(),
            description: String::new(),
            parameters,
            samples: Vec::new(),
        }
    }

    /// Tolerant by rule: a missing parameter takes its default, an unknown key
    /// or a value of the wrong kind is skipped and reported, an out-of-range
    /// number is clamped. A preset for another device is refused.
    pub fn resolve(&self, descriptor: &DeviceDescriptor) -> Result<ResolvedPreset, PresetError> {
        if self.device != descriptor.id {
            return Err(PresetError(format!(
                "Preset \"{}\" is for {}, not {}",
                self.name, self.device, descriptor.id
            )));
        }
        let mut skipped: Vec<String> = self
            .parameters
            .keys()
            .filter(|key| descriptor.parameter_by_key(key).is_none())
            .cloned()
            .collect();
        let values = descriptor
            .parameters
            .iter()
            .map(|spec| {
                let normalized = match self.parameters.get(spec.key) {
                    None => spec.default_normalized(),
                    Some(value) => {
                        let parsed = match (value, spec.range) {
                            (Value::String(label), ParamRange::Choice(_)) => {
                                spec.parse(label).or_else(|| {
                                    descriptor
                                        .legacy_labels
                                        .iter()
                                        .find(|(key, old, _)| {
                                            *key == spec.key
                                                && old.eq_ignore_ascii_case(label.trim())
                                        })
                                        .and_then(|(_, _, current)| spec.parse(current))
                                })
                            }
                            (Value::Bool(on), ParamRange::Toggle) => Some(f32::from(u8::from(*on))),
                            (Value::Number(number), _) => number
                                .as_f64()
                                .map(|plain| spec.to_normalized(plain as f32)),
                            _ => None,
                        };
                        parsed.unwrap_or_else(|| {
                            skipped.push(spec.key.to_string());
                            spec.default_normalized()
                        })
                    }
                };
                (spec.id, normalized)
            })
            .collect();
        Ok(ResolvedPreset { values, skipped })
    }
}

/// The compiled-in presets of a device. A broken factory preset is a build
/// defect caught by the conformance tests, so it is left out here.
pub fn factory_presets(descriptor: &DeviceDescriptor) -> Vec<Preset> {
    descriptor
        .factory_presets
        .iter()
        .filter_map(|text| Preset::parse(text).ok())
        .collect()
}

/// The values a roll of the dice gives a device's parameters, as normalized
/// `(id, value)` pairs of the ones that change. `amount` (0–1) is how far a
/// value may move across its range: a continuous one lands anywhere within
/// that distance of where it is, and a choice or switch moves to another
/// option, picked at random, with that chance. The output level and volume
/// stay as they are, so no roll is a jump in loudness, and so do the
/// parameters the descriptor fixes ([`DeviceDescriptor::fixed_on_roll`]).
/// `seed` picks the roll.
pub fn randomized(
    descriptor: &DeviceDescriptor,
    value: impl Fn(u32) -> f32,
    amount: f32,
    seed: u64,
) -> Vec<(u32, f32)> {
    let amount = amount.clamp(0.0, 1.0);
    // SplitMix64: a whole roll from one seed, uniform in 0..1.
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    descriptor
        .parameters
        .iter()
        .filter(|spec| {
            let last = spec.key.rsplit('.').next().unwrap_or(spec.key);
            !matches!(last, "output" | "volume") && !descriptor.fixed_on_roll.contains(&last)
        })
        .filter_map(|spec| {
            let current = value(spec.id);
            let rolled = match spec.range {
                ParamRange::Choice(_) | ParamRange::Toggle => {
                    let count = spec.steps().unwrap_or(1);
                    if count < 2 || next() >= amount {
                        return None;
                    }
                    let index = spec.to_plain(current) as usize;
                    let other = 1 + ((next() * (count - 1) as f32) as usize).min(count - 2);
                    spec.to_normalized(((index + other) % count) as f32)
                }
                _ => {
                    let low = (current - amount).max(0.0);
                    let high = (current + amount).min(1.0);
                    // Whole numbers snap to the nearest one.
                    spec.to_normalized(spec.to_plain(low + next() * (high - low)))
                }
            };
            (rolled != current).then_some((spec.id, rolled))
        })
        .collect()
}
