use super::*;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Effect, EffectSlot,
    PluginParameterValue,
};
use daw_devices::devices::filter;

const RATE: u32 = 48_000;

/// Two seconds of white-ish noise on an audio track; returns the project, the
/// track and the temporary directory that holds the source.
fn noise_project() -> (Project, u64, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("noise.wav");
    let mut state = 1u32;
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

fn add_filter(project: &mut Project, track: u64, cutoff_hz: f32) -> u64 {
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
    let id = slot.id;
    project.track_mut(track).unwrap().inserts.push(slot);
    id
}

/// Energy above 6 kHz relative to the total, from first differences (a
/// cheap high-pass) over the settled middle of the render.
fn high_band_ratio(samples: &[f32]) -> f32 {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    let middle = &left[RATE as usize / 2..RATE as usize * 3 / 2];
    let total: f32 = middle.iter().map(|x| x * x).sum();
    let high: f32 = middle.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    high / total.max(1e-12)
}

#[test]
fn builtin_insert_filters_rendered_audio() {
    let (mut project, track, _directory) = noise_project();
    let dry = render(&project, RATE, 4.0).unwrap();
    add_filter(&mut project, track, 500.0);
    let filtered = render(&project, RATE, 4.0).unwrap();
    assert!(
        high_band_ratio(&filtered) < high_band_ratio(&dry) * 0.1,
        "{} vs {}",
        high_band_ratio(&filtered),
        high_band_ratio(&dry)
    );
}

#[test]
fn builtin_latency_is_compensated() {
    let (mut project, track, _directory) = noise_project();
    let renderer =
        Renderer::new_offline_with_options(&project, RATE, RenderOptions::default()).unwrap();
    let base = renderer.latency;
    add_filter(&mut project, track, 500.0);
    let renderer =
        Renderer::new_offline_with_options(&project, RATE, RenderOptions::default()).unwrap();
    assert_eq!(renderer.latency, base + 31);
}

#[test]
fn automating_a_builtin_parameter_matches_the_manual_value() {
    let (mut project, track, _directory) = noise_project();
    let effect = add_filter(&mut project, track, 8000.0);
    let manual = {
        let mut manual = project.clone();
        let slot = &mut manual.track_mut(track).unwrap().inserts[0];
        let spec = filter::DESCRIPTOR.parameter(filter::CUTOFF).unwrap();
        slot.parameters[0].value = f64::from(spec.to_normalized(400.0));
        render(&manual, RATE, 4.0).unwrap()
    };
    let spec = filter::DESCRIPTOR.parameter(filter::CUTOFF).unwrap();
    let value = spec.to_normalized(400.0);
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
    let automation = project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap();
    automation.points = vec![AutomationPoint::new(0.0, value)];
    let automated = render(&project, RATE, 4.0).unwrap();
    // The lane applies from the first block; the manual project starts there.
    let start = RATE as usize / 4 * 2;
    let difference = automated[start..]
        .iter()
        .zip(&manual[start..])
        .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()));
    assert!(difference < 1e-4, "{difference}");
}

#[test]
fn unknown_builtin_effects_pass_audio_through() {
    let (mut project, track, _directory) = noise_project();
    let dry = render(&project, RATE, 4.0).unwrap();
    project
        .track_mut(track)
        .unwrap()
        .inserts
        .push(EffectSlot::new(
            "Future device",
            Effect::Builtin {
                device: "r9-future".into(),
            },
        ));
    assert_eq!(render(&project, RATE, 4.0).unwrap(), dry);
}

#[test]
fn builtin_parameters_change_live_without_a_rebuild() {
    let (mut project, track, _directory) = noise_project();
    let effect = add_filter(&mut project, track, 8000.0);
    let mut renderer = Renderer::new(&project, RATE, false).unwrap();
    let spec = filter::DESCRIPTOR.parameter(filter::CUTOFF).unwrap();
    renderer
        .set_plugin_parameter(
            track,
            Some(effect),
            filter::CUTOFF,
            f64::from(spec.to_normalized(300.0)),
        )
        .unwrap();
    let mut output = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    let mut rendered = Vec::new();
    for _ in 0..(RATE as usize * 2 / BLOCK) {
        let allocations = crate::allocation_test_support::count_allocations(|| {
            renderer.render(&mut output, &mut positions).unwrap()
        });
        assert_eq!(allocations, 0);
        rendered.extend(output.iter().flatten());
    }
    let dry = render(&noise_project().0, RATE, 4.0).unwrap();
    assert!(high_band_ratio(&rendered) < high_band_ratio(&dry) * 0.1);
}
