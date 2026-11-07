//! Real native VST3 multi-bus routing across the shipped isolation helper.
#[path = "common/editor_probe_fixture.rs"]
#[allow(dead_code)]
mod editor_probe_fixture;

use daw_plugins::{
    MidiMessage, PluginInfo, StereoBusInput, StereoBusOutput, VstEffect, VstInstrument,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};

fn instrument(fixture: &EditorProbeFixture) -> VstInstrument {
    VstInstrument::load_class(&fixture.path, Some(CLASS_ID), 48_000.0, 64).unwrap()
}
fn effect(fixture: &EditorProbeFixture) -> VstEffect {
    VstEffect::load_class(&fixture.path, Some(CLASS_ID), 48_000.0, 64).unwrap()
}

#[test]
fn effect_fixture_is_discoverable_and_applies_saved_gain_without_midi() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let info = daw_plugins::scan_paths(std::slice::from_ref(&fixture.path))
        .unwrap()
        .remove(0);
    assert_eq!(info.is_effect(), Some(true));
    let mut effect = VstEffect::load_info(&info, 48_000.0, 64).unwrap();
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    effect
        .process(&[0.8; 8], &[0.4; 8], &mut left, &mut right)
        .unwrap();
    assert_eq!(left, [0.4; 8]);
    assert_eq!(right, [0.2; 8]);
    effect.set_parameter(7, 0.25).unwrap();
    let state = effect.save_state().unwrap();
    effect.set_parameter(7, 0.75).unwrap();
    effect.load_state(&state).unwrap();
    effect
        .process(&[0.8; 8], &[0.4; 8], &mut left, &mut right)
        .unwrap();
    assert_eq!(left, [0.2; 8]);
    assert_eq!(right, [0.1; 8]);
}
fn note(offset: u32, on: bool) -> MidiMessage {
    MidiMessage {
        offset,
        pitch: 60,
        velocity: if on { 100 } else { 0 },
        on,
    }
}
fn copy(plugin: &VstInstrument, bus: usize, first_channel: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![99.0; 8];
    let mut right = left.clone();
    plugin
        .copy_output_pair(bus, first_channel, &mut left, &mut right)
        .unwrap();
    (left, right)
}

#[test]
fn native_metadata_preserves_names_inactive_buses_and_captured_state() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut plugin = instrument(&fixture);
    let pid = plugin.isolation_pid();
    let details = plugin.audio_bus_details().unwrap();
    assert_eq!(
        details
            .inputs
            .iter()
            .map(|bus| bus.name.as_str())
            .collect::<Vec<_>>(),
        ["Main", "Sidechain"]
    );
    assert!(details.inputs[0].main && !details.inputs[1].main);
    assert_eq!(
        details
            .outputs
            .iter()
            .map(|bus| bus.channels)
            .collect::<Vec<_>>(),
        [2, 5, 1]
    );
    assert!(!details.outputs[1].active && !details.outputs[2].active);
    plugin.set_parameter(7, 0.75).unwrap();
    let state = plugin.save_state().unwrap();
    let info = PluginInfo {
        path: fixture.path.clone(),
        class_id: Some(CLASS_ID.into()),
        ..PluginInfo::default()
    };
    assert_eq!(
        daw_plugins::inspect_plugin(&info).unwrap().inputs[1].name,
        "Sidechain"
    );
    assert_eq!(
        daw_plugins::inspect_plugin_with_state(&info, &state)
            .unwrap()
            .inputs[1]
            .name,
        "Restored sidechain"
    );
    assert_eq!(
        daw_plugins::inspect_plugin_with_configuration(&info, &state, &[(7, 0.25)])
            .unwrap()
            .inputs[1]
            .name,
        "Sidechain",
        "Normalized overrides apply after opaque state"
    );
    let invalid = PluginInfo::default();
    assert!(
        daw_plugins::inspect_plugin_with_configuration(&invalid, &[], &[(7, f64::NAN)])
            .unwrap_err()
            .to_string()
            .contains("Invalid normalized")
    );
    assert!(
        daw_plugins::inspect_plugin_with_configuration(&invalid, &[], &vec![(7, 0.5); 4097])
            .unwrap_err()
            .to_string()
            .contains("Too many")
    );
    assert_eq!(
        plugin.isolation_pid(),
        pid,
        "Inspection never replaces the playback instance"
    );
}

#[test]
fn native_instrument_outputs_and_wide_pairs_share_one_process_block() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut plugin = instrument(&fixture);
    assert_eq!(plugin.main_output_bus(), 0);
    plugin.configure_audio_buses(&[], &[1, 2]).unwrap();
    let pid = plugin.isolation_pid();
    plugin.process_bus_inputs(&[note(4, true)], &[], 8).unwrap();
    for (bus, channel, left_value, right_value) in [
        (0, 0, 0.125, 0.125),
        (1, 0, 0.25, 0.375),
        (1, 2, 0.5, 0.625),
        (1, 4, 0.75, 0.75),
        (2, 0, 1.0, 1.0),
    ] {
        let (left, right) = copy(&plugin, bus, channel);
        assert_eq!(left, [vec![0.0; 4], vec![left_value; 4]].concat());
        assert_eq!(right, [vec![0.0; 4], vec![right_value; 4]].concat());
    }
    assert_eq!(plugin.isolation_pid(), pid);
    plugin
        .process_bus_inputs(&[note(0, false)], &[], 8)
        .unwrap();
    assert_eq!(copy(&plugin, 1, 2), (vec![0.0; 8], vec![0.0; 8]));
}

#[test]
fn auxiliary_stereo_and_final_mono_inputs_mix_without_stale_audio() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut plugin = effect(&fixture);
    plugin.configure_audio_buses(&[1], &[]).unwrap();
    let main_left = [1.0; 8];
    let main_right = [2.0; 8];
    let aux_left = [4.0; 8];
    let aux_right = [6.0; 8];
    let mono_left = [8.0; 8];
    let mono_right = [12.0; 8];
    let inputs = [
        StereoBusInput {
            bus: 0,
            first_channel: 0,
            left: &main_left,
            right: &main_right,
        },
        StereoBusInput {
            bus: 1,
            first_channel: 0,
            left: &aux_left,
            right: &aux_right,
        },
        StereoBusInput {
            bus: 1,
            first_channel: 2,
            left: &mono_left,
            right: &mono_right,
        },
    ];
    let mut left = [99.0; 8];
    let mut right = [99.0; 8];
    plugin
        .process_buses(
            &inputs,
            &mut [StereoBusOutput {
                bus: 0,
                first_channel: 0,
                left: &mut left,
                right: &mut right,
            }],
        )
        .unwrap();
    assert_eq!(left, [5.5; 8]);
    assert_eq!(right, [7.5; 8]);
    plugin.process_bus_inputs(&inputs[..1], 8).unwrap();
    plugin
        .copy_output_pair(0, 0, &mut left, &mut right)
        .unwrap();
    assert_eq!(left, main_left);
    assert_eq!(right, main_right);
    plugin
        .process(&main_left, &main_right, &mut left, &mut right)
        .unwrap();
    assert_eq!(left, main_left);
    assert_eq!(right, main_right);
}

#[test]
fn invalid_bus_pairs_fail_before_midi_and_silence_destinations() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut plugin = instrument(&fixture);
    let mut left = [99.0; 8];
    let mut right = [99.0; 8];
    for (bus, channel) in [(1, 0), (99, 0), (0, 1), (0, 2)] {
        assert!(
            plugin
                .process_buses(
                    &[note(0, true)],
                    &[],
                    &mut [StereoBusOutput {
                        bus,
                        first_channel: channel,
                        left: &mut left,
                        right: &mut right
                    }]
                )
                .is_err()
        );
        assert_eq!(left, [0.0; 8]);
        assert_eq!(right, [0.0; 8]);
    }
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert_eq!(
        left, [0.0; 8],
        "Invalid output mappings must not queue the note"
    );
    plugin.configure_audio_buses(&[1], &[1]).unwrap();
    let input = [1.0; 8];
    let duplicate = StereoBusInput {
        bus: 1,
        first_channel: 0,
        left: &input,
        right: &input,
    };
    assert!(
        plugin
            .process_bus_inputs(&[note(0, true)], &[duplicate, duplicate], 8)
            .is_err()
    );
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert_eq!(left, [0.0; 8]);
    assert!(plugin.configure_audio_buses(&[999], &[]).is_err());
    plugin
        .process(&[note(0, true)], &mut left, &mut right)
        .unwrap();
    assert_eq!(
        left, [0.125; 8],
        "Rejected configuration leaves the working instance usable"
    );
    plugin.process(&[], &mut [], &mut []).unwrap();
}

#[test]
fn configured_native_buses_survive_state_restore_and_child_recovery() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut plugin = instrument(&fixture);
    plugin.configure_audio_buses(&[1], &[1, 2]).unwrap();
    plugin.set_parameter(7, 0.75).unwrap();
    let state = plugin.save_state().unwrap();
    plugin.load_state(&state).unwrap();
    plugin.process_bus_inputs(&[note(0, true)], &[], 8).unwrap();
    assert_eq!(copy(&plugin, 1, 2), (vec![0.75; 8], vec![0.9375; 8]));
    #[cfg(unix)]
    {
        let old_pid = plugin.isolation_pid().unwrap();
        assert!(
            std::process::Command::new("/bin/kill")
                .args(["-KILL", &old_pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let mut left = [99.0; 8];
        let mut right = [99.0; 8];
        assert!(plugin.process(&[], &mut left, &mut right).is_err());
        assert_eq!(left, [0.0; 8]);
        assert_eq!(right, [0.0; 8]);
        plugin.recover().unwrap();
        assert_ne!(plugin.isolation_pid(), Some(old_pid));
        plugin.load_state(&state).unwrap();
        plugin.process_bus_inputs(&[note(0, true)], &[], 8).unwrap();
        assert_eq!(copy(&plugin, 1, 4), (vec![1.125; 8], vec![1.125; 8]));
        assert!(plugin.audio_bus_layout().inputs[1].active);
    }
}

#[test]
fn nonzero_native_main_indices_survive_activating_lower_auxiliary_buses() {
    let fixture = EditorProbeFixture::build_multi_bus_nonzero_main();
    let mut plugin = instrument(&fixture);
    assert_eq!(plugin.main_input_bus(), Some(1));
    assert_eq!(plugin.main_output_bus(), 1);
    let details = plugin.audio_bus_details().unwrap();
    assert!(!details.outputs[0].main && details.outputs[1].main);
    plugin.configure_audio_buses(&[0], &[0, 2]).unwrap();
    assert_eq!(plugin.main_input_bus(), Some(1));
    assert_eq!(plugin.main_output_bus(), 1);
    let mut left = [0.0; 8];
    let mut right = [0.0; 8];
    plugin
        .process(&[note(0, true)], &mut left, &mut right)
        .unwrap();
    assert_eq!(left, [0.125; 8]);
    assert_eq!(right, [0.125; 8]);
    assert_eq!(copy(&plugin, 0, 2), (vec![0.5; 8], vec![0.625; 8]));
    let state = plugin.save_state().unwrap();
    plugin.load_state(&state).unwrap();
    plugin
        .process(&[note(0, true)], &mut left, &mut right)
        .unwrap();
    assert_eq!(plugin.main_output_bus(), 1);
    assert_eq!(copy(&plugin, 0, 4), (vec![0.75; 8], vec![0.75; 8]));
    let fixture = EditorProbeFixture::build_multi_bus_effect_nonzero_main();
    let mut effect = effect(&fixture);
    assert!(
        effect.audio_bus_layout().inputs[0].active,
        "Fixture also activates lower-index auxiliary input by default"
    );
    assert_eq!(
        effect.main_input_bus(),
        Some(1),
        "Native main flags take priority over first-active order"
    );
    effect.configure_audio_buses(&[0], &[]).unwrap();
    effect
        .process(&[0.8; 8], &[0.4; 8], &mut left, &mut right)
        .unwrap();
    assert_eq!(effect.main_input_bus(), Some(1));
    assert_eq!(effect.main_output_bus(), 1);
    assert_eq!(left, [0.4; 8]);
    assert_eq!(right, [0.2; 8]);
}
