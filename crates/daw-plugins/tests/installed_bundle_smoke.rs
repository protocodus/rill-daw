//! Smoke-test an installed bundle of any format, for example:
//! RILL_TEST_PLUGIN_BUNDLE=/usr/lib/lv2/calf.lv2 cargo test -p daw-plugins \
//!   --test installed_bundle_smoke -- --ignored --nocapture
//! Optional RILL_TEST_PLUGIN_LIMIT bounds how many plug-ins are loaded (default 8),
//! and RILL_TEST_PLUGIN_NAME keeps only names containing that text. With
//! RILL_TEST_PLUGIN_EDITOR=1 (and a display) the first plug-in's editor is captured.
use daw_plugins::{MidiMessage, VstEffect, VstInstrument};

#[test]
#[ignore = "Requires RILL_TEST_PLUGIN_BUNDLE"]
fn installed_bundle_scans_loads_processes_and_round_trips_state() {
    let bundle = std::path::PathBuf::from(
        std::env::var_os("RILL_TEST_PLUGIN_BUNDLE").expect("Set RILL_TEST_PLUGIN_BUNDLE"),
    );
    let limit = std::env::var("RILL_TEST_PLUGIN_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let name = std::env::var("RILL_TEST_PLUGIN_NAME").unwrap_or_default();
    let found: Vec<_> = daw_plugins::scan_paths(&[bundle])
        .unwrap()
        .into_iter()
        .filter(|info| info.name.contains(&name))
        .collect();
    eprintln!("Scanned {} plug-ins", found.len());
    assert!(!found.is_empty());
    let mut loaded = 0;
    for info in found.iter().take(limit) {
        let outcome = if info.is_instrument() == Some(true) {
            VstInstrument::load_info(info, 48_000.0, 256).and_then(|mut plugin| {
                let (mut left, mut right) = ([0.0; 256], [0.0; 256]);
                let on = MidiMessage {
                    offset: 0,
                    pitch: 60,
                    velocity: 100,
                    on: true,
                };
                let mut peak = 0.0_f32;
                for block in 0..32 {
                    let events = if block == 0 { vec![on] } else { vec![] };
                    plugin.process(&events, &mut left, &mut right)?;
                    peak = left.iter().chain(&right).fold(peak, |p, s| p.max(s.abs()));
                }
                let state = plugin.save_state()?;
                plugin.load_state(&state)?;
                Ok(format!(
                    "instrument peak {peak:.3}, {} parameters, {}-byte state",
                    plugin.parameters()?.len(),
                    state.len()
                ))
            })
        } else {
            VstEffect::load_info(info, 48_000.0, 256).and_then(|mut plugin| {
                let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
                let (mut left, mut right) = ([0.0; 256], [0.0; 256]);
                for _ in 0..8 {
                    plugin.process(&input, &input, &mut left, &mut right)?;
                }
                assert!(left.iter().chain(&right).all(|s| s.is_finite()));
                let state = plugin.save_state()?;
                plugin.load_state(&state)?;
                Ok(format!(
                    "effect latency {}, {} parameters, {}-byte state",
                    plugin.latency_samples(),
                    plugin.parameters()?.len(),
                    state.len()
                ))
            })
        };
        match outcome {
            Ok(summary) => {
                loaded += 1;
                eprintln!("OK   {} [{}]: {summary}", info.name, info.format.label());
            }
            Err(error) => eprintln!("FAIL {} [{}]: {error:#}", info.name, info.format.label()),
        }
    }
    assert!(loaded > 0, "No installed plug-in loaded");
    if std::env::var_os("RILL_TEST_PLUGIN_EDITOR").is_some() {
        let directory = tempfile::tempdir().unwrap();
        let png = directory.path().join("editor.png");
        daw_plugins::capture_editor_info(&found[0], &png).unwrap();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&png).unwrap()))
                .read_info()
                .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        let mut present = [false; 256];
        for value in &pixels[..frame.buffer_size()] {
            present[*value as usize] = true;
        }
        let levels = present.iter().filter(|&&p| p).count();
        eprintln!(
            "Editor of {}: {}x{} with {levels} channel levels",
            found[0].name, frame.width, frame.height
        );
        assert!(levels > 32, "The editor rendered no detail");
    }
}
