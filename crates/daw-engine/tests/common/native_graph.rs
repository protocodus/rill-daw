//! Shared native DSP fixture and graph construction for regression tests/benchmarks.
#![allow(dead_code)]

#[path = "../../../daw-plugins/tests/common/editor_probe_fixture.rs"]
pub mod plugin_fixture;

use anyhow::{Context, Result};
use daw_core::{Clip, Effect, EffectSlot, Instrument, Note, Project};
use plugin_fixture::{CLASS_ID, DspProbeOptions, EditorProbeFixture};
use std::{collections::HashSet, path::Path};

pub struct NativeFixtures {
    pub instrument: EditorProbeFixture,
    pub effect: EditorProbeFixture,
}

impl NativeFixtures {
    pub fn build(stages: usize, trace_directory: Option<&Path>) -> Self {
        Self {
            instrument: EditorProbeFixture::build_cpu_load(DspProbeOptions {
                stages,
                trace_directory,
                ..Default::default()
            }),
            effect: EditorProbeFixture::build_cpu_load(DspProbeOptions {
                stages,
                effect: true,
                trace_directory,
                ..Default::default()
            }),
        }
    }

    pub fn project(&self, topology: Topology, length_beats: f64) -> Result<Project> {
        project(
            &self.instrument.path,
            &self.effect.path,
            topology,
            length_beats,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Topology {
    Independent,
    Uneven,
    Branched,
    SerialChain,
}

pub fn project(
    instrument: &Path,
    effect: &Path,
    topology: Topology,
    length_beats: f64,
) -> Result<Project> {
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = false;
    let count = if matches!(topology, Topology::SerialChain) {
        1
    } else {
        4
    };
    let mut tracks = Vec::new();
    for index in 0..count {
        let id = project.add_midi_track(format!("Native DSP {index}"));
        let track = project.track_mut(id).unwrap();
        track.instrument = Instrument::Vst3 {
            path: instrument.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        };
        track.gain = 0.5 / count as f32;
        track.pan = (index as f32 - (count - 1) as f32 * 0.5) * 0.15;
        track.clips.push(Clip::midi(
            "Deterministic note",
            0.0,
            length_beats,
            vec![Note::new(60 + index as u8, 0.0, length_beats, 100)],
        ));
        let effects = match topology {
            Topology::Uneven => [3, 0, 1, 2][index],
            Topology::SerialChain => 3,
            _ => 0,
        };
        for slot in 0..effects {
            track.inserts.push(effect_slot(effect, slot));
        }
        tracks.push(id);
    }
    if matches!(topology, Topology::Branched) {
        let left = project.add_bus("Left branch");
        let right = project.add_bus("Right branch");
        let sum = project.add_bus("Joined branches");
        for (index, track) in tracks.into_iter().enumerate() {
            project.set_output(track, Some(if index % 2 == 0 { left } else { right }))?;
            project.add_send(track, if index % 2 == 0 { right } else { left }, 0.2, true)?;
        }
        for bus in [left, right] {
            project.set_output(bus, Some(sum))?;
            project
                .track_mut(bus)
                .unwrap()
                .inserts
                .push(effect_slot(effect, 0));
        }
        project.track_mut(left).unwrap().latency_samples = 17;
        project.track_mut(right).unwrap().latency_samples = 63;
        project
            .track_mut(sum)
            .unwrap()
            .inserts
            .push(effect_slot(effect, 0));
    }
    project.validate()?;
    Ok(project)
}

fn effect_slot(path: &Path, index: usize) -> EffectSlot {
    EffectSlot::new(
        format!("Native filter {index}"),
        Effect::Vst3 {
            path: path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    )
}

#[derive(Debug)]
pub struct CallbackInterval {
    pub pid: u32,
    pub sequence: u64,
    pub start_ns: u128,
    pub end_ns: u128,
}

pub fn read_trace(directory: &Path) -> Result<Vec<CallbackInterval>> {
    let mut intervals = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "trace")
        {
            continue;
        }
        for line in std::fs::read_to_string(&path)?.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            anyhow::ensure!(fields.len() == 4, "Malformed callback trace");
            let interval = CallbackInterval {
                pid: fields[0].parse()?,
                sequence: fields[1].parse()?,
                start_ns: fields[2].parse()?,
                end_ns: fields[3].parse()?,
            };
            anyhow::ensure!(
                interval.end_ns >= interval.start_ns,
                "Callback clock went backwards"
            );
            intervals.push(interval);
        }
    }
    Ok(intervals)
}

pub fn clear_trace(directory: &Path) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "trace")
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Independent process intervals, not worker-thread or IPC-wait intervals.
pub fn maximum_callback_overlap(intervals: &[CallbackInterval]) -> usize {
    let mut events: Vec<_> = intervals
        .iter()
        .flat_map(|interval| [(interval.start_ns, 1_i32), (interval.end_ns, -1_i32)])
        .collect();
    events.sort_unstable();
    let (mut active, mut peak) = (0_i32, 0_i32);
    for (_, delta) in events {
        active += delta;
        peak = peak.max(active);
    }
    peak as usize
}

pub fn process_count(intervals: &[CallbackInterval]) -> usize {
    intervals
        .iter()
        .map(|interval| interval.pid)
        .collect::<HashSet<_>>()
        .len()
}

/// Linux total process CPU (all threads plus still-running descendants). None on
/// other platforms. Call outside timed block loops, before dropping the renderer.
pub fn process_tree_cpu_ticks() -> Option<u64> {
    fn visit(pid: u32, seen: &mut HashSet<u32>) -> Option<u64> {
        if !seen.insert(pid) {
            return Some(0);
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        let mut ticks =
            fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
        for task in std::fs::read_dir(format!("/proc/{pid}/task"))
            .ok()?
            .flatten()
        {
            let children =
                std::fs::read_to_string(task.path().join("children")).unwrap_or_default();
            for child in children
                .split_whitespace()
                .filter_map(|pid| pid.parse().ok())
            {
                ticks += visit(child, seen).unwrap_or_default();
            }
        }
        Some(ticks)
    }
    visit(std::process::id(), &mut HashSet::new())
}

pub fn clock_ticks_per_second() -> Result<f64> {
    let result = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()?;
    anyhow::ensure!(result.status.success(), "getconf CLK_TCK failed");
    String::from_utf8(result.stdout)?
        .trim()
        .parse()
        .context("Invalid CLK_TCK")
}
