use super::*;
use daw_core::{AutomationTarget, Clip, EffectSlot, PluginParameterValue};

fn level(samples: &[[f32; 2]], channel: usize) -> f64 {
    (samples
        .iter()
        .map(|frame| f64::from(frame[channel]).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

#[test]
fn bypassed_effect_presets_roundtrip_without_loading_and_preserve_audio() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let audio = temp.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.25; 4_000],
    }
    .write_wav(&audio)?;
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Fixture",
        0.0,
        1.0,
        audio.to_string_lossy(),
    ));
    let reference = render(&project, 8_000, 1.0)?;
    let mut slot = EffectSlot::new(
        "Bypassed",
        Effect::Vst3 {
            path: "/not-installed.vst3".into(),
            class_id: None,
        },
    );
    slot.enabled = false;
    slot.state = vec![0, 255, 17, 1];
    slot.parameters = vec![PluginParameterValue { id: 5, value: 0.7 }];
    project.track_mut(track).unwrap().inserts.push(slot);
    let path = temp.path().join("project.daw.json");
    project.save(&path)?;
    let restored = Project::load(path)?;
    assert_eq!(restored, project);
    assert_eq!(render(&restored, 8_000, 1.0)?, reference);
    Ok(())
}

#[test]
#[ignore = "requires CHOWTapeModel in DAW_TEST_VST3_EFFECT"]
fn native_effect_state_recall_overrides_and_live_slot_updates_change_audio() -> Result<()> {
    let path = std::env::var("DAW_TEST_VST3_EFFECT").context("Set DAW_TEST_VST3_EFFECT")?;
    let mut probe = VstEffect::load(Path::new(&path), 48_000.0, BLOCK)?;
    let parameter = probe
        .parameters()?
        .into_iter()
        .find(|parameter| {
            parameter.name.eq_ignore_ascii_case("Output Gain") && !parameter.read_only
        })
        .context("CHOWTapeModel Output Gain parameter is required")?;
    probe.set_parameter(parameter.id, 0.1)?;
    probe.process(
        &[0.0; BLOCK],
        &[0.0; BLOCK],
        &mut [0.0; BLOCK],
        &mut [0.0; BLOCK],
    )?;
    let state = probe.save_state()?;
    assert!(!state.is_empty());
    drop(probe);

    let temp = tempfile::tempdir()?;
    let audio = temp.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..48_000)
            .map(|sample| (std::f32::consts::TAU * 440.0 * sample as f32 / 48_000.0).sin() * 0.1)
            .collect(),
    }
    .write_wav(&audio)?;
    let mut project = Project::new();
    project.tempo = 120.0;
    let mut targets = Vec::new();
    for (name, pan) in [("Left", -1.0), ("Right", 1.0)] {
        let track = project.add_audio_track(name);
        let channel = project.track_mut(track).unwrap();
        channel.gain = 0.25;
        channel.pan = pan;
        channel
            .clips
            .push(Clip::audio("Tone", 0.0, 2.0, audio.to_string_lossy()));
        // The prepared insert index deliberately differs from its project index.
        let mut bypassed = EffectSlot::new(
            "Bypassed",
            Effect::Vst3 {
                path: "/missing.vst3".into(),
                class_id: None,
            },
        );
        bypassed.enabled = false;
        channel.inserts.push(bypassed);
        let mut slot = EffectSlot::new(
            "CHOW",
            Effect::Vst3 {
                path: path.clone(),
                class_id: None,
            },
        );
        slot.state = state.clone();
        targets.push((track, slot.id));
        channel.inserts.push(slot);
    }
    let output = render(&project, 48_000, 2.0)?;
    let baseline = level(&output.as_chunks::<2>().0[12_000..24_000], 0);
    assert!(baseline > 1e-5);
    let saved = temp.path().join("presets.daw.json");
    project.save(&saved)?;
    let recalled = Project::load(saved)?;
    assert_eq!(recalled, project);
    let output = render(&recalled, 48_000, 2.0)?;
    let recalled_level = level(&output.as_chunks::<2>().0[12_000..24_000], 0);
    assert!((recalled_level / baseline - 1.0).abs() < 0.01);

    let (track, slot) = targets[0];
    let mut overridden = recalled.clone();
    overridden.track_mut(track).unwrap().inserts[1]
        .parameters
        .push(PluginParameterValue {
            id: parameter.id,
            value: 0.9,
        });
    let output = render(&overridden, 48_000, 2.0)?;
    let overridden_level = level(&output.as_chunks::<2>().0[12_000..24_000], 0);
    assert!(
        overridden_level > baseline * 4.0,
        "Override must apply after opaque state"
    );
    assert!((level(&output.as_chunks::<2>().0[12_000..24_000], 1) / baseline - 1.0).abs() < 0.05);

    let mut renderer = Renderer::new(&recalled, 48_000, false)?;
    let mut frames = vec![[0.0; 2]; 12_000];
    let mut positions = vec![0.0; frames.len()];
    renderer.render(&mut frames, &mut positions)?;
    renderer.set_plugin_parameter(track, Some(slot), parameter.id, 0.9)?;
    assert!(
        renderer
            .set_plugin_parameter(track, Some(u64::MAX), parameter.id, 0.5)
            .is_err()
    );
    renderer.render(&mut frames, &mut positions)?;
    let hot_level = level(&frames[6_000..], 0);
    assert!(
        hot_level > baseline * 4.0,
        "Live edit must reach the exact active slot"
    );
    assert!((level(&frames[6_000..], 1) / baseline - 1.0).abs() < 0.05);
    drop(renderer);

    let mut automated = recalled;
    automated.add_automation_track_with_value(
        "Gain automation",
        AutomationTarget {
            track,
            parameter: AutomationParameter::VstParameter {
                effect: Some(slot),
                id: parameter.id,
            },
        },
        0.1,
    )?;
    let mut renderer = Renderer::new(&automated, 48_000, false)?;
    renderer.render(&mut frames, &mut positions)?;
    renderer.set_plugin_parameter(track, Some(slot), parameter.id, 0.9)?;
    renderer.render(&mut frames, &mut positions)?;
    let automated_level = level(&frames[6_000..], 0);
    assert!(
        (automated_level / baseline - 1.0).abs() < 0.05,
        "Automation must reassert after manual edits"
    );
    eprintln!(
        "CHOW preset RMS: original={baseline:.6}, recalled={recalled_level:.6}, override={overridden_level:.6}, live={hot_level:.6}, automated={automated_level:.6}"
    );
    Ok(())
}
