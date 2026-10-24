use crate::{
    app::{DawApp, PopupKind},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Vec2};

impl DawApp {
    pub fn editor(&mut self, ui: &mut egui::Ui) {
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
            return;
        }
        self.piano_roll(ui);
    }
    pub(crate) fn mixer(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut popup = None;
        let names: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .map(|track| (track.id, track.name.clone()))
            .collect();
        let meter = self
            .engine
            .as_ref()
            .map(|engine| engine.snapshot())
            .unwrap_or_default();
        let strip_width = 180.0;
        let content_width = strip_width - 2.0 * daw_ui::tokens::SPACE_MD;
        let fader_height = 180.0;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (channel, track) in self
                        .project
                        .tracks
                        .iter_mut()
                        .filter(|track| {
                            !matches!(track.kind, TrackKind::Folder | TrackKind::Automation)
                        })
                        .enumerate()
                    {
                        let route = format!(
                            "Ch {} > {}",
                            channel + 1,
                            track
                                .output_bus
                                .and_then(|id| names.get(&id))
                                .map(String::as_str)
                                .unwrap_or("Master")
                        );
                        daw_ui::mixer_strip(
                            ui,
                            track.id,
                            strip_width,
                            Some(color(track.color)),
                            self.selected_track == Some(track.id),
                            |ui| {
                                ui.set_min_height(420.0);
                                if ui
                                    .add_sized(
                                        Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                                        daw_ui::button_widget(
                                            daw_ui::text::TextRole::Label
                                                .rich(ui.ctx(), &track.name),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&track.name)
                                    .clicked()
                                {
                                    self.selected_track = Some(track.id);
                                }
                                if ui
                                    .add_sized(
                                        Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                                        daw_ui::button_widget(egui::RichText::new(&route).small())
                                            .truncate(),
                                    )
                                    .on_hover_text("Output and sends")
                                    .clicked()
                                {
                                    self.selected_track = Some(track.id);
                                    popup = Some(PopupKind::Routing);
                                }
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{:.1} dB",
                                            20.0 * track.gain.max(0.00001).log10()
                                        ))
                                        .monospace(),
                                    );
                                    ui.spacing_mut().slider_width = fader_height;
                                    changed |= ui
                                        .add_sized(
                                            Vec2::new(30.0, fader_height),
                                            daw_ui::slider(&mut track.gain, 0.0..=1.5)
                                                .vertical()
                                                .show_value(false),
                                        )
                                        .on_hover_text("Channel volume")
                                        .changed();
                                });
                                ui.spacing_mut().slider_width = content_width - 38.0;
                                changed |= ui
                                    .add(
                                        daw_ui::slider(&mut track.pan, -1.0..=1.0)
                                            .show_value(false)
                                            .text("Pan"),
                                    )
                                    .on_hover_text("Stereo balance")
                                    .changed();
                                ui.horizontal(|ui| {
                                    changed |=
                                        daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                                    changed |=
                                        daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                                });
                                daw_ui::action_menu(ui, "Channel", |ui| {
                                    for (label, kind, visible) in [
                                        (
                                            "Instrument",
                                            PopupKind::Instrument,
                                            track.kind == TrackKind::Midi,
                                        ),
                                        ("Effects", PopupKind::Effects, true),
                                        ("Routing", PopupKind::Routing, true),
                                    ] {
                                        if visible && daw_ui::button(ui, label).clicked() {
                                            self.selected_track = Some(track.id);
                                            popup = Some(kind);
                                            ui.close();
                                        }
                                    }
                                });
                            },
                        );
                    }
                    daw_ui::mixer_strip(ui, "master", strip_width, None, false, |ui| {
                        ui.set_min_height(420.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                daw_ui::text::label(ui, "Master");
                            },
                        );
                        ui.allocate_space(Vec2::new(content_width, daw_ui::tokens::CONTROL_HEIGHT));
                        ui.vertical_centered(|ui| {
                            let peak = meter.peak_left.max(meter.peak_right);
                            ui.label(
                                egui::RichText::new(if peak > 0.00001 {
                                    format!("{:.1} dB", 20.0 * peak.log10())
                                } else {
                                    "−∞ dB".to_owned()
                                })
                                .monospace(),
                            );
                            daw_ui::stereo_meter(
                                ui,
                                [meter.peak_left, meter.peak_right],
                                Vec2::new(56.0, fader_height),
                            );
                            daw_ui::text::note(ui, "Stereo output");
                        });
                    });
                });
            });
        if changed {
            self.changed();
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
    }

    pub fn instrument_editor(&mut self, ctx: &egui::Context) {
        self.plugin_instrument_editor(ctx);
    }
}
