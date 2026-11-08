//! Prepared dependencies and deterministic input reduction. Nodes can finish
//! in any order, but each destination keeps the serial level/source/route order.
use super::{Insert, Processor, RenderTrack, Route, RouteKind};
use crate::performance::TrackRenderPerformance;

/// Preparation-only linear DAG analysis. A native node pair can overlap iff
/// not every native node belongs to one dependency path. Level widths alone
/// miss independent native tasks separated by different amounts of cheap work.
pub(super) fn parallelism(tracks: &[RenderTrack], level_width: usize) -> (bool, usize) {
    let (native_parallel, frontier) = analyze_parallelism(
        tracks.len(),
        |index| {
            tracks[index].enabled.then_some(
                matches!(tracks[index].processor, Processor::Vst(_))
                    || !tracks[index].inserts.is_empty(),
            )
        },
        |index| tracks[index].routes.iter().filter_map(|route| route.target),
    );
    // Both are observed feasible antichains. This conservative capacity avoids
    // an expensive exact maximum-antichain calculation; the CPU cap is applied
    // by the caller. The frontier can span multiple levels.
    let capacity = level_width.max(frontier);
    (native_parallel || capacity >= 32, capacity)
}

fn analyze_parallelism<I: Iterator<Item = usize>>(
    count: usize,
    node: impl Fn(usize) -> Option<bool>,
    successors: impl Fn(usize) -> I,
) -> (bool, usize) {
    let mut native_depth = vec![0; count];
    let mut pending = vec![0_usize; count];
    let mut native_count = 0;
    let mut longest_native_path = 0;
    for index in 0..count {
        let Some(native) = node(index) else { continue };
        native_count += usize::from(native);
        native_depth[index] += usize::from(native);
        longest_native_path = longest_native_path.max(native_depth[index]);
        for target in successors(index).filter(|&target| node(target).is_some()) {
            native_depth[target] = native_depth[target].max(native_depth[index]);
            // Count and consume every edge here, including main+send pairs.
            // The runtime scheduler separately deduplicates parent completion.
            pending[target] += 1;
        }
    }
    let mut ready = (0..count)
        .filter(|&index| node(index).is_some() && pending[index] == 0)
        .count();
    let mut frontier = ready;
    for index in 0..count {
        if node(index).is_none() {
            continue;
        }
        debug_assert_eq!(pending[index], 0, "source-before-target graph");
        ready -= 1;
        for target in successors(index).filter(|&target| node(target).is_some()) {
            pending[target] -= 1;
            if pending[target] == 0 {
                ready += 1;
            }
        }
        frontier = frontier.max(ready);
    }
    (native_count > longest_native_path, frontier)
}

pub(super) struct Schedule {
    pub predecessors: Vec<usize>,
    pub successors: Vec<Vec<usize>>,
    pub roots: Vec<usize>,
    pub order: Vec<usize>,
    pub ranks: Vec<usize>,
    incoming: Vec<Vec<(usize, usize)>>,
    terminal: Vec<(usize, usize)>,
}

impl Schedule {
    pub fn new(tracks: &[RenderTrack], levels: &[Vec<usize>]) -> Self {
        let order: Vec<_> = levels.iter().flatten().copied().collect();
        let mut schedule = Self {
            predecessors: vec![0; tracks.len()],
            successors: vec![Vec::new(); tracks.len()],
            roots: Vec::new(),
            ranks: vec![0; tracks.len()],
            incoming: vec![Vec::new(); tracks.len()],
            terminal: Vec::new(),
            order,
        };
        for (rank, &source) in schedule.order.iter().enumerate() {
            schedule.ranks[source] = rank;
            for (edge, route) in tracks[source].routes.iter().enumerate() {
                if let Some(target) = route.target {
                    if tracks[target].enabled {
                        schedule.incoming[target].push((source, edge));
                        schedule.successors[source].push(target);
                    }
                } else {
                    schedule.terminal.push((source, edge));
                }
            }
            // Preserve main/send/sidechain edges, but release each target once.
            schedule.successors[source].sort_unstable();
            schedule.successors[source].dedup();
            for &target in &schedule.successors[source] {
                schedule.predecessors[target] += 1;
            }
        }
        schedule.roots = schedule
            .order
            .iter()
            .copied()
            .filter(|&index| schedule.predecessors[index] == 0)
            .collect();
        schedule
    }

    pub fn initialize_costs(&self, tracks: &[RenderTrack], timings: &mut [TrackRenderPerformance]) {
        for &index in &self.order {
            // Initial preference only; observations replace this estimate.
            let plugins = usize::from(matches!(tracks[index].processor, Processor::Vst(_)))
                + tracks[index].inserts.len();
            timings[index].estimated_process_ns = 1_000 + plugins as u64 * 100_000;
        }
        self.update_priorities(timings);
    }

    pub fn update_priorities(&self, timings: &mut [TrackRenderPerformance]) {
        // Reverse level order visits successors before their predecessors.
        for &index in self.order.iter().rev() {
            let tail = self.successors[index]
                .iter()
                .map(|&next| timings[next].critical_path_ns)
                .max()
                .unwrap_or(0);
            timings[index].critical_path_ns =
                timings[index].estimated_process_ns.saturating_add(tail);
        }
    }

    pub fn gather(
        &self,
        index: usize,
        destination: &mut RenderTrack,
        tracks: &[Option<Box<RenderTrack>>],
        count: usize,
    ) {
        for &(source, edge) in &self.incoming[index] {
            let source = tracks[source].as_ref().expect("completed upstream node");
            // Insert failure may retain healthy raw instrument output. Match
            // route_track; channel_active is deliberately not this predicate.
            if !source.enabled || source.plugin_failed {
                continue;
            }
            mix_route(&source.routes[edge], destination, count);
        }
    }

    pub fn gather_terminal(&self, tracks: &[Option<Box<RenderTrack>>], output: &mut [[f64; 2]]) {
        for &(source, edge) in &self.terminal {
            let source = tracks[source].as_ref().expect("completed terminal node");
            if !source.enabled || source.plugin_failed {
                continue;
            }
            mix_terminal(&source.routes[edge], output);
        }
    }
}

/// Shared by serial routing and parallel destination gathers. Delays have
/// already run once at the source; preserve f64 accumulation until processing.
pub(super) fn mix_route(route: &Route, destination: &mut RenderTrack, count: usize) {
    let (left, right) = match route.kind {
        RouteKind::PluginInput { effect, input } => {
            let buffer = if let Some(effect) = effect {
                let Insert::Vst { inputs, .. } = &mut destination.inserts[effect];
                &mut inputs[input]
            } else {
                &mut destination.instrument_inputs[input]
            };
            (&mut buffer.mix_left, &mut buffer.mix_right)
        }
        _ => (&mut destination.mix_left, &mut destination.mix_right),
    };
    for (sample, frame) in route.output[..count].iter().enumerate() {
        left[sample] += f64::from(frame[0]);
        right[sample] += f64::from(frame[1]);
    }
}

pub(super) fn mix_terminal(route: &Route, output: &mut [[f64; 2]]) {
    for (destination, frame) in output.iter_mut().zip(&route.output) {
        destination[0] += f64::from(frame[0]);
        destination[1] += f64::from(frame[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(nodes: &[(Option<bool>, &[usize])]) -> (bool, usize) {
        analyze_parallelism(
            nodes.len(),
            |index| nodes[index].0,
            |index| nodes[index].1.iter().copied(),
        )
    }

    #[test]
    fn independent_native_nodes_at_different_depths_enable_parallelism() {
        // Native root 0 is independent of native node 2 behind cheap root 1.
        // Duplicate parent edges must not release 2 twice or distort width.
        assert_eq!(
            policy(&[
                (Some(true), &[3]),
                (Some(false), &[2, 2]),
                (Some(true), &[3]),
                (Some(false), &[]),
            ]),
            (true, 2)
        );
    }

    #[test]
    fn serial_native_chain_and_disabled_native_branches_stay_serial() {
        assert_eq!(
            policy(&[
                (Some(true), &[2]),
                (None, &[2]),
                (Some(true), &[3]),
                (Some(false), &[]),
            ]),
            (false, 1)
        );
        assert_eq!(policy(&[]), (false, 0));
    }

    #[test]
    fn ready_frontier_can_be_wider_than_every_single_level() {
        // Levels have widths 3,2,1; after cheap root 0 finishes, native roots
        // 1,2 and native children 3,4 can all run concurrently.
        assert_eq!(
            policy(&[
                (Some(false), &[3, 4]),
                (Some(true), &[5]),
                (Some(true), &[5]),
                (Some(true), &[5]),
                (Some(true), &[5]),
                (Some(false), &[]),
            ]),
            (true, 4)
        );
    }

    #[test]
    fn measured_costs_prioritize_the_longest_remaining_dependency_path() {
        // 0 -> 2 -> 3, 0 -> 3 and 1 -> 3. Parallel successors are not summed.
        let schedule = Schedule {
            predecessors: vec![0, 0, 1, 3],
            successors: vec![vec![2, 3], vec![3], vec![3], vec![]],
            roots: vec![0, 1],
            order: vec![0, 1, 2, 3],
            ranks: vec![0, 1, 2, 3],
            incoming: vec![vec![]; 4],
            terminal: vec![],
        };
        let mut timings: Vec<_> = [10, 100, 20, 5]
            .into_iter()
            .map(|estimated_process_ns| TrackRenderPerformance {
                estimated_process_ns,
                ..Default::default()
            })
            .collect();
        schedule.update_priorities(&mut timings);
        assert_eq!(
            timings
                .iter()
                .map(|timing| timing.critical_path_ns)
                .collect::<Vec<_>>(),
            [35, 105, 25, 5]
        );
        // A newly expensive descendant raises its ancestor's priority.
        timings[2].record(2_000, 0);
        schedule.update_priorities(&mut timings);
        assert!(timings[0].critical_path_ns > timings[1].critical_path_ns);
        assert_eq!(
            timings[0].critical_path_ns,
            10 + timings[2].critical_path_ns
        );
        // A stalled process cannot overflow path arithmetic.
        timings[2].estimated_process_ns = u64::MAX;
        schedule.update_priorities(&mut timings);
        assert_eq!(timings[0].critical_path_ns, u64::MAX);
    }
}
