use crate::{app::DawApp, theme::*};
use daw_core::*;
use daw_ui as widgets;
use eframe::egui::{self, RichText};

impl DawApp {
    pub fn channel_rack(&mut self, ctx: &egui::Context) {
        if !self.effects_open {
            return;
        }
        let Some(track_id) = self.selected_track else {
            self.effects_open = false;
            return;
        };
        let Some(track) = self.project.track(track_id) else {
            self.effects_open = false;
            return;
        };
        if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
            self.effects_open = false;
            return;
        }
        let track_name = track.name.clone();
        let mut open = true;
        let mut edit_effect = None;
        let mut add_effect = false;
        widgets::popup(format!("Effects · {track_name}"), ctx)
            .id(egui::Id::new("effects"))
            .open(&mut open)
            .default_width(620.0)
            .vscroll(true)
            .show(ctx, |ui| {
                let mut remove = None;
                let mut move_up = None;
                let mut move_down = None;
                let mut dragged = None;
                let mut changed = false;
                let track = self.project.track_mut(track_id).unwrap();
                let count = track.inserts.len();
                for (index, slot) in track.inserts.iter_mut().enumerate() {
                    ui.push_id(slot.id, |ui| {
                        let row = widgets::card(ui.ctx()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                widgets::reorder_handle(ui, (track_id, slot.id));
                                changed |= widgets::switch(
                                    ui,
                                    &mut slot.enabled,
                                    format!("{:02}  {}", index + 1, slot.name),
                                )
                                .changed();
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if widgets::small_button(ui, "Open editor")
                                            .on_hover_text("Open native effect editor")
                                            .clicked()
                                        {
                                            edit_effect = Some(slot.id);
                                        }
                                        if widgets::small_button(ui, "×")
                                            .on_hover_text("Remove insert")
                                            .clicked()
                                        {
                                            remove = Some(index);
                                        }
                                        if ui
                                            .add_enabled_ui(index + 1 < count, |ui| {
                                                widgets::small_button(ui, "Down")
                                                    .on_hover_text("Move insert down")
                                            })
                                            .inner
                                            .clicked()
                                        {
                                            move_down = Some(index);
                                        }
                                        if ui
                                            .add_enabled_ui(index > 0, |ui| {
                                                widgets::small_button(ui, "Up")
                                                    .on_hover_text("Move insert up")
                                            })
                                            .inner
                                            .clicked()
                                        {
                                            move_up = Some(index);
                                        }
                                    },
                                );
                            });
                            let Effect::Vst3 { path, .. } = &slot.effect;
                            ui.label(RichText::new("VST3").small().color(palette(ui.ctx()).muted))
                                .on_hover_text(path);
                        });
                        if let Some(payload) =
                            widgets::reorder_drop_target::<(u64, u64)>(ui, &row.response)
                            && payload.0 == track_id
                            && payload.1 != slot.id
                        {
                            dragged = Some((payload.1, index));
                        }
                    });
                }
                if let Some(index) = remove {
                    track.inserts.remove(index);
                    changed = true;
                } else if let Some(index) = move_up {
                    track.inserts.swap(index, index - 1);
                    changed = true;
                } else if let Some(index) = move_down {
                    track.inserts.swap(index, index + 1);
                    changed = true;
                }
                if let Some((id, destination)) = dragged
                    && let Some(source) = track.inserts.iter().position(|slot| slot.id == id)
                {
                    let slot = track.inserts.remove(source);
                    track.inserts.insert(destination, slot);
                    changed = true;
                }
                add_effect = widgets::button(ui, "+ Insert").clicked();
                if changed {
                    self.changed();
                }
            });
        if !open {
            self.effects_open = false;
        }
        if add_effect {
            self.open_effect_library(track_id, false);
        }
        if let Some(effect) = edit_effect {
            self.open_effect_editor(track_id, effect);
        }
    }

    pub fn routing_popup(&mut self, ctx: &egui::Context) {
        if !self.routing_open {
            return;
        }
        let Some(track_id) = self.selected_track else {
            self.routing_open = false;
            return;
        };
        let Some(track) = self.project.track(track_id) else {
            self.routing_open = false;
            return;
        };
        if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
            self.routing_open = false;
            return;
        }
        let track_name = track.name.clone();
        let mut create_send = false;
        let buses: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Bus && track.id != track_id)
            .map(|track| (track.id, track.name.clone()))
            .collect();
        let mut open = true;
        widgets::popup(format!("Routing · {track_name}"), ctx)
            .id(egui::Id::new("routing"))
            .open(&mut open)
            .default_width(620.0)
            .vscroll(true)
            .show(ctx, |ui| {
                label(ui, "SENDS");
                let mut remove = None;
                let mut changed = false;
                let track = self.project.track_mut(track_id).unwrap();
                for (index, send) in track.sends.iter_mut().enumerate() {
                    ui.push_id(send.target_bus, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                buses
                                    .iter()
                                    .find(|bus| bus.0 == send.target_bus)
                                    .map(|bus| bus.1.as_str())
                                    .unwrap_or("Unavailable bus"),
                            );
                            changed |= ui
                                .add(widgets::slider(&mut send.gain, 0.0..=1.5).text("Level"))
                                .changed();
                            changed |= widgets::radio_value(ui, &mut send.pre_fader, true, "Pre")
                                .on_hover_text("Send after inserts, before track gain and pan")
                                .changed();
                            changed |= widgets::radio_value(ui, &mut send.pre_fader, false, "Post")
                                .on_hover_text("Send after inserts, track gain, and pan")
                                .changed();
                            if widgets::small_button(ui, "×")
                                .on_hover_text("Remove send")
                                .clicked()
                            {
                                remove = Some(index);
                            }
                        });
                    });
                }
                if let Some(index) = remove {
                    track.sends.remove(index);
                    changed = true;
                }
                let mut add_bus = None;
                widgets::menu_button(ui, "+ Send", |ui| {
                    if buses.is_empty() {
                        ui.label("No buses available");
                    }
                    for (id, name) in &buses {
                        if widgets::button(ui, name).clicked() {
                            add_bus = Some(*id);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Send to a bus; set its effect to 100% wet for a shared return");
                if changed {
                    self.changed();
                }
                if let Some(bus) = add_bus {
                    let result = self.project.add_send(track_id, bus, 0.25, false);
                    if result.is_ok() {
                        self.changed();
                    }
                    self.message(result, "Send added");
                }

                create_send = widgets::button(ui, "Create send track").clicked();

                ui.add_space(16.0);
                label(ui, "OUTPUT");
                let current = self.project.track(track_id).unwrap().output_bus;
                let mut output = current;
                widgets::select(
                    "channel_output",
                    current
                        .and_then(|id| buses.iter().find(|bus| bus.0 == id))
                        .map(|bus| bus.1.as_str())
                        .unwrap_or("Master output"),
                )
                .show_ui(ui, |ui| {
                    widgets::selectable_value(ui, &mut output, None, "Master output");
                    for (id, name) in &buses {
                        widgets::selectable_value(ui, &mut output, Some(*id), name);
                    }
                })
                .response
                .on_hover_text(
                    "Routes cannot form feedback cycles; all paths are delay compensated",
                );
                if output != current {
                    let result = self.project.set_output(track_id, output);
                    if result.is_ok() {
                        self.changed();
                    }
                    self.message(result, "Output routing updated");
                }
            });
        if !open {
            self.routing_open = false;
        }
        if create_send {
            self.open_effect_library(track_id, true);
        }
    }
}

/// Preorder traversal preserves track order within each parent and hides descendants
/// of collapsed folders. The core model validates parent cycles before persistence.
pub fn visible_tracks(project: &Project) -> Vec<(usize, usize)> {
    fn visit(project: &Project, parent: Option<u64>, depth: usize, out: &mut Vec<(usize, usize)>) {
        if depth > 64 {
            return;
        }
        for (index, track) in project.tracks.iter().enumerate() {
            if track.parent_folder == parent {
                out.push((index, depth));
                if track.kind == TrackKind::Folder && !track.collapsed {
                    visit(project, Some(track.id), depth + 1, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    visit(project, None, 0, &mut out);
    out
}
