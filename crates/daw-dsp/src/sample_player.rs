//! Composable sample selection shared by every sampling instrument.
//!
//! Documents are validated and compiled away from the audio thread. Triggering
//! visits a bounded tree, allocates nothing, and produces independent leaves;
//! envelopes, pitch and filtering remain the receiving instrument's job.
use crate::{Markers, Random, db_to_gain};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_PLAYER_NODES: usize = 256;
pub const MAX_PLAYER_DEPTH: usize = 8;
pub const MAX_SAMPLE_SLOTS: u32 = 128;
pub const MAX_TRIGGER_LAYERS: usize = 32;
pub const MAX_PLAYER_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerNode {
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub kind: PlayerKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlayerKind {
    Sample {
        slot: u32,
        start: f32,
        end: f32,
        reverse: bool,
        gain_db: f32,
    },
    All {
        children: Vec<PlayerNode>,
    },
    RoundRobin {
        children: Vec<PlayerNode>,
    },
    Random {
        children: Vec<PlayerNode>,
    },
    Velocity {
        children: Vec<VelocityChild>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VelocityChild {
    pub min: u8,
    pub max: u8,
    pub child: PlayerNode,
}

impl PlayerNode {
    pub fn sample(id: u64, slot: u32) -> Self {
        Self {
            id,
            name: String::new(),
            kind: PlayerKind::Sample {
                slot,
                start: 0.0,
                end: 1.0,
                reverse: false,
                gain_db: 0.0,
            },
        }
    }

    pub fn find(&self, id: u64) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        match &self.kind {
            PlayerKind::Sample { .. } => None,
            PlayerKind::All { children }
            | PlayerKind::RoundRobin { children }
            | PlayerKind::Random { children } => children.iter().find_map(|child| child.find(id)),
            PlayerKind::Velocity { children } => {
                children.iter().find_map(|child| child.child.find(id))
            }
        }
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Self> {
        if self.id == id {
            return Some(self);
        }
        match &mut self.kind {
            PlayerKind::Sample { .. } => None,
            PlayerKind::All { children }
            | PlayerKind::RoundRobin { children }
            | PlayerKind::Random { children } => {
                children.iter_mut().find_map(|child| child.find_mut(id))
            }
            PlayerKind::Velocity { children } => children
                .iter_mut()
                .find_map(|child| child.child.find_mut(id)),
        }
    }

    pub fn uses_sample(&self, slot: u32) -> bool {
        match &self.kind {
            PlayerKind::Sample { slot: source, .. } => *source == slot,
            PlayerKind::All { children }
            | PlayerKind::RoundRobin { children }
            | PlayerKind::Random { children } => {
                children.iter().any(|child| child.uses_sample(slot))
            }
            PlayerKind::Velocity { children } => {
                children.iter().any(|child| child.child.uses_sample(slot))
            }
        }
    }

    /// Names are presentation metadata; changing them need not rebuild audio.
    pub fn playback_eq(&self, other: &Self) -> bool {
        if self.id != other.id {
            return false;
        }
        match (&self.kind, &other.kind) {
            (PlayerKind::All { children: a }, PlayerKind::All { children: b })
            | (PlayerKind::RoundRobin { children: a }, PlayerKind::RoundRobin { children: b })
            | (PlayerKind::Random { children: a }, PlayerKind::Random { children: b }) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.playback_eq(b))
            }
            (PlayerKind::Velocity { children: a }, PlayerKind::Velocity { children: b }) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        a.min == b.min && a.max == b.max && a.child.playback_eq(&b.child)
                    })
            }
            (PlayerKind::Sample { .. }, PlayerKind::Sample { .. }) => self.kind == other.kind,
            _ => false,
        }
    }

    /// A stable identity for a newly appended node in this tree.
    pub fn next_id(&self) -> u64 {
        fn highest(node: &PlayerNode) -> u64 {
            let below = match &node.kind {
                PlayerKind::Sample { .. } => 0,
                PlayerKind::All { children }
                | PlayerKind::RoundRobin { children }
                | PlayerKind::Random { children } => {
                    children.iter().map(highest).max().unwrap_or(0)
                }
                PlayerKind::Velocity { children } => children
                    .iter()
                    .map(|child| highest(&child.child))
                    .max()
                    .unwrap_or(0),
            };
            node.id.max(below)
        }
        highest(self).checked_add(1).unwrap_or_else(|| {
            (0..=MAX_PLAYER_NODES as u64)
                .find(|id| self.find(*id).is_none())
                .unwrap_or(0)
        })
    }

    pub fn validate(&self, capacity: u32) -> Result<(), String> {
        validate(self, capacity).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(id: u64, kind: PlayerKind) -> PlayerNode {
        PlayerNode {
            id,
            name: String::new(),
            kind,
        }
    }

    fn leaves(first: u64, slots: &[u32]) -> Vec<PlayerNode> {
        slots
            .iter()
            .enumerate()
            .map(|(index, slot)| PlayerNode::sample(first + index as u64, *slot))
            .collect()
    }

    fn play(
        program: &mut PlayerProgram,
        velocity: u8,
        available: impl Fn(u32) -> bool,
    ) -> Vec<u32> {
        let mut output = [SampleTrigger::default(); MAX_TRIGGER_LAYERS];
        let count = program.trigger(velocity as f32 / 127.0, available, &mut output);
        output[..count].iter().map(|sample| sample.slot).collect()
    }

    #[test]
    fn nested_velocity_round_robin_and_random_keep_independent_state() {
        // Quiet hits have their own alternating children; hard hits alternate
        // a fixed source and a random group of five other source recordings.
        let tree = branch(
            0,
            PlayerKind::Velocity {
                children: vec![
                    VelocityChild {
                        min: 1,
                        max: 63,
                        child: branch(
                            1,
                            PlayerKind::RoundRobin {
                                children: leaves(2, &[0, 1]),
                            },
                        ),
                    },
                    VelocityChild {
                        min: 64,
                        max: 127,
                        child: branch(
                            4,
                            PlayerKind::RoundRobin {
                                children: vec![
                                    PlayerNode::sample(5, 2),
                                    branch(
                                        6,
                                        PlayerKind::Random {
                                            children: leaves(7, &[3, 4, 5, 6, 7]),
                                        },
                                    ),
                                ],
                            },
                        ),
                    },
                ],
            },
        );
        let mut program = PlayerProgram::compile(&tree, MAX_SAMPLE_SLOTS).unwrap();
        assert_eq!(program.maximum_layers(), 1);
        let mut sequence = Vec::new();
        for _ in 0..32 {
            assert_eq!(play(&mut program, 63, |_| true), [0]);
            assert_eq!(play(&mut program, 64, |_| true), [2]);
            assert_eq!(play(&mut program, 1, |_| true), [1]);
            let random = play(&mut program, 127, |_| true);
            assert_eq!(random.len(), 1);
            assert!((3..=7).contains(&random[0]));
            sequence.push(random[0]);
        }
        assert!(sequence.iter().any(|slot| *slot != sequence[0]));
        program.reset();
        for slot in sequence {
            assert_eq!(play(&mut program, 64, |_| true), [2]);
            assert_eq!(play(&mut program, 127, |_| true), [slot]);
        }
    }

    #[test]
    fn unavailable_and_empty_branches_are_skipped_without_consuming_descendant_state() {
        let tree = branch(
            0,
            PlayerKind::RoundRobin {
                children: vec![
                    branch(1, PlayerKind::All { children: vec![] }),
                    branch(
                        2,
                        PlayerKind::All {
                            children: leaves(3, &[0, 1]),
                        },
                    ),
                    branch(
                        5,
                        PlayerKind::RoundRobin {
                            children: leaves(6, &[2, 3]),
                        },
                    ),
                ],
            },
        );
        let mut program = PlayerProgram::compile(&tree, MAX_SAMPLE_SLOTS).unwrap();
        assert!(play(&mut program, 127, |_| false).is_empty());
        assert_eq!(play(&mut program, 127, |slot| slot != 0), [1]);
        assert_eq!(play(&mut program, 127, |_| true), [2]);
        assert_eq!(play(&mut program, 127, |_| true), [0, 1]);
        assert_eq!(play(&mut program, 127, |_| true), [3]);
    }

    #[test]
    fn velocity_ranges_have_inclusive_edges_overlap_and_silent_gaps() {
        let tree = branch(
            0,
            PlayerKind::Velocity {
                children: vec![
                    VelocityChild {
                        min: 1,
                        max: 64,
                        child: PlayerNode::sample(1, 0),
                    },
                    VelocityChild {
                        min: 64,
                        max: 100,
                        child: PlayerNode::sample(2, 1),
                    },
                ],
            },
        );
        let mut program = PlayerProgram::compile(&tree, MAX_SAMPLE_SLOTS).unwrap();
        assert_eq!(program.maximum_layers(), 2);
        for (velocity, expected) in [
            (1, vec![0]),
            (63, vec![0]),
            (64, vec![0, 1]),
            (65, vec![1]),
            (100, vec![1]),
            (101, vec![]),
            (127, vec![]),
        ] {
            assert_eq!(play(&mut program, velocity, |_| true), expected);
        }
    }

    #[test]
    fn regions_and_gain_are_per_leaf_with_relative_master_markers() {
        let tree = branch(
            0,
            PlayerKind::All {
                children: vec![
                    PlayerNode {
                        id: 1,
                        name: "Short reverse".into(),
                        kind: PlayerKind::Sample {
                            slot: 4,
                            start: 0.2,
                            end: 0.8,
                            reverse: true,
                            gain_db: -6.0,
                        },
                    },
                    PlayerNode::sample(2, 4),
                ],
            },
        );
        let mut program = PlayerProgram::compile(&tree, MAX_SAMPLE_SLOTS).unwrap();
        let mut output = [SampleTrigger::default(); MAX_TRIGGER_LAYERS];
        assert_eq!(program.trigger(1.0, |_| true, &mut output), 2);
        assert!((output[0].gain - 0.5011872).abs() < 1e-6);
        assert_eq!(output[1].gain, 1.0);
        let markers = output[0].markers(Markers {
            start: 0.25,
            end: 0.75,
            loop_start: 0.3,
            loop_end: 0.6,
            crossfade: 32.0,
            mode: crate::LoopMode::Forward,
            reverse: true,
        });
        assert!((markers.start - 0.35).abs() < 1e-6);
        assert!((markers.end - 0.65).abs() < 1e-6);
        assert!((markers.loop_start - 0.38).abs() < 1e-6);
        assert!(!markers.reverse);
        assert!(tree.uses_sample(4) && !tree.uses_sample(5));
        assert_eq!(tree.find(1).unwrap().name, "Short reverse");
        assert_eq!(tree.next_id(), 3);
    }

    #[test]
    fn validation_bounds_work_and_rejects_invalid_documents() {
        let max = branch(
            0,
            PlayerKind::All {
                children: leaves(1, &(0..32).collect::<Vec<_>>()),
            },
        );
        assert!(max.validate(128).is_ok());
        let too_many = branch(
            0,
            PlayerKind::All {
                children: leaves(1, &(0..33).collect::<Vec<_>>()),
            },
        );
        assert!(too_many.validate(128).is_err());
        let duplicate = branch(
            0,
            PlayerKind::All {
                children: vec![PlayerNode::sample(0, 1)],
            },
        );
        assert!(duplicate.validate(128).is_err());
        assert!(PlayerNode::sample(0, 128).validate(128).is_err());
        let mut deep = PlayerNode::sample(0, 0);
        for id in 1..MAX_PLAYER_DEPTH {
            deep = branch(
                id as u64,
                PlayerKind::All {
                    children: vec![deep],
                },
            );
        }
        assert!(deep.validate(128).is_ok());
        deep = branch(
            99,
            PlayerKind::All {
                children: vec![deep],
            },
        );
        assert!(deep.validate(128).is_err());
        let many = branch(
            999,
            PlayerKind::RoundRobin {
                children: leaves(0, &vec![0; 256]),
            },
        );
        assert!(many.validate(128).is_err());
        let mut invalid = PlayerNode::sample(0, 0);
        if let PlayerKind::Sample { start, .. } = &mut invalid.kind {
            *start = f32::NAN;
        }
        assert!(invalid.validate(128).is_err());
    }
}

/// Validate both storage and worst-case work before the audio thread sees it.
fn validate(root: &PlayerNode, capacity: u32) -> Result<usize, String> {
    fn visit(
        node: &PlayerNode,
        depth: usize,
        capacity: u32,
        ids: &mut HashSet<u64>,
    ) -> Result<[usize; 127], String> {
        if depth > MAX_PLAYER_DEPTH {
            return Err(format!(
                "Sample players may be at most {MAX_PLAYER_DEPTH} levels deep"
            ));
        }
        if ids.len() >= MAX_PLAYER_NODES || !ids.insert(node.id) {
            return Err(format!(
                "Sample players need unique IDs and at most {MAX_PLAYER_NODES} nodes"
            ));
        }
        if node.name.len() > MAX_PLAYER_NAME_BYTES {
            return Err("Sample player name is too long".into());
        }
        let mut maximum = [0; 127];
        match &node.kind {
            PlayerKind::Sample {
                slot,
                start,
                end,
                gain_db,
                ..
            } => {
                if *slot >= capacity || *slot >= MAX_SAMPLE_SLOTS {
                    return Err("Sample player references an invalid source slot".into());
                }
                if !start.is_finite()
                    || !end.is_finite()
                    || !(0.0..=1.0).contains(start)
                    || !(0.0..=1.0).contains(end)
                    || start >= end
                {
                    return Err("Sample region must have 0 ≤ start < end ≤ 1".into());
                }
                if !gain_db.is_finite() || !(-60.0..=12.0).contains(gain_db) {
                    return Err("Sample gain must be between −60 and 12 dB".into());
                }
                maximum.fill(1);
            }
            PlayerKind::All { children }
            | PlayerKind::RoundRobin { children }
            | PlayerKind::Random { children } => {
                let all = matches!(node.kind, PlayerKind::All { .. });
                for child in children {
                    let child = visit(child, depth + 1, capacity, ids)?;
                    for (maximum, child) in maximum.iter_mut().zip(child) {
                        *maximum = if all {
                            *maximum + child
                        } else {
                            (*maximum).max(child)
                        };
                    }
                }
            }
            PlayerKind::Velocity { children } => {
                for child in children {
                    if child.min == 0 || child.min > child.max || child.max > 127 {
                        return Err("Velocity ranges must satisfy 1 ≤ min ≤ max ≤ 127".into());
                    }
                    let child_maximum = visit(&child.child, depth + 1, capacity, ids)?;
                    for velocity in child.min..=child.max {
                        maximum[velocity as usize - 1] += child_maximum[velocity as usize - 1];
                    }
                }
            }
        }
        if maximum.iter().any(|count| *count > MAX_TRIGGER_LAYERS) {
            return Err(format!(
                "A hit may play at most {MAX_TRIGGER_LAYERS} samples together"
            ));
        }
        Ok(maximum)
    }
    visit(root, 1, capacity, &mut HashSet::new())
        .map(|counts| counts.into_iter().max().unwrap_or(0))
}

/// One chosen recording. Several leaves may reference the same source with
/// different regions or gains; each remains an independent playhead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleTrigger {
    pub node_id: u64,
    pub slot: u32,
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    pub gain: f32,
}

impl Default for SampleTrigger {
    fn default() -> Self {
        Self {
            node_id: 0,
            slot: 0,
            start: 0.0,
            end: 1.0,
            reverse: false,
            gain: 1.0,
        }
    }
}

impl SampleTrigger {
    /// Instrument markers address the chosen leaf's region. A full-range leaf
    /// therefore preserves legacy marker and loop behavior exactly.
    pub fn markers(&self, master: Markers) -> Markers {
        let width = self.end - self.start;
        let position = |value: f32| self.start + value * width;
        Markers {
            start: position(master.start),
            end: position(master.end),
            loop_start: position(master.loop_start),
            loop_end: position(master.loop_end),
            reverse: master.reverse ^ self.reverse,
            ..master
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Sample(SampleTrigger),
    All,
    RoundRobin,
    Random,
    Velocity,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    child: usize,
    min: u8,
    max: u8,
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    first: usize,
    count: usize,
    next: usize,
    random: Random,
}

/// A prepared program owns selection state for one pad or sampler. Its
/// vectors never grow during `trigger`; state advances only on visited nodes.
#[derive(Clone, Debug)]
pub struct PlayerProgram {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    maximum: usize,
}

impl PlayerProgram {
    pub fn compile(root: &PlayerNode, capacity: u32) -> Result<Self, String> {
        let maximum = validate(root, capacity)?;
        let mut program = Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            maximum,
        };
        fn add(program: &mut PlayerProgram, source: &PlayerNode) -> usize {
            let kind = match &source.kind {
                PlayerKind::Sample {
                    slot,
                    start,
                    end,
                    reverse,
                    gain_db,
                } => Kind::Sample(SampleTrigger {
                    node_id: source.id,
                    slot: *slot,
                    start: *start,
                    end: *end,
                    reverse: *reverse,
                    gain: db_to_gain(*gain_db),
                }),
                PlayerKind::All { .. } => Kind::All,
                PlayerKind::RoundRobin { .. } => Kind::RoundRobin,
                PlayerKind::Random { .. } => Kind::Random,
                PlayerKind::Velocity { .. } => Kind::Velocity,
            };
            let index = program.nodes.len();
            program.nodes.push(Node {
                kind,
                first: 0,
                count: 0,
                next: 0,
                random: Random::stream(0x5231_5021, source.id as u32, (source.id >> 32) as u32),
            });
            let children: Vec<Edge> = match &source.kind {
                PlayerKind::Sample { .. } => Vec::new(),
                PlayerKind::All { children }
                | PlayerKind::RoundRobin { children }
                | PlayerKind::Random { children } => children
                    .iter()
                    .map(|child| Edge {
                        child: add(program, child),
                        min: 1,
                        max: 127,
                    })
                    .collect(),
                PlayerKind::Velocity { children } => children
                    .iter()
                    .map(|child| Edge {
                        child: add(program, &child.child),
                        min: child.min,
                        max: child.max,
                    })
                    .collect(),
            };
            program.nodes[index].first = program.edges.len();
            program.nodes[index].count = children.len();
            program.edges.extend(children);
            index
        }
        add(&mut program, root);
        Ok(program)
    }

    pub fn maximum_layers(&self) -> usize {
        self.maximum
    }

    /// All referenced leaves, for off-thread tail estimation and inspection.
    pub fn samples(&self) -> impl Iterator<Item = SampleTrigger> + '_ {
        self.nodes.iter().filter_map(|node| match node.kind {
            Kind::Sample(sample) => Some(sample),
            _ => None,
        })
    }

    pub fn reset(&mut self) {
        for node in &mut self.nodes {
            node.next = 0;
            node.random.reset();
        }
    }

    pub fn trigger(
        &mut self,
        velocity: f32,
        available: impl Fn(u32) -> bool,
        output: &mut [SampleTrigger],
    ) -> usize {
        if !velocity.is_finite() || velocity <= 0.0 || output.len() < self.maximum {
            return 0;
        }
        let velocity = (velocity * 127.0).round().clamp(1.0, 127.0) as u8;
        let mut eligible = [false; MAX_PLAYER_NODES];
        // Parents precede descendants, so availability is computed bottom-up.
        for index in (0..self.nodes.len()).rev() {
            let node = &self.nodes[index];
            eligible[index] = match node.kind {
                Kind::Sample(sample) => available(sample.slot),
                _ => self.edges[node.first..node.first + node.count]
                    .iter()
                    .any(|edge| eligible[edge.child] && (edge.min..=edge.max).contains(&velocity)),
            };
        }
        fn emit(
            program: &mut PlayerProgram,
            index: usize,
            velocity: u8,
            eligible: &[bool; MAX_PLAYER_NODES],
            output: &mut [SampleTrigger],
            count: &mut usize,
        ) {
            if !eligible[index] {
                return;
            }
            let node = &program.nodes[index];
            let (kind, first, children, next) = (node.kind, node.first, node.count, node.next);
            match kind {
                Kind::Sample(sample) => {
                    output[*count] = sample;
                    *count += 1;
                }
                Kind::All | Kind::Velocity => {
                    for offset in 0..children {
                        let edge = program.edges[first + offset];
                        if (edge.min..=edge.max).contains(&velocity) {
                            emit(program, edge.child, velocity, eligible, output, count);
                        }
                    }
                }
                Kind::RoundRobin => {
                    for offset in 0..children {
                        let choice = (next + offset) % children;
                        let edge = program.edges[first + choice];
                        if eligible[edge.child] {
                            program.nodes[index].next = (choice + 1) % children;
                            emit(program, edge.child, velocity, eligible, output, count);
                            break;
                        }
                    }
                }
                Kind::Random => {
                    let choices = program.edges[first..first + children]
                        .iter()
                        .filter(|edge| eligible[edge.child])
                        .count();
                    let mut choice = ((u64::from(program.nodes[index].random.next_u32())
                        * choices as u64)
                        >> 32) as usize;
                    for offset in 0..children {
                        let edge = program.edges[first + offset];
                        if eligible[edge.child] {
                            if choice == 0 {
                                emit(program, edge.child, velocity, eligible, output, count);
                                break;
                            }
                            choice -= 1;
                        }
                    }
                }
            }
        }
        let mut count = 0;
        emit(self, 0, velocity, &eligible, output, &mut count);
        count
    }
}
