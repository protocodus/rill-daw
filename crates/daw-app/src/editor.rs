use crate::{
    app::{DawApp, PopupKind},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Vec2};

impl DawApp {
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
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Select a clip in Arrangement")
                        .color(palette(ui.ctx()).muted),
                );
            });
            return;
        };
        if matches!(clip.content, ClipContent::Audio { .. }) {
            self.audio_editor(ui);
        } else {
            self.piano_roll(ui);
        }
        self.finish_clip_navigation(ui.ctx());
    }
    pub(crate) fn mixer_add_bus(&mut self) {
        self.begin_edit();
        let name = self.available_bus_name();
        self.selected_track = Some(self.project.add_bus(name));
        self.selected_clip = None;
        self.selected_notes.clear();
        self.edit_label = Some("Add bus".into());
        self.changed();
    }

    pub(crate) fn mixer_add_send_menu(&mut self, ui: &mut egui::Ui) {
        let source = self.selected_track.filter(|id| {
            self.project.track(*id).is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
        });
        let mut destination = None;
        ui.add_enabled_ui(source.is_some(), |ui| {
            daw_ui::action_menu(ui, "Add send…", |ui| {
                destination = source.and_then(|source| self.mixer_send_choices(ui, source));
            })
            .response
            .on_hover_text(source.and_then(|id| self.project.track(id)).map_or_else(
                || "Select a channel to add a send".to_owned(),
                |track| format!("Add a post-fader send from {}", track.name),
            ));
        });
        if let Some((source, destination)) = source.zip(destination) {
            self.mixer_create_send(source, destination);
        }
    }

    /// Both entry points name their source and require a destination choice.
    /// A channel-strip menu does not retarget the toolbar's selected channel.
    fn mixer_send_choices(&self, ui: &mut egui::Ui, source: u64) -> Option<Option<u64>> {
        daw_ui::command_menu_contents(ui, |ui| {
            let track = self.project.track(source)?;
            daw_ui::text::truncated(
                ui,
                daw_ui::text::TextRole::Label,
                format!("Send from {}", track.name),
            );
            ui.separator();
            let buses = self
                .project
                .eligible_output_buses(source)
                .unwrap_or_default();
            let mut destination = None;
            let mut available = false;
            for bus in buses
                .iter()
                .filter(|bus| !track.sends.iter().any(|send| send.target_bus == bus.id))
            {
                available = true;
                if daw_ui::button(ui, &bus.name).clicked() {
                    destination = Some(Some(bus.id));
                    ui.close();
                }
            }
            if !available {
                daw_ui::text::note(ui, "No eligible buses");
            }
            ui.separator();
            if daw_ui::button(ui, "New bus and send").clicked() {
                destination = Some(None);
                ui.close();
            }
            destination
        })
    }

    fn mixer_create_send(&mut self, source: u64, destination: Option<u64>) {
        self.begin_edit();
        let created_bus = destination.is_none();
        let bus = destination.unwrap_or_else(|| {
            let name = self.available_bus_name();
            self.project.add_bus(name)
        });
        let result = self.project.add_send(source, bus, 0.25, false);
        if result.is_ok() {
            self.edit_label = Some(
                if created_bus {
                    "Add bus send"
                } else {
                    "Add send"
                }
                .into(),
            );
            self.changed();
        } else if created_bus {
            self.project.delete_track(bus).ok();
        }
        self.message(result, "Send added");
    }

    pub(crate) fn available_bus_name(&self) -> String {
        for index in 1.. {
            let name = format!("Bus {index}");
            if !self.project.tracks.iter().any(|track| track.name == name) {
                return name;
            }
        }
        unreachable!()
    }

    pub(crate) fn mixer(&mut self, ui: &mut egui::Ui) {
        self.mixer_batch_routing_menu(ui);
        let mut changed = false;
        let mut popup = None;
        let mut channel_target = None;
        let mut toggle_selection = None;
        let mut channel_selection = None;
        let mut automation_navigation = None;
        let mut automation_overrides = Vec::new();
        let mut empty_send_menus = Vec::new();
        let mut track_reorder = None;
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
        let channel_indices: Vec<_> = self
            .project
            .ordered_track_ids()
            .into_iter()
            .filter_map(|id| channel_indices_by_id.get(&id).copied())
            .collect();
        let channels: Vec<_> = channel_indices
            .iter()
            .map(|index| self.project.tracks[*index].id)
            .collect();
        let channel_numbers: std::collections::HashMap<_, _> = channels
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index + 1))
            .collect();
        // Separate contiguous channel kinds visually without imposing a second
        // order over the project. A bus can be placed between ordinary tracks.
        let mut channel_groups: Vec<(bool, usize)> = Vec::new();
        for index in &channel_indices {
            let bus = self.project.tracks[*index].kind == TrackKind::Bus;
            if let Some((_, count)) = channel_groups
                .last_mut()
                .filter(|(previous, _)| *previous == bus)
            {
                *count += 1;
            } else {
                channel_groups.push((bus, 1));
            }
        }
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
        let mut reveal_channel = None;
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
            if let Some(track) =
                daw_ui::adjacent_selection_item(&items, self.selected_track, direction)
            {
                self.mixer_select_channel(track, extend);
                reveal_channel = Some(track);
            }
        }
        let selected_channels: std::collections::HashSet<_> =
            self.finishing_selected_channels().into_iter().collect();
        let group_gap = daw_ui::tokens::SPACE_LG;
        let group_header_height = 18.0 + daw_ui::tokens::SPACE_XS;
        let preview_id = egui::Id::new("mixer_flow_preview");
        let selection_before = self.selected_track;
        let generation = crate::spectrogram::waveform_generation(ui.ctx());
        let frame = ui.ctx().cumulative_frame_nr();
        let previous = ui
            .ctx()
            .data(|data| data.get_temp::<(u64, u64, Option<u64>, Option<u64>)>(preview_id));
        let selected = self.selected_track.filter(|id| channels.contains(id));
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
        let route_height =
            route_rows as f32 * daw_ui::tokens::MIXER_ROUTE_LANE_HEIGHT + daw_ui::tokens::SPACE_SM;
        let meter = self
            .engine
            .as_ref()
            .map(|engine| engine.snapshot())
            .unwrap_or_default();
        let strip_width = 180.0;
        let group_count = channel_groups.len() + 1;
        let mixer_width =
            (channels.len() + 1) as f32 * strip_width + (group_count - 1) as f32 * group_gap;
        let content_width = strip_width - 2.0 * daw_ui::tokens::SPACE_MD;
        let spacing = daw_ui::tokens::SPACE_XS;
        let fixed_height = 2.0 * daw_ui::tokens::SPACE_MD
            + 7.0 * daw_ui::tokens::CONTROL_HEIGHT
            + daw_ui::tokens::PARAMETER_ROW_HEIGHT
            + daw_ui::tokens::COMPACT_HEIGHT
            + daw_ui::MIXER_LEVEL_METER_HEIGHT
            + 18.0
            + 11.0 * spacing;
        // Size controls against the unobscured workspace, before scrolling.
        // Reserve an overview of the main output and one send, plus room for
        // an ordinary status notice. These fixed reservations let the useful
        // routes remain visible without making controls jump when a send or
        // notice appears. Additional lanes use the existing scroll pane.
        let workspace_height = ui.ctx().content_rect().bottom()
            - daw_ui::tokens::BORDER_WIDTH
            - ui.available_rect_before_wrap().top();
        let routing_overview_height =
            2.0 * daw_ui::tokens::MIXER_ROUTE_LANE_HEIGHT + daw_ui::tokens::SPACE_SM;
        let notice_reserve = daw_ui::tokens::CONTROL_HEIGHT + 2.0 * daw_ui::tokens::SPACE_SM;
        let strip_height =
            (workspace_height - routing_overview_height - notice_reserve - group_header_height)
                .max(fixed_height + 120.0);
        let scroll = daw_ui::scroll_area(ui, "mixer_scroll", [true, true]);
        let panning = daw_ui::scroll_area_is_panning(ui, "mixer_scroll");
        let mut diagnostics = Vec::new();
        let mut route_diagnostics = Vec::new();
        let mut master_diagnostics = None;
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
                })
            })
        };
        let knob_json = |knob: Option<daw_ui::KnobGeometry>| {
            knob.map(|knob| {
                serde_json::json!({
                    "dial":rect_json(knob.dial),
                    "number":knob.number.is_positive().then(|| rect_json(knob.number)),
                    "center":[knob.center.x,knob.center.y],
                    "index":[knob.index.x,knob.index.y],
                    "min_position":[knob.minimum.x,knob.minimum.y],
                    "max_position":[knob.maximum.x,knob.maximum.y],
                    "radius":knob.radius,"range":knob.range,"value":knob.value,
                    "divisions":knob.divisions,
                })
            })
        };
        scroll.auto_shrink([false, false]).show_viewport(ui, |ui, _viewport| {
            let reorder_rect = ui.clip_rect();
            reorder_viewport = Some(rect_json(reorder_rect));
            let scroll_delta = daw_ui::track_reorder_scroll_delta(ui, reorder_rect, daw_ui::TrackReorderAxis::Horizontal);
            let strip_size = Vec2::new(strip_width, strip_height);
            let fader_height = strip_height - fixed_height;
            let mut anchors = std::collections::HashMap::new();
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(mixer_width);
            let (headings, _) = ui.allocate_exact_size(Vec2::new(mixer_width, 18.0), egui::Sense::hover());
            let mut group_x = headings.left();
            for (label, count) in channel_groups.iter().map(|(bus, count)| (if *bus { "Buses" } else { "Tracks" }, *count)).chain(std::iter::once(("Master", 1))) {
                let rect = egui::Rect::from_min_size(egui::pos2(group_x, headings.top()), Vec2::new(count as f32 * strip_width, 18.0));
                daw_ui::paint_mixer_group_header(ui.painter(), rect, label);
                group_diagnostics.push(serde_json::json!({"label":label,"rect":rect_json(rect)}));
                group_x = rect.right() + group_gap;
            }
            ui.add_space(daw_ui::tokens::SPACE_XS);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut previous_bus = None;
                for index in channel_indices.iter().copied() {
                    let track = &mut self.project.tracks[index];
                    let bus = track.kind == TrackKind::Bus;
                    let gain_target = AutomationTarget { track: track.id, parameter: AutomationParameter::TrackGain };
                    let pan_target = AutomationTarget { track: track.id, parameter: AutomationParameter::TrackPan };
                    let gain_automation = automation.get(&gain_target).copied();
                    let pan_automation = automation.get(&pan_target).copied();
                    let mut channel_automation: Vec<_> = automation.iter().filter(|(target, _)| target.track == track.id).collect();
                    channel_automation.sort_by_key(|(_, state)| state.0);
                    let channel_state = channel_automation.iter().map(|(_, (_, state, _))| *state)
                        .max_by_key(|state| match state { daw_ui::AutomationState::Bypassed => 0, daw_ui::AutomationState::Read => 1, daw_ui::AutomationState::Overridden => 2 });
                    if previous_bus.is_some_and(|previous| previous != bus) { ui.add_space(group_gap); }
                    previous_bus = Some(bus);
                    let destination = track.output_bus.and_then(|id| names.get(&id))
                        .map(String::as_str).unwrap_or("Master");
                    let route = format!("Ch {} > {}", channel_numbers[&track.id], destination);
                    let channel_peaks = self.engine.as_ref().map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                    let strip = daw_ui::mixer_strip(ui, track.id, strip_size,
                        Some(color(track.color)), selected_channels.contains(&track.id), |ui| {
                        ui.spacing_mut().item_spacing.y = spacing;
                        let name = if bus { daw_ui::mixer_bus_button(ui, &track.name, content_width, color(track.color)) }
                            else { daw_ui::mixer_channel_button(ui, &track.name, content_width, color(track.color)) }
                            .on_hover_text(format!("Open channel · {} · Drag to reorder · Right-click for selection actions · Shift-click to add/remove", track.name));
                        let name = daw_ui::track_reorder_source(ui, &name, track.id);
                        name.context_menu(|ui| {
                            if let Some(choice) = daw_ui::channel_selection_choices(ui, selected_channels.contains(&track.id)) {
                                channel_selection = Some((track.id, choice));
                            }
                        });
                        if name.clicked() {
                            if ui.input(|input| input.modifiers.shift) { toggle_selection = Some(track.id); }
                            else { channel_target = Some(track.id); }
                        }
                        let source_kind = match track.kind {
                            TrackKind::Audio => "Audio", TrackKind::Midi => "MIDI", _ => "Bus",
                        };
                        let source = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_SM;
                            let menu = daw_ui::automation_channel_menu(ui, channel_state, |ui| {
                                for (title, control, state) in [
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
                                    if matches!(target.parameter, AutomationParameter::TrackGain | AutomationParameter::TrackPan) { continue; }
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
                                ui.separator();
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
                            });
                            let kind = if bus {
                                daw_ui::mixer_bus_input_summary(ui,
                                    incoming.get(&track.id).map(Vec::as_slice).unwrap_or(&[]).iter()
                                        .map(|(id, pre)| (names.get(id).map(String::as_str).unwrap_or("Unavailable channel"), *pre)))
                            } else {
                                daw_ui::text::note(ui, source_kind).on_hover_text(match track.kind {
                                    TrackKind::Audio => "Audio clips and monitored input, including their source processing",
                                    _ => "Instrument output, before channel inserts",
                                })
                            };
                            (menu.response.rect, kind.rect)
                        });
                        let effects = daw_ui::mixer_effects_button(ui, content_width,
                            track.inserts.len(), track.inserts.iter().filter(|slot| slot.enabled).count());
                        if effects.clicked() {
                            self.selected_track = Some(track.id); popup = Some(PopupKind::Effects);
                        }
                        let insert_summary = daw_ui::mixer_insert_summary(ui, content_width,
                            track.inserts.iter().map(|slot| (slot.name.as_str(), slot.enabled)));
                        let (gain_rect, fader_rect, fader_scale) = ui.vertical_centered(|ui| {
                            let displayed_gain = gain_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value)).unwrap_or(track.gain);
                            let mut db = daw_ui::channel_gain_to_db(displayed_gain);
                            let gain = ui.add_sized(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
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
                            let fader = ui.horizontal(|ui| {
                                ui.add_space((content_width - daw_ui::MIXER_FADER_WIDTH) * 0.5);
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
                                (response.rect, scale)
                            }).inner;
                            (gain.rect, fader.0, fader.1)
                        }).inner;
                        let mut pan = pan_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                            .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value)).unwrap_or(track.pan);
                        let pan_response = daw_ui::mixer_pan(ui, &mut pan, content_width)
                            .on_hover_text("Stereo balance");
                        let (action, exact) = daw_ui::automatable_parameter_value_context_named(ui, &pan_response,
                            daw_ui::AutomationControl::Pan { track: track.id }, pan_automation.map(|(lane, state, _)| (lane, state)),
                            f64::from(pan) * 100.0, -100.0..=100.0, "%", &format!("{} · Pan", track.name));
                        if let Some(action) = action {
                            automation_navigation = Some(action);
                        }
                        if let Some(value) = exact { pan = (value / 100.0) as f32; }
                        if (pan_response.changed() || exact.is_some()) && !panning { track.pan = pan; changed = true;
                            automation_overrides.push((pan_target, AutomationParameter::TrackPan.normalize(pan))); }
                        let mute_solo = daw_ui::mixer_mute_solo(ui, &mut track.mute, &mut track.solo, content_width);
                        changed |= mute_solo.mute.changed() || mute_solo.solo.changed();
                        let mute_solo_rect = mute_solo.mute.rect.union(mute_solo.solo.rect);
                        let level = daw_ui::mixer_level_meter(ui, channel_peaks, content_width)
                            .on_hover_text("Channel output level · after inserts, volume and pan");
                        let output_label = ui.allocate_ui_with_layout(Vec2::new(content_width, 18.0),
                            egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            daw_ui::text::note(ui, "Output");
                        });
                        let target = daw_ui::mixer_output_button(ui, content_width, &route, destination,
                            track.output_bus.and_then(|id| channel_colors.get(&id)).copied());
                        if target.clicked() { self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); }
                        let send_details: Vec<_> = track.sends.iter().map(|send| daw_ui::MixerSend {
                            destination: names.get(&send.target_bus).map(String::as_str).unwrap_or("Bus"),
                            color: channel_colors.get(&send.target_bus).copied().unwrap_or(palette(ui.ctx()).accent),
                            gain: automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::SendGain { bus: send.target_bus } })
                                .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::SendGain { bus: send.target_bus }.denormalize(*value)).unwrap_or(send.gain),
                            pre_fader: send.pre_fader,
                        }).collect();
                        let sends = daw_ui::mixer_sends_button(ui, content_width, &send_details);
                        if track.sends.is_empty() {
                            empty_send_menus.push((track.id, sends.clone()));
                        } else if sends.clicked() {
                            self.selected_track = Some(track.id); popup = Some(PopupKind::Routing);
                        }
                        serde_json::json!({"track":track.id,"channel":channel_numbers[&track.id],"bus":bus,"peaks":channel_peaks,"name":rect_json(name.rect),"reorder_source":rect_json(name.rect),"source":rect_json(source.response.rect),
                            "source_kind":source_kind,"source_menu":rect_json(source.inner.0),"source_label":rect_json(source.inner.1),
                            "automation_state":channel_state.map(daw_ui::automation_state_label),
                            "effects":rect_json(effects.rect),"insert_summary":rect_json(insert_summary.rect),
                            "incoming_count":incoming.get(&track.id).map_or(0, Vec::len),
                            "gain":rect_json(gain_rect),"fader":rect_json(fader_rect),"fader_scale":scale_json(fader_scale),
                            "pan":rect_json(pan_response.rect),"pan_knob":knob_json(daw_ui::knob_geometry(ui.ctx(), pan_response.id)),
                            "mute_solo":rect_json(mute_solo_rect),"mute":rect_json(mute_solo.mute.rect),"solo":rect_json(mute_solo.solo.rect),
                            "meter":rect_json(level.rect),"target":rect_json(target.rect),"sends":rect_json(sends.rect),
                            "stages":{"input":rect_json(source.response.rect),"processing":rect_json(effects.rect.union(mute_solo_rect)),
                                "measuring":rect_json(level.rect),"output":rect_json(output_label.response.rect.union(sends.rect))}})
                    });
                    let drop = daw_ui::track_reorder_target(ui, strip.response.rect, track.id,
                        daw_ui::TrackReorderAxis::Horizontal, false, |intent| intent.source != intent.target);
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
                        ui.scroll_to_rect(strip.response.rect, Some(egui::Align::Center));
                    }
                    if ui.is_enabled() && !panning && !egui::Popup::is_any_open(ui.ctx())
                        && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pos| strip.response.rect.contains(pos))
                    { flow_source = Some(track.id); }
                    let mut channel_diagnostics = strip.inner;
                    channel_diagnostics["strip"] = rect_json(strip.response.rect);
                    channel_diagnostics["drop"] = rect_json(strip.response.rect);
                    diagnostics.push(channel_diagnostics);
                }
                if !channels.is_empty() { ui.add_space(group_gap); }
                let master = daw_ui::mixer_strip(ui, "master", strip_size, None, false, |ui| {
                    ui.spacing_mut().item_spacing.y = spacing;
                    daw_ui::mixer_master_title(ui, content_width);
                    for label in ["Stereo mix", ""] {
                        ui.allocate_ui_with_layout(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                            ui.set_min_size(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                            if !label.is_empty() { daw_ui::text::label(ui, label); }
                        });
                    }
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::COMPACT_HEIGHT));
                    ui.vertical_centered(|ui| {
                        let peak = meter.peak_left.max(meter.peak_right);
                        ui.allocate_ui_with_layout(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                            ui.label(egui::RichText::new(if peak > 0.00001 {
                                format!("{:.1} dB", 20.0 * peak.log10())
                            } else { "−∞ dB".to_owned() }).monospace());
                        });
                        daw_ui::stereo_meter(ui, [meter.peak_left, meter.peak_right], Vec2::new(56.0, fader_height));
                    });
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::PARAMETER_ROW_HEIGHT));
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                    ui.allocate_space(Vec2::new(content_width, daw_ui::MIXER_LEVEL_METER_HEIGHT));
                    ui.allocate_ui_with_layout(Vec2::new(content_width, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center), |ui| { daw_ui::text::note(ui, "Output"); });
                    ui.allocate_ui_with_layout(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                        egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| { daw_ui::text::note(ui, "Stereo output"); });
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                });
                anchors.insert(None, master.response.rect.center_bottom());
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
                        let lane = daw_ui::mixer_route_lane(ui,
                            daw_ui::MixerRouteEndpoint { position: source_position, color: color(track.color) },
                            daw_ui::MixerRouteEndpoint { position: target_position,
                                color: target.and_then(|id| channel_colors.get(&id)).copied().unwrap_or(palette(ui.ctx()).accent) },
                            &caption, connection_clip);
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
        // Attach the shared destination choices to the existing strip button
        // after releasing the mutable track borrow. The captured source owns
        // this action even when another channel remains selected.
        for (source, response) in empty_send_menus {
            if let Some(destination) = egui::Popup::menu(&response)
                .show(|ui| self.mixer_send_choices(ui, source))
                .and_then(|shown| shown.inner)
            {
                self.mixer_create_send(source, destination);
            }
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(preview_id, (generation, frame, self.selected_track, flow_source));
            data.insert_temp(egui::Id::new("mixer_diagnostics"),
                serde_json::json!({"channels":diagnostics,"master_strip":master_diagnostics,"master_peaks":[meter.peak_left,meter.peak_right],"groups":group_diagnostics,"routes":route_diagnostics,"flow_source":flow_source,"route_height":route_height,"reorder_preview":reorder_preview,"reorder_viewport":reorder_viewport}));
        });
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
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
        if let Some(track) = channel_target {
            self.open_channel(track);
        }
    }

    pub fn instrument_editor(&mut self, ctx: &egui::Context) {
        self.plugin_instrument_editor(ctx);
    }
}
