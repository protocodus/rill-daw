//! Real VST3 event precision and voice targeting through the isolation worker.
#[path = "common/editor_probe_fixture.rs"]
#[allow(dead_code)]
mod editor_probe_fixture;

use daw_plugins::{
    ExpressiveController, ExpressiveMidiData as Data, ExpressiveMidiMessage as Event,
    ExpressiveVelocity as Velocity, VstInstrument,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};

fn plugin(fixture: &EditorProbeFixture) -> VstInstrument {
    VstInstrument::load_class(&fixture.path, Some(CLASS_ID), 48_000.0, 64).unwrap()
}
fn on(offset: u32, channel: u8, id: i32, velocity: Velocity) -> Event {
    Event {
        offset,
        data: Data::NoteOn {
            channel,
            pitch: 60,
            velocity,
            note_id: id,
            tuning: 0.125,
        },
    }
}
fn render(plugin: &mut VstInstrument, events: &[Event]) -> ([f32; 8], [f32; 8]) {
    plugin
        .process_bus_inputs_expressive(&[], events, &[], 8)
        .unwrap();
    let (mut left, mut right) = ([0.0; 8], [0.0; 8]);
    plugin
        .copy_output_pair(plugin.main_output_bus(), 0, &mut left, &mut right)
        .unwrap();
    (left, right)
}
fn parameter(plugin: &VstInstrument, id: u32) -> f64 {
    plugin
        .parameters()
        .unwrap()
        .into_iter()
        .find(|parameter| parameter.id == id)
        .unwrap()
        .value
}

#[test]
fn native_velocity_keeps_midi1_normalization_and_adjacent_midi2_values_distinct() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut legacy = plugin(&fixture);
    let (left, right) = render(&mut legacy, &[on(2, 0, 10, Velocity::Midi1(64))]);
    assert_eq!(&left[..2], &[0.0; 2]);
    assert_eq!(&left[2..], &[64.0_f32 / 127.0 * 0.5; 6]);
    assert_eq!(right, [0.0; 8]);
    assert_eq!(parameter(&legacy, 8), f64::from(64.0_f32 / 127.0));
    assert_eq!(parameter(&legacy, 11), 0.125);
    let mut precise = plugin(&fixture);
    let (left, right) = render(
        &mut precise,
        &[
            on(0, 0, 20, Velocity::Midi2(32768)),
            on(4, 1, 21, Velocity::Midi2(32769)),
        ],
    );
    assert_eq!(left, [32768.0_f32 / 65535.0 * 0.5; 8]);
    assert_eq!(&right[..4], &[0.0; 4]);
    assert_eq!(&right[4..], &[32769.0_f32 / 65535.0 * 0.5; 4]);
    assert_ne!(left[7], right[7]);
    assert_ne!(parameter(&legacy, 8), parameter(&precise, 8));
}

#[test]
fn double_precision_expression_and_release_target_only_the_addressed_voice() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut plugin = plugin(&fixture);
    assert_eq!(
        plugin.supported_note_expression_types(5).unwrap(),
        [2, 4, 201074, 202000]
    );
    let exact = f64::from(0x8000_0001_u32) / f64::from(u32::MAX);
    let (left, right) = render(
        &mut plugin,
        &[
            on(0, 0, 100, Velocity::Midi2(65535)),
            on(0, 1, 101, Velocity::Midi2(65535)),
            Event {
                offset: 2,
                data: Data::NoteExpressionValue {
                    type_id: 201074,
                    note_id: 100,
                    value: exact,
                },
            },
            Event {
                offset: 5,
                data: Data::NoteOff {
                    channel: 0,
                    pitch: 60,
                    velocity: Velocity::Midi2(40001),
                    note_id: 100,
                    tuning: 0.0,
                },
            },
        ],
    );
    assert_eq!(&left[..2], &[0.5; 2]);
    assert_eq!(&left[2..5], &[(exact * 0.5) as f32; 3]);
    assert_eq!(&left[5..], &[0.0; 3]);
    assert_eq!(
        right, [0.5; 8],
        "The same pitch on another voice remains unchanged"
    );
    assert_eq!(
        parameter(&plugin, 9),
        exact,
        "Native f64 expression retains the original 32-bit LSB"
    );
    assert_eq!(parameter(&plugin, 10), f64::from(40001.0_f32 / 65535.0));
    let next = f64::from(0x8000_0002_u32) / f64::from(u32::MAX);
    render(
        &mut plugin,
        &[Event {
            offset: 0,
            data: Data::NoteExpressionValue {
                type_id: 202000,
                note_id: 101,
                value: next,
            },
        }],
    );
    assert_eq!(parameter(&plugin, 9), next);
    assert_ne!(exact, next);
}

#[test]
fn channel_controller_mapping_preserves_exact_values_and_sample_offsets() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut plugin = plugin(&fixture);
    let (left, right) = render(
        &mut plugin,
        &[
            on(0, 3, 7, Velocity::Midi2(65535)),
            Event {
                offset: 2,
                data: Data::Controller {
                    channel: 3,
                    controller: ExpressiveController::ControlChange(7),
                    value: 0.25,
                },
            },
            Event {
                offset: 5,
                data: Data::Controller {
                    channel: 3,
                    controller: ExpressiveController::PitchBend,
                    value: 0.75,
                },
            },
        ],
    );
    assert_eq!(left, [0.0; 8]);
    assert_eq!(right, [0.5, 0.5, 0.25, 0.25, 0.25, 0.75, 0.75, 0.75]);
    assert_eq!(parameter(&plugin, 7), 0.75);
}
