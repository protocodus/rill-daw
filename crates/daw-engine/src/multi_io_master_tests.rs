use super::*;
use daw_core::{
    AutomationPoint, AutomationTarget, Clip, EffectSlot, Interpolation, Note, PluginInputRoute,
};

#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
#[allow(dead_code)]
pub(super) mod fixture;
use fixture::{CLASS_ID, EditorProbeFixture};

fn audio(
    project: &mut Project,
    directory: &Path,
    name: &str,
    value: [f32; 2],
    impulse: bool,
) -> u64 {
    let path = directory.join(format!("{name}.wav"));
    let mut samples = vec![0.0; 8_000];
    for frame in samples.chunks_mut(2).take(if impulse { 1 } else { 4_000 }) {
        frame.copy_from_slice(&value);
    }
    AudioData {
        sample_rate: 8_000,
        channels: 2,
        samples,
    }
    .write_wav(&path)
    .unwrap();
    let id = project.add_audio_track(name);
    let track = project.track_mut(id).unwrap();
    track.gain = 1.0;
    track
        .clips
        .push(Clip::audio(name, 0.0, 1.0, path.to_string_lossy()));
    id
}

fn constant_lane(project: &mut Project, track: u64, parameter: AutomationParameter, value: f32) {
    let lane = project
        .add_automation_track(parameter.name(), AutomationTarget { track, parameter })
        .unwrap();
    let lane = project.automation_lane_mut(lane).unwrap();
    lane.interpolation = Interpolation::Step;
    lane.points = vec![AutomationPoint::new(0.0, parameter.normalize(value))];
}

fn approx(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

#[test]
fn master_and_channel_trim_volume_pan_are_automated_in_signal_order_and_live_editable() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let source = audio(&mut project, directory.path(), "source", [0.8, -0.4], false);
    project.track_mut(source).unwrap().gain = 0.5;
    project.track_mut(source).unwrap().pan = -0.5;
    constant_lane(
        &mut project,
        source,
        AutomationParameter::TrackInputGain,
        2.0,
    );
    constant_lane(&mut project, 0, AutomationParameter::TrackInputGain, 0.5);
    constant_lane(&mut project, 0, AutomationParameter::TrackGain, 0.5);
    constant_lane(&mut project, 0, AutomationParameter::TrackPan, 1.0);
    let mut renderer = Renderer::new(&project, 8_000, false).unwrap();
    let mut block = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    renderer.render(&mut block, &mut positions).unwrap();
    for frame in block {
        approx(frame[0], 0.0);
        approx(frame[1], -0.05);
    }
    let mut preparer = renderer.live_edit_preparer();
    project.master.input_gain = 0.75;
    project.master.gain = 0.8;
    project.master.pan = -0.25;
    assert!(
        preparer.compatible(&project),
        "Master mix gestures preserve the graph"
    );
    let mut edit = preparer.prepare(&project).unwrap();
    let allocations = crate::allocation_test_support::count_allocations(|| {
        renderer.apply_live_edit(&mut edit).unwrap();
        renderer.render(&mut block, &mut positions).unwrap();
    });
    assert_eq!(allocations, 0);
    for frame in block {
        approx(frame[0], 0.0);
        approx(frame[1], -0.05);
    }
}

#[test]
fn muted_audio_is_not_loaded_and_reveals_the_next_visible_source() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let source = audio(&mut project, directory.path(), "under", [0.2, -0.1], false);
    let mut hidden = Clip::audio("muted", 0.1, 0.5, "missing-but-muted.wav");
    hidden.muted = true;
    project.track_mut(source).unwrap().clips.insert(0, hidden);
    let result = render(&project, 8_000, 0.8).unwrap();
    for frame in result.as_chunks::<2>().0 {
        approx(frame[0], 0.2);
        approx(frame[1], -0.1);
    }
}

#[test]
fn native_sidechain_stage_pdc_aligns_impulses_without_solo_feed_leakage() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let detector = audio(&mut project, directory.path(), "detector", [0.8, 0.2], true);
    project.track_mut(detector).unwrap().latency_samples = 37;
    project.track_mut(detector).unwrap().gain = 0.5;
    project.track_mut(detector).unwrap().pan = -0.5;
    let target = audio(&mut project, directory.path(), "main", [0.4, -0.2], true);
    project.track_mut(target).unwrap().solo = true;
    let first = EffectSlot::new(
        "First",
        Effect::Vst3 {
            path: fixture.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    let mut second = first.clone();
    second.id = daw_core::new_id();
    second.inputs.push(PluginInputRoute {
        source_track: detector,
        bus: 1,
        first_channel: 0,
        gain: 1.0,
        pre_fader: true,
    });
    project
        .track_mut(target)
        .unwrap()
        .inserts
        .extend([first, second]);
    let mut renderer =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert_eq!(renderer.latency, 37);
    let mut output = [[0.0; 2]; BLOCK];
    renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
    approx(output[0][0], 0.3);
    approx(output[0][1], 0.0);
    assert!(
        output[1..].iter().all(|frame| *frame == [0.0; 2]),
        "Detector and main stay sample-aligned"
    );
    let parallel = audio(
        &mut project,
        directory.path(),
        "parallel",
        [0.4, -0.2],
        true,
    );
    project.track_mut(parallel).unwrap().latency_samples = 11;
    project.track_mut(target).unwrap().inserts[1]
        .inputs
        .push(PluginInputRoute {
            source_track: parallel,
            bus: 1,
            first_channel: 0,
            gain: 0.5,
            pre_fader: true,
        });
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.35);
    approx(result[1], -0.025);
    assert!(
        result[2..].iter().all(|sample| *sample == 0.0),
        "Parallel detector feeds are aligned and summed once"
    );
    let slot = &mut project.track_mut(target).unwrap().inserts[1];
    slot.inputs.pop();
    slot.inputs[0].pre_fader = false;
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.2);
    approx(result[1], -0.0375);
}

#[test]
fn instrument_audio_inputs_align_precise_note_edges_with_late_sidechain_pairs() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let detector = audio(&mut project, directory.path(), "input", [0.8, 0.2], true);
    project.track_mut(detector).unwrap().latency_samples = 37;
    let owner = project.add_midi_track("Instrument");
    let track = project.track_mut(owner).unwrap();
    track.gain = 1.0;
    track.solo = true;
    track.instrument = Instrument::Vst3 {
        path: fixture.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.instrument_inputs = vec![
        PluginInputRoute {
            source_track: detector,
            bus: 0,
            first_channel: 0,
            gain: 1.0,
            pre_fader: true,
        },
        PluginInputRoute {
            source_track: detector,
            bus: 1,
            first_channel: 2,
            gain: 1.0,
            pre_fader: true,
        },
    ];
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let mut renderer =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert_eq!(renderer.latency, 37);
    let mut output = [[0.0; 2]; BLOCK];
    renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
    approx(output[0][0], 1.05);
    approx(output[0][1], 0.45);
    for frame in &output[1..] {
        approx(frame[0], 0.125);
        approx(frame[1], 0.125);
    }
}

#[test]
fn master_inserts_and_sidechain_are_in_the_compensated_delivery_path() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let detector = audio(&mut project, directory.path(), "detector", [0.8, 0.2], true);
    project.track_mut(detector).unwrap().latency_samples = 37;
    let source = audio(&mut project, directory.path(), "main", [0.4, -0.2], true);
    project.track_mut(source).unwrap().solo = true;
    let mut insert = EffectSlot::new(
        "Master processor",
        Effect::Vst3 {
            path: fixture.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    insert.inputs.push(PluginInputRoute {
        source_track: detector,
        bus: 1,
        first_channel: 0,
        gain: 1.0,
        pre_fader: true,
    });
    project.master.inserts.push(insert);
    project.master.input_gain = 2.0;
    project.master.gain = 0.5;
    project.master.pan = -0.5;
    let mut renderer =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert_eq!(renderer.latency, 37);
    let mut output = [[0.0; 2]; BLOCK];
    renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
    approx(output[0][0], 0.3);
    approx(output[0][1], -0.0375);
    assert!(output[1..].iter().all(|frame| *frame == [0.0; 2]));
}

#[test]
fn nonzero_native_main_uses_real_bus_roles_for_instrument_inputs_outputs_and_insert_sidechains() {
    let instrument = EditorProbeFixture::build_multi_bus_nonzero_main();
    let effect = EditorProbeFixture::build_multi_bus_effect_nonzero_main();
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let detector = audio(
        &mut project,
        directory.path(),
        "nonzero-input",
        [0.8, 0.2],
        true,
    );
    project.track_mut(detector).unwrap().latency_samples = 37;
    let owner = project.add_midi_track("Main is bus 1");
    let track = project.track_mut(owner).unwrap();
    track.gain = 1.0;
    track.solo = true;
    track.instrument = Instrument::Vst3 {
        path: instrument.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    track.instrument_inputs = vec![
        PluginInputRoute {
            source_track: detector,
            bus: 1,
            first_channel: 0,
            gain: 1.0,
            pre_fader: true,
        },
        PluginInputRoute {
            source_track: detector,
            bus: 0,
            first_channel: 0,
            gain: 1.0,
            pre_fader: true,
        },
    ];
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 1.325);
    approx(result[1], 0.425);
    project.track_mut(owner).unwrap().instrument_inputs.clear();
    let child = project
        .add_instrument_output(owner, "Auxiliary is bus 0", 0, 0)
        .unwrap();
    project.track_mut(child).unwrap().solo = true;
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.375);
    approx(result[1], 0.5);
    let mut invalid = project.clone();
    invalid
        .track_mut(child)
        .unwrap()
        .instrument_output
        .as_mut()
        .unwrap()
        .bus = 1;
    assert!(
        Renderer::new_offline_with_options(&invalid, 8_000, RenderOptions::default())
            .err()
            .unwrap()
            .to_string()
            .contains("duplicates its primary")
    );
    invalid
        .track_mut(child)
        .unwrap()
        .instrument_output
        .as_mut()
        .unwrap()
        .first_channel = 2;
    assert!(
        Renderer::new_offline_with_options(&invalid, 8_000, RenderOptions::default())
            .err()
            .unwrap()
            .to_string()
            .contains("pair is unavailable")
    );
    let mut project = Project::new();
    let detector = audio(
        &mut project,
        directory.path(),
        "nonzero-sidechain",
        [0.8, 0.2],
        true,
    );
    let target = audio(
        &mut project,
        directory.path(),
        "nonzero-main",
        [0.4, -0.2],
        true,
    );
    project.track_mut(target).unwrap().solo = true;
    let mut insert = EffectSlot::new(
        "Nonzero primary",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    insert.inputs.push(PluginInputRoute {
        source_track: detector,
        bus: 0,
        first_channel: 0,
        gain: 1.0,
        pre_fader: true,
    });
    project.track_mut(target).unwrap().inserts.push(insert);
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.4);
    approx(result[1], -0.05);
    project.track_mut(target).unwrap().inserts[0].inputs[0].bus = 1;
    assert!(
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default())
            .err()
            .unwrap()
            .to_string()
            .contains("cannot replace")
    );
}

#[test]
fn native_wide_instrument_outputs_have_one_instance_and_independent_solo_mute() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut project = Project::new();
    let owner = project.add_midi_track("Instrument");
    let track = project.track_mut(owner).unwrap();
    track.gain = 1.0;
    track.instrument = Instrument::Vst3 {
        path: fixture.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let outputs = project
        .add_instrument_outputs(
            owner,
            vec![
                ("Pair".into(), 1, 0),
                ("Wide".into(), 1, 2),
                ("Odd".into(), 1, 4),
                ("Mono".into(), 2, 0),
            ],
        )
        .unwrap();
    let mut renderer =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert_eq!(
        renderer
            .tracks
            .iter()
            .flatten()
            .filter(|track| matches!(track.processor, Processor::Vst(_)))
            .count(),
        1
    );
    let mut output = [[0.0; 2]; BLOCK];
    renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
    approx(output[0][0], 2.625);
    approx(output[0][1], 2.875);
    project.track_mut(owner).unwrap().mute = true;
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 2.5);
    approx(result[1], 2.75);
    project.track_mut(outputs[1]).unwrap().solo = true;
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.5);
    approx(result[1], 0.625);
    let wide = project.track_mut(outputs[1]).unwrap();
    wide.input_gain = 2.0;
    wide.gain = 0.5;
    wide.pan = -1.0;
    let result = render(&project, 8_000, 0.01).unwrap();
    approx(result[0], 0.5);
    approx(result[1], 0.0);
    project.track_mut(outputs[1]).unwrap().mute = true;
    let renderer =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert!(
        renderer
            .tracks
            .iter()
            .flatten()
            .all(|track| !matches!(track.processor, Processor::Vst(_))),
        "A wholly silent group loads no instrument"
    );
}

#[test]
fn automatic_workers_cover_native_branches_at_different_depths_but_not_serial_chains() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let directory = tempfile::tempdir().unwrap();
    let insert = || {
        EffectSlot::new(
            "Native",
            Effect::Vst3 {
                path: fixture.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
    };
    let mut project = Project::new();
    let detector = audio(
        &mut project,
        directory.path(),
        "detector",
        [0.8, 0.2],
        false,
    );
    let direct = audio(&mut project, directory.path(), "direct", [0.4, -0.2], false);
    let later = audio(&mut project, directory.path(), "later", [0.1, -0.1], false);
    project.track_mut(direct).unwrap().inserts.push(insert());
    let mut effect = insert();
    effect.inputs.push(PluginInputRoute {
        source_track: detector,
        bus: 1,
        first_channel: 0,
        gain: 1.0,
        pre_fader: true,
    });
    project.track_mut(later).unwrap().inserts.push(effect);
    let mut automatic =
        Renderer::new_offline_with_options(&project, 8_000, RenderOptions::default()).unwrap();
    assert!(
        automatic.levels.iter().all(|level| level
            .iter()
            .filter(|&&index| !automatic.tracks[index].as_ref().unwrap().inserts.is_empty())
            .count()
            <= 1),
        "fixture must separate native processors across levels"
    );
    assert_eq!(
        automatic.workers.is_some(),
        std::thread::available_parallelism().unwrap().get() > 1
    );
    let mut serial = Renderer::new_offline_with_workers(&project, 8_000, 1).unwrap();
    let mut actual = [[0.0; 2]; BLOCK];
    let mut expected = actual;
    automatic.render(&mut actual, &mut [0.0; BLOCK]).unwrap();
    serial.render(&mut expected, &mut [0.0; BLOCK]).unwrap();
    assert_eq!(actual, expected);

    let mut chain = Project::new();
    let source = audio(&mut chain, directory.path(), "chain", [0.4, -0.2], false);
    let first = chain.add_bus("First native node");
    let second = chain.add_bus("Second native node");
    chain.track_mut(first).unwrap().inserts.push(insert());
    chain.track_mut(second).unwrap().inserts.push(insert());
    chain.set_output(source, Some(first)).unwrap();
    chain.set_output(first, Some(second)).unwrap();
    let serial =
        Renderer::new_offline_with_options(&chain, 8_000, RenderOptions::default()).unwrap();
    assert!(
        serial.workers.is_none(),
        "dependent native nodes cannot run concurrently"
    );
}

#[test]
fn parallel_native_sidechains_and_muted_shared_outputs_are_sample_identical() {
    let instrument = EditorProbeFixture::build_multi_bus();
    let effect = EditorProbeFixture::build_multi_bus_effect();
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let detector = audio(&mut project, directory.path(), "detector", [0.8, 0.2], true);
    project.track_mut(detector).unwrap().latency_samples = 37;
    let independent = audio(
        &mut project,
        directory.path(),
        "independent",
        [0.1, -0.1],
        false,
    );
    project.track_mut(independent).unwrap().latency_samples = 11;
    let owner = project.add_midi_track("Muted shared instrument");
    let track = project.track_mut(owner).unwrap();
    track.mute = true;
    track.instrument = Instrument::Vst3 {
        path: instrument.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    track.instrument_inputs = vec![
        PluginInputRoute {
            source_track: detector,
            bus: 0,
            first_channel: 0,
            gain: 1.0,
            pre_fader: true,
        },
        PluginInputRoute {
            source_track: detector,
            bus: 1,
            first_channel: 2,
            gain: 0.5,
            pre_fader: false,
        },
    ];
    let child = project
        .add_instrument_output(owner, "Wide output", 1, 2)
        .unwrap();
    let mut insert = EffectSlot::new(
        "Sidechain",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    insert.inputs.push(PluginInputRoute {
        source_track: detector,
        bus: 1,
        first_channel: 0,
        gain: 0.75,
        pre_fader: true,
    });
    project.track_mut(child).unwrap().inserts.push(insert);
    let mut serial = Renderer::new_offline_with_workers(&project, 8_000, 1).unwrap();
    let mut parallel = Renderer::new_offline_with_workers(&project, 8_000, 4).unwrap();
    assert!(parallel.workers.is_some());
    let source = parallel
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == owner)
        .unwrap();
    assert!(source.enabled && !source.channel_active);
    for beat in [0.0, 0.25, 0.0] {
        serial.seek(beat).unwrap();
        parallel.seek(beat).unwrap();
        for count in [1, 17, 256, 511] {
            let mut expected = vec![[0.0; 2]; count];
            let mut actual = expected.clone();
            let mut expected_positions = vec![0.0; count];
            let mut actual_positions = expected_positions.clone();
            serial
                .render(&mut expected, &mut expected_positions)
                .unwrap();
            parallel.render(&mut actual, &mut actual_positions).unwrap();
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)),
                "Parallel sidechain/raw output reduction changed PCM"
            );
            assert_eq!(actual_positions, expected_positions);
            assert!(actual.iter().flatten().any(|sample| sample.abs() > 0.01));
        }
    }
}

#[test]
fn primary_insert_crash_keeps_healthy_instrument_output_children_playing() {
    let instrument = EditorProbeFixture::build_multi_bus();
    let effect = EditorProbeFixture::build_multi_bus_effect();
    let mut project = Project::new();
    let owner = project.add_midi_track("Shared instrument");
    let track = project.track_mut(owner).unwrap();
    track.gain = 1.0;
    track.instrument = Instrument::Vst3 {
        path: instrument.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi(
        "Notes",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    track.inserts.push(EffectSlot::new(
        "Primary insert",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    ));
    let child = project
        .add_instrument_output(owner, "Independent wide output", 1, 2)
        .unwrap();
    project.add_audio_track("Independent silent root");
    for workers in [1, 4] {
        let mut renderer = Renderer::new_with_workers(&project, 8_000, true, workers).unwrap();
        assert_eq!(renderer.workers.is_some(), workers > 1);
        let source = renderer
            .tracks
            .iter()
            .flatten()
            .find(|track| track.id == owner)
            .unwrap();
        let Processor::Vst(plugin) = &source.processor else {
            panic!("missing instrument")
        };
        let instrument_pid = plugin.isolation_pid().unwrap();
        let Insert::Vst { effect, .. } = &source.inserts[0];
        let effect_pid = effect.isolation_pid().unwrap();
        assert_ne!(instrument_pid, effect_pid);
        let mut output = [[0.0; 2]; BLOCK];
        renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
        approx(output[0][0], 0.5625);
        approx(output[0][1], 0.6875);
        let killed = if cfg!(windows) {
            std::process::Command::new("taskkill")
                .args(["/PID", &effect_pid.to_string(), "/F"])
                .output()
                .unwrap()
        } else {
            std::process::Command::new("kill")
                .args(["-KILL", &effect_pid.to_string()])
                .output()
                .unwrap()
        };
        assert!(killed.status.success());
        for _ in 0..2 {
            renderer.render(&mut output, &mut [0.0; BLOCK]).unwrap();
            for frame in output {
                approx(frame[0], 0.5);
                approx(frame[1], 0.625);
            }
        }
        assert_eq!(
            renderer.plugin_errors, 1,
            "Primary failure is reported once"
        );
        let source = renderer
            .tracks
            .iter()
            .flatten()
            .find(|track| track.id == owner)
            .unwrap();
        let Processor::Vst(plugin) = &source.processor else {
            panic!("missing instrument")
        };
        assert_eq!(plugin.isolation_pid(), Some(instrument_pid));
        assert!(
            !renderer
                .tracks
                .iter()
                .flatten()
                .find(|track| track.id == child)
                .unwrap()
                .plugin_failed
        );
    }
}

#[test]
fn freeze_prints_pre_fader_source_position_and_rolls_back_failed_batches() {
    let directory = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let source = audio(&mut project, directory.path(), "print", [0.4, 0.2], false);
    let track = project.track_mut(source).unwrap();
    track.clips[0].start = 2.0;
    track.input_gain = 2.0;
    track.gain = 0.25;
    track.pan = 1.0;
    track.latency_samples = 37;
    let clip = track.clips[0].id;
    project.master.inserts.push(EffectSlot::new(
        "Excluded Master",
        Effect::Vst3 {
            path: "missing-master.vst3".into(),
            class_id: None,
        },
    ));
    let request = FreezeRenderRequest {
        track: source,
        clips: Some(vec![clip]),
        include_tails: false,
    };
    let report = freeze_track_wav(
        &project,
        staging.path(),
        &request,
        8_000,
        RenderOptions::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let output = &report.outputs[0];
    assert_eq!(output.start_beat, 2.0);
    assert_eq!(output.frames, 4_000);
    assert_eq!(output.length_beats, 1.0);
    let data = AudioData::load_wav(&output.path).unwrap();
    for frame in data.samples.as_chunks::<2>().0 {
        approx(frame[0], 0.8);
        approx(frame[1], 0.4);
    }
    let bytes = std::fs::read(&output.path).unwrap();
    assert!(
        freeze_track_wav(
            &project,
            staging.path(),
            &request,
            8_000,
            RenderOptions::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(&output.path).unwrap(),
        bytes,
        "Freeze never overwrites existing media"
    );
    let cancelled = tempfile::tempdir().unwrap();
    assert!(
        freeze_track_wav(
            &project,
            cancelled.path(),
            &request,
            8_000,
            RenderOptions::default(),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(cancelled.path()).unwrap().count(), 0);
}

#[test]
fn freeze_multi_output_batch_keeps_pairs_distinct_and_preflights_total_media_size() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut project = Project::new();
    let owner = project.add_midi_track("Group");
    let track = project.track_mut(owner).unwrap();
    track.instrument = Instrument::Vst3 {
        path: fixture.path.to_string_lossy().into_owned(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi(
        "Notes",
        1.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let child = project.add_instrument_output(owner, "Wide", 1, 2).unwrap();
    project.track_mut(child).unwrap().mute = true;
    let staging = tempfile::tempdir().unwrap();
    let request = FreezeRenderRequest {
        track: owner,
        clips: None,
        include_tails: false,
    };
    let report = freeze_track_wav(
        &project,
        staging.path(),
        &request,
        8_000,
        RenderOptions::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.outputs.len(), 2);
    for (id, expected) in [(owner, [0.125, 0.125]), (child, [0.5, 0.625])] {
        let output = report
            .outputs
            .iter()
            .find(|output| output.source_track == id)
            .unwrap();
        assert_eq!(output.start_beat, 1.0);
        let data = AudioData::load_wav(&output.path).unwrap();
        for frame in data.samples.as_chunks::<2>().0 {
            approx(frame[0], expected[0]);
            approx(frame[1], expected[1]);
        }
    }
    let failed = tempfile::tempdir().unwrap();
    let keep = failed.path().join(format!("channel-{child}.wav"));
    std::fs::write(&keep, b"keep").unwrap();
    assert!(
        freeze_track_wav(
            &project,
            failed.path(),
            &request,
            8_000,
            RenderOptions::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(failed.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read(keep).unwrap(), b"keep");
    project.track_mut(owner).unwrap().clips[0].length = 20_000.0;
    let oversized = tempfile::tempdir().unwrap();
    assert!(
        freeze_track_wav(
            &project,
            oversized.path(),
            &request,
            8_000,
            RenderOptions::default(),
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("512 MiB")
    );
    assert_eq!(std::fs::read_dir(oversized.path()).unwrap().count(), 0);
}
