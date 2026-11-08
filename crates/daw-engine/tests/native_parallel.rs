//! Self-contained native VST3 callbacks through the shipped helper and renderer.
//! Build the helper first: cargo build -p daw-plugins --bin daw-plugin-host.
#[path = "common/native_graph.rs"]
mod native_graph;

use anyhow::Result;
use daw_engine::OfflineRenderer;
use native_graph::plugin_fixture::{DspProbeOptions, EditorProbeFixture};
use native_graph::{
    NativeFixtures, Topology, clear_trace, maximum_callback_overlap, process_count, read_trace,
};

fn render_blocks(renderer: &mut OfflineRenderer, frames: &[usize]) -> Result<Vec<[f32; 2]>> {
    let mut result = Vec::new();
    for &count in frames {
        let mut output = vec![[0.0; 2]; count];
        let mut positions = vec![0.0; count];
        renderer.render(&mut output, &mut positions)?;
        assert!(output.iter().flatten().all(|sample| sample.is_finite()));
        result.extend(output);
    }
    Ok(result)
}

fn assert_exact_samples(actual: &[[f32; 2]], expected: &[[f32; 2]], context: &str) {
    assert_eq!(actual.len(), expected.len());
    if let Some((index, (actual, expected))) = actual
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (actual, expected))| actual.map(f32::to_bits) != expected.map(f32::to_bits))
    {
        panic!("{context}: sample mismatch at frame {index}: {actual:?} != {expected:?}");
    }
}

#[test]
fn real_native_callbacks_overlap_and_keep_exact_samples() -> Result<()> {
    let trace = tempfile::tempdir()?;
    let fixtures = NativeFixtures::build(512, Some(trace.path()));
    let project = fixtures.project(Topology::Independent, 4.0)?;
    let sizes = [256; 16];
    let mut serial = OfflineRenderer::new(&project, 48_000, Some(1))?;
    let expected = render_blocks(&mut serial, &sizes)?;
    let serial_trace = read_trace(trace.path())?;
    assert_eq!(process_count(&serial_trace), 4);
    assert_eq!(maximum_callback_overlap(&serial_trace), 1);
    drop(serial);
    clear_trace(trace.path())?;

    let mut parallel = OfflineRenderer::new(&project, 48_000, Some(4))?;
    let actual = render_blocks(&mut parallel, &sizes)?;
    let parallel_trace = read_trace(trace.path())?;
    assert_eq!(process_count(&parallel_trace), 4);
    let mut helper_pids: Vec<_> = parallel_trace.iter().map(|interval| interval.pid).collect();
    helper_pids.sort_unstable();
    helper_pids.dedup();
    eprintln!(
        "Native callback proof: helper PIDs {helper_pids:?}; {} callbacks per render; serial peak {}; parallel peak {}",
        parallel_trace.len(),
        maximum_callback_overlap(&serial_trace),
        maximum_callback_overlap(&parallel_trace)
    );
    assert_exact_samples(&actual, &expected, "Native DSP under parallel scheduling");
    assert!(actual.iter().flatten().any(|sample| sample.abs() > 1e-5));
    assert_eq!(
        parallel_trace.len(),
        serial_trace.len(),
        "A graph worker duplicated or skipped processing"
    );
    if std::thread::available_parallelism().is_ok_and(|count| count.get() > 1) {
        assert!(
            maximum_callback_overlap(&parallel_trace) >= 2,
            "Expected overlapping actual native callbacks from independent helpers, not just overlapping IPC waits"
        );
    } else {
        eprintln!(
            "Skipping native overlap assertion: multiple available CPUs were not reported; exact samples and callback counts were still verified"
        );
    }
    Ok(())
}

#[test]
fn native_graphs_preserve_variable_blocks_routes_and_serial_chains() -> Result<()> {
    let fixtures = NativeFixtures::build(16, None);
    for topology in [
        Topology::Independent,
        Topology::Uneven,
        Topology::Branched,
        Topology::SerialChain,
    ] {
        let project = fixtures.project(topology, 4.0)?;
        let mut serial = OfflineRenderer::new(&project, 48_000, Some(1))?;
        let mut parallel = OfflineRenderer::new(&project, 48_000, Some(4))?;
        let mut nonzero = false;
        for count in [1, 17, 256, 511, 3, 128, 4093] {
            let mut expected = vec![[0.0; 2]; count];
            let mut actual = expected.clone();
            let mut expected_positions = vec![0.0; count];
            let mut actual_positions = expected_positions.clone();
            serial.render(&mut expected, &mut expected_positions)?;
            parallel.render(&mut actual, &mut actual_positions)?;
            assert_exact_samples(&actual, &expected, &format!("{topology:?}, block {count}"));
            assert_eq!(
                actual_positions, expected_positions,
                "Timeline mismatch for {topology:?}"
            );
            assert!(actual.iter().flatten().all(|sample| sample.is_finite()));
            nonzero |= actual.iter().flatten().any(|sample| sample.abs() > 1e-5);
        }
        assert!(
            nonzero,
            "{topology:?} must exercise audible native processing"
        );
    }
    Ok(())
}

#[test]
fn native_processing_errors_propagate_without_retrying_the_failed_block() -> Result<()> {
    let trace = tempfile::tempdir()?;
    let fixture = EditorProbeFixture::build_cpu_load(DspProbeOptions {
        stages: 8,
        trace_directory: Some(trace.path()),
        fail_after_blocks: Some(3),
        ..Default::default()
    });
    let project = native_graph::project(&fixture.path, &fixture.path, Topology::Independent, 4.0)?;
    for workers in [1, 4] {
        clear_trace(trace.path())?;
        let mut renderer = OfflineRenderer::new(&project, 48_000, Some(workers))?;
        let mut failed = false;
        for _ in 0..8 {
            if renderer
                .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
                .is_err()
            {
                failed = true;
                break;
            }
        }
        assert!(
            failed,
            "Native failure must reach the offline caller with {workers} workers"
        );
        let intervals = read_trace(trace.path())?;
        assert!(intervals.iter().any(|interval| interval.sequence == 3));
        assert!(
            intervals.iter().all(|interval| interval.sequence <= 3),
            "A failed native block was retried"
        );
    }
    Ok(())
}
