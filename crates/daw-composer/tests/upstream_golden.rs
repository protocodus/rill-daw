//! Semantic reference outputs from the separately executed pinned Python source.
//! The fixtures intentionally omit upstream file, provider, and key-profile data.
use daw_composer::{
    PartSpec, TimedNote, catalog, compose, euclidean, get_chord, get_scale, progression,
};
use serde_json::Value;

const SOURCE: &str = "72a25722f12fed96e8e19c637f0b15260fcb51a7";

fn fixture(text: &str) -> Value {
    let fixture: Value = serde_json::from_str(text).expect("valid reference fixture");
    assert_eq!(fixture["source_commit"], SOURCE);
    fixture
}

fn text<'a>(case: &'a Value, key: &str) -> &'a str {
    case[key].as_str().expect("fixture text field")
}

fn degrees(case: &Value) -> Vec<u8> {
    case["degrees"]
        .as_array()
        .expect("fixture degrees")
        .iter()
        .map(|degree| u8::try_from(degree.as_u64().expect("degree integer")).unwrap())
        .collect()
}

#[test]
fn every_scale_and_chord_matches_the_pinned_octave_aware_reference() {
    let fixture = fixture(include_str!("fixtures/lookups.json"));
    let scales = fixture["scales"].as_array().unwrap();
    let chords = fixture["chords"].as_array().unwrap();
    assert_eq!(scales.len(), 50);
    assert_eq!(chords.len(), 37);
    let vocabulary = catalog();
    assert_eq!(vocabulary.scales.len(), scales.len());
    assert_eq!(vocabulary.chords.len(), chords.len());
    for case in scales {
        let kind = text(case, "scale_type");
        let actual = get_scale(text(case, "root"), kind).expect(kind);
        assert_eq!(
            serde_json::to_value(actual.notes).unwrap(),
            case["notes"],
            "{kind}"
        );
        assert_eq!(
            serde_json::to_value(actual.midi).unwrap(),
            case["midi"],
            "{kind}"
        );
        let definition = vocabulary
            .scales
            .iter()
            .find(|scale| scale.name == kind)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&definition.intervals).unwrap(),
            case["intervals"],
            "{kind}"
        );
    }
    for case in chords {
        let kind = text(case, "chord_type");
        let actual = get_chord(text(case, "root"), kind).expect(kind);
        assert_eq!(
            serde_json::to_value(actual.notes).unwrap(),
            case["notes"],
            "{kind}"
        );
        assert_eq!(
            serde_json::to_value(actual.midi).unwrap(),
            case["midi"],
            "{kind}"
        );
        let definition = vocabulary
            .chords
            .iter()
            .find(|chord| chord.name == kind)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&definition.intervals).unwrap(),
            case["intervals"],
            "{kind}"
        );
    }
}

#[test]
fn aliases_and_enharmonic_spelling_keep_the_reference_pitches() {
    let fixture = fixture(include_str!("fixtures/lookups.json"));
    for case in fixture["aliases_and_spelling"].as_array().unwrap() {
        let actual = if text(case, "kind") == "scale" {
            get_scale(text(case, "root"), text(case, "type"))
        } else {
            get_chord(text(case, "root"), text(case, "type"))
        }
        .unwrap();
        assert_eq!(
            serde_json::to_value(actual.notes).unwrap(),
            case["notes"],
            "{case}"
        );
        assert_eq!(
            serde_json::to_value(actual.midi).unwrap(),
            case["midi"],
            "{case}"
        );
    }
}

#[test]
fn diatonic_progressions_and_smooth_voicings_match_the_reference() {
    let fixture = fixture(include_str!("fixtures/progressions.json"));
    for case in fixture["progressions"].as_array().unwrap() {
        let actual = progression(
            text(case, "root"),
            text(case, "scale_type"),
            &degrees(case),
            case["sevenths"].as_bool().unwrap(),
            case["smooth"].as_bool().unwrap(),
        )
        .unwrap();
        let notes: Vec<_> = actual.chords.iter().map(|chord| &chord.notes).collect();
        let midi: Vec<_> = actual.chords.iter().map(|chord| &chord.midi).collect();
        assert_eq!(
            serde_json::to_value(notes).unwrap(),
            case["notes"],
            "{case}"
        );
        assert_eq!(serde_json::to_value(midi).unwrap(), case["midi"], "{case}");
    }
}

#[test]
fn euclidean_patterns_match_the_reference_including_rotation_and_edge_cases() {
    let fixture = fixture(include_str!("fixtures/rhythms.json"));
    for case in fixture["rhythms"].as_array().unwrap() {
        let actual = euclidean(
            u16::try_from(case["pulses"].as_u64().unwrap()).unwrap(),
            u16::try_from(case["steps"].as_u64().unwrap()).unwrap(),
            i32::try_from(case["rotation"].as_i64().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.pattern, text(case, "pattern"), "{case}");
    }
}

#[test]
fn matching_accompaniment_patterns_preserve_pitch_timing_gate_and_velocity() {
    let fixture = fixture(include_str!("fixtures/accompaniment.json"));
    for case in fixture["patterns"].as_array().unwrap() {
        let args = &case["progression"];
        let chords = progression(
            text(args, "root"),
            text(args, "scale"),
            &degrees(args),
            args["sevenths"].as_bool().unwrap(),
            args["smooth"].as_bool().unwrap(),
        )
        .unwrap();
        let part: PartSpec = serde_json::from_value(case["part"].clone()).unwrap();
        let mut actual = compose(&chords, 4.0, &[part]).unwrap();
        assert_eq!(actual.len(), 1);
        let actual = actual.remove(0);
        assert!((actual.length - case["length"].as_f64().unwrap()).abs() < 1e-10);
        let expected: Vec<TimedNote> = serde_json::from_value(case["notes"].clone()).unwrap();
        assert_eq!(actual.notes.len(), expected.len(), "{case}");
        for (got, want) in actual.notes.iter().zip(expected) {
            assert_eq!(got.pitch, want.pitch, "{case}");
            assert_eq!(got.velocity, want.velocity, "{case}");
            assert!((got.start - want.start).abs() < 1e-10, "{case}");
            assert!((got.length - want.length).abs() < 1e-10, "{case}");
        }
    }
}
