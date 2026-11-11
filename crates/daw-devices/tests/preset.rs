//! The `.rillpreset` rules: strict about the document, tolerant about its
//! parameters, so presets survive devices gaining and renaming parameters.
use daw_devices::devices::{filter, mini};
use daw_devices::{DeviceDescriptor, Preset};

fn preset(device: &str, parameters: &str) -> String {
    format!(r#"{{"format": 1, "device": "{device}", "name": "Test", "parameters": {parameters}}}"#)
}

fn value(descriptor: &DeviceDescriptor, values: &[(u32, f32)], key: &str) -> f32 {
    let id = descriptor.parameter_by_key(key).unwrap().id;
    values.iter().find(|(entry, _)| *entry == id).unwrap().1
}

#[test]
fn documents_are_validated_before_any_parameter() {
    for (text, message) in [
        ("not json", "Invalid preset"),
        (
            r#"{"format": 2, "device": "r1-filter", "name": "Next"}"#,
            "format 2",
        ),
        (
            r#"{"format": 1, "device": "r1-filter", "name": "  "}"#,
            "name",
        ),
        (
            &format!(
                r#"{{"format": 1, "device": "r1-filter", "name": "{}"}}"#,
                "x".repeat(129)
            ),
            "name",
        ),
        (
            r#"{"format": 1, "device": "r1-filter", "name": "A", "colour": "red"}"#,
            "unknown field",
        ),
        (&" ".repeat(1024 * 1024 + 1), "larger than 1 MiB"),
    ] {
        let error = Preset::parse(text).unwrap_err().to_string();
        assert!(error.contains(message), "{error:?} lacks {message:?}");
    }
}

#[test]
fn a_preset_for_another_device_is_refused() {
    let preset = Preset::parse(&preset("r1-mini", "{}")).unwrap();
    let error = preset.resolve(&filter::DESCRIPTOR).unwrap_err();
    assert!(error.to_string().contains("r1-mini"), "{error}");
}

#[test]
fn missing_unknown_and_mistyped_values_fall_back_to_defaults() {
    let descriptor = &filter::DESCRIPTOR;
    let preset = Preset::parse(&preset(
        "r1-filter",
        r#"{"model": "comb", "cutoff": 99999, "resonance": "loud",
            "type": "Ultra-pass", "legacy.knob": 3, "mix": -50}"#,
    ))
    .unwrap();
    let resolved = preset.resolve(descriptor).unwrap();

    assert_eq!(resolved.values.len(), descriptor.parameters.len());
    let mut skipped = resolved.skipped.clone();
    skipped.sort();
    assert_eq!(skipped, ["legacy.knob", "resonance", "type"]);
    let default = |key: &str| {
        descriptor
            .parameter_by_key(key)
            .unwrap()
            .default_normalized()
    };
    let values = &resolved.values;
    // Choice labels match in any case.
    assert_eq!(
        descriptor
            .parameter_by_key("model")
            .unwrap()
            .choice_label(value(descriptor, values, "model")),
        Some("Comb")
    );
    // Out-of-range numbers clamp instead of being skipped.
    assert_eq!(value(descriptor, values, "cutoff"), 1.0);
    assert_eq!(value(descriptor, values, "mix"), 0.0);
    for key in ["resonance", "type", "drive", "output"] {
        assert_eq!(value(descriptor, values, key), default(key), "{key}");
    }
}

#[test]
fn toggles_take_booleans_and_numbers_but_not_words() {
    let descriptor = &mini::DESCRIPTOR;
    for (json, expected, skipped) in [
        ("true", 1.0, false),
        ("false", 0.0, false),
        ("1", 1.0, false),
        (r#""on""#, 0.0, true),
    ] {
        let preset =
            Preset::parse(&preset("r1-mini", &format!(r#"{{"osc2.sync": {json}}}"#))).unwrap();
        let resolved = preset.resolve(descriptor).unwrap();
        assert_eq!(
            value(descriptor, &resolved.values, "osc2.sync"),
            expected,
            "{json}"
        );
        assert_eq!(resolved.skipped == ["osc2.sync"], skipped, "{json}");
    }
}

#[test]
fn captured_presets_round_trip_through_text() {
    for descriptor in [&filter::DESCRIPTOR, &mini::DESCRIPTOR] {
        // A distinct value per parameter, snapped to what each range can hold.
        let source: Vec<(u32, f32)> = descriptor
            .parameters
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                let normalized = (index as f32 * 0.37).fract();
                (spec.id, spec.to_normalized(spec.to_plain(normalized)))
            })
            .collect();
        let captured = Preset::capture(descriptor, "Round trip", |id| {
            source.iter().find(|(entry, _)| *entry == id).unwrap().1
        });
        let reparsed = Preset::parse(&captured.to_json()).unwrap();
        assert_eq!(reparsed, captured);
        let resolved = reparsed.resolve(descriptor).unwrap();
        assert!(resolved.skipped.is_empty(), "{:?}", resolved.skipped);
        for ((id, expected), (resolved_id, actual)) in source.iter().zip(&resolved.values) {
            assert_eq!(id, resolved_id);
            let spec = descriptor.parameter(*id).unwrap();
            // Saved files keep four decimals of the plain value.
            assert!(
                (expected - actual).abs() < 1e-3,
                "{} {}: {expected} → {actual}",
                descriptor.id,
                spec.key
            );
        }
    }
}
