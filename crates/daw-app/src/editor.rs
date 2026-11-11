use crate::{
    app::{DawApp, PopupKind},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Vec2};

#[derive(Clone, Default)]
struct MixerPeakHistory {
    generation: u64,
    channels: std::collections::BTreeMap<u64, [f32; 2]>,
    master: [f32; 2],
}

fn maximum_peaks(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|index| left[index].max(right[index]))
}

fn mixer_peak_hold(
    ctx: &egui::Context,
    engine: Option<&daw_engine::AudioEngine>,
    generation: u64,
    track: Option<u64>,
) -> [f32; 2] {
    let live = engine.map_or([0.0; 2], |engine| {
        track.map_or_else(
            || engine.channel_peak_hold(daw_core::MASTER_CHANNEL_ID),
            |track| engine.channel_peak_hold(track),
        )
    });
    let retained = ctx
        .data(|data| {
            data.get_temp::<std::sync::Arc<MixerPeakHistory>>(egui::Id::new("mixer_peak_history"))
        })
        .filter(|history| history.generation == generation)
        .map_or([0.0; 2], |history| {
            track.map_or(history.master, |track| {
                history.channels.get(&track).copied().unwrap_or([0.0; 2])
            })
        });
    maximum_peaks(live, retained)
}

fn reset_mixer_peak_hold(
    ctx: &egui::Context,
    engine: Option<&daw_engine::AudioEngine>,
    generation: u64,
    track: Option<u64>,
) {
    if let Some(engine) = engine {
        if let Some(track) = track {
            engine.reset_channel_peak_hold(track);
        } else {
            engine.reset_channel_peak_hold(daw_core::MASTER_CHANNEL_ID);
        }
    }
    ctx.data_mut(|data| {
        let id = egui::Id::new("mixer_peak_history");
        if let Some(history) = data
            .get_temp::<std::sync::Arc<MixerPeakHistory>>(id)
            .filter(|history| history.generation == generation)
        {
            let mut history = (*history).clone();
            if let Some(track) = track {
                history.channels.remove(&track);
            } else {
                history.master = [0.0; 2];
            }
            data.insert_temp(id, std::sync::Arc::new(history));
        }
    });
}

impl DawApp {
    /// Retain the engine's latched peaks before Stop or a graph rebuild retires
    /// it. Document replacement invalidates this cache through view generation.
    pub(crate) fn retain_mixer_peak_holds(&self, ctx: &egui::Context) {
        let Some(engine) = &self.engine else {
            return;
        };
        let generation = crate::spectrogram::waveform_generation(ctx);
        ctx.data_mut(|data| {
            let id = egui::Id::new("mixer_peak_history");
            let mut history = data
                .get_temp::<std::sync::Arc<MixerPeakHistory>>(id)
                .filter(|history| history.generation == generation)
                .map_or_else(
                    || MixerPeakHistory {
                        generation,
                        ..Default::default()
                    },
                    |history| (*history).clone(),
                );
            let channels = self
                .project
                .tracks
                .iter()
                .filter(|track| {
                    matches!(
                        track.kind,
                        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                    )
                })
                .map(|track| track.id)
                .collect::<std::collections::BTreeSet<_>>();
            history.channels.retain(|track, _| channels.contains(track));
            for track in self
                .project
                .tracks
                .iter()
                .filter(|track| channels.contains(&track.id))
            {
                let peaks = engine.channel_peak_hold(track.id);
                let held = history.channels.entry(track.id).or_default();
                *held = maximum_peaks(*held, peaks);
            }
            history.master = maximum_peaks(
                history.master,
                engine.channel_peak_hold(daw_core::MASTER_CHANNEL_ID),
            );
            data.insert_temp(id, std::sync::Arc::new(history));
        });
    }

    pub(crate) fn selected_clip_ai_scope(&self) -> Option<daw_tools::ToolScope> {
        let track_id = self.selected_track?;
        let clip = self.selected()?;
        Some(match clip.content {
            ClipContent::Midi { .. } => daw_tools::ToolScope::MidiClip {
                track_id,
                clip_id: clip.id,
            },
            ClipContent::Audio { .. } => daw_tools::ToolScope::AudioClip {
                track_id,
                clip_id: clip.id,
            },
        })
    }

    pub fn editor(&mut self, ui: &mut egui::Ui) {
        self.audio_clip_selection_navigation(ui);
        let Some(clip) = self.selected() else {
            ui.add_space((ui.available_height() * 0.3).max(24.0));
            let empty = daw_ui::empty_workspace(
                ui,
                "Select a clip in Arrangement",
                "Choose a MIDI or audio clip on the timeline to edit its notes or waveform.",
                &[daw_ui::EmptyWorkspaceAction {
                    id: "open_arrangement",
                    label: "Open Arrangement",
                    description: "Switch to the arrangement timeline",
                    primary: true,
                }],
            );
            if empty.action.as_deref() == Some("open_arrangement") {
                self.open_workspace(crate::app::WorkspaceView::Arrangement);
            }
            return;
        };
        if matches!(clip.content, ClipContent::Audio { .. }) {
            self.audio_editor(ui);
        } else {
            self.piano_roll(ui);
        }
        self.finish_clip_navigation(ui.ctx());
    }
    pub(crate) fn mixer(&mut self, ui: &mut egui::Ui) {
        self.mixer_batch_routing_menu(ui);
        let order = crate::mixer_flow::MixerOrder::read(ui.ctx());
        let reorder_enabled = order == crate::mixer_flow::MixerOrder::Song;
        let mut changed = false;
        let mut popup = None;
        let mut channel_target = None;
        let mut arranger_target = None;
        let mut device_target = None;
        let mut insert_target = None;
        let mut device_diagnostics = Vec::new();
        let mut toggle_selection = None;
        let mut channel_selection = None;
        let mut rename_track = None;
        let mut automation_navigation = None;
        let mut automation_overrides = Vec::new();
        let mut output_menus = Vec::new();
        let mut send_menus = Vec::new();
        let mut track_reorder = None;
        let mut reorder_menu = None;
        let mut use_song_order = false;
        let mut reorder_preview = None;
        let mut reorder_viewport = None;
        let automation: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .filter_map(|track| track.automation.as_ref())
            .filter_map(|lane| {
                self.automation_control_state(lane.target)
                    .map(|state| (lane.target, state))
            })
            .collect();
        let names: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .map(|track| (track.id, track.name.clone()))
            .collect();
        let channel_colors: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .map(|track| (track.id, color(track.color)))
            .collect();
        let channel_indices_by_id: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| !matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
            .map(|(index, track)| (track.id, index))
            .collect();
        let song_channels =
            crate::mixer_flow::ordered_channels(&self.project, crate::mixer_flow::MixerOrder::Song);
        let channels = if order == crate::mixer_flow::MixerOrder::Song {
            song_channels.clone()
        } else {
            crate::mixer_flow::ordered_channels(&self.project, order)
        };
        let shift_targets = crate::mixer_flow::mixer_shift_targets(&self.project);
        let channel_indices: Vec<_> = channels
            .iter()
            .filter_map(|id| channel_indices_by_id.get(id).copied())
            .collect();
        // Channel numbers identify the song's channels, independently of the
        // current mixer presentation and its visible strip positions.
        let channel_numbers: std::collections::HashMap<_, _> = song_channels
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index + 1))
            .collect();
        let output_owners: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .filter_map(|track| {
                track
                    .instrument_output
                    .as_ref()
                    .map(|output| (track.id, output.track))
            })
            .collect();
        let instrument_groups: std::collections::HashSet<_> =
            output_owners.values().copied().collect();
        let mut channel_groups: Vec<(Option<u64>, bool, Vec<u64>)> = Vec::new();
        for index in &channel_indices {
            let track = &self.project.tracks[*index];
            let bus = track.kind == TrackKind::Bus;
            let owner = output_owners
                .get(&track.id)
                .copied()
                .or_else(|| instrument_groups.contains(&track.id).then_some(track.id));
            if let Some((_, _, ids)) =
                channel_groups
                    .last_mut()
                    .filter(|(previous_owner, previous_bus, _)| {
                        *previous_owner == owner && (owner.is_some() || *previous_bus == bus)
                    })
            {
                ids.push(track.id);
            } else {
                channel_groups.push((owner, bus, vec![track.id]));
            }
        }
        let channel_group_ids: std::collections::HashMap<_, _> = channel_groups
            .iter()
            .enumerate()
            .flat_map(|(index, (_, _, ids))| ids.iter().map(move |id| (*id, index)))
            .collect();
        // Gather routing identities once, without cloning clip or plug-in state.
        let mut incoming: std::collections::HashMap<u64, Vec<(u64, Option<bool>)>> =
            std::collections::HashMap::new();
        for track in self
            .project
            .tracks
            .iter()
            .filter(|track| !matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
        {
            if let Some(target) = track.output_bus {
                incoming.entry(target).or_default().push((track.id, None));
            }
            for send in &track.sends {
                incoming
                    .entry(send.target_bus)
                    .or_default()
                    .push((track.id, Some(send.pre_fader)));
            }
        }
        let mut reveal_channel = self
            .mixer_reveal_channel
            .take()
            .or(ui
                .ctx()
                .data_mut(|data| data.remove_temp::<u64>(egui::Id::new("mixer_reveal_channel"))))
            .and_then(|id| crate::mixer_flow::channel_owner(&self.project, id));
        if let Some((direction, extend)) = daw_ui::selection_navigation_input(ui, None) {
            let direction = match direction {
                daw_ui::SelectionDirection::Up => daw_ui::SelectionDirection::Left,
                daw_ui::SelectionDirection::Down => daw_ui::SelectionDirection::Right,
                direction => direction,
            };
            let items: Vec<_> = channel_indices
                .iter()
                .enumerate()
                .map(|(position, index)| daw_ui::SelectionItem {
                    id: self.project.tracks[*index].id,
                    start: position as f64,
                    end: position as f64,
                    row: 0.0,
                })
                .collect();
            if let Some(track) = daw_ui::adjacent_selection_item(
                &items,
                self.selected_track
                    .and_then(|id| crate::mixer_flow::channel_owner(&self.project, id)),
                direction,
            ) {
                self.mixer_select_channel(track, extend);
                reveal_channel = Some(track);
            }
        }
        let focused_channel = self
            .selected_track
            .and_then(|id| crate::mixer_flow::channel_owner(&self.project, id));
        let mut selected_channels: std::collections::HashSet<_> =
            self.finishing_selected_channels().into_iter().collect();
        if selected_channels.is_empty()
            && let Some(owner) = focused_channel
        {
            selected_channels.insert(owner);
        }
        let group_gap = daw_ui::tokens::SPACE_LG;
        let channel_gap = daw_ui::tokens::SPACE_SM;
        let group_header_height = daw_ui::tokens::COMPACT_HEIGHT + daw_ui::tokens::SPACE_XS;
        let preview_id = egui::Id::new("mixer_flow_preview");
        let selection_before = self.selected_track;
        let generation = crate::spectrogram::waveform_generation(ui.ctx());
        let mut output_view = crate::mixer_flow::MixerOutputView::read(ui.ctx(), generation);
        output_view
            .expanded
            .retain(|id| output_owners.contains_key(id));
        if let Some(channel) = reveal_channel.filter(|id| output_owners.contains_key(id)) {
            output_view.expanded.insert(channel);
        }
        let channel_expanded: std::collections::HashMap<_, _> = channels
            .iter()
            .map(|id| {
                (
                    *id,
                    !output_owners.contains_key(id) || output_view.expanded.contains(id),
                )
            })
            .collect();
        let frame = ui.ctx().cumulative_frame_nr();
        let previous = ui
            .ctx()
            .data(|data| data.get_temp::<(u64, u64, Option<u64>, Option<u64>)>(preview_id));
        let selected = focused_channel.filter(|id| channels.contains(id));
        // Keep a hover while reading its route lane, but begin with the current
        // selection after another workspace has rendered in between.
        let mut flow_source = previous
            .filter(|(document, last_frame, selection, _)| {
                *document == generation
                    && frame <= last_frame.saturating_add(1)
                    && *selection == self.selected_track
            })
            .and_then(|(_, _, _, preview)| preview.filter(|id| channels.contains(id)))
            .or(selected)
            .or_else(|| channels.first().copied());
        let route_rows = self
            .project
            .tracks
            .iter()
            .map(|track| track.sends.len() + 1)
            .max()
            .unwrap_or(1)
            .clamp(1, 4);
        let master_peaks = self
            .engine
            .as_ref()
            .map(|engine| engine.channel_peaks(daw_core::MASTER_CHANNEL_ID))
            .unwrap_or([0.0; 2]);
        let strip_width = daw_ui::MIXER_CHANNEL_WIDTH;
        let group_count = channel_groups.len() + 1;
        let channel_widths: std::collections::HashMap<_, _> = channels
            .iter()
            .map(|id| {
                (
                    *id,
                    if channel_expanded[id] {
                        strip_width
                    } else {
                        daw_ui::MIXER_COLLAPSED_CHANNEL_WIDTH
                    },
                )
            })
            .collect();
        let mixer_width = channel_widths.values().sum::<f32>()
            + strip_width
            + (channels.len() + 1 - group_count) as f32 * channel_gap
            + (group_count - 1) as f32 * group_gap;
        let content_width = daw_ui::MIXER_CONTENT_WIDTH;
        let spacing = daw_ui::MIXER_ROW_GAP;
        let workspace_height = ui.ctx().content_rect().bottom()
            - daw_ui::tokens::BORDER_WIDTH
            - ui.available_rect_before_wrap().top();
        let compact = workspace_height < 740.0;
        let control = if compact {
            daw_ui::tokens::COMPACT_HEIGHT
        } else {
            daw_ui::tokens::CONTROL_HEIGHT
        };
        let section_gap = if compact {
            daw_ui::tokens::SPACE_SM
        } else {
            daw_ui::MIXER_SECTION_GAP
        };
        let role_height = if channel_indices
            .iter()
            .any(|index| self.project.tracks[*index].kind == TrackKind::Bus)
        {
            control
        } else {
            0.0
        };
        let pan_row_height = daw_ui::tokens::COMPACT_HEIGHT + spacing + control;
        let send_area_height = 2.0 * daw_ui::MIXER_SEND_ROW_HEIGHT + spacing;
        // The same identity band sits above every processing/fader row. Bus
        // names remain lower beneath their role icon, without changing gain
        // travel or meter calibration compared with ordinary channels.
        let footer_height = daw_ui::tokens::COMPACT_HEIGHT
            + spacing
            + section_gap
            + control
            + spacing
            + send_area_height
            + spacing
            + daw_ui::MIXER_SEND_ROW_HEIGHT;
        let mut fixed_height = 2.0 * daw_ui::MIXER_CHANNEL_MARGIN
            + 3.0 * control
            + spacing
            + spacing
            + role_height
            + section_gap
            + control
            + spacing
            + section_gap
            + pan_row_height
            + section_gap
            + control
            + spacing
            + footer_height;
        // Routing remains available in its own scrollable overview. A short
        // workspace uses shorter lanes; the complete channel Routing view and all
        // send controls retain their existing one-click entry points.
        let route_lane_height = if compact {
            daw_ui::tokens::COMPACT_HEIGHT
        } else {
            daw_ui::MIXER_ROUTE_LANE
        };
        let routing_overview_height = 2.0 * route_lane_height + daw_ui::tokens::SPACE_SM;
        let route_height = route_rows.min(2) as f32 * route_lane_height + daw_ui::tokens::SPACE_SM;
        let notice_reserve = daw_ui::tokens::CONTROL_HEIGHT + 2.0 * daw_ui::tokens::SPACE_SM;
        let device_rows = self
            .project
            .tracks
            .iter()
            .chain(std::iter::once(&self.project.master))
            .map(|track| track.inserts.len())
            .max()
            .unwrap_or(0)
            .min(3);
        let device_height = if device_rows == 0 {
            daw_ui::MIXER_SLOT_HEIGHT
        } else {
            let budget = workspace_height
                - routing_overview_height
                - notice_reserve
                - group_header_height
                - fixed_height
                - daw_ui::tokens::MIXER_FADER_MIN;
            let rows = (budget / (daw_ui::DEVICE_IDENTITY_HEIGHT + spacing))
                .floor()
                .max(1.0)
                .min(device_rows as f32);
            (rows * daw_ui::DEVICE_IDENTITY_HEIGHT + (rows - 1.0) * spacing)
                .max(daw_ui::MIXER_SLOT_HEIGHT)
        };
        fixed_height += device_height;
        let strip_height =
            (workspace_height - routing_overview_height - notice_reserve - group_header_height)
                .max(fixed_height + daw_ui::tokens::MIXER_FADER_MIN);
        // The `[scale][rail][gap][meter]` unit sits on the strip axis.
        let unit_offset = ((content_width - daw_ui::MIXER_FADER_UNIT_WIDTH) * 0.5)
            .max(0.0)
            .floor();
        let meter_offset = unit_offset + daw_ui::MIXER_FADER_WIDTH + daw_ui::MIXER_METER_GAP;
        let scroll = daw_ui::scroll_area(ui, "mixer_scroll", [true, true]);
        let panning = daw_ui::scroll_area_is_panning(ui, "mixer_scroll");
        let mut diagnostics = Vec::new();
        let mut route_diagnostics = Vec::new();
        let mut master_diagnostics = None;
        let mut master_details = None;
        let mut master_identity_rect = egui::Rect::NOTHING;
        let mut master_output = false;
        let mut fader_calibration: Option<egui::Rect> = None;
        let mut analysis_target = None;
        let mut group_diagnostics = Vec::new();
        let rect_json = |rect: egui::Rect| serde_json::json!({"min":[rect.left(),rect.top()],"max":[rect.right(),rect.bottom()]} );
        let scale_json = |scale: Option<daw_ui::SliderScaleGeometry>| {
            scale.map(|scale| {
                serde_json::json!({
                    "rail":rect_json(scale.rail),
                    "min_position":[scale.minimum.x,scale.minimum.y],
                    "max_position":[scale.maximum.x,scale.maximum.y],
                    "handle_center":[scale.handle.x,scale.handle.y],
                    "divisions":scale.divisions,"range":scale.range,"value":scale.value,
                    "ticks":scale.ticks.iter().map(|tick| serde_json::json!({"fraction":tick.fraction,"y":tick.center.y,"major":tick.major})).collect::<Vec<_>>(),
                })
            })
        };
        let master_inputs = self
            .project
            .tracks
            .iter()
            .filter(|track| {
                !matches!(track.kind, TrackKind::Folder | TrackKind::Automation)
                    && track.output_bus.is_none()
            })
            .count();
        scroll.auto_shrink([false, false]).show_viewport(ui, |ui, _viewport| {
            let reorder_rect = ui.clip_rect();
            reorder_viewport = Some(rect_json(reorder_rect));
            let scroll_delta = if reorder_enabled {
                daw_ui::track_reorder_scroll_delta(ui, reorder_rect, daw_ui::TrackReorderAxis::Horizontal)
            } else { Vec2::ZERO };
            let strip_size = Vec2::new(strip_width, strip_height);
            let mut anchors = std::collections::HashMap::new();
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(mixer_width);
            let (headings, _) = ui.allocate_exact_size(Vec2::new(mixer_width, daw_ui::tokens::COMPACT_HEIGHT), egui::Sense::hover());
            let mut group_x = headings.left();
            for (owner, bus, ids) in &channel_groups {
                let label = owner.and_then(|id| names.get(&id).map(String::as_str)).unwrap_or(if *bus { "Buses" } else { "Tracks" });
                let width = ids.iter().map(|id| channel_widths[id]).sum::<f32>() + ids.len().saturating_sub(1) as f32 * channel_gap;
                let rect = egui::Rect::from_min_size(egui::pos2(group_x, headings.top()), Vec2::new(width, daw_ui::tokens::COMPACT_HEIGHT));
                let expanded = ids.iter().filter(|id| output_owners.contains_key(id)).all(|id| channel_expanded[id]);
                if let Some(owner) = owner {
                    let response = ui.push_id(("instrument_outputs", owner), |ui| daw_ui::mixer_output_group_header(ui, rect, label, ids.len(), expanded)).inner;
                    if response.clicked() { for id in ids.iter().filter(|id| output_owners.contains_key(id)) {
                        if expanded { output_view.expanded.remove(id); } else { output_view.expanded.insert(*id); }
                    } }
                } else { daw_ui::paint_mixer_group_header(ui.painter(), rect, label); }
                group_diagnostics.push(serde_json::json!({"label":label,"rect":rect_json(rect),"instrument":owner,"expanded":expanded,"channels":ids}));
                group_x = rect.right() + group_gap;
            }
            let output_rect = egui::Rect::from_min_size(egui::pos2(group_x, headings.top()), Vec2::new(strip_width, daw_ui::tokens::COMPACT_HEIGHT));
            daw_ui::paint_mixer_group_header(ui.painter(), output_rect, "Output");
            group_diagnostics.push(serde_json::json!({"label":"Output","rect":rect_json(output_rect)}));
            ui.add_space(daw_ui::tokens::SPACE_XS);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut previous_group = None;
                for index in channel_indices.iter().copied() {
                    let track = &mut self.project.tracks[index];
                    let bus = track.kind == TrackKind::Bus;
                    let input_target = AutomationTarget { track: track.id, parameter: AutomationParameter::TrackInputGain };
                    let gain_target = AutomationTarget { track: track.id, parameter: AutomationParameter::TrackGain };
                    let pan_target = AutomationTarget { track: track.id, parameter: AutomationParameter::TrackPan };
                    let gain_automation = automation.get(&gain_target).copied();
                    let input_automation = automation.get(&input_target).copied();
                    let pan_automation = automation.get(&pan_target).copied();
                    let mut channel_automation: Vec<_> = automation.iter().filter(|(target, _)| target.track == track.id).collect();
                    channel_automation.sort_by_key(|(_, state)| state.0);
                    let channel_state = channel_automation.iter().map(|(_, (_, state, _))| *state)
                        .max_by_key(|state| match state { daw_ui::AutomationState::Bypassed => 0, daw_ui::AutomationState::Read => 1, daw_ui::AutomationState::Overridden => 2 });
                    let group = channel_group_ids[&track.id];
                    if let Some(previous) = previous_group {
                        ui.add_space(if previous != group { group_gap } else { channel_gap });
                    }
                    previous_group = Some(group);
                    let destination = track.output_bus.and_then(|id| names.get(&id))
                        .map(String::as_str).unwrap_or("Master");
                    let channel_peaks = self.engine.as_ref().map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                    let channel_peak_hold = mixer_peak_hold(ui.ctx(), self.engine.as_ref(), generation, Some(track.id));
                    if !channel_expanded[&track.id] {
                        let width = daw_ui::MIXER_COLLAPSED_CHANNEL_WIDTH;
                        let inner_width = width - 2.0 * daw_ui::MIXER_CHANNEL_MARGIN;
                        let folded = daw_ui::mixer_strip(ui, track.id, Vec2::new(width, strip_height),
                            Some(color(track.color)), selected_channels.contains(&track.id), |ui| {
                            daw_ui::mixer_density_style(ui, compact);
                            ui.spacing_mut().item_spacing.y = spacing;
                            let name = daw_ui::mixer_folded_output_name(ui, &track.name, inner_width, color(track.color));
                            if name.clicked() { channel_target = Some(track.id); }
                            name.context_menu(|ui| {
                                let (previous,next)=shift_targets[&track.id];
                                let actions = daw_ui::track_reorder_menu(ui,daw_ui::TrackReorderAxis::Horizontal,
                                    reorder_enabled&&previous.is_some(),reorder_enabled&&next.is_some());
                                if let Some(direction)=actions.direction { track_reorder=if direction { next } else { previous }; ui.close(); }
                                if !reorder_enabled && daw_ui::button(ui,"Use Song order").clicked() { use_song_order=true; ui.close(); }
                                ui.separator();
                                if daw_ui::button(ui, "Expand output").clicked() { output_view.expanded.insert(track.id); ui.close(); }
                                if daw_ui::button(ui, "Show in Arranger").clicked() { arranger_target = Some(track.id); ui.close(); }
                                if daw_ui::button(ui, "Effects").clicked() { self.selected_track = Some(track.id); popup = Some(PopupKind::Effects); ui.close(); }
                                if daw_ui::button(ui, "Routing").clicked() { self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); ui.close(); }
                            });
                            let disclosure = daw_ui::mixer_output_disclosure(ui, &track.name, false, inner_width);
                            if disclosure.clicked() { output_view.expanded.insert(track.id); }
                            let source_label = track.instrument_output.map(|output| format!("B{}.{}",output.bus+1,output.first_channel+1)).unwrap_or_else(|| "Output".into());
                            let source = daw_ui::hover_text(daw_ui::mixer_source_caption(ui, &source_label, inner_width),
                                track.instrument_output.map(|output| format!("Native instrument bus {} · starting at channel {}",output.bus+1,output.first_channel+1)).unwrap_or_else(|| "Instrument output".into()));
                            let calibration = fader_calibration.unwrap_or_else(|| egui::Rect::from_min_size(
                                egui::pos2(ui.cursor().left(), ui.cursor().top() + 2.0 * control), Vec2::new(daw_ui::tokens::MIXER_METER_WIDTH, daw_ui::tokens::MIXER_FADER_MIN)));
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space((calibration.top() - control - spacing - ui.cursor().top()).max(0.0));
                            let (readout_row, _) = ui.allocate_exact_size(Vec2::new(inner_width, control), egui::Sense::hover());
                            let meter_id = egui::Id::new(("mixer_channel_meter", track.id));
                            let readout = daw_ui::channel_peak_indicator(ui, meter_id, channel_peak_hold, readout_row.center().x, readout_row);
                            ui.add_space(spacing);
                            let level = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space((inner_width - daw_ui::tokens::MIXER_METER_WIDTH) * 0.5);
                                daw_ui::channel_meter_with_peak_hold(ui, meter_id, channel_peaks, channel_peak_hold, calibration.height(), daw_ui::MIXER_FADER_INSET)
                            }).inner;
                            if readout.clicked() || level.clicked() { reset_mixer_peak_hold(ui.ctx(), self.engine.as_ref(), generation, Some(track.id)); }
                            ui.spacing_mut().item_spacing.y = spacing;
                            let rms = self.engine.as_ref().map(|engine| engine.channel_rms(track.id)).unwrap_or([0.0; 2]);
                            let statistics = daw_ui::channel_meter_statistics(ui, inner_width, rms);
                            let switches = daw_ui::mixer_folded_mute_solo(ui, &mut track.mute, &mut track.solo, inner_width);
                            changed |= switches.mute.changed() || switches.solo.changed();
                            let output = daw_ui::mixer_folded_output_route(ui, inner_width, destination);
                            output_menus.push((track.id, output.clone()));
                            serde_json::json!({"track":track.id,"selected":selected_channels.contains(&track.id),"channel":channel_numbers[&track.id],"bus":true,"expanded":false,"instrument_group":output_owners.get(&track.id),
                                "name":rect_json(name.rect),"disclosure":rect_json(disclosure.rect),"source":rect_json(source.rect),"peaks":channel_peaks,"peak_hold":channel_peak_hold,
                                "peak_clipped":channel_peak_hold.iter().any(|peak| *peak >= 1.0),"meter":rect_json(level.rect),"meter_readout":rect_json(readout.rect),"statistics":rect_json(statistics.rect),"rms":rms,
                                "mute":rect_json(switches.mute.rect),"solo":rect_json(switches.solo.rect),"target":rect_json(output.rect),"role":"Instrument output"})
                        });
                        anchors.insert(Some(track.id), folded.response.rect.center_bottom());
                        let mut details = folded.inner;
                        details["strip"] = rect_json(folded.response.rect);
                        diagnostics.push(details);
                        if ui.is_enabled() && !panning && !egui::Popup::is_any_open(ui.ctx())
                            && ui.input(|input| input.pointer.hover_pos()).is_some_and(|position| folded.response.rect.contains(position)) {
                            flow_source = Some(track.id);
                        }
                        continue;
                    }
                    let role = bus.then(|| {
                        if track.instrument_output.is_some() { daw_ui::MixerChannelRole::InstrumentOutput }
                        else if incoming.get(&track.id).is_some_and(|routes| routes.iter().any(|(_, send)| send.is_some())) {
                            daw_ui::MixerChannelRole::SendReturn
                        } else { daw_ui::MixerChannelRole::Bus }
                    });
                    let mut role_heading = None;
                    let mut output_disclosure = None;
                    let mut identity_rect = egui::Rect::NOTHING;
                    let strip = ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        daw_ui::mixer_density_style(ui, compact);
                        let offset = if let Some(role) = role {
                            let heading = if track.instrument_output.is_some() {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    let heading = daw_ui::mixer_role_heading(ui, role, strip_width - control, color(track.color));
                                    let disclosure = daw_ui::mixer_output_disclosure(ui, &track.name, true, control);
                                    output_disclosure = Some(disclosure.rect);
                                    if disclosure.clicked() { output_view.expanded.remove(&track.id); }
                                    heading.union(disclosure)
                                }).inner
                            } else { daw_ui::mixer_role_heading(ui, role, strip_width, color(track.color)) };
                            role_heading = Some(heading.rect);
                            role_height
                        } else { 0.0 };
                        let channel_size = Vec2::new(strip_width, strip_height - offset);
                        daw_ui::mixer_strip(ui, track.id, channel_size,
                        Some(color(track.color)), selected_channels.contains(&track.id), |ui| {
                        ui.spacing_mut().item_spacing.y = spacing;
                        // Identity
                        let (name, grip) = daw_ui::mixer_name_plate_with_grip(ui, if bus { daw_ui::MixerNamePlateKind::Bus } else { daw_ui::MixerNamePlateKind::Track },
                            &track.name, content_width, color(track.color), track.id, reorder_enabled);
                        let name = daw_ui::hover_text(name, format!("Open channel · {} · {} · Right-click for channel and selection actions · Shift-click to add/remove", track.name, if reorder_enabled { "Drag to reorder" } else { "Signal flow order; choose Song order to drag" }));
                        let name = if reorder_enabled { daw_ui::track_reorder_source(ui, &name, track.id) } else { name };
                        name.context_menu(|ui| {
                            let (previous, next) = shift_targets[&track.id];
                            let actions = daw_ui::track_reorder_menu(ui, daw_ui::TrackReorderAxis::Horizontal,
                                reorder_enabled && previous.is_some(), reorder_enabled && next.is_some());
                            let switch = if !reorder_enabled {
                                let switch = daw_ui::button(ui, "Use Song order");
                                if switch.clicked() { use_song_order = true; ui.close(); }
                                Some(rect_json(switch.rect))
                            } else { None };
                            reorder_menu = Some(serde_json::json!({"track":track.id,
                                "previous":{"rect":rect_json(actions.previous.rect),"enabled":actions.previous.enabled()},
                                "next":{"rect":rect_json(actions.next.rect),"enabled":actions.next.enabled()},
                                "use_song_order":switch}));
                            if let Some(direction) = actions.direction {
                                track_reorder = if direction { next } else { previous };
                                ui.close();
                            }
                            ui.separator();
                            if daw_ui::button(ui, "Show in Arranger").clicked() {
                                arranger_target = Some(track.id); ui.close();
                            }
                            if daw_ui::button(ui, "Open channel").clicked() {
                                channel_target = Some(track.id); ui.close();
                            }
                            if track.kind == TrackKind::Midi && daw_ui::button(ui, "Instrument").clicked() {
                                self.selected_track = Some(track.id); popup = Some(PopupKind::Instrument); ui.close();
                            }
                            if daw_ui::button(ui, "Effects").clicked() {
                                self.selected_track = Some(track.id); popup = Some(PopupKind::Effects); ui.close();
                            }
                            if daw_ui::button(ui, "Routing").clicked() {
                                self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); ui.close();
                            }
                            ui.separator();
                            if daw_ui::button(ui, "Rename…").clicked() {
                                rename_track = Some(track.id);
                                ui.close();
                            }
                            if let Some(choice) = daw_ui::channel_selection_choices(ui, selected_channels.contains(&track.id)) {
                                channel_selection = Some((track.id, choice));
                            }
                        });
                        if name.clicked() {
                            if ui.input(|input| input.modifiers.shift) { toggle_selection = Some(track.id); }
                            else { channel_target = Some(track.id); }
                        }
                        // Source: the kind and the actual input or instrument.
                        let source_kind = if track.instrument_output.is_some() { "Output" } else { match track.kind {
                            TrackKind::Audio => "Audio", TrackKind::Midi => "MIDI", _ => "Bus",
                        } };
                        let source = ui.allocate_ui_with_layout(Vec2::new(content_width, control),
                            egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
                            let menu = daw_ui::automation_channel_menu_named(ui, channel_state, &track.name, |ui| {
                                for (title, control, state) in [
                                    ("Input gain", daw_ui::AutomationControl::InputGain { track: track.id }, input_automation),
                                    ("Volume", daw_ui::AutomationControl::Gain { track: track.id }, gain_automation),
                                    ("Pan", daw_ui::AutomationControl::Pan { track: track.id }, pan_automation),
                                ] {
                                    daw_ui::action_menu(ui, title, |ui| {
                                        if let Some(action) = daw_ui::automatable_menu_contents(ui, control, state.map(|(lane, state, _)| (lane, state))) {
                                            automation_navigation = Some(action);
                                        }
                                    });
                                }
                                for (target, (lane, state, _)) in &channel_automation {
                                    if matches!(target.parameter, AutomationParameter::TrackInputGain | AutomationParameter::TrackGain | AutomationParameter::TrackPan) { continue; }
                                    let title = match target.parameter {
                                        AutomationParameter::SendGain { bus } => format!("Send · {}", names.get(&bus).map(String::as_str).unwrap_or("Bus")),
                                        AutomationParameter::VstParameter { effect, id } => format!("{} · Parameter {id}", effect.and_then(|effect| track.inserts.iter().find(|slot| slot.id == effect)).map(|slot| slot.name.as_str()).unwrap_or("Instrument")),
                                        parameter => parameter.name().to_owned(),
                                    };
                                    ui.push_id(lane, |ui| { daw_ui::action_menu(ui, title, |ui| {
                                        if let Some(action) = daw_ui::automation_menu_contents(ui, *lane, *state) {
                                            automation_navigation = Some(action);
                                        }
                                    }); });
                                }
                            });
                            let width = ui.available_width();
                            let kind = ui.allocate_ui_with_layout(Vec2::new(width, control), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                if let Some(output) = &track.instrument_output {
                                    let owner = names.get(&output.track).map(String::as_str).unwrap_or("Instrument");
                                    daw_ui::mixer_source_caption(ui, &format!("From {owner}"), width)
                                        .on_hover_text(format!("Independent stereo or mono instrument output\n{owner} · Bus {} · Starting at channel {}", output.bus + 1, output.first_channel + 1))
                                } else if bus {
                                    daw_ui::mixer_bus_input_summary(ui, width,
                                        incoming.get(&track.id).map(Vec::as_slice).unwrap_or(&[]).iter()
                                            .map(|(id, pre)| (names.get(id).map(String::as_str).unwrap_or("Unavailable channel"), *pre)))
                                } else if let Some(model) = crate::library::instrument_model(&self.vst_catalog, &track.instrument) {
                                    let instrument = if track.instrument_name.is_empty() { model.split(" · ").last().unwrap_or(&model) } else { &track.instrument_name };
                                    let response = daw_ui::mixer_row(ui, width, control, &format!("MIDI · {instrument}"),
                                        daw_ui::MixerRowText::Caption, None, None)
                                        .on_hover_text(format!("Instrument · {model}\nInstrument output, before channel inserts · Click to open"));
                                    if response.clicked() { device_target = Some((track.id, None)); }
                                    device_diagnostics.push(serde_json::json!({"track":track.id,"effect":null,"name":instrument,"model":model,"rect":rect_json(response.rect)}));
                                    response
                                } else {
                                    let text = match (track.kind, track.audio_input) {
                                        (TrackKind::Midi, _) => "MIDI · None".to_owned(),
                                        (_, Some(AudioInputChannels::Mono { channel })) => format!("Audio · In {}", channel + 1),
                                        (_, Some(AudioInputChannels::Stereo { left, right })) => format!("Audio · In {}/{}", left + 1, right + 1),
                                        (_, None) => "Audio".to_owned(),
                                    };
                                    daw_ui::mixer_source_caption(ui, &text, width).on_hover_text(match track.kind {
                                        TrackKind::Audio => "Audio clips and monitored input, including their source processing",
                                        _ => "No instrument · choose one in the channel's Instrument window",
                                    })
                                }
                            }).inner;
                            (menu.response.rect, kind.rect)
                        });
                        identity_rect = name.rect.union(source.response.rect);
                        if !bus { ui.add_space(role_height); }
                        ui.add_space(section_gap - spacing);
                        let mut input_gain = input_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackInputGain.denormalize(value)).unwrap_or(track.input_gain);
                        let input_gain_response = daw_ui::mixer_input_gain(ui, &mut input_gain, content_width, &format!("{} · Input gain", track.name));
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &input_gain_response,
                            daw_ui::AutomationControl::InputGain { track: track.id }, input_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(daw_ui::channel_gain_to_db(input_gain)), -60.0..=f64::from(daw_ui::channel_gain_to_db(16.0)), "dB", &format!("{} · Input gain", track.name));
                        if let Some(action) = action { automation_navigation = Some(action); }
                        if let Some(value) = exact { input_gain = daw_ui::channel_gain_from_db(value as f32).min(16.0); }
                        if (input_gain_response.changed() || exact.is_some()) && !panning {
                            track.input_gain = input_gain; changed = true;
                            automation_overrides.push((input_target, AutomationParameter::TrackInputGain.normalize(input_gain)));
                        }
                        let processing = daw_ui::mixer_processing_row(ui, content_width,
                            track.inserts.len(), track.inserts.iter().filter(|slot| slot.enabled).count());
                        let analyze = processing.analyze;
                        let effects = processing.effects;
                        if analyze.clicked() { analysis_target = Some(daw_engine::SignalMonitorTarget::Channel(track.id)); }
                        if effects.clicked() {
                            self.selected_track = Some(track.id); popup = Some(PopupKind::Effects);
                        }
                        effects.context_menu(|ui| {
                            if daw_ui::button(ui, "+ Insert").clicked() { insert_target = Some(track.id); ui.close(); }
                        });
                        let insert_summary = ui.allocate_ui_with_layout(Vec2::new(content_width, device_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_size(Vec2::new(content_width, device_height));
                            if track.inserts.is_empty() {
                                if daw_ui::mixer_insert_slot(ui, content_width).clicked() {
                                    insert_target = Some(track.id);
                                }
                            } else {
                                egui::ScrollArea::vertical().id_salt(("mixer_devices", track.id)).min_scrolled_height(0.0).max_height(device_height).auto_shrink([false, false]).show(ui, |ui| {
                                    for slot in &track.inserts {
                                        let model = crate::library::effect_model(&self.vst_catalog, &slot.effect);
                                        let response = daw_ui::device_identity_button(ui, egui::Id::new(("mixer_insert", track.id, slot.id)), content_width, daw_ui::DeviceIdentity { name: &slot.name, model: &model, preview: None, enabled: slot.enabled });
                                        if response.clicked() { device_target = Some((track.id, Some(slot.id))); }
                                        device_diagnostics.push(serde_json::json!({"track":track.id,"effect":slot.id,"name":slot.name,"model":model,"rect":rect_json(response.rect)}));
                                    }
                                });
                            }
                        }).response;
                        // Volume & pan: pan with mute/solo, then the fader unit and its readouts.
                        ui.add_space(section_gap - spacing);
                        let mut pan = pan_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value)).unwrap_or(track.pan);
                        let pan_row = daw_ui::mixer_pan_row(ui, &mut pan, &mut track.mute, &mut track.solo, content_width);
                        changed |= pan_row.mute.changed() || pan_row.solo.changed();
                        let pan_response = pan_row.pan;
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &pan_response,
                            daw_ui::AutomationControl::Pan { track: track.id }, pan_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(pan) * 100.0, -100.0..=100.0, "%", &format!("{} · Pan", track.name));
                        if let Some(action) = action {
                            automation_navigation = Some(action);
                        }
                        if let Some(value) = exact { pan = (value / 100.0) as f32; }
                        if (pan_response.changed() || exact.is_some()) && !panning { track.pan = pan; changed = true;
                            automation_overrides.push((pan_target, AutomationParameter::TrackPan.normalize(pan))); }
                        let mute_solo_rect = pan_row.mute.rect.union(pan_row.solo.rect);
                        let meter_id = egui::Id::new(("mixer_channel_meter", track.id));
                        ui.add_space(section_gap - spacing);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let displayed_gain = gain_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value)).unwrap_or(track.gain);
                        // The gain readout over the rail, the held peak over the meter.
                        let (readout_row, _) = ui.allocate_exact_size(Vec2::new(content_width, control), egui::Sense::hover());
                        let mut db = daw_ui::channel_gain_to_db(displayed_gain);
                        let gain = ui.put(egui::Rect::from_min_size(readout_row.min, Vec2::new(meter_offset - daw_ui::tokens::SPACE_SM, control)),
                            daw_ui::channel_gain_number(&mut db))
                            .on_hover_text("Channel volume · drag or double-click to type");
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &gain,
                            daw_ui::AutomationControl::Gain { track: track.id }, gain_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(db), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB", &format!("{} · Volume", track.name));
                        if let Some(action) = action {
                            automation_navigation = Some(action);
                        }
                        if let Some(value) = exact { db = value as f32; }
                        if (gain.changed() || exact.is_some()) && !panning {
                            track.gain = daw_ui::channel_gain_from_db(db); changed = true;
                            automation_overrides.push((gain_target, AutomationParameter::TrackGain.normalize(track.gain)));
                        }
                        let readout = daw_ui::channel_peak_indicator(ui, meter_id, channel_peak_hold,
                            readout_row.left() + meter_offset + daw_ui::tokens::MIXER_METER_WIDTH * 0.5, readout_row);
                        ui.add_space(spacing);
                        let fader_height = (ui.max_rect().bottom() - ui.cursor().min.y - footer_height).max(daw_ui::tokens::MIXER_FADER_MIN);
                        let (fader_rect, fader_scale, level) = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(unit_offset);
                            let mut gain = displayed_gain;
                            let response = daw_ui::mixer_fader(ui, &mut gain, fader_height);
                            let response = daw_ui::hover_text(response, "Channel volume");
                            let scale = daw_ui::slider_scale_geometry(ui.ctx(), response.id);
                            let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &response,
                                daw_ui::AutomationControl::Gain { track: track.id }, gain_automation.map(|(lane, state, _)| (lane, state)),
                                f64::from(daw_ui::channel_gain_to_db(gain)), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB", &format!("{} · Volume", track.name));
                            if let Some(action) = action {
                                automation_navigation = Some(action);
                            }
                            if let Some(value) = exact { gain = daw_ui::channel_gain_from_db(value as f32); }
                            if (response.changed() || exact.is_some()) && !panning { track.gain = gain; changed = true;
                                automation_overrides.push((gain_target, AutomationParameter::TrackGain.normalize(gain))); }
                            ui.add_space(daw_ui::MIXER_METER_GAP);
                            let level = daw_ui::channel_meter_with_peak_hold(ui, meter_id, channel_peaks, channel_peak_hold, fader_height, daw_ui::MIXER_FADER_INSET);
                            (response.rect, scale, level)
                        }).inner;
                        if readout.clicked() || level.clicked() {
                            reset_mixer_peak_hold(ui.ctx(), self.engine.as_ref(), generation, Some(track.id));
                        }
                        fader_calibration.get_or_insert(level.rect);
                        ui.spacing_mut().item_spacing.y = spacing;
                        let rms = self.engine.as_ref().map(|engine| engine.channel_rms(track.id)).unwrap_or([0.0; 2]);
                        let statistics = daw_ui::channel_meter_statistics(ui, content_width, rms);
                        // Output & sends
                        ui.add_space(section_gap);
                        let target = daw_ui::mixer_output_button(ui, content_width, destination,
                            track.output_bus.and_then(|id| channel_colors.get(&id)).copied())
                            .on_hover_text(format!("Output → {destination}\nAfter inserts, volume and pan · Click to choose output"));
                        output_menus.push((track.id, target.clone()));
                        let send_details: Vec<_> = track.sends.iter().map(|send| daw_ui::MixerSend {
                            destination: names.get(&send.target_bus).map(String::as_str).unwrap_or("Bus"),
                            color: channel_colors.get(&send.target_bus).copied().unwrap_or(palette(ui.ctx()).accent),
                            gain: automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::SendGain { bus: send.target_bus } })
                                .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::SendGain { bus: send.target_bus }.denormalize(*value)).unwrap_or(send.gain),
                            pre_fader: send.pre_fader,
                        }).collect();
                        let mut send_rows = Vec::new();
                        let mut send_automation = Vec::new();
                        let send_area = ui.allocate_ui_with_layout(Vec2::new(content_width, send_area_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_size(Vec2::new(content_width, send_area_height));
                            ui.spacing_mut().item_spacing.y = spacing;
                            let visible = if send_details.len() > 2 { 1 } else { send_details.len() };
                            let mut open = false;
                            for (index, send) in send_details[..visible].iter().enumerate() {
                                let row = daw_ui::mixer_send_row(ui, content_width, send);
                                let bus = track.sends[index].target_bus;
                                let state = automation.get(&AutomationTarget { track: track.id,
                                    parameter: AutomationParameter::SendGain { bus } }).copied();
                                if let Some(action) = daw_ui::automatable_parameter_context(ui, &row,
                                    daw_ui::AutomationControl::Send { track: track.id, bus },
                                    state.map(|(lane, status, _)| (lane, status))) {
                                    automation_navigation = Some(action);
                                }
                                send_automation.push(serde_json::json!({"bus":bus,"rect":rect_json(row.rect),
                                    "lane":state.map(|(lane, _, _)| lane),
                                    "state":state.map(|(_, status, _)| daw_ui::automation_state_label(status))}));
                                open |= row.clicked();
                                send_rows.push(rect_json(row.rect));
                            }
                            if send_details.len() > visible {
                                open |= daw_ui::mixer_more_sends(ui, content_width, send_details.len() - visible).clicked();
                            }
                            open
                        });
                        if send_area.inner { self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); }
                        let add_send = daw_ui::mixer_add_send_button(ui, content_width);
                        send_menus.push((track.id, add_send.clone()));
                        let sends_rect = send_area.response.rect.union(add_send.rect);
                        let level_rect = pan_response.rect.union(mute_solo_rect).union(gain.rect).union(fader_rect).union(level.rect);
                        let sections = [name.rect.union(source.response.rect), effects.rect.union(insert_summary.rect), level_rect, target.rect.union(sends_rect)];
                        let section_diagnostics: Vec<_> = daw_ui::MixerSection::ALL.into_iter().zip(sections).map(|(section, bounds)| serde_json::json!({"name":section.name(),"rect":rect_json(bounds)})).collect();
                        let mut details = serde_json::json!({"track":track.id,"selected":selected_channels.contains(&track.id),"channel":channel_numbers[&track.id],"bus":bus,"peaks":channel_peaks,"name":rect_json(name.rect),"reorder_grip":rect_json(grip.rect),"reorder_source":reorder_enabled.then(|| rect_json(name.rect)),"source":rect_json(source.response.rect),
                            "source_kind":source_kind,"source_menu":rect_json(source.inner.0),"source_label":rect_json(source.inner.1),
                            "automation_state":channel_state.map(daw_ui::automation_state_label),
                            "effects":rect_json(effects.rect),"insert_summary":rect_json(insert_summary.rect),
                            "incoming_count":incoming.get(&track.id).map_or(0, Vec::len),
                            "gain":rect_json(gain.rect),"fader":rect_json(fader_rect),"fader_scale":scale_json(fader_scale),
                            "pan":rect_json(pan_response.rect),"pan_scale":scale_json(daw_ui::slider_scale_geometry(ui.ctx(), pan_response.id)),
                            "mute_solo":rect_json(mute_solo_rect),"mute":rect_json(pan_row.mute.rect),"solo":rect_json(pan_row.solo.rect),
                            "meter":rect_json(level.rect),"meter_readout":rect_json(readout.rect),"peak_hold":channel_peak_hold,"peak_clipped":channel_peak_hold.iter().any(|peak| *peak >= 1.0),"target":rect_json(target.rect),"sends":rect_json(sends_rect),
                            "send_rows":send_rows,"send_automation":send_automation,"add_send":rect_json(add_send.rect),
                            "analyze":rect_json(analyze.rect),"sections":section_diagnostics,
                            "stages":{"input":rect_json(source.response.rect),"processing":rect_json(effects.rect.union(insert_summary.rect)),
                                "measuring":rect_json(level.rect),"output":rect_json(target.rect.union(sends_rect))}});
                        details["input_gain"] = rect_json(input_gain_response.rect);
                        details["input_gain_value"] = serde_json::json!(input_gain);
                        details["statistics"] = rect_json(statistics.rect);
                        details["rms"] = serde_json::json!(rms);
                        details
                    })
                    }).inner;
                    let drop = if reorder_enabled {
                        daw_ui::track_reorder_target(ui, strip.response.rect, track.id,
                            daw_ui::TrackReorderAxis::Horizontal, false, |intent| intent.source != intent.target
                                && output_owners.get(&intent.source)==output_owners.get(&intent.target))
                    } else { daw_ui::TrackDropResponse::default() };
                    if let Some(preview) = drop.preview {
                        let placement = if preview.intent.placement == daw_ui::TrackDropPlacement::Before { "Before" } else { "After" };
                        let parent = track.parent_folder.and_then(|id| names.get(&id)).map(String::as_str).unwrap_or("Top level");
                        daw_ui::track_reorder_preview_label(ui, &preview, &format!("{placement} {} · {parent}", track.name));
                        reorder_preview = Some(serde_json::json!({"source":preview.intent.source,"target":preview.intent.target,
                            "placement":format!("{:?}",preview.intent.placement),"parent":track.parent_folder,
                            "indicator":rect_json(preview.indicator)}));
                    }
                    if let Some(intent) = drop.dropped { track_reorder = Some(intent); }
                    anchors.insert(Some(track.id), strip.response.rect.center_bottom());
                    if reveal_channel == Some(track.id) {
                        // Oversized strips cannot fit a short viewport. Reveal
                        // the bounded identity/source band instead of centering
                        // the full card and clipping its clickable name above it.
                        // Navigation must make its mouse targets available on
                        // arrival, rather than leaving them offscreen mid-animation.
                        let identity = role_heading.map_or(identity_rect, |heading| heading.union(identity_rect));
                        ui.scroll_to_rect_animation(identity.expand(daw_ui::MIXER_CHANNEL_MARGIN),
                            None, egui::style::ScrollAnimation::none());
                    }
                    if ui.is_enabled() && !panning && !egui::Popup::is_any_open(ui.ctx())
                        // A strip scrolled under the transport must not preview from there.
                        && ui.input(|input| input.pointer.hover_pos())
                        .is_some_and(|pos| strip.response.rect.intersect(reorder_rect).contains(pos))
                    { flow_source = Some(track.id); }
                    let mut channel_diagnostics = strip.inner;
                    channel_diagnostics["strip"] = rect_json(strip.response.rect);
                    channel_diagnostics["role"] = role.map(|role| serde_json::json!(role.label())).unwrap_or(serde_json::json!("Track"));
                    channel_diagnostics["role_heading"] = role_heading.map(rect_json).unwrap_or(serde_json::Value::Null);
                    channel_diagnostics["expanded"] = serde_json::json!(true);
                    channel_diagnostics["disclosure"] = output_disclosure.map(rect_json).into();
                    channel_diagnostics["instrument_group"] = output_owners.get(&track.id).or_else(|| instrument_groups.contains(&track.id).then_some(&track.id)).copied().into();
                    channel_diagnostics["drop"] = if reorder_enabled { rect_json(strip.response.rect) } else { serde_json::Value::Null };
                    diagnostics.push(channel_diagnostics);
                }
                if !channels.is_empty() { ui.add_space(group_gap); }
                // Master uses the same real signal controls and calibration as
                // every source/output channel, after the final graph sum.
                let master_track = &mut self.project.master;
                let master_id = daw_core::MASTER_CHANNEL_ID;
                let gain_target = AutomationTarget { track: master_id, parameter: AutomationParameter::TrackGain };
                let input_target = AutomationTarget { track: master_id, parameter: AutomationParameter::TrackInputGain };
                let pan_target = AutomationTarget { track: master_id, parameter: AutomationParameter::TrackPan };
                let gain_automation = automation.get(&gain_target).copied();
                let input_automation = automation.get(&input_target).copied();
                let pan_automation = automation.get(&pan_target).copied();
                let master = daw_ui::mixer_strip(ui, "master", strip_size, None, focused_channel == Some(master_id), |ui| {
                        daw_ui::mixer_density_style(ui, compact);
                        ui.spacing_mut().item_spacing.y = spacing;
                        let name = daw_ui::mixer_name_plate(ui, daw_ui::MixerNamePlateKind::Master, "Master", content_width, palette(ui.ctx()).raised);
                        if name.clicked() { channel_target = Some(master_id); }
                        name.context_menu(|ui| {
                            if daw_ui::button(ui, "Open channel").clicked() { channel_target = Some(master_id); ui.close(); }
                            if daw_ui::button(ui, "Effects").clicked() { self.selected_track = Some(master_id); popup = Some(PopupKind::Effects); ui.close(); }
                            if daw_ui::button(ui, "Analyze").clicked() { analysis_target = Some(daw_engine::SignalMonitorTarget::Master); ui.close(); }
                        });
                        let source = ui.allocate_ui_with_layout(Vec2::new(content_width, control), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
                            let channel_state = [input_automation, gain_automation, pan_automation].into_iter().flatten().map(|(_, state, _)| state)
                                .max_by_key(|state| match state { daw_ui::AutomationState::Bypassed => 0, daw_ui::AutomationState::Read => 1, daw_ui::AutomationState::Overridden => 2 });
                            let menu = daw_ui::automation_channel_menu_named(ui, channel_state, "Master", |ui| {
                                for (title, control, state) in [
                                    ("Input gain", daw_ui::AutomationControl::InputGain { track: master_id }, input_automation),
                                    ("Volume", daw_ui::AutomationControl::Gain { track: master_id }, gain_automation),
                                    ("Pan", daw_ui::AutomationControl::Pan { track: master_id }, pan_automation),
                                ] {
                                    daw_ui::action_menu(ui, title, |ui| {
                                        if let Some(action) = daw_ui::automatable_menu_contents(ui, control, state.map(|(lane, state, _)| (lane, state))) { automation_navigation = Some(action); }
                                    });
                                }
                            });
                            let caption = daw_ui::mixer_source_caption(ui, &format!("Sum · {master_inputs} in"), ui.available_width())
                                .on_hover_text("Final channel sum · before Master input gain and inserts");
                            (menu.response.rect, caption.rect)
                        });
                        master_identity_rect = name.rect.union(source.response.rect);
                        ui.add_space(role_height);
                        ui.add_space(section_gap - spacing);
                        let mut input_gain = input_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackInputGain.denormalize(value)).unwrap_or(master_track.input_gain);
                        let input = daw_ui::mixer_input_gain(ui, &mut input_gain, content_width, "Master · Input gain");
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &input,
                            daw_ui::AutomationControl::InputGain { track: master_id }, input_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(daw_ui::channel_gain_to_db(input_gain)), -60.0..=f64::from(daw_ui::channel_gain_to_db(16.0)), "dB", "Master · Input gain");
                        if let Some(action) = action { automation_navigation = Some(action); }
                        if let Some(value) = exact { input_gain = daw_ui::channel_gain_from_db(value as f32).min(16.0); }
                        if (input.changed() || exact.is_some()) && !panning {
                            master_track.input_gain = input_gain; changed = true;
                            automation_overrides.push((input_target, AutomationParameter::TrackInputGain.normalize(input_gain)));
                        }
                        let processing = daw_ui::mixer_processing_row(ui, content_width, master_track.inserts.len(), master_track.inserts.iter().filter(|slot| slot.enabled).count());
                        let effects = processing.effects;
                        let analyze = processing.analyze;
                        if analyze.clicked() { analysis_target = Some(daw_engine::SignalMonitorTarget::Master); }
                        if effects.clicked() { self.selected_track = Some(master_id); popup = Some(PopupKind::Effects); }
                        effects.context_menu(|ui| { if daw_ui::button(ui, "+ Insert").clicked() { insert_target = Some(master_id); ui.close(); } });
                        let insert_summary = ui.allocate_ui_with_layout(Vec2::new(content_width, device_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_size(Vec2::new(content_width, device_height));
                            if master_track.inserts.is_empty() {
                                if daw_ui::mixer_insert_slot(ui, content_width).clicked() { insert_target = Some(master_id); }
                            } else {
                                egui::ScrollArea::vertical().id_salt(("mixer_devices", master_id)).min_scrolled_height(0.0).max_height(device_height).auto_shrink([false, false]).show(ui, |ui| {
                                    for slot in &master_track.inserts {
                                        let model = crate::library::effect_model(&self.vst_catalog, &slot.effect);
                                        let response = daw_ui::device_identity_button(ui, egui::Id::new(("mixer_insert", master_id, slot.id)), content_width, daw_ui::DeviceIdentity { name: &slot.name, model: &model, preview: None, enabled: slot.enabled });
                                        if response.clicked() { device_target = Some((master_id, Some(slot.id))); }
                                        device_diagnostics.push(serde_json::json!({"track":master_id,"effect":slot.id,"name":slot.name,"model":model,"rect":rect_json(response.rect)}));
                                    }
                                });
                            }
                        }).response;
                        ui.add_space(section_gap - spacing);
                        let mut pan = pan_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value)).unwrap_or(master_track.pan);
                        let pan_row = daw_ui::mixer_master_pan_row(ui, &mut pan, &mut master_track.mute, content_width);
                        changed |= pan_row.mute.changed();
                        let pan_response = pan_row.pan;
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &pan_response,
                            daw_ui::AutomationControl::Pan { track: master_id }, pan_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(pan) * 100.0, -100.0..=100.0, "%", "Master · Pan");
                        if let Some(action) = action { automation_navigation = Some(action); }
                        if let Some(value) = exact { pan = (value / 100.0) as f32; }
                        if (pan_response.changed() || exact.is_some()) && !panning {
                            master_track.pan = pan; changed = true;
                            automation_overrides.push((pan_target, AutomationParameter::TrackPan.normalize(pan)));
                        }
                        let meter_id = egui::Id::new("mixer_master_meter");
                        let master_peak_hold = mixer_peak_hold(ui.ctx(), self.engine.as_ref(), generation, None);
                        ui.add_space(section_gap - spacing);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let displayed_gain = gain_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value)).unwrap_or(master_track.gain);
                        let (readout_row, _) = ui.allocate_exact_size(Vec2::new(content_width, control), egui::Sense::hover());
                        let mut db = daw_ui::channel_gain_to_db(displayed_gain);
                        let gain = ui.put(egui::Rect::from_min_size(readout_row.min, Vec2::new(meter_offset - daw_ui::tokens::SPACE_SM, control)), daw_ui::channel_gain_number(&mut db))
                            .on_hover_text("Master volume · after inserts · drag or double-click to type");
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &gain,
                            daw_ui::AutomationControl::Gain { track: master_id }, gain_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(db), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB", "Master · Volume");
                        if let Some(action) = action { automation_navigation = Some(action); }
                        if let Some(value) = exact { db = value as f32; }
                        if (gain.changed() || exact.is_some()) && !panning {
                            master_track.gain = daw_ui::channel_gain_from_db(db); changed = true;
                            automation_overrides.push((gain_target, AutomationParameter::TrackGain.normalize(master_track.gain)));
                        }
                        let readout = daw_ui::channel_peak_indicator(ui, meter_id, master_peak_hold,
                            readout_row.left() + meter_offset + daw_ui::tokens::MIXER_METER_WIDTH * 0.5, readout_row);
                        ui.add_space(spacing);
                        let fader_height = (ui.max_rect().bottom() - ui.cursor().min.y - footer_height).max(daw_ui::tokens::MIXER_FADER_MIN);
                        let (fader, level) = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(unit_offset);
                            let mut value = displayed_gain;
                            let fader = daw_ui::mixer_fader(ui, &mut value, fader_height);
                            let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &fader,
                                daw_ui::AutomationControl::Gain { track: master_id }, gain_automation.map(|(lane, state, _)| (lane, state)),
                                f64::from(daw_ui::channel_gain_to_db(value)), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB", "Master · Volume");
                            if let Some(action) = action { automation_navigation = Some(action); }
                            if let Some(value_edit) = exact { value = daw_ui::channel_gain_from_db(value_edit as f32); }
                            if (fader.changed() || exact.is_some()) && !panning {
                                master_track.gain = value; changed = true;
                                automation_overrides.push((gain_target, AutomationParameter::TrackGain.normalize(value)));
                            }
                            ui.add_space(daw_ui::MIXER_METER_GAP);
                            let level = daw_ui::hover_text(daw_ui::channel_meter_with_peak_hold(ui, meter_id, master_peaks, master_peak_hold, fader_height, daw_ui::MIXER_FADER_INSET),
                                "Master channel · after inserts, pan and volume\nBefore device clipping, dry input monitoring and the click");
                            (fader, level)
                        }).inner;
                        if readout.clicked() || level.clicked() { reset_mixer_peak_hold(ui.ctx(), self.engine.as_ref(), generation, None); }
                        ui.spacing_mut().item_spacing.y = spacing;
                        let rms = self.engine.as_ref().map(|engine| engine.channel_rms(master_id)).unwrap_or([0.0; 2]);
                        let statistics = daw_ui::channel_meter_statistics(ui, content_width, rms);
                        ui.add_space(section_gap);
                        let output = daw_ui::mixer_output_button(ui, content_width, "Audio output", Some(palette(ui.ctx()).muted))
                            .on_hover_text("Master → audio device output · Click to open Setup › Audio");
                        if output.clicked() { master_output = true; }
                        serde_json::json!({"track":master_id,"meter":rect_json(level.rect),"meter_readout":rect_json(readout.rect),"peak_hold":master_peak_hold,"peak_clipped":master_peak_hold.iter().any(|peak| *peak >= 1.0),"target":rect_json(output.rect),
                            "name":rect_json(name.rect),"source":rect_json(source.response.rect),"source_menu":rect_json(source.inner.0),"analyze":rect_json(analyze.rect),"effects":rect_json(effects.rect),"insert_summary":rect_json(insert_summary.rect),
                            "input_gain":rect_json(input.rect),"input_gain_value":input_gain,"pan":rect_json(pan_response.rect),"mute":rect_json(pan_row.mute.rect),"gain":rect_json(gain.rect),"fader":rect_json(fader.rect),"fader_scale":scale_json(daw_ui::slider_scale_geometry(ui.ctx(), fader.id)),"statistics":rect_json(statistics.rect),"rms":rms})
                });
                anchors.insert(None, master.response.rect.center_bottom());
                if reveal_channel == Some(daw_core::MASTER_CHANNEL_ID) {
                    ui.scroll_to_rect_animation(master_identity_rect.expand(daw_ui::MIXER_CHANNEL_MARGIN),
                        None, egui::style::ScrollAnimation::none());
                }
                master_details = Some(master.inner);
                master_diagnostics = Some(rect_json(master.response.rect));
            });
            if self.selected_track != selection_before {
                flow_source = self.selected_track.filter(|id| channels.contains(id));
            }
            let connection_clip = ui.clip_rect();
            daw_ui::scroll_area(ui, "mixer_route_lanes", [false, true])
                .max_width(mixer_width).max_height(route_height)
                // Nested panes otherwise shrink to the height remaining below
                // the strips. Reserve every visible lane in the outer pane;
                // the inner scroll still handles more than four routes.
                .min_scrolled_height(route_height)
                .auto_shrink([false, false]).show(ui, |ui| {
                ui.set_min_width(mixer_width);
                if let Some(track) = flow_source.and_then(|id| self.project.track(id)) {
                    let source_position = anchors.get(&Some(track.id)).copied().unwrap_or_default();
                    let routes = std::iter::once((track.output_bus, "Output"))
                        .chain(track.sends.iter().map(|send| (Some(send.target_bus), if send.pre_fader { "Pre send" } else { "Post send" })));
                    for (target, kind) in routes {
                        let target_position = anchors.get(&target).copied().unwrap_or(source_position);
                        let target_name = target.and_then(|id| names.get(&id)).map(String::as_str).unwrap_or("Master");
                        let caption = format!("{kind} → {target_name}");
                        let lane = daw_ui::mixer_route_lane_with_height(ui,
                            daw_ui::MixerRouteEndpoint { position: source_position, color: color(track.color) },
                            daw_ui::MixerRouteEndpoint { position: target_position,
                                color: target.and_then(|id| channel_colors.get(&id)).copied().unwrap_or(palette(ui.ctx()).accent) },
                            &caption, connection_clip, route_lane_height);
                        if let Some(painted) = lane.painted {
                            route_diagnostics.push(serde_json::json!({"source":track.id,"target":target,"kind":kind,
                                "lane":rect_json(painted.lane),"shaft":rect_json(painted.shaft),"head":rect_json(painted.head),
                                "from":[painted.from.x,painted.from.y],"to":[painted.to.x,painted.to.y],
                                "path":painted.path.map(|point| [point.x,point.y]),
                                "source_color":painted.source_color.to_array(),"target_color":painted.target_color.to_array(),
                                "stroke_width":painted.stroke_width,
                                "source_clipped":painted.source_clipped,"target_clipped":painted.target_clipped}));
                        }
                        let hint = match kind {
                                "Pre send" => "After inserts, before volume and pan",
                                "Post send" => "After inserts, volume and pan",
                                _ => "Main channel output, after volume and pan",
                            };
                        lane.response.on_hover_text(format!("{kind} · {} → {target_name}\n{hint}", track.name));
                    }
                }
            });
            // Submit to the outer channel pane after its nested routing scroll
            // area; otherwise that vertical-only area consumes the horizontal request.
            ui.scroll_with_delta(scroll_delta);
        });
        if master_output {
            self.setup_page = crate::app::SetupPage::Audio;
            self.open_popup(PopupKind::Setup);
        }
        output_view.store(ui.ctx());
        // Attach the shared destination choices to the existing strip button
        // after releasing the mutable track borrow. The captured source owns
        // this action even when another channel remains selected.
        for (source, response) in output_menus {
            self.mixer_output_menu(ui, source, &response);
        }
        for (source, response) in send_menus {
            self.mixer_send_menu(ui, source, &response);
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(preview_id, (generation, frame, self.selected_track, flow_source));
            data.insert_temp(egui::Id::new("mixer_diagnostics"),
                serde_json::json!({"order":order.label(),"compact":compact,"role_height":role_height,"channel_order":channels,"focused_channel":focused_channel,"reorder_enabled":reorder_enabled,"channels":diagnostics,"master_strip":master_diagnostics,"master":master_details,"master_peaks":master_peaks,"groups":group_diagnostics,"routes":route_diagnostics,"flow_source":flow_source,"route_height":route_height,"reorder_preview":reorder_preview,"reorder_viewport":reorder_viewport,"reorder_menu":reorder_menu}));
        });
        if use_song_order {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("mixer_order"),
                    crate::mixer_flow::MixerOrder::Song,
                )
            });
        }
        for (target, value) in automation_overrides {
            self.override_automated_parameter(target, value);
        }
        if changed {
            self.changed();
        }
        if let Some(intent) = track_reorder {
            self.apply_track_reorder(intent);
        }
        self.apply_automation_navigation(automation_navigation);
        if let Some(track) = toggle_selection {
            self.mixer_toggle_channel_selection(track);
        }
        if let Some((track, choice)) = channel_selection {
            self.mixer_channel_selection_action(track, choice);
        }
        if let Some(track) = rename_track {
            self.start_track_rename(ui.ctx(), track);
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
        if let Some(track) = arranger_target {
            self.show_channel_in_arranger(track);
        }
        if let Some(target) = analysis_target {
            self.open_signal_analysis(target);
        }
        if let Some(track) = channel_target {
            self.open_channel(track);
        }
        if let Some((track, effect)) = device_target {
            if let Some(effect) = effect {
                self.open_effect_editor(track, effect);
            } else {
                self.selected_track = Some(track);
                self.open_popup(PopupKind::Instrument);
            }
        }
        if let Some(track) = insert_target {
            self.selected_track = Some(track);
            self.open_effect_library(track, false);
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("mixer_device_diagnostics"),
                serde_json::json!(device_diagnostics),
            )
        });
    }

    pub fn instrument_editor(&mut self, ctx: &egui::Context) {
        self.plugin_instrument_editor(ctx);
    }
}
