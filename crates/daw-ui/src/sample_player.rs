//! Composable sample players: a readable tree and one focused node inspector.
use daw_dsp::sample_player::{PlayerKind, PlayerNode, VelocityChild};
use egui::Ui;
use std::collections::BTreeSet;

const MIN_REGION: f32 = 0.000_001;

#[derive(Debug, Default)]
pub struct SamplePlayerEditorState {
    pub selected: Option<u64>,
    pub collapsed: BTreeSet<u64>,
    /// Last rendered geometry, also useful to accessibility/automation hosts.
    pub rows: Vec<(u64, egui::Rect)>,
    pub waveform: Option<egui::Rect>,
    pub controls: Vec<(&'static str, egui::Rect)>,
    initialized: bool,
}

/// The host supplies decoded previews; this component never reads files.
pub struct PlayerSampleView<'a> {
    pub slot: u32,
    pub name: &'a str,
    pub seconds: f32,
    pub peaks: &'a [(f32, f32)],
    pub audible_region: Option<(f32, f32)>,
}

#[derive(Default)]
pub struct SamplePlayerEditorOutput {
    pub changed: bool,
    pub sample_action: Option<(u32, crate::SampleAction)>,
    pub file: Option<(u32, std::path::PathBuf)>,
    pub source: Option<(u32, crate::SampleDrop)>,
}

const GROUPS: [&str; 4] = ["All at once", "Round robin", "Random", "Velocity"];

fn kind_index(kind: &PlayerKind) -> Option<usize> {
    match kind {
        PlayerKind::Sample { .. } => None,
        PlayerKind::All { .. } => Some(0),
        PlayerKind::RoundRobin { .. } => Some(1),
        PlayerKind::Random { .. } => Some(2),
        PlayerKind::Velocity { .. } => Some(3),
    }
}

fn children(node: &PlayerNode) -> Vec<&PlayerNode> {
    match &node.kind {
        PlayerKind::Sample { .. } => Vec::new(),
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => children.iter().collect(),
        PlayerKind::Velocity { children } => children.iter().map(|entry| &entry.child).collect(),
    }
}

fn into_children(kind: PlayerKind) -> Vec<PlayerNode> {
    match kind {
        PlayerKind::Sample { .. } => Vec::new(),
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => children,
        PlayerKind::Velocity { children } => {
            children.into_iter().map(|entry| entry.child).collect()
        }
    }
}

fn group_kind(index: usize, children: Vec<PlayerNode>) -> PlayerKind {
    match index {
        1 => PlayerKind::RoundRobin { children },
        2 => PlayerKind::Random { children },
        3 => {
            let count = children.len().max(1);
            PlayerKind::Velocity {
                children: children
                    .into_iter()
                    .enumerate()
                    .map(|(index, child)| VelocityChild {
                        min: (1 + index * 127 / count) as u8,
                        max: ((index + 1) * 127 / count).max(1 + index * 127 / count) as u8,
                        child,
                    })
                    .collect(),
            }
        }
        _ => PlayerKind::All { children },
    }
}

fn title(node: &PlayerNode, samples: &[PlayerSampleView<'_>]) -> String {
    if !node.name.is_empty() {
        return node.name.clone();
    }
    match &node.kind {
        PlayerKind::Sample { slot, .. } => samples
            .iter()
            .find(|sample| sample.slot == *slot)
            .map_or_else(|| "Empty sample".into(), |sample| sample.name.to_owned()),
        kind => GROUPS[kind_index(kind).unwrap()].into(),
    }
}

fn initial_folds(node: &PlayerNode, depth: usize, collapsed: &mut BTreeSet<u64>) {
    if depth >= 2 && kind_index(&node.kind).is_some() {
        collapsed.insert(node.id);
    }
    for child in children(node) {
        initial_folds(child, depth + 1, collapsed);
    }
}

fn tree_rows(
    ui: &mut Ui,
    node: &PlayerNode,
    depth: usize,
    state: &mut SamplePlayerEditorState,
    samples: &[PlayerSampleView<'_>],
    velocity: Option<(u8, u8)>,
    output: &mut SamplePlayerEditorOutput,
) {
    let child_nodes = children(node);
    let group = kind_index(&node.kind);
    ui.push_id(node.id, |ui| {
        crate::control_row(ui, |ui| {
            ui.add_space(depth as f32 * crate::tokens::SPACE_MD);
            if group.is_some() {
                let folded = state.collapsed.contains(&node.id);
                if ui
                    .add(crate::IconButton::icon_only(
                        if folded {
                            crate::Icon::ChevronRight
                        } else {
                            crate::Icon::ChevronDown
                        },
                        if folded {
                            "Expand player"
                        } else {
                            "Collapse player"
                        },
                    ))
                    .clicked()
                {
                    if folded {
                        state.collapsed.remove(&node.id);
                    } else {
                        state.collapsed.insert(node.id);
                    }
                }
            } else {
                ui.add_space(crate::tokens::CONTROL_HEIGHT);
            }
            let prefix = group.map_or("Sample", |index| GROUPS[index]);
            let name = title(node, samples);
            let kind = if name == prefix {
                String::new()
            } else {
                format!("  ·  {prefix}")
            };
            let label = format!(
                "{name}{kind}{}",
                velocity.map_or_else(String::new, |(min, max)| format!("  ·  {min}–{max}"))
            );
            let response = crate::selectable(
                ui,
                state.selected == Some(node.id),
                egui::RichText::new(label),
            );
            state.rows.push((node.id, response.rect));
            if response.clicked() {
                state.selected = Some(node.id);
            }
            if let PlayerKind::Sample { slot, .. } = node.kind {
                let dropped = response
                    .dnd_release_payload::<crate::SampleDrop>()
                    .map(|payload| (*payload).clone());
                let native = if ui
                    .input(|input| input.pointer.hover_pos())
                    .is_some_and(|at| response.rect.contains(at))
                {
                    ui.input_mut(|input| std::mem::take(&mut input.raw.dropped_files))
                        .into_iter()
                        .find_map(|file| file.path)
                } else {
                    None
                };
                if let Some(path) = native {
                    output.file = Some((slot, path));
                    state.selected = Some(node.id);
                }
                if let Some(source) = dropped {
                    output.source = Some((slot, source));
                    state.selected = Some(node.id);
                }
            }
        });
    });
    if !state.collapsed.contains(&node.id) {
        for (index, child) in child_nodes.into_iter().enumerate() {
            let velocity = if let PlayerKind::Velocity { children } = &node.kind {
                Some((children[index].min, children[index].max))
            } else {
                None
            };
            tree_rows(ui, child, depth + 1, state, samples, velocity, output);
        }
    }
}

fn append_child(node: &mut PlayerNode, child: PlayerNode) {
    match &mut node.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => children.push(child),
        PlayerKind::Velocity { children } => children.push(VelocityChild {
            min: 1,
            max: 127,
            child,
        }),
        PlayerKind::Sample { .. } => unreachable!("caller wraps a sample before adding"),
    }
}

fn refresh_ids(node: &mut PlayerNode, next: &mut u64) {
    node.id = *next;
    *next += 1;
    match &mut node.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => {
            for child in children {
                refresh_ids(child, next);
            }
        }
        PlayerKind::Velocity { children } => {
            for entry in children {
                refresh_ids(&mut entry.child, next);
            }
        }
        PlayerKind::Sample { .. } => {}
    }
}

fn parent_of(node: &mut PlayerNode, id: u64) -> Option<&mut PlayerNode> {
    if children(node).iter().any(|child| child.id == id) {
        return Some(node);
    }
    match &mut node.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => {
            children.iter_mut().find_map(|child| parent_of(child, id))
        }
        PlayerKind::Velocity { children } => children
            .iter_mut()
            .find_map(|entry| parent_of(&mut entry.child, id)),
        PlayerKind::Sample { .. } => None,
    }
}

fn insert_after(parent: &mut PlayerNode, id: u64, child: PlayerNode) {
    match &mut parent.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => {
            let index = children
                .iter()
                .position(|child| child.id == id)
                .expect("known parent");
            children.insert(index + 1, child);
        }
        PlayerKind::Velocity { children } => {
            let index = children
                .iter()
                .position(|entry| entry.child.id == id)
                .expect("known parent");
            children.insert(
                index + 1,
                VelocityChild {
                    min: children[index].min,
                    max: children[index].max,
                    child,
                },
            );
        }
        PlayerKind::Sample { .. } => unreachable!("a parent has children"),
    }
}

fn remove_child(node: &mut PlayerNode, id: u64) -> bool {
    match &mut node.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => {
            if let Some(index) = children.iter().position(|child| child.id == id) {
                children.remove(index);
                return true;
            }
            children.iter_mut().any(|child| remove_child(child, id))
        }
        PlayerKind::Velocity { children } => {
            if let Some(index) = children.iter().position(|entry| entry.child.id == id) {
                children.remove(index);
                return true;
            }
            children
                .iter_mut()
                .any(|entry| remove_child(&mut entry.child, id))
        }
        PlayerKind::Sample { .. } => false,
    }
}

fn move_child(node: &mut PlayerNode, id: u64, down: bool) -> bool {
    match &mut node.kind {
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => {
            if let Some(index) = children.iter().position(|child| child.id == id) {
                let next = if down {
                    (index + 1).min(children.len() - 1)
                } else {
                    index.saturating_sub(1)
                };
                children.swap(index, next);
                return index != next;
            }
            children.iter_mut().any(|child| move_child(child, id, down))
        }
        PlayerKind::Velocity { children } => {
            if let Some(index) = children.iter().position(|entry| entry.child.id == id) {
                let next = if down {
                    (index + 1).min(children.len() - 1)
                } else {
                    index.saturating_sub(1)
                };
                children.swap(index, next);
                return index != next;
            }
            children
                .iter_mut()
                .any(|entry| move_child(&mut entry.child, id, down))
        }
        PlayerKind::Sample { .. } => false,
    }
}

fn velocity_child(node: &mut PlayerNode, id: u64) -> Option<&mut VelocityChild> {
    match &mut node.kind {
        PlayerKind::Velocity { children } => {
            if let Some(index) = children.iter().position(|entry| entry.child.id == id) {
                return children.get_mut(index);
            }
            children
                .iter_mut()
                .find_map(|entry| velocity_child(&mut entry.child, id))
        }
        PlayerKind::All { children }
        | PlayerKind::RoundRobin { children }
        | PlayerKind::Random { children } => children
            .iter_mut()
            .find_map(|child| velocity_child(child, id)),
        PlayerKind::Sample { .. } => None,
    }
}

#[derive(Clone, Copy)]
enum TreeAction {
    AddSample,
    AddGroup(usize),
    Wrap(usize),
    Remove,
    Up,
    Down,
    Duplicate,
}

/// A full workspace editor. The caller owns the player transaction and sample
/// pool. Every structural action is explicit; nested players retain stable IDs.
pub fn sample_player_editor(
    ui: &mut Ui,
    root: &mut PlayerNode,
    state: &mut SamplePlayerEditorState,
    samples: &[PlayerSampleView<'_>],
    free_slot: Option<u32>,
) -> SamplePlayerEditorOutput {
    if !state.initialized {
        initial_folds(root, 0, &mut state.collapsed);
        state.initialized = true;
    }
    state.rows.clear();
    state.waveform = None;
    state.controls.clear();
    let before = root.clone();
    let mut output = SamplePlayerEditorOutput::default();
    let selected = state
        .selected
        .filter(|id| root.find(*id).is_some())
        .unwrap_or(root.id);
    state.selected = Some(selected);
    let mut action = None;
    crate::control_row_wrapped(ui, |ui| {
        ui.add_enabled_ui(free_slot.is_some(), |ui| {
            if crate::button(ui, "Add sample").clicked() {
                action = Some(TreeAction::AddSample);
            }
        });
        for (label, wrap) in [("Add group", false), ("Wrap in", true)] {
            crate::choice_menu(ui, label, |ui| {
                crate::command_menu_contents(ui, |ui| {
                    for (index, label) in GROUPS.iter().enumerate() {
                        if crate::menu_item(ui, *label, None).clicked() {
                            action = Some(if wrap {
                                TreeAction::Wrap(index)
                            } else {
                                TreeAction::AddGroup(index)
                            });
                            ui.close();
                        }
                    }
                });
            });
        }
        if crate::button(ui, "Duplicate")
            .on_hover_text("Copy this player and its children; recordings are shared")
            .clicked()
        {
            action = Some(TreeAction::Duplicate);
        }
        ui.add_enabled_ui(selected != root.id, |ui| {
            if crate::button(ui, "Move up").clicked() {
                action = Some(TreeAction::Up);
            }
            if crate::button(ui, "Move down").clicked() {
                action = Some(TreeAction::Down);
            }
        });
        if crate::button(ui, "Remove").clicked() {
            action = Some(TreeAction::Remove);
        }
    });
    if let Some(action) = action {
        let next = root.next_id();
        match action {
            TreeAction::Wrap(index) => {
                let node = root.find_mut(selected).expect("selected node exists");
                let child = node.clone();
                *node = PlayerNode {
                    id: next,
                    name: String::new(),
                    kind: group_kind(index, vec![child]),
                };
                state.selected = Some(next);
            }
            TreeAction::AddSample | TreeAction::AddGroup(_) | TreeAction::Duplicate => {
                let selected_node = root.find(selected).expect("selected node exists");
                let into_group = kind_index(&selected_node.kind).is_some()
                    && !matches!(action, TreeAction::Duplicate);
                let new = match action {
                    TreeAction::AddSample => {
                        PlayerNode::sample(next, free_slot.expect("enabled with free slot"))
                    }
                    TreeAction::AddGroup(index) => PlayerNode {
                        id: next,
                        name: String::new(),
                        kind: group_kind(index, vec![]),
                    },
                    _ => {
                        let mut copy = selected_node.clone();
                        let mut next = next;
                        refresh_ids(&mut copy, &mut next);
                        copy
                    }
                };
                let new_id = new.id;
                if into_group {
                    append_child(root.find_mut(selected).unwrap(), new);
                    state.collapsed.remove(&selected);
                } else if let Some(parent) = parent_of(root, selected) {
                    insert_after(parent, selected, new);
                } else {
                    let child = root.clone();
                    let wrapper_id = new.next_id().max(next + 1);
                    *root = PlayerNode {
                        id: wrapper_id,
                        name: String::new(),
                        kind: PlayerKind::All {
                            children: vec![child, new],
                        },
                    };
                    state.collapsed.remove(&wrapper_id);
                }
                state.selected = Some(new_id);
            }
            TreeAction::Remove => {
                if selected == root.id {
                    root.kind = PlayerKind::All {
                        children: Vec::new(),
                    };
                } else {
                    remove_child(root, selected);
                }
                state.selected = Some(root.id);
            }
            TreeAction::Up => {
                move_child(root, selected, false);
            }
            TreeAction::Down => {
                move_child(root, selected, true);
            }
        }
    }
    ui.add_space(crate::tokens::SPACE_SM);
    ui.columns(2, |columns| {
        crate::card(columns[0].ctx()).show(&mut columns[0], |ui| {
            ui.set_min_width(ui.available_width());
            crate::device_section_title(ui, "Player");
            egui::ScrollArea::both()
                .id_salt("player_tree")
                .max_height(ui.available_height().max(240.0))
                .show(ui, |ui| {
                    tree_rows(ui, root, 0, state, samples, None, &mut output)
                });
        });
        egui::ScrollArea::vertical()
            .id_salt("player_inspector")
            .auto_shrink([false, false])
            .show(&mut columns[1], |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    node_inspector(ui, root, state, samples, &mut output);
                });
            });
    });
    output.changed = *root != before;
    output
}

fn node_inspector(
    ui: &mut Ui,
    root: &mut PlayerNode,
    state: &mut SamplePlayerEditorState,
    samples: &[PlayerSampleView<'_>],
    output: &mut SamplePlayerEditorOutput,
) {
    let selected = state.selected.unwrap_or(root.id);
    if let Some(entry) = velocity_child(root, selected) {
        crate::control_row(ui, |ui| {
            crate::text::label(ui, "Velocity");
            let from = ui.add(
                crate::number(&mut entry.min)
                    .range(1..=entry.max)
                    .prefix("From "),
            );
            let to = ui.add(
                crate::number(&mut entry.max)
                    .range(entry.min..=127)
                    .prefix("To "),
            );
            state
                .controls
                .extend([("Velocity min", from.rect), ("Velocity max", to.rect)]);
        });
    }
    let node = root.find_mut(selected).expect("selected node exists");
    let label = title(node, samples);
    crate::sample_identity(
        ui,
        &label,
        kind_index(&node.kind).map_or("Sample", |index| GROUPS[index]),
    );
    if let Some(name) = crate::sample_name_editor(ui, &node.name, "Player name") {
        node.name = name;
    }
    if let Some(mut index) = kind_index(&node.kind) {
        let previous = index;
        crate::control_row(ui, |ui| {
            crate::text::label(ui, "Playback");
            crate::select("player_kind", GROUPS[index]).show_ui(ui, |ui| {
                for (choice, label) in GROUPS.iter().enumerate() {
                    crate::selectable_value(ui, &mut index, choice, *label);
                }
            });
        });
        if index != previous {
            node.kind = group_kind(index, into_children(node.kind.clone()));
        }
        if let PlayerKind::Velocity { children } = &mut node.kind
            && crate::button(ui, "Distribute velocities")
                .on_hover_text("Divide 1–127 evenly between child players")
                .clicked()
        {
            let count = children.len().max(1);
            for (index, child) in children.iter_mut().enumerate() {
                child.min = (1 + index * 127 / count) as u8;
                child.max = ((index + 1) * 127 / count).max(usize::from(child.min)) as u8;
            }
        }
        crate::text::note(
            ui,
            match index {
                1 => "Each hit advances to the next child player.",
                2 => "Each hit chooses one child player.",
                3 => "Matching velocity ranges play together. Select a child to set its range.",
                _ => "Every child player sounds together.",
            },
        );
    }
    if let PlayerKind::Sample {
        slot,
        start,
        end,
        reverse,
        gain_db,
    } = &mut node.kind
    {
        let sample = samples.iter().find(|sample| sample.slot == *slot);
        crate::control_row(ui, |ui| {
            if let Some(action) = crate::sample_actions(
                ui,
                sample.is_some(),
                sample.is_some_and(|s| s.audible_region.is_some()),
            ) {
                match action {
                    crate::SampleAction::TrimSilence
                    | crate::SampleAction::ReverseAudibleRegion => {
                        if let Some((a, b)) = sample.and_then(|s| s.audible_region) {
                            let a = a.max(*start);
                            let b = b.min(*end);
                            if a < b {
                                *start = a;
                                *end = b;
                                if action == crate::SampleAction::ReverseAudibleRegion {
                                    *reverse = !*reverse;
                                }
                            }
                        }
                    }
                    _ => output.sample_action = Some((*slot, action)),
                }
            }
        });
        crate::sample_identity(
            ui,
            sample.map_or("No sample", |sample| sample.name),
            &sample.map_or_else(String::new, |sample| format!("{:.2} s", sample.seconds)),
        );
        let plot =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 180.0));
        let wave = crate::sample_view(
            ui,
            &crate::SampleViewData {
                audible_region: sample.and_then(|s| s.audible_region),
                peaks: sample.map_or(&[], |sample| sample.peaks),
                start: *start,
                end: *end,
                loop_range: None,
                playheads: &[],
                envelope: &[],
                slices: 0,
            },
            egui::vec2(ui.available_width(), 180.0),
        );
        state.waveform = Some(plot);
        for (marker, value) in wave.drags {
            match marker {
                crate::SampleMarker::Start => *start = value.min(*end - MIN_REGION).max(0.0),
                crate::SampleMarker::End => *end = value.max(*start + MIN_REGION).min(1.0),
                _ => {}
            }
        }
        if let Some(path) = wave.files.into_iter().next() {
            output.file = Some((*slot, path));
        }
        if let Some(source) = wave.dropped {
            output.source = Some((*slot, source));
        }
        crate::control_row(ui, |ui| {
            let mut a = *start * 100.0;
            let mut b = *end * 100.0;
            let from = ui.add(
                crate::number(&mut a)
                    .range(0.0..=(b - MIN_REGION * 100.0).max(0.0))
                    .prefix("Start ")
                    .suffix("%")
                    .speed(0.1),
            );
            let to = ui.add(
                crate::number(&mut b)
                    .range((a + MIN_REGION * 100.0).min(100.0)..=100.0)
                    .prefix("End ")
                    .suffix("%")
                    .speed(0.1),
            );
            if from.changed() {
                *start = a / 100.0;
            }
            if to.changed() {
                *end = b / 100.0;
            }
            state
                .controls
                .extend([("Start", from.rect), ("End", to.rect)]);
        });
        crate::control_row(ui, |ui| {
            let gain = ui.add(
                crate::number(gain_db)
                    .range(-60.0..=12.0)
                    .prefix("Gain ")
                    .suffix(" dB")
                    .speed(0.1),
            );
            let reversed = crate::checkbox(ui, reverse, "Reverse");
            state
                .controls
                .extend([("Gain", gain.rect), ("Reverse", reversed.rect)]);
        });
    }
}
