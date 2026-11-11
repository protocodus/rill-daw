//! Presets saved by any earlier version must keep loading: every parameter
//! they name keeps its value, every parameter they predate takes its default.
//! `tests/presets/<device>/<version>/` holds frozen presets from each version
//! and is never edited; see "Preset compatibility tests" in the README.
use daw_devices::{DeviceDescriptor, ParamRange, ParamSpec, Preset, descriptor, registry};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/presets")
}

/// Every frozen preset, as (device, version, file).
fn fixtures() -> Vec<(String, String, PathBuf)> {
    let mut found = Vec::new();
    for device in std::fs::read_dir(fixtures_root()).unwrap() {
        let device = device.unwrap().path();
        for version in std::fs::read_dir(&device).unwrap() {
            let version = version.unwrap().path();
            for file in std::fs::read_dir(&version).unwrap() {
                let file = file.unwrap().path();
                if file.extension().is_some_and(|ext| ext == "rillpreset") {
                    let name =
                        |path: &Path| path.file_name().unwrap().to_string_lossy().into_owned();
                    found.push((name(&device), name(&version), file));
                }
            }
        }
    }
    found.sort();
    found
}

/// The current label an older preset's choice label stands for.
fn current_label<'a>(descriptor: &DeviceDescriptor, key: &str, label: &'a str) -> &'a str {
    descriptor
        .legacy_labels
        .iter()
        .find(|(legacy_key, old, _)| *legacy_key == key && old.eq_ignore_ascii_case(label))
        .map_or(label, |(_, _, current)| current)
}

fn assert_loaded(descriptor: &DeviceDescriptor, spec: &ParamSpec, stored: &Value, normalized: f32) {
    let key = spec.key;
    match (stored, spec.range) {
        (Value::String(label), ParamRange::Choice(_)) => {
            let loaded = spec.choice_label(normalized).unwrap();
            let expected = current_label(descriptor, key, label);
            assert!(
                loaded.eq_ignore_ascii_case(expected),
                "{key}: {label} loaded as {loaded}"
            );
        }
        (Value::Bool(on), ParamRange::Toggle) => {
            assert_eq!(spec.to_plain(normalized) >= 0.5, *on, "{key}");
        }
        (Value::Number(number), _) => {
            let stored = number.as_f64().unwrap() as f32;
            let expected = stored.clamp(spec.plain_min(), spec.plain_max());
            let loaded = spec.to_plain(normalized);
            assert!(
                (loaded - expected).abs() <= 1e-4 * expected.abs().max(1.0),
                "{key}: {stored} loaded as {loaded}"
            );
        }
        _ => panic!("{key}: {stored} is not a value this parameter can take"),
    }
}

#[test]
fn every_earlier_preset_loads_with_its_values_and_current_defaults() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty());
    for (device, version, file) in fixtures {
        let context = format!("{device} {version} {}", file.display());
        let descriptor = descriptor(&device).unwrap_or_else(|| panic!("{context}: no such device"));
        let text = std::fs::read_to_string(&file).unwrap();
        let preset = Preset::parse(&text).unwrap_or_else(|error| panic!("{context}: {error}"));
        let resolved = preset
            .resolve(descriptor)
            .unwrap_or_else(|error| panic!("{context}: {error}"));
        assert!(
            resolved.skipped.is_empty(),
            "{context} skips {:?}",
            resolved.skipped
        );
        assert_eq!(
            resolved.values.len(),
            descriptor.parameters.len(),
            "{context}"
        );
        for (spec, (id, normalized)) in descriptor.parameters.iter().zip(&resolved.values) {
            assert_eq!(spec.id, *id, "{context}");
            match preset.parameters.get(spec.key) {
                Some(stored) => assert_loaded(descriptor, spec, stored, *normalized),
                None => assert_eq!(
                    *normalized,
                    spec.default_normalized(),
                    "{context}: {} should take its default",
                    spec.key
                ),
            }
        }
    }
}

#[test]
fn every_device_with_presets_keeps_frozen_first_version_presets() {
    for descriptor in registry() {
        if descriptor.factory_presets.is_empty() {
            continue;
        }
        let folder = fixtures_root().join(descriptor.id).join("v1");
        let count = std::fs::read_dir(&folder)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert!(
            count > 0,
            "{} needs frozen presets in {}",
            descriptor.id,
            folder.display()
        );
    }
}

#[test]
fn legacy_labels_point_at_current_options() {
    for descriptor in registry() {
        for (key, old, current) in descriptor.legacy_labels {
            let spec = descriptor
                .parameter_by_key(key)
                .unwrap_or_else(|| panic!("{}: no parameter {key}", descriptor.id));
            assert!(
                matches!(spec.range, ParamRange::Choice(options) if options.contains(current)),
                "{} {key}: {old} → {current} is not a current option",
                descriptor.id
            );
            assert!(
                spec.parse(old).is_none(),
                "{} {key}: {old} is a current option and needs no alias",
                descriptor.id
            );
        }
    }
}
