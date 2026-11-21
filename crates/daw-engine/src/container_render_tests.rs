use super::*;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Effect, EffectLane,
    EffectSlot, PluginParameterValue,
};
use daw_devices::devices::{container, filter};

const RATE: u32 = 48_000;

fn noise_project() -> (Project, u64, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("noise.wav");
    let mut state = 7u32;
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..RATE as usize * 2 * 2)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Noise");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Noise",
        0.0,
        4.0,
        path.to_string_lossy(),
    ));
    (project, track, directory)
}

fn filter_slot(cutoff_hz: f32) -> EffectSlot {
    let spec = filter::DESCRIPTOR.parameter(filter::CUTOFF).unwrap();
    let mut slot = EffectSlot::new(
        "R1 Filter",
        Effect::Builtin {
            device: "r1-filter".into(),
        },
    );
    slot.parameters = vec![PluginParameterValue {
        id: filter::CUTOFF,
        value: f64::from(spec.to_normalized(cutoff_hz)),
    }];
    slot
}

fn container_slot(device: &str, rule: &str, lanes: Vec<Vec<EffectSlot>>) -> EffectSlot {
    let mut slot = EffectSlot::new(
        device,
        Effect::Container {
            device: device.into(),
            lanes: lanes
                .into_iter()
                .map(|inserts| EffectLane { inserts })
                .collect(),
        },
    );
    if device == "r1-split" {
        let spec = container::SPLIT.parameter(container::RULE).unwrap();
        slot.parameters = vec![PluginParameterValue {
            id: container::RULE,
            value: f64::from(spec.parse(rule).unwrap()),
        }];
    }
    slot
}

fn left(samples: &[f32]) -> Vec<f32> {
    samples.iter().step_by(2).copied().collect()
}

fn rms(samples: &[f32]) -> f32 {
    let middle = &samples[RATE as usize / 2..RATE as usize * 3 / 2];
    (middle.iter().map(|x| x * x).sum::<f32>() / middle.len() as f32).sqrt()
}

fn max_difference(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .skip(RATE as usize / 4)
        .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()))
}

#[test]
fn empty_mid_side_and_left_right_splits_are_transparent() {
    let (mut project, track, _directory) = noise_project();
    let dry = render(&project, RATE, 4.0).unwrap();
    for rule in ["Mid/Side", "Left/Right"] {
        project.track_mut(track).unwrap().inserts =
            vec![container_slot("r1-split", rule, vec![vec![], vec![]])];
        let split = render(&project, RATE, 4.0).unwrap();
        assert!(max_difference(&split, &dry) < 1e-5, "{rule}");
    }
}

#[test]
fn empty_bands_keep_the_level() {
    let (mut project, track, _directory) = noise_project();
    let dry = rms(&left(&render(&project, RATE, 4.0).unwrap()));
    project.track_mut(track).unwrap().inserts = vec![container_slot(
        "r1-split",
        "Bands",
        vec![vec![], vec![], vec![]],
    )];
    let split = rms(&left(&render(&project, RATE, 4.0).unwrap()));
    assert!((split / dry - 1.0).abs() < 0.02, "{split} vs {dry}");
}

#[test]
fn a_chain_sounds_like_its_effects_in_series() {
    let (mut project, track, _directory) = noise_project();
    project.track_mut(track).unwrap().inserts = vec![filter_slot(500.0), filter_slot(4000.0)];
    let serial = render(&project, RATE, 4.0).unwrap();
    project.track_mut(track).unwrap().inserts = vec![container_slot(
        "r1-chain",
        "",
        vec![vec![filter_slot(500.0), filter_slot(4000.0)]],
    )];
    let chained = render(&project, RATE, 4.0).unwrap();
    assert!(max_difference(&chained, &serial) < 1e-5);
}

#[test]
fn lanes_are_aligned_to_the_slowest_and_compensated() {
    let (mut project, track, _directory) = noise_project();
    let base = Renderer::new_offline_with_options(&project, RATE, RenderOptions::default())
        .unwrap()
        .latency;
    // The filter's 31 samples in the side lane delay the mid lane too, so
    // a fully open filter leaves the signal as it was, only later.
    project.track_mut(track).unwrap().inserts = vec![container_slot(
        "r1-split",
        "Mid/Side",
        vec![vec![], vec![filter_slot(20_000.0)]],
    )];
    let renderer =
        Renderer::new_offline_with_options(&project, RATE, RenderOptions::default()).unwrap();
    assert_eq!(renderer.latency, base + 31);
}

#[test]
fn nested_effects_follow_their_automation() {
    let (mut project, track, _directory) = noise_project();
    let nested = filter_slot(8000.0);
    let effect = nested.id;
    let split = container_slot("r1-split", "All", vec![vec![nested]]);
    project.track_mut(track).unwrap().inserts = vec![split];
    let spec = filter::DESCRIPTOR.parameter(filter::CUTOFF).unwrap();
    let value = spec.to_normalized(400.0);
    let manual = {
        let mut manual = project.clone();
        manual
            .track_mut(track)
            .unwrap()
            .effect_mut(effect)
            .unwrap()
            .parameters[0]
            .value = f64::from(value);
        render(&manual, RATE, 4.0).unwrap()
    };
    let lane = project
        .add_automation_track_with_value(
            "Cutoff",
            AutomationTarget {
                track,
                parameter: AutomationParameter::VstParameter {
                    effect: Some(effect),
                    id: filter::CUTOFF,
                },
            },
            value,
        )
        .unwrap();
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![AutomationPoint::new(0.0, value)];
    let automated = render(&project, RATE, 4.0).unwrap();
    assert!(max_difference(&automated, &manual) < 1e-4);
}

#[test]
fn containers_nest() {
    let (mut project, track, _directory) = noise_project();
    project.track_mut(track).unwrap().inserts = vec![filter_slot(500.0)];
    let direct = render(&project, RATE, 4.0).unwrap();
    let inner = container_slot("r1-chain", "", vec![vec![filter_slot(500.0)]]);
    let outer = container_slot("r1-chain", "", vec![vec![inner]]);
    project.track_mut(track).unwrap().inserts = vec![outer];
    let nested = render(&project, RATE, 4.0).unwrap();
    assert!(max_difference(&nested, &direct) < 1e-5);
}

#[test]
fn nested_parameters_edit_live_and_structure_rebuilds() {
    let (mut project, track, _directory) = noise_project();
    let nested = filter_slot(8000.0);
    let effect = nested.id;
    let split = container_slot("r1-split", "Bands", vec![vec![], vec![nested], vec![]]);
    let container = split.id;
    project.track_mut(track).unwrap().inserts = vec![split];
    let mut renderer = Renderer::new(&project, RATE, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    let checker = prepare.compatibility();

    let mut changed = project.clone();
    changed
        .track_mut(track)
        .unwrap()
        .effect_mut(effect)
        .unwrap()
        .parameters[0]
        .value = 0.2;
    assert!(!checker.compatible(&changed));
    checker.acknowledge_parameter(track, Some(effect), filter::CUTOFF, 0.2);
    assert!(checker.compatible(&changed));
    renderer
        .apply_live_edit(&mut prepare.prepare(&changed).unwrap())
        .unwrap();
    renderer
        .set_plugin_parameter(track, Some(effect), filter::CUTOFF, 0.3)
        .unwrap();
    renderer
        .set_plugin_parameter(track, Some(container), container::RULE, 1.0)
        .unwrap();

    let mut added = changed.clone();
    let Effect::Container { lanes, .. } = &mut added.track_mut(track).unwrap().inserts[0].effect
    else {
        unreachable!()
    };
    lanes[0].inserts.push(filter_slot(500.0));
    assert!(!checker.compatible(&added));
}
