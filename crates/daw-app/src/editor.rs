use crate::{
    app::{DawApp, PopupKind},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Vec2};

impl DawApp {
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

    pub(crate) fn available_bus_name(&self) -> String {
        for index in 1.. {
            let name = if index == 1 {
                "New bus".to_owned()
            } else {
                format!("New bus {index}")
            };
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
        let names: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .map(|track| (track.id, track.name.clone()))
            .collect();
        let channels: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| !matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
            .map(|track| track.id)
            .collect();
        let channel_numbers: std::collections::HashMap<_, _> = channels
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index + 1))
            .collect();
        let tracks: Vec<_> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
            .map(|(index, _)| index)
            .collect();
        let buses: Vec<_> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.kind == TrackKind::Bus)
            .map(|(index, _)| index)
            .collect();
        let mut reveal_channel = None;
        if let Some((direction, extend)) = daw_ui::selection_navigation_input(ui, None) {
            let direction = match direction {
                daw_ui::SelectionDirection::Up => daw_ui::SelectionDirection::Left,
                daw_ui::SelectionDirection::Down => daw_ui::SelectionDirection::Right,
                direction => direction,
            };
            let items: Vec<_> = tracks
                .iter()
                .chain(&buses)
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
            route_rows as f32 * daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_SM;
        let meter = self
            .engine
            .as_ref()
            .map(|engine| engine.snapshot())
            .unwrap_or_default();
        let strip_width = 180.0;
        let group_count = usize::from(!tracks.is_empty()) + usize::from(!buses.is_empty()) + 1;
        let mixer_width =
            (channels.len() + 1) as f32 * strip_width + (group_count - 1) as f32 * group_gap;
        let content_width = strip_width - 2.0 * daw_ui::tokens::SPACE_MD;
        let spacing = daw_ui::tokens::SPACE_XS;
        let fixed_height = 2.0 * daw_ui::tokens::SPACE_MD
            + 7.0 * daw_ui::tokens::CONTROL_HEIGHT
            + 18.0
            + 8.0 * spacing;
        let scroll = daw_ui::scroll_area(ui, "mixer_scroll", [true, true]);
        let panning = daw_ui::scroll_area_is_panning(ui, "mixer_scroll");
        let mut diagnostics = Vec::new();
        let mut route_diagnostics = Vec::new();
        let mut group_diagnostics = Vec::new();
        let rect_json = |rect: egui::Rect| serde_json::json!({"min":[rect.left(),rect.top()],"max":[rect.right(),rect.bottom()]} );
        scroll.auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let strip_height = (viewport.height() - route_height - group_header_height).max(fixed_height + 120.0);
            let strip_size = Vec2::new(strip_width, strip_height);
            let fader_height = strip_height - fixed_height;
            let mut anchors = std::collections::HashMap::new();
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(mixer_width);
            let (headings, _) = ui.allocate_exact_size(Vec2::new(mixer_width, 18.0), egui::Sense::hover());
            let mut group_x = headings.left();
            for (label, count) in [("Tracks", tracks.len()), ("Buses", buses.len()), ("Master", 1)] {
                if count == 0 { continue; }
                let rect = egui::Rect::from_min_size(egui::pos2(group_x, headings.top()), Vec2::new(count as f32 * strip_width, 18.0));
                daw_ui::paint_mixer_group_header(ui.painter(), rect, label);
                group_diagnostics.push(serde_json::json!({"label":label,"rect":rect_json(rect)}));
                group_x = rect.right() + group_gap;
            }
            ui.add_space(daw_ui::tokens::SPACE_XS);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut previous_bus = None;
                for index in tracks.iter().chain(&buses).copied() {
                    let track = &mut self.project.tracks[index];
                    let bus = track.kind == TrackKind::Bus;
                    if previous_bus.is_some_and(|previous| previous != bus) { ui.add_space(group_gap); }
                    previous_bus = Some(bus);
                    let destination = track.output_bus.and_then(|id| names.get(&id))
                        .map(String::as_str).unwrap_or("Master");
                    let route = format!("Ch {} > {}", channel_numbers[&track.id], destination);
                    let strip = daw_ui::mixer_strip(ui, track.id, strip_size,
                        Some(color(track.color)), selected_channels.contains(&track.id), |ui| {
                        ui.spacing_mut().item_spacing.y = spacing;
                        let name = if bus { daw_ui::mixer_bus_button(ui, &track.name, content_width, color(track.color)) }
                            else { daw_ui::mixer_channel_button(ui, &track.name, content_width, color(track.color)) }
                            .on_hover_text(format!("Open channel · {} · Right-click for selection actions · Shift-click to add/remove", track.name));
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
                            let menu = daw_ui::compact_action_menu(ui, "Channel", |ui| {
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
                            let kind = daw_ui::text::note(ui, source_kind).on_hover_text(match track.kind {
                                TrackKind::Audio => "Audio clips and monitored input, including their source processing",
                                TrackKind::Midi => "Instrument output, before channel inserts",
                                _ => "Incoming channels and auxiliary sends",
                            });
                            (menu.response.rect, kind.rect)
                        });
                        let effects = ui.add_sized(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            daw_ui::button_widget("Effects"))
                            .on_hover_text(format!("{} inserts · before volume and pan", track.inserts.len()));
                        if effects.clicked() {
                            self.selected_track = Some(track.id); popup = Some(PopupKind::Effects);
                        }
                        let (gain_rect, fader_rect) = ui.vertical_centered(|ui| {
                            let mut db = daw_ui::channel_gain_to_db(track.gain);
                            let gain = ui.add_sized(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                                daw_ui::channel_gain_number(&mut db))
                                .on_hover_text("Channel volume · drag or double-click to type");
                            if gain.changed() && !panning {
                                track.gain = daw_ui::channel_gain_from_db(db); changed = true;
                            }
                            ui.spacing_mut().slider_width = fader_height;
                            let fader = ui.horizontal(|ui| {
                                ui.add_space((content_width - 52.0) * 0.5);
                                let mut gain = track.gain;
                                let response = ui.add_sized(Vec2::new(30.0, fader_height),
                                    daw_ui::slider(&mut gain, 0.0..=1.5)
                                        .clamping(egui::SliderClamping::Edits).vertical().show_value(false))
                                    .on_hover_text("Channel volume");
                                if response.changed() && !panning { track.gain = gain; changed = true; }
                                let peaks = self.engine.as_ref().map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                                daw_ui::stereo_meter(ui, peaks, Vec2::new(14.0, fader_height))
                                    .on_hover_text("Channel output level · post-fader");
                                response.rect
                            }).inner;
                            (gain.rect, fader)
                        }).inner;
                        ui.spacing_mut().slider_width = content_width - 38.0;
                        let mut pan = track.pan;
                        let pan_response = ui.add(daw_ui::slider(&mut pan, -1.0..=1.0)
                            .clamping(egui::SliderClamping::Edits).show_value(false).text("Pan"))
                            .on_hover_text("Stereo balance");
                        if pan_response.changed() && !panning { track.pan = pan; changed = true; }
                        let mute_solo = ui.horizontal(|ui| {
                            changed |= daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                            changed |= daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                        }).response.rect;
                        ui.allocate_ui_with_layout(Vec2::new(content_width, 18.0),
                            egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            daw_ui::text::note(ui, if track.sends.is_empty() { "Target".to_owned() }
                                else { format!("Target · {} {}", track.sends.len(), if track.sends.len() == 1 { "send" } else { "sends" }) });
                        });
                        let target = ui.add_sized(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            daw_ui::button_widget(egui::RichText::new(&route).small()).truncate())
                            .on_hover_text("Main output and auxiliary bus sends");
                        if target.clicked() { self.selected_track = Some(track.id); popup = Some(PopupKind::Routing); }
                        serde_json::json!({"track":track.id,"channel":channel_numbers[&track.id],"bus":bus,"name":rect_json(name.rect),"source":rect_json(source.response.rect),
                            "source_kind":source_kind,"source_menu":rect_json(source.inner.0),"source_label":rect_json(source.inner.1),
                            "effects":rect_json(effects.rect),"gain":rect_json(gain_rect),"fader":rect_json(fader_rect),
                            "pan":rect_json(pan_response.rect),"mute_solo":rect_json(mute_solo),"target":rect_json(target.rect)})
                    });
                    anchors.insert(Some(track.id), strip.response.rect.center().x);
                    if reveal_channel == Some(track.id) {
                        ui.scroll_to_rect(strip.response.rect, Some(egui::Align::Center));
                    }
                    if ui.is_enabled() && !panning && !egui::Popup::is_any_open(ui.ctx())
                        && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pos| strip.response.rect.contains(pos))
                    { flow_source = Some(track.id); }
                    diagnostics.push(strip.inner);
                }
                if !channels.is_empty() { ui.add_space(group_gap); }
                let master = daw_ui::mixer_strip(ui, "master", strip_size, None, false, |ui| {
                    ui.spacing_mut().item_spacing.y = spacing;
                    for label in ["Master", "Stereo mix", ""] {
                        ui.allocate_ui_with_layout(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                            ui.set_min_size(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                            if !label.is_empty() { daw_ui::text::label(ui, label); }
                        });
                    }
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
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                    ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                    ui.allocate_ui_with_layout(Vec2::new(content_width, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center), |ui| { daw_ui::text::note(ui, "Target"); });
                    ui.allocate_ui_with_layout(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                        egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| { daw_ui::text::note(ui, "Stereo output"); });
                });
                anchors.insert(None, master.response.rect.center().x);
            });
            if self.selected_track != selection_before {
                flow_source = self.selected_track.filter(|id| channels.contains(id));
            }
            daw_ui::scroll_area(ui, "mixer_route_lanes", [false, true])
                .max_width(mixer_width).max_height(route_height)
                .auto_shrink([false, false]).show(ui, |ui| {
                ui.set_min_width(mixer_width);
                if let Some(track) = flow_source.and_then(|id| self.project.track(id)) {
                    let source_x = anchors.get(&Some(track.id)).copied().unwrap_or(0.0);
                    let routes = std::iter::once((track.output_bus, "Output"))
                        .chain(track.sends.iter().map(|send| (Some(send.target_bus), if send.pre_fader { "Pre send" } else { "Post send" })));
                    for (target, kind) in routes {
                        let target_x = anchors.get(&target).copied().unwrap_or(source_x);
                        let target_name = target.and_then(|id| names.get(&id)).map(String::as_str).unwrap_or("Master");
                        let caption = format!("{kind} · {} → {target_name}", track.name);
                        let lane = daw_ui::mixer_route_lane(ui, source_x, target_x, &caption, color(track.color));
                        if let Some(painted) = lane.painted {
                            route_diagnostics.push(serde_json::json!({"source":track.id,"target":target,"kind":kind,
                                "lane":rect_json(painted.lane),"shaft":rect_json(painted.shaft),"head":rect_json(painted.head),
                                "from":[painted.from.x,painted.from.y],"to":[painted.to.x,painted.to.y],
                                "source_clipped":painted.source_clipped,"target_clipped":painted.target_clipped}));
                        }
                        let hint = match kind {
                                "Pre send" => "After inserts, before volume and pan",
                                "Post send" => "After inserts, volume and pan",
                                _ => "Main channel output, after volume and pan",
                            };
                        lane.response.on_hover_text(format!("{caption}\n{hint}"));
                    }
                }
            });
        });
        ui.ctx().data_mut(|data| {
            data.insert_temp(preview_id, (generation, frame, self.selected_track, flow_source));
            data.insert_temp(egui::Id::new("mixer_diagnostics"),
                serde_json::json!({"channels":diagnostics,"groups":group_diagnostics,"routes":route_diagnostics,"flow_source":flow_source,"route_height":route_height}));
        });
        if changed {
            self.changed();
        }
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
