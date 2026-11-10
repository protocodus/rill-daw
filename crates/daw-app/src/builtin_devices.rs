//! The app's view of Rill's built-in devices: Library entries, the project
//! variants they create, parameter metadata and user presets. Everything
//! comes from the `daw-devices` registry, so a new device needs no change here.
use daw_core::{Effect, Instrument, PluginParameterValue};
use daw_devices::{DeviceDescriptor, DeviceKind, PRESET_EXTENSION, Preset};
use daw_plugins::{
    ParameterInfo, PluginInfo,
    catalog::{CatalogEntry, ThumbnailStatus},
};
use std::{path::PathBuf, sync::OnceLock};

/// Library entries name their device as `rill:<id>`, which is also the
/// catalog ID the tools accept.
const LOCATOR: &str = "rill:";
const FINGERPRINT: &str = "rill";
pub(crate) const VENDOR: &str = "Rill";

/// Built-in devices as Library entries, in registry order.
pub(crate) fn catalog() -> &'static [CatalogEntry] {
    static CATALOG: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        daw_devices::registry()
            .iter()
            .map(|descriptor| CatalogEntry {
                plugin: PluginInfo {
                    name: descriptor.name.into(),
                    path: PathBuf::from(format!("{LOCATOR}{}", descriptor.id)),
                    vendor: VENDOR.into(),
                    category: match descriptor.kind {
                        DeviceKind::Instrument => format!("Instrument|{}", descriptor.category),
                        DeviceKind::Effect => format!("Fx|{}", descriptor.category),
                    },
                    class_id: Some(descriptor.id.into()),
                    version: env!("CARGO_PKG_VERSION").into(),
                    architectures: Vec::new(),
                    format: Default::default(),
                },
                fingerprint: FINGERPRINT.into(),
                thumbnail: None,
                thumbnail_size: None,
                status: ThumbnailStatus::Missing,
                added_unix_ms: 0,
            })
            .collect()
    })
}

/// The parameter browser's `(path, class_id)` for a built-in device.
pub(crate) fn locator(device: &str) -> (String, Option<String>) {
    (format!("{LOCATOR}{device}"), Some(device.into()))
}

pub(crate) fn device_of_locator(path: &str) -> Option<&str> {
    path.strip_prefix(LOCATOR)
}

pub(crate) fn descriptor_for(plugin: &PluginInfo) -> Option<&'static DeviceDescriptor> {
    device_of_locator(plugin.path.to_str()?).and_then(daw_devices::descriptor)
}

pub(crate) fn is_builtin(plugin: &PluginInfo) -> bool {
    descriptor_for(plugin).is_some()
}

/// The insert a Library entry creates.
pub(crate) fn effect(plugin: &PluginInfo) -> Effect {
    match descriptor_for(plugin) {
        Some(descriptor) => Effect::Builtin {
            device: descriptor.id.into(),
        },
        None => Effect::Vst3 {
            path: plugin.path.to_string_lossy().into_owned(),
            class_id: plugin.class_id.clone(),
        },
    }
}

/// The instrument a Library entry creates.
pub(crate) fn instrument(plugin: &PluginInfo) -> Instrument {
    match descriptor_for(plugin) {
        Some(descriptor) => Instrument::Builtin {
            device: descriptor.id.into(),
        },
        None => Instrument::Vst3 {
            path: plugin.path.to_string_lossy().into_owned(),
            class_id: plugin.class_id.clone(),
        },
    }
}

pub(crate) fn format_label(plugin: &PluginInfo) -> &'static str {
    if is_builtin(plugin) {
        "Built-in"
    } else {
        plugin.format.label()
    }
}

/// "Rill · R1 Filter", or a visible advisory for an ID this version lacks.
pub(crate) fn model(device: &str) -> String {
    match daw_devices::descriptor(device) {
        Some(descriptor) => format!("{VENDOR} · {}", descriptor.name),
        None => format!("Unknown device · {device}"),
    }
}

/// Parameter metadata in the shape the parameter browser and tools use for
/// hosted plug-ins. Read from the registry: no helper process, no waiting.
pub(crate) fn parameters(
    device: &str,
    overrides: &[PluginParameterValue],
) -> Option<Vec<ParameterInfo>> {
    let descriptor = daw_devices::descriptor(device)?;
    Some(
        descriptor
            .parameters
            .iter()
            .map(|spec| {
                let default = f64::from(spec.default_normalized());
                ParameterInfo {
                    id: spec.id,
                    name: format!("{} {}", spec.group, spec.name),
                    value: overrides
                        .iter()
                        .find(|parameter| parameter.id == spec.id)
                        .map_or(default, |parameter| parameter.value),
                    default,
                    unit: spec.unit.into(),
                    // VST3 convention: number of steps, 0 for continuous.
                    steps: spec.steps().map_or(0, |count| count as i32 - 1),
                    read_only: false,
                }
            })
            .collect(),
    )
}

/// Normalized value of every parameter: saved override or default.
pub(crate) fn value(
    descriptor: &DeviceDescriptor,
    overrides: &[PluginParameterValue],
    id: u32,
) -> f64 {
    overrides
        .iter()
        .find(|parameter| parameter.id == id)
        .map(|parameter| parameter.value)
        .or_else(|| {
            descriptor
                .parameter(id)
                .map(|spec| f64::from(spec.default_normalized()))
        })
        .unwrap_or(0.0)
}

fn preset_dir(device: &str) -> PathBuf {
    crate::settings::config_dir().join("presets").join(device)
}

/// Presets the user saved for a device. Unreadable files are skipped.
pub(crate) fn user_presets(device: &str) -> Vec<Preset> {
    let Ok(entries) = std::fs::read_dir(preset_dir(device)) else {
        return Vec::new();
    };
    let mut presets: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == PRESET_EXTENSION))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| Preset::parse(&text).ok())
        .filter(|preset| preset.device == device)
        .collect();
    presets.sort_by_key(|preset| preset.name.to_lowercase());
    presets
}

/// Writes into the user preset directory only; the name never becomes a
/// path outside it. Saving under an existing name replaces that preset.
pub(crate) fn save_user_preset(preset: &Preset) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        daw_devices::descriptor(&preset.device).is_some(),
        "Unknown device {}",
        preset.device
    );
    let stem: String = preset
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim();
    anyhow::ensure!(
        !stem.is_empty(),
        "Preset name must contain a letter or digit"
    );
    let directory = preset_dir(&preset.device);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{stem}.{PRESET_EXTENSION}"));
    std::fs::write(&path, preset.to_json())?;
    Ok(path)
}

/// Factory presets first, then the user's.
pub(crate) fn all_presets(descriptor: &DeviceDescriptor) -> Vec<(bool, Preset)> {
    daw_devices::factory_presets(descriptor)
        .into_iter()
        .map(|preset| (true, preset))
        .chain(
            user_presets(descriptor.id)
                .into_iter()
                .map(|preset| (false, preset)),
        )
        .collect()
}

/// Tool view of one parameter: plain value and range in the device's own
/// units, so callers never have to guess a normalized mapping.
pub(crate) fn describe_parameter(
    spec: &daw_devices::ParamSpec,
    normalized: f64,
) -> serde_json::Value {
    let plain = |normalized: f32| f64::from(spec.to_plain(normalized));
    let mut description = serde_json::json!({
        "id": spec.id,
        "key": spec.key,
        "name": format!("{} {}", spec.group, spec.name),
        "value": plain(normalized as f32),
        "display": spec.format(normalized as f32),
        "default": f64::from(spec.default),
        "unit": spec.unit,
        "normalized_value": normalized,
        "read_only": false,
    });
    match spec.range {
        daw_devices::ParamRange::Choice(options) => {
            description["choices"] = serde_json::json!(options);
            description["value"] = serde_json::json!(spec.choice_label(normalized as f32));
            description["default"] =
                serde_json::json!(spec.choice_label(spec.default_normalized()));
        }
        daw_devices::ParamRange::Toggle => {
            description["value"] = serde_json::json!(normalized >= 0.5);
            description["default"] = serde_json::json!(spec.default >= 0.5);
        }
        daw_devices::ParamRange::Linear { .. }
        | daw_devices::ParamRange::Log { .. }
        | daw_devices::ParamRange::Integer { .. } => {
            description["min"] = serde_json::json!(f64::from(spec.plain_min()));
            description["max"] = serde_json::json!(f64::from(spec.plain_max()));
        }
    }
    description
}

/// Normalized value for a tool-supplied plain value: a number in the
/// parameter's unit, a choice label, a boolean, or text such as "1.2 kHz".
pub(crate) fn normalized_from_tool(
    spec: &daw_devices::ParamSpec,
    value: &serde_json::Value,
) -> Result<f64, String> {
    let normalized = match value {
        serde_json::Value::Bool(on) => match spec.range {
            daw_devices::ParamRange::Toggle => Some(f32::from(u8::from(*on))),
            _ => None,
        },
        serde_json::Value::Number(number) => number.as_f64().and_then(|plain| {
            let plain = plain as f32;
            match spec.range {
                daw_devices::ParamRange::Choice(_) | daw_devices::ParamRange::Toggle => {
                    (plain.fract() == 0.0 && plain >= 0.0 && plain < spec.steps()? as f32)
                        .then(|| spec.to_normalized(plain))
                }
                daw_devices::ParamRange::Integer { .. } => (plain.fract() == 0.0
                    && (spec.plain_min()..=spec.plain_max()).contains(&plain))
                .then(|| spec.to_normalized(plain)),
                _ => (spec.plain_min()..=spec.plain_max())
                    .contains(&plain)
                    .then(|| spec.to_normalized(plain)),
            }
        }),
        // Text keeps the panel's typing rules, including clamping to the range.
        serde_json::Value::String(text) => spec.parse(text),
        _ => None,
    };
    normalized.map(f64::from).ok_or_else(|| {
        let expected = match spec.range {
            daw_devices::ParamRange::Choice(options) => format!("one of {}", options.join(", ")),
            daw_devices::ParamRange::Toggle => "true or false".into(),
            _ => format!("{} to {}", spec.format(0.0), spec.format(1.0)),
        };
        format!(
            "Invalid value for \"{} {}\" (id {}); expected {expected}",
            spec.group, spec.name, spec.id
        )
    })
}
