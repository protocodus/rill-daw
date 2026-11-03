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
        let mut device_target = None;
        let mut insert_target = None;
        let mut device_diagnostics = Vec::new();
        let mut toggle_selection = None;
        let mut channel_selection = None;
        let mut rename_track = None;
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
        let channel_gap = daw_ui::tokens::SPACE_SM;
        let group_header_height = daw_ui::tokens::COMPACT_HEIGHT + daw_ui::tokens::SPACE_XS;
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
        let route_height = route_rows as f32 * daw_ui::MIXER_ROUTE_LANE + daw_ui::tokens::SPACE_SM;
        let meter = self
            .engine
            .as_ref()
            .map(|engine| engine.snapshot())
            .unwrap_or_default();
        let strip_width = daw_ui::tokens::MIXER_STRIP_WIDTH;
        let group_count = channel_groups.len() + 1;
        let mixer_width = (channels.len() + 1) as f32 * strip_width
            + (channels.len() + 1 - group_count) as f32 * channel_gap
            + (group_count - 1) as f32 * group_gap;
        let content_width = strip_width - 2.0 * daw_ui::tokens::SPACE_MD;
        let spacing = daw_ui::tokens::SPACE_SM;
        let section_gap = daw_ui::MIXER_SECTION_GAP;
        let control = daw_ui::tokens::CONTROL_HEIGHT;
        let send_area_height = 2.0 * daw_ui::MIXER_SEND_ROW_HEIGHT + daw_ui::tokens::SPACE_XS;
        // Signal order: identity and source, effects, pan with mute/solo, the
        // gain and peak readouts over the fader unit, then output and sends. Every strip
        // reserves the same rows, so sections line up across channels.
        let mut fixed_height = 2.0 * daw_ui::tokens::SPACE_MD
            + 2.0 * control
            + spacing
            + control
            + spacing
            + daw_ui::MIXER_PAN_ROW_HEIGHT
            + spacing
            + daw_ui::tokens::SPACE_XS
            + control
            + control
            + spacing
            + send_area_height
            + spacing
            + control
            + 3.0 * section_gap;
        // Size controls against the unobscured workspace, before scrolling.
        // Reserve the output lane and one send lane plus room for an ordinary
        // status notice, so controls never jump when a send or notice appears.
        // Further lanes use the lane pane's own scroll.
        let workspace_height = ui.ctx().content_rect().bottom()
            - daw_ui::tokens::BORDER_WIDTH
            - ui.available_rect_before_wrap().top();
        let routing_overview_height = 2.0 * daw_ui::MIXER_ROUTE_LANE + daw_ui::tokens::SPACE_SM;
        let notice_reserve = daw_ui::tokens::CONTROL_HEIGHT + 2.0 * daw_ui::tokens::SPACE_SM;
        let device_rows = self
            .project
            .tracks
            .iter()
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
        let unit_offset = ((content_width - daw_ui::MIXER_FADER_UNIT_WIDTH) * 0.5).max(0.0);
        let meter_offset = unit_offset + daw_ui::MIXER_FADER_WIDTH + daw_ui::tokens::SPACE_SM;
        let scroll = daw_ui::scroll_area(ui, "mixer_scroll", [true, true]);
        let panning = daw_ui::scroll_area_is_panning(ui, "mixer_scroll");
        let mut diagnostics = Vec::new();
        let mut route_diagnostics = Vec::new();
        let mut master_diagnostics = None;
        let mut master_details = None;
        let mut master_output = false;
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
            let scroll_delta = daw_ui::track_reorder_scroll_delta(ui, reorder_rect, daw_ui::TrackReorderAxis::Horizontal);
            let strip_size = Vec2::new(strip_width, strip_height);
            let fader_height = strip_height - fixed_height;
            let mut anchors = std::collections::HashMap::new();
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(mixer_width);
            let (headings, _) = ui.allocate_exact_size(Vec2::new(mixer_width, daw_ui::tokens::COMPACT_HEIGHT), egui::Sense::hover());
            let mut group_x = headings.left();
            for (label, count) in channel_groups.iter().map(|(bus, count)| (if *bus { "Buses" } else { "Tracks" }, *count)).chain(std::iter::once(("Output", 1))) {
                let width = count as f32 * strip_width + count.saturating_sub(1) as f32 * channel_gap;
                let rect = egui::Rect::from_min_size(egui::pos2(group_x, headings.top()), Vec2::new(width, daw_ui::tokens::COMPACT_HEIGHT));
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
                    if let Some(previous) = previous_bus {
                        ui.add_space(if previous != bus { group_gap } else { channel_gap });
                    }
                    previous_bus = Some(bus);
                    let destination = track.output_bus.and_then(|id| names.get(&id))
                        .map(String::as_str).unwrap_or("Master");
                    let channel_peaks = self.engine.as_ref().map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                    let strip = daw_ui::mixer_strip(ui, track.id, strip_size,
                        Some(color(track.color)), selected_channels.contains(&track.id), |ui| {
                        daw_ui::mixer_sections(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = spacing;
                        // Identity
                        let name = daw_ui::mixer_name_plate(ui, if bus { daw_ui::MixerNamePlateKind::Bus } else { daw_ui::MixerNamePlateKind::Track },
                            &track.name, content_width, color(track.color))
                            .on_hover_text(format!("Open channel · {} · Drag to reorder · Right-click for channel and selection actions · Shift-click to add/remove", track.name));
                        let name = daw_ui::track_reorder_source(ui, &name, track.id);
                        name.context_menu(|ui| {
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
                        let source_kind = match track.kind {
                            TrackKind::Audio => "Audio", TrackKind::Midi => "MIDI", _ => "Bus",
                        };
                        let source = ui.allocate_ui_with_layout(Vec2::new(content_width, control),
                            egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
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
                            });
                            let width = ui.available_width();
                            let kind = ui.allocate_ui_with_layout(Vec2::new(width, control), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                if bus {
                                    daw_ui::mixer_bus_input_summary(ui, width,
                                        incoming.get(&track.id).map(Vec::as_slice).unwrap_or(&[]).iter()
                                            .map(|(id, pre)| (names.get(id).map(String::as_str).unwrap_or("Unavailable channel"), *pre)))
                                } else if let Instrument::Vst3 { path, class_id } = &track.instrument {
                                    let model = crate::library::plugin_model(&self.vst_catalog, path, class_id);
                                    let instrument = if track.instrument_name.is_empty() { model.split(" · ").last().unwrap_or(&model) } else { &track.instrument_name };
                                    let response = ui.add_sized(Vec2::new(width, control),
                                        daw_ui::button_widget(format!("MIDI · {instrument}")).truncate().presentation())
                                        .on_hover_text(format!("Instrument · {model}\nInstrument output, before channel inserts · Click to open"));
                                    if response.clicked() { device_target = Some((track.id, None)); }
                                    device_diagnostics.push(serde_json::json!({"track":track.id,"effect":null,"name":instrument,"model":model,"rect":rect_json(response.rect)}));
                                    response
                                } else {
                                    let text = match (track.kind, track.audio_input) {
                                        (TrackKind::Midi, _) => "MIDI · No instrument".to_owned(),
                                        (_, Some(AudioInputChannels::Mono { channel })) => format!("Audio · In {}", channel + 1),
                                        (_, Some(AudioInputChannels::Stereo { left, right })) => format!("Audio · In {}/{}", left + 1, right + 1),
                                        (_, None) => "Audio".to_owned(),
                                    };
                                    daw_ui::mixer_source_caption(ui, &text, width).on_hover_text(match track.kind {
                                        TrackKind::Audio => "Audio clips and monitored input, including their source processing",
                                        _ => "Instrument output, before channel inserts",
                                    })
                                }
                            }).inner;
                            (menu.response.rect, kind.rect)
                        });
                        // Effects
                        ui.add_space(section_gap - spacing);
                        let effects = daw_ui::mixer_effects_button(ui, content_width,
                            track.inserts.len(), track.inserts.iter().filter(|slot| slot.enabled).count());
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
                                        let Effect::Vst3 { path, class_id } = &slot.effect;
                                        let model = crate::library::plugin_model(&self.vst_catalog, path, class_id);
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
                        let readout = daw_ui::channel_meter_readout(ui, meter_id,
                            readout_row.left() + meter_offset + daw_ui::tokens::MIXER_METER_WIDTH * 0.5, readout_row);
                        ui.add_space(daw_ui::tokens::SPACE_XS);
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
                            ui.add_space(daw_ui::tokens::SPACE_SM);
                            let level = daw_ui::channel_meter(ui, meter_id, channel_peaks, fader_height, daw_ui::MIXER_FADER_INSET);
                            (response.rect, scale, level)
                        }).inner;
                        ui.spacing_mut().item_spacing.y = spacing;
                        // Output & sends
                        ui.add_space(section_gap);
                        let target = daw_ui::mixer_output_button(ui, content_width, destination,
                            track.output_bus.and_then(|id| channel_colors.get(&id)).copied())
                            .on_hover_text(format!("Output → {destination}\nAfter inserts, volume and pan · Click to edit routing"));
                        if target.clicked() { self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); }
                        let send_details: Vec<_> = track.sends.iter().map(|send| daw_ui::MixerSend {
                            destination: names.get(&send.target_bus).map(String::as_str).unwrap_or("Bus"),
                            color: channel_colors.get(&send.target_bus).copied().unwrap_or(palette(ui.ctx()).accent),
                            gain: automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::SendGain { bus: send.target_bus } })
                                .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::SendGain { bus: send.target_bus }.denormalize(*value)).unwrap_or(send.gain),
                            pre_fader: send.pre_fader,
                        }).collect();
                        let mut send_rows = Vec::new();
                        let send_area = ui.allocate_ui_with_layout(Vec2::new(content_width, send_area_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_size(Vec2::new(content_width, send_area_height));
                            ui.spacing_mut().item_spacing.y = daw_ui::tokens::SPACE_XS;
                            let visible = if send_details.len() > 2 { 1 } else { send_details.len() };
                            let mut open = false;
                            for send in &send_details[..visible] {
                                let row = daw_ui::mixer_send_row(ui, content_width, send);
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
                        empty_send_menus.push((track.id, add_send.clone()));
                        let sends_rect = send_area.response.rect.union(add_send.rect);
                        let level_rect = pan_response.rect.union(mute_solo_rect).union(gain.rect).union(fader_rect).union(level.rect);
                        let sections = [Some(name.rect.union(source.response.rect)), Some(effects.rect.union(insert_summary.rect)), Some(level_rect), Some(target.rect.union(sends_rect))];
                        let section_diagnostics: Vec<_> = daw_ui::MixerSection::ALL.into_iter().zip(sections).filter_map(|(section, bounds)| bounds.map(|bounds| serde_json::json!({"name":section.name(),"rect":rect_json(bounds),"frame":rect_json(egui::emath::GuiRounding::round_to_pixels(bounds.expand(daw_ui::MIXER_SECTION_PADDING), ui.pixels_per_point())),"fill":section.fill(ui.ctx()).to_array()}))).collect();
                        (serde_json::json!({"track":track.id,"channel":channel_numbers[&track.id],"bus":bus,"peaks":channel_peaks,"name":rect_json(name.rect),"reorder_source":rect_json(name.rect),"source":rect_json(source.response.rect),
                            "source_kind":source_kind,"source_menu":rect_json(source.inner.0),"source_label":rect_json(source.inner.1),
                            "automation_state":channel_state.map(daw_ui::automation_state_label),
                            "effects":rect_json(effects.rect),"insert_summary":rect_json(insert_summary.rect),
                            "incoming_count":incoming.get(&track.id).map_or(0, Vec::len),
                            "gain":rect_json(gain.rect),"fader":rect_json(fader_rect),"fader_scale":scale_json(fader_scale),
                            "pan":rect_json(pan_response.rect),"pan_scale":scale_json(daw_ui::slider_scale_geometry(ui.ctx(), pan_response.id)),
                            "mute_solo":rect_json(mute_solo_rect),"mute":rect_json(pan_row.mute.rect),"solo":rect_json(pan_row.solo.rect),
                            "meter":rect_json(level.rect),"meter_readout":rect_json(readout.rect),"target":rect_json(target.rect),"sends":rect_json(sends_rect),
                            "send_rows":send_rows,"add_send":rect_json(add_send.rect),
                            "sections":section_diagnostics,
                            "stages":{"input":rect_json(source.response.rect),"processing":rect_json(effects.rect.union(insert_summary.rect)),
                                "measuring":rect_json(level.rect),"output":rect_json(target.rect.union(sends_rect))}}), sections)
                        })
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
                // Master mirrors the channel rows: identity, its summed source,
                // a meter on the fader scale, and the hardware output.
                let master = daw_ui::mixer_strip(ui, "master", strip_size, None, false, |ui| {
                    daw_ui::mixer_sections(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = spacing;
                        let name = daw_ui::mixer_name_plate(ui, daw_ui::MixerNamePlateKind::Master, "Master", content_width, palette(ui.ctx()).raised);
                        let source = daw_ui::mixer_source_caption(ui, &format!("Sum · {master_inputs} in"), content_width)
                            .on_hover_text("Channels and buses whose output is Master");
                        ui.add_space(section_gap - spacing);
                        ui.allocate_space(Vec2::new(content_width, control + spacing + device_height));
                        ui.add_space(section_gap - spacing);
                        let (pan_space, _) = ui.allocate_exact_size(Vec2::new(content_width, daw_ui::MIXER_PAN_ROW_HEIGHT), egui::Sense::hover());
                        let meter_id = egui::Id::new("mixer_master_meter");
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let (gain_space, _) = ui.allocate_exact_size(Vec2::new(content_width, control), egui::Sense::hover());
                        let readout = daw_ui::channel_meter_readout(ui, meter_id,
                            gain_space.left() + meter_offset + daw_ui::tokens::MIXER_METER_WIDTH * 0.5, gain_space);
                        ui.add_space(daw_ui::tokens::SPACE_XS);
                        let level = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(meter_offset);
                            daw_ui::channel_meter(ui, meter_id, [meter.peak_left, meter.peak_right], fader_height, daw_ui::MIXER_FADER_INSET)
                                .on_hover_text("Master output level")
                        }).inner;
                        daw_ui::paint_mixer_scale(ui, level.rect.left() - daw_ui::tokens::SPACE_XS,
                            level.rect.bottom() - daw_ui::MIXER_FADER_INSET, level.rect.top() + daw_ui::MIXER_FADER_INSET);
                        ui.spacing_mut().item_spacing.y = spacing;
                        ui.add_space(section_gap);
                        let output = daw_ui::mixer_output_button(ui, content_width, "Audio output", Some(palette(ui.ctx()).muted))
                            .on_hover_text("Master → audio device output · Click to open Setup › Audio");
                        if output.clicked() { master_output = true; }
                        let (rest, _) = ui.allocate_exact_size(Vec2::new(content_width, send_area_height + spacing + control), egui::Sense::hover());
                        let sections = [Some(name.rect.union(source.rect)), None, Some(pan_space.union(level.rect)), Some(output.rect.union(rest))];
                        (serde_json::json!({"meter":rect_json(level.rect),"meter_readout":rect_json(readout.rect),"target":rect_json(output.rect),
                            "name":rect_json(name.rect),"source":rect_json(source.rect)}), sections)
                    })
                });
                anchors.insert(None, master.response.rect.center_bottom());
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
        if master_output {
            self.setup_page = crate::app::SetupPage::Audio;
            self.open_popup(PopupKind::Setup);
        }
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
                serde_json::json!({"channels":diagnostics,"master_strip":master_diagnostics,"master":master_details,"master_peaks":[meter.peak_left,meter.peak_right],"groups":group_diagnostics,"routes":route_diagnostics,"flow_source":flow_source,"route_height":route_height,"reorder_preview":reorder_preview,"reorder_viewport":reorder_viewport}));
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
        if let Some(track) = rename_track {
            self.start_track_rename(ui.ctx(), track);
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
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
