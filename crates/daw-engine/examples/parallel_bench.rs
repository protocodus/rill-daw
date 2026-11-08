//! Prepared native-DSP block benchmark, with no third-party plug-in requirement.
//! cargo build --release -p daw-plugins --bin daw-plugin-host
//! cargo run --release -p daw-engine --example parallel_bench
//!
//! RILL_BENCH_BLOCKS=2048 RILL_BENCH_WARMUP=64 RILL_BENCH_STAGES=128
//! RILL_BENCH_WORKERS=4 RILL_BENCH_TOPOLOGY=independent|uneven|branched|chain|all
//! RILL_BENCH_TRANSPORT=legacy|batched|auto|all (default all)
//! Optional RILL_BENCH_VST3 / RILL_BENCH_VST_EFFECT use installed plug-ins instead.
//! Output is CSV. Tracing is disabled; setup/first-block time is separate from
//! steady-state quantiles. Native fixture output must match bit for bit in every
//! scheduler/transport mode. Timing has no pass/fail speedup threshold.
//! Blocks above the nominal device budget are not measured underruns: this is
//! offline processing, and the live output queue can absorb individual spikes.
//! Actual device underruns are reported separately by EngineSnapshot.
#[path = "../tests/common/native_graph.rs"]
mod native_graph;

use anyhow::{Context, Result, ensure};
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use daw_engine::OfflineRenderer;
use daw_plugins::BlockTransport;
use native_graph::{NativeFixtures, Topology, clock_ticks_per_second, process_tree_cpu_ticks};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK: usize = 256;

fn main() -> Result<()> {
    let blocks = number("RILL_BENCH_BLOCKS", 2048)?;
    let warmup = number("RILL_BENCH_WARMUP", 64)?;
    let stages = number("RILL_BENCH_STAGES", 128)?;
    let workers = number("RILL_BENCH_WORKERS", 4)?;
    ensure!(
        (1..=100_000).contains(&blocks)
            && warmup <= 10_000
            && stages <= 4096
            && (1..=32).contains(&workers),
        "Invalid benchmark bounds"
    );
    let instrument = std::env::var("RILL_BENCH_VST3").ok();
    let effect = std::env::var("RILL_BENCH_VST_EFFECT").ok();
    let external = instrument.is_some() || effect.is_some();
    let fixture = (!external).then(|| NativeFixtures::build(stages, None));
    let temporary = tempfile::tempdir()?;
    let topology = std::env::var("RILL_BENCH_TOPOLOGY").unwrap_or_else(|_| "independent".into());
    ensure!(
        !external || topology == "independent",
        "External plug-in mode supports the independent topology; use the native fixture for graph comparisons"
    );
    let topologies = match topology.as_str() {
        "independent" => vec![Topology::Independent],
        "uneven" => vec![Topology::Uneven],
        "branched" => vec![Topology::Branched],
        "chain" => vec![Topology::SerialChain],
        "all" => vec![
            Topology::Independent,
            Topology::Uneven,
            Topology::Branched,
            Topology::SerialChain,
        ],
        _ => anyhow::bail!("Unknown RILL_BENCH_TOPOLOGY"),
    };
    let mode = std::env::var("RILL_BENCH_TRANSPORT").unwrap_or_else(|_| "all".into());
    let transports = match mode.as_str() {
        "legacy" => vec![BlockTransport::Legacy],
        "batched" => vec![BlockTransport::Batched],
        "auto" => vec![BlockTransport::Auto],
        "all" => vec![
            BlockTransport::Legacy,
            BlockTransport::Batched,
            BlockTransport::Auto,
        ],
        _ => anyhow::bail!("Unknown RILL_BENCH_TRANSPORT"),
    };
    let cpu_hz = clock_ticks_per_second().ok();
    let length_beats = (blocks + warmup + 2) as f64 * BLOCK as f64 * 2.0 / f64::from(SAMPLE_RATE);
    println!(
        "# {} DSP, {SAMPLE_RATE} Hz, {BLOCK} frames, {stages} filter stages, {warmup} warmup blocks; nominal block budget {:.3} ms",
        if external {
            "external"
        } else {
            "deterministic native fixture"
        },
        BLOCK as f64 / f64::from(SAMPLE_RATE) * 1000.0
    );
    println!(
        "# CPU includes host threads and live helper descendants on Linux; quantized by CLK_TCK. Callback traces disabled. Preparation includes latency priming. Timing instrumentation is enabled for all modes. Blocks over budget are offline observations, not device underruns; the live output queue may absorb spikes."
    );
    println!(
        "# Graph worker threads exclude the coordinator; dispatched tasks do not prove simultaneous native DSP. Track wall timings include IPC waits and are summed per block."
    );
    println!(
        "topology,transport,requested_workers,graph_worker_threads,blocks,prepare_ms,first_block_ms,p50_us,p95_us,p99_us,p99_9_us,max_us,wall_ms,total_cpu_ms,blocks_over_budget,requests_per_block,request_bytes_per_block,response_bytes_per_block,peak_dispatched_tasks,sum_track_wall_per_block_us,sum_ready_wait_per_block_us,coordinator_wait_per_block_us,max_track_us"
    );
    for topology in topologies {
        let project = match &fixture {
            Some(fixture) => fixture.project(topology, length_beats)?,
            None => external_project(
                instrument.as_deref(),
                effect.as_deref(),
                temporary.path(),
                length_beats,
            )?,
        };
        let mut reference: Option<Vec<[f32; 2]>> = None;
        for &transport in &transports {
            for requested in [Some(1), Some(workers), None] {
                let started = Instant::now();
                let mut renderer = OfflineRenderer::new(&project, SAMPLE_RATE, requested)?;
                renderer.set_block_transport(transport)?;
                let prepare_ms = started.elapsed().as_secs_f64() * 1000.0;
                let mut output = [[0.0; 2]; BLOCK];
                let mut positions = [0.0; BLOCK];
                let started = Instant::now();
                renderer.render(&mut output, &mut positions)?;
                let first_ms = started.elapsed().as_secs_f64() * 1000.0;
                for _ in 0..warmup {
                    renderer.render(&mut output, &mut positions)?;
                }
                let mut timings = Vec::with_capacity(blocks);
                let mut samples = Vec::with_capacity(blocks * BLOCK);
                let mut processing_ns = 0_u128;
                let mut ready_wait_ns = 0_u128;
                let mut coordinator_wait_ns = 0_u128;
                let mut max_track_ns = 0_u64;
                let mut peak_workers = 0;
                let before_ipc = renderer.ipc_stats();
                let before_cpu = process_tree_cpu_ticks();
                let total_started = Instant::now();
                for _ in 0..blocks {
                    let started = Instant::now();
                    renderer.render(&mut output, &mut positions)?;
                    timings.push(started.elapsed().as_nanos() as u64);
                    samples.extend_from_slice(&output);
                    let stats = renderer.performance_snapshot();
                    processing_ns += u128::from(stats.processing_ns);
                    ready_wait_ns += u128::from(stats.ready_wait_ns);
                    coordinator_wait_ns += u128::from(stats.coordinator_wait_ns);
                    max_track_ns = max_track_ns.max(stats.max_track_ns);
                    peak_workers = peak_workers.max(stats.peak_active_workers);
                }
                let wall_ms = total_started.elapsed().as_secs_f64() * 1000.0;
                let after_cpu = process_tree_cpu_ticks();
                let after_ipc = renderer.ipc_stats();
                let cpu_ms = before_cpu
                    .zip(after_cpu)
                    .zip(cpu_hz)
                    .map(|((before, after), hz)| {
                        format!("{:.3}", after.saturating_sub(before) as f64 / hz * 1000.0)
                    })
                    .unwrap_or_else(|| "n/a".into());
                ensure!(
                    samples.iter().flatten().all(|sample| sample.is_finite()),
                    "Nonfinite native output"
                );
                ensure!(
                    samples.iter().flatten().any(|sample| sample.abs() > 1e-6),
                    "Fixture/default preset produced no audio"
                );
                if !external {
                    if let Some(expected) = &reference {
                        ensure!(
                            expected.len() == samples.len()
                                && expected
                                    .iter()
                                    .zip(&samples)
                                    .all(|(expected, actual)| expected.map(f32::to_bits)
                                        == actual.map(f32::to_bits)),
                            "{topology:?}/{transport:?}/{requested:?} changed deterministic output"
                        );
                    } else {
                        reference = Some(samples);
                    }
                }
                let budget_ns = BLOCK as u64 * 1_000_000_000 / u64::from(SAMPLE_RATE);
                let blocks_over_budget = timings.iter().filter(|&&ns| ns > budget_ns).count();
                timings.sort_unstable();
                let quantile = |fraction: f64| {
                    timings[((timings.len() as f64 * fraction).ceil() as usize)
                        .saturating_sub(1)
                        .min(timings.len() - 1)] as f64
                        / 1000.0
                };
                let stats = renderer.performance_snapshot();
                println!(
                    "{topology:?},{transport:?},{},{},{blocks},{prepare_ms:.3},{first_ms:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{wall_ms:.3},{cpu_ms},{blocks_over_budget},{:.3},{:.3},{:.3},{peak_workers},{:.3},{:.3},{:.3},{:.3}",
                    requested
                        .map(|count| count.to_string())
                        .unwrap_or_else(|| "automatic".into()),
                    stats.worker_count,
                    quantile(0.50),
                    quantile(0.95),
                    quantile(0.99),
                    quantile(0.999),
                    quantile(1.0),
                    after_ipc.requests.saturating_sub(before_ipc.requests) as f64 / blocks as f64,
                    after_ipc
                        .request_bytes
                        .saturating_sub(before_ipc.request_bytes) as f64
                        / blocks as f64,
                    after_ipc
                        .response_bytes
                        .saturating_sub(before_ipc.response_bytes) as f64
                        / blocks as f64,
                    processing_ns as f64 / blocks as f64 / 1000.0,
                    ready_wait_ns as f64 / blocks as f64 / 1000.0,
                    coordinator_wait_ns as f64 / blocks as f64 / 1000.0,
                    max_track_ns as f64 / 1000.0
                );
            }
        }
    }
    if external {
        println!(
            "# External fresh instances can use random/modulated presets; bit equality is enforced only for the built-in deterministic test fixture."
        );
    }
    Ok(())
}

fn external_project(
    instrument: Option<&str>,
    effect: Option<&str>,
    directory: &Path,
    length: f64,
) -> Result<Project> {
    let tracks = number("RILL_BENCH_TRACKS", 4)?;
    ensure!((1..=64).contains(&tracks), "Track count must be 1..64");
    let source = directory.join("source.wav");
    if instrument.is_none() {
        AudioData {
            sample_rate: SAMPLE_RATE,
            channels: 2,
            samples: (0..(length * 24_000.0).ceil() as usize)
                .flat_map(|frame| {
                    let phase = frame as f32 * 0.03;
                    [phase.sin() * 0.1, (phase * 1.03).sin() * 0.1]
                })
                .collect(),
        }
        .write_wav(&source)?;
    }
    let canonical = |path: &str| -> Result<String> {
        Ok(PathBuf::from(path)
            .canonicalize()?
            .to_string_lossy()
            .into_owned())
    };
    let mut project = Project::new();
    for index in 0..tracks {
        let id = if let Some(path) = instrument {
            let id = project.add_midi_track(format!("Instrument {index}"));
            let track = project.track_mut(id).unwrap();
            track.instrument = Instrument::Vst3 {
                path: canonical(path)?,
                class_id: None,
            };
            track.clips.push(Clip::midi(
                "Held note",
                0.0,
                length,
                vec![Note::new(60, 0.0, length, 100)],
            ));
            id
        } else {
            let id = project.add_audio_track(format!("Audio {index}"));
            project.track_mut(id).unwrap().clips.push(Clip::audio(
                "Audio",
                0.0,
                length,
                source.to_string_lossy(),
            ));
            id
        };
        let track = project.track_mut(id).unwrap();
        track.gain = 1.0 / tracks as f32;
        if let Some(path) = effect {
            track.inserts.push(EffectSlot::new(
                "External effect",
                Effect::Vst3 {
                    path: canonical(path)?,
                    class_id: None,
                },
            ));
        }
    }
    project.validate()?;
    Ok(project)
}

fn number(name: &str, default: usize) -> Result<usize> {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().with_context(|| format!("Invalid {name}")))
        .unwrap_or(Ok(default))
}
