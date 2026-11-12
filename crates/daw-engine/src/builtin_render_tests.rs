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

mod sidechain {
    use super::*;
    use daw_core::PluginInputRoute;
    use daw_devices::devices::{compressor, saturator};

    /// Where the detector signal starts, in frames.
    const BURST: usize = 12_000;

    /// A track playing two seconds of `level` from frame `from` on.
    fn steady(
        project: &mut Project,
        directory: &std::path::Path,
        name: &str,
        from: usize,
        level: f32,
    ) -> u64 {
        let path = directory.join(format!("{name}.wav"));
        AudioData {
            sample_rate: RATE,
            channels: 2,
            samples: (0..RATE as usize * 2)
                .flat_map(|frame| [if frame >= from { level } else { 0.0 }; 2])
                .collect(),
        }
        .write_wav(&path)
        .unwrap();
        let track = project.add_audio_track(name);
        project.track_mut(track).unwrap().gain = 1.0;
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            name,
            0.0,
            4.0,
            path.to_string_lossy(),
        ));
        track
    }

    fn builtin(descriptor: &daw_devices::DeviceDescriptor, values: &[(u32, f32)]) -> EffectSlot {
        let mut slot = EffectSlot::new(
            descriptor.name,
            Effect::Builtin {
                device: descriptor.id.into(),
            },
        );
        slot.parameters = values
            .iter()
            .map(|(id, plain)| PluginParameterValue {
                id: *id,
                value: f64::from(descriptor.parameter(*id).unwrap().to_normalized(*plain)),
            })
            .collect();
        slot
    }

    /// A fast, hard compressor keyed from its sidechain input.
    fn ducker() -> EffectSlot {
        use compressor::*;
        builtin(
            &DESCRIPTOR,
            &[
                (SIDECHAIN, 1.0),
                (THRESHOLD, -30.0),
                (RATIO, 20.0),
                (KNEE, 0.0),
                (ATTACK, 0.02),
            ],
        )
    }

    fn route(source: u64, bus: u32) -> PluginInputRoute {
        PluginInputRoute {
            source_track: source,
            bus,
            first_channel: 0,
            gain: 1.0,
            pre_fader: true,
        }
    }

    /// A silent key track and a steady target that ducks under it.
    fn ducking_project() -> (Project, u64, u64, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        project.tempo = 120.0;
        let key = steady(&mut project, directory.path(), "key", BURST, 1.0);
        // Heard by the detector only: the route is taken before the fader.
        project.track_mut(key).unwrap().gain = 0.0;
        let target = steady(&mut project, directory.path(), "target", 0, 0.5);
        let mut slot = ducker();
        slot.inputs.push(route(key, 1));
        project.track_mut(target).unwrap().inserts.push(slot);
        (project, key, target, directory)
    }

    /// The first frame the target plays below its steady level.
    fn first_ducked(project: &Project) -> usize {
        let rendered = render(project, RATE, 4.0).unwrap();
        let left: Vec<f32> = rendered.iter().step_by(2).copied().collect();
        assert!((left[BURST - 1] - 0.5).abs() < 1e-4, "{}", left[BURST - 1]);
        assert!(left[BURST + 480] < 0.05, "{}", left[BURST + 480]);
        left.iter().position(|sample| *sample < 0.49).unwrap()
    }

    #[test]
    fn builtin_sidechain_ducks_and_is_sample_aligned() {
        let (project, key, target, _directory) = ducking_project();
        assert_eq!(first_ducked(&project), BURST);
        // A key that arrives late delays the main signal at the insert.
        let mut late = project.clone();
        late.track_mut(key)
            .unwrap()
            .inserts
            .push(builtin(&filter::DESCRIPTOR, &[(filter::MIX, 0.0)]));
        assert_eq!(first_ducked(&late), BURST);
        let mut shifted = project.clone();
        shifted.track_mut(target).unwrap().latency_samples = 37;
        assert_eq!(first_ducked(&shifted), BURST);
        // Without the route the device hears no key and leaves the level.
        let mut unrouted = project;
        unrouted.track_mut(target).unwrap().inserts[0]
            .inputs
            .clear();
        let rendered = render(&unrouted, RATE, 4.0).unwrap();
        assert!((rendered[2 * (BURST + 480)] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn builtin_input_routes_need_a_sidechain_bus() {
        let (mut project, key, target, _directory) = ducking_project();
        let error = |project: &Project| format!("{:#}", render(project, RATE, 1.0).unwrap_err());
        project.track_mut(target).unwrap().inserts[0].inputs[0].bus = 2;
        assert!(error(&project).contains("Plug-in input bus is unavailable"));
        project.track_mut(target).unwrap().inserts[0].inputs[0].bus = 0;
        assert!(error(&project).contains("cannot replace an insert's main input"));
        // A device without a sidechain has only its main input.
        let mut slot = builtin(&filter::DESCRIPTOR, &[]);
        slot.inputs.push(route(key, 1));
        project.track_mut(target).unwrap().inserts[0] = slot;
        let message = error(&project);
        assert!(
            message.contains("Plug-in input bus is unavailable"),
            "{message}"
        );
        assert!(message.contains("R1 Filter"), "{message}");
    }

    /// Two tracks with the same click, one through `slot`: the frames of the
    /// master that are not silent.
    fn summed_clicks(slot: EffectSlot) -> Vec<(usize, f32)> {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        project.tempo = 120.0;
        let path = directory.path().join("click.wav");
        let mut samples = vec![0.0; RATE as usize * 2];
        samples[2 * 1000] = 0.25;
        samples[2 * 1000 + 1] = 0.25;
        AudioData {
            sample_rate: RATE,
            channels: 2,
            samples,
        }
        .write_wav(&path)
        .unwrap();
        for name in ["plain", "processed"] {
            let track = project.add_audio_track(name);
            project.track_mut(track).unwrap().gain = 1.0;
            project.track_mut(track).unwrap().clips.push(Clip::audio(
                name,
                0.0,
                2.0,
                path.to_string_lossy(),
            ));
        }
        let processed = project.tracks.last().unwrap().id;
        project.track_mut(processed).unwrap().inserts.push(slot);
        render(&project, RATE, 2.0)
            .unwrap()
            .iter()
            .step_by(2)
            .copied()
            .enumerate()
            .filter(|(_, sample)| sample.abs() > 1e-4)
            .collect()
    }

    #[test]
    fn compressor_lookahead_latency_is_compensated() {
        use compressor::*;
        let clicks = summed_clicks(builtin(&DESCRIPTOR, &[(LOOKAHEAD, 5.0), (RATIO, 1.0)]));
        assert_eq!(clicks.len(), 1, "{clicks:?}");
        assert_eq!(clicks[0].0, 1000);
        assert!((clicks[0].1 - 0.5).abs() < 1e-3, "{}", clicks[0].1);
    }

    #[test]
    fn a_latency_changing_edit_rebuilds_and_others_stay_live() {
        use compressor::*;
        let (mut project, _key, target, _directory) = ducking_project();
        let effect = project.track(target).unwrap().inserts[0].id;
        let renderer = Renderer::new(&project, RATE, false).unwrap();
        let preparer = renderer.live_edit_preparer();
        // The panel's edit: the value goes to the live queue, then the project.
        let edit =
            |preparer: &live_edit::LiveEditPreparer, project: &mut Project, id: u32, plain: f32| {
                let value = f64::from(DESCRIPTOR.parameter(id).unwrap().to_normalized(plain));
                preparer
                    .compatibility()
                    .acknowledge_parameter(target, Some(effect), id, value);
                let parameters = &mut project.track_mut(target).unwrap().inserts[0].parameters;
                match parameters.iter_mut().find(|parameter| parameter.id == id) {
                    Some(parameter) => parameter.value = value,
                    None => parameters.push(PluginParameterValue { id, value }),
                }
                preparer.compatible(project)
            };
        assert!(edit(&preparer, &mut project, THRESHOLD, -12.0));
        assert!(
            !edit(&preparer, &mut project, LOOKAHEAD, 5.0),
            "lookahead is latency"
        );
        assert!(edit(&preparer, &mut project, LOOKAHEAD, 0.0));
        assert!(
            !edit(&preparer, &mut project, MODE, 1.0),
            "the limiter looks ahead"
        );
        assert!(edit(&preparer, &mut project, MODE, 0.0));
        // FET has no lookahead, so the knob moves nothing there.
        edit(&preparer, &mut project, CHARACTER, 3.0);
        let renderer = Renderer::new(&project, RATE, false).unwrap();
        let preparer = renderer.live_edit_preparer();
        assert!(edit(&preparer, &mut project, LOOKAHEAD, 5.0));
    }

    #[test]
    fn saturator_latency_is_the_same_at_every_quality() {
        use saturator::*;
        for quality in 0..3 {
            let clicks = summed_clicks(builtin(
                &DESCRIPTOR,
                &[(QUALITY, quality as f32), (MIX, 0.0)],
            ));
            assert_eq!(clicks.len(), 1, "{quality}: {clicks:?}");
            assert_eq!(clicks[0].0, 1000);
        }
        let values = |_: u32| 0.0;
        assert_eq!((DESCRIPTOR.latency)(&values, 48_000.0), LATENCY);
        assert_eq!((DESCRIPTOR.latency)(&|_| 1.0, 96_000.0), LATENCY);
    }
}

/// A MIDI track playing `pitch` for two beats through R1 Sampler, holding the
/// recording at `sample` (if any) in slot 0.
fn sampler_project(sample: Option<&std::path::Path>, pitch: u8) -> Project {
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_instrument_track("Sampler");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    if let Some(path) = sample {
        project
            .load_device_sample(
                track,
                daw_core::DeviceSample {
                    slot: 0,
                    path: path.to_string_lossy().into_owned(),
                    name: "A".into(),
                },
            )
            .unwrap();
    }
    let channel = project.track_mut(track).unwrap();
    channel.gain = 1.0;
    channel.clips.push(Clip::midi(
        "Note",
        0.0,
        2.0,
        vec![daw_core::Note::new(pitch, 0.0, 2.0, 127)],
    ));
    project
}

fn write_sine(path: &std::path::Path, hz: f32, seconds: f32) {
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: (0..(44_100.0 * seconds) as usize)
            .map(|n| 0.5 * (std::f32::consts::TAU * hz * n as f32 / 44_100.0).sin())
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

/// Upward zero crossings per second of the left channel between `from` and
/// `to` seconds.
fn frequency(samples: &[f32], from: f32, to: f32) -> f32 {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    let window = &left[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
    let crossings = window
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    crossings as f32 / (to - from)
}

#[test]
fn the_sampler_plays_its_sample_relative_to_the_root() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.wav");
    write_sine(&path, 440.0, 2.0);
    // The root key defaults to C4 (60); an octave up doubles the pitch.
    let rendered = render(&sampler_project(Some(&path), 72), RATE, 2.0).unwrap();
    let hz = frequency(&rendered, 0.1, 0.9);
    assert!((hz - 880.0).abs() < 4.0, "{hz} Hz");
    let rendered = render(&sampler_project(Some(&path), 60), RATE, 2.0).unwrap();
    let hz = frequency(&rendered, 0.1, 0.9);
    assert!((hz - 440.0).abs() < 4.0, "{hz} Hz");
    // Without a sample the instrument is silent.
    let rendered = render(&sampler_project(None, 60), RATE, 2.0).unwrap();
    assert!(rendered.iter().all(|x| *x == 0.0));
}

#[test]
fn a_missing_or_oversized_sample_fails_preparation_clearly() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("gone.wav");
    let project = sampler_project(Some(&missing), 60);
    let error = format!("{:#}", render(&project, RATE, 2.0).unwrap_err());
    assert!(error.contains("Loading the sample of Sampler"), "{error}");
    assert!(error.contains("missing"), "{error}");
    // 16-bit PCM, so the file is half its decoded size of just over 32 MiB.
    let large = directory.path().join("large.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&large, spec).unwrap();
    for _ in 0..(daw_devices::SampleData::MAX_BYTES / 4 + 1) {
        writer.write_sample(0_i16).unwrap();
    }
    writer.finalize().unwrap();
    let project = sampler_project(Some(&large), 60);
    let error = format!("{:#}", render(&project, RATE, 2.0).unwrap_err());
    assert!(error.contains("limited to 32 MiB"), "{error}");
}
