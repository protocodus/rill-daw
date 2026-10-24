use crate::{
    app::{DawApp, PopupKind},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Pos2, Rect, Sense, Vec2};

impl DawApp {
    pub fn editor(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if let Some(clip) = self.selected() {
                ui.label(egui::RichText::new(&clip.name).color(palette(ui.ctx()).accent));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if daw_ui::button(ui, "Properties").clicked() {
                    self.open_popup(PopupKind::Inspector);
                }
            });
        });
        ui.separator();
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
            .map(|t| (t.id, t.name.clone()))
            .collect();
        let meter = self
            .engine
            .as_ref()
            .map(|e| e.snapshot())
            .unwrap_or_default();
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (channel, track) in self
                        .project
                        .tracks
                        .iter_mut()
                        .filter(|t| !matches!(t.kind, TrackKind::Folder | TrackKind::Automation))
                        .enumerate()
                    {
                        daw_ui::card(ui.ctx()).inner_margin(12).show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(148.0);
                                ui.spacing_mut().slider_width = 116.0;
                                ui.label(
                                    egui::RichText::new(&track.name)
                                        .color(readable_track_color(ui.ctx(), track.color))
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Ch {} > {}",
                                        channel + 1,
                                        track
                                            .output_bus
                                            .and_then(|id| names.get(&id))
                                            .map(String::as_str)
                                            .unwrap_or("Master")
                                    ))
                                    .small()
                                    .color(palette(ui.ctx()).muted),
                                );
                                ui.horizontal(|ui| {
                                    changed |= ui
                                        .add(
                                            daw_ui::slider(&mut track.gain, 0.0..=1.5)
                                                .vertical()
                                                .show_value(false),
                                        )
                                        .changed();
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{:.1} dB",
                                                20.0 * track.gain.max(0.00001).log10()
                                            ))
                                            .monospace(),
                                        );
                                        changed |=
                                            daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                                        changed |=
                                            daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                                    });
                                });
                                changed |= ui
                                    .add(
                                        daw_ui::slider(&mut track.pan, -1.0..=1.0)
                                            .show_value(false)
                                            .text("Pan"),
                                    )
                                    .changed();
                                for (name, kind, visible) in [
                                    (
                                        "Instrument",
                                        PopupKind::Instrument,
                                        track.kind == TrackKind::Midi,
                                    ),
                                    ("Effects", PopupKind::Effects, true),
                                    ("Routing", PopupKind::Routing, true),
                                ] {
                                    if visible && daw_ui::button(ui, name).clicked() {
                                        self.selected_track = Some(track.id);
                                        popup = Some(kind);
                                    }
                                }
                            });
                        });
                    }
                    daw_ui::card(ui.ctx())
                        .fill(palette(ui.ctx()).panel)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(110.0);
                                ui.label(
                                    egui::RichText::new("Master")
                                        .strong()
                                        .color(palette(ui.ctx()).accent),
                                );
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::new(78.0, 116.0), Sense::hover());
                                let p = ui.painter_at(rect);
                                for (index, peak) in
                                    [meter.peak_left, meter.peak_right].iter().enumerate()
                                {
                                    let bar = Rect::from_min_size(
                                        rect.min + Vec2::new(index as f32 * 28.0 + 8.0, 0.0),
                                        Vec2::new(18.0, rect.height()),
                                    );
                                    p.rect_filled(bar, 2.0, palette(ui.ctx()).bg);
                                    let level = ((20.0 * peak.max(0.001).log10() + 60.0) / 60.0)
                                        .clamp(0.0, 1.0);
                                    p.rect_filled(
                                        Rect::from_min_max(
                                            Pos2::new(
                                                bar.left(),
                                                bar.bottom() - bar.height() * level,
                                            ),
                                            bar.max,
                                        ),
                                        2.0,
                                        if *peak >= 1.0 {
                                            palette(ui.ctx()).orange
                                        } else {
                                            palette(ui.ctx()).accent
                                        },
                                    );
                                }
                                ui.label(
                                    egui::RichText::new("L      R")
                                        .monospace()
                                        .color(palette(ui.ctx()).muted),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} Hz",
                                        self.settings.sample_rate
                                    ))
                                    .small()
                                    .color(palette(ui.ctx()).muted),
                                );
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
