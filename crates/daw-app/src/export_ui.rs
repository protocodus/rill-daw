//! A focused export decision; rendering and native plug-in snapshots stay asynchronous.
use crate::{
    app::{DawApp, PopupKind},
    document_ui::DocumentDialog,
};
use eframe::egui;

impl DawApp {
    pub(crate) fn export(&mut self) {
        if self.export_job.is_some() {
            self.status = "A mixdown is already rendering".into();
            return;
        }
        self.open_popup(PopupKind::Export);
    }

    pub(crate) fn export_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.export_open;
        if !open {
            return;
        }
        let duration = self.project.end_beat() * 60.0 / self.project.tempo;
        let milliseconds = (duration * 1000.0).round() as u64;
        let duration = format!(
            "Arrangement · {}:{:02}.{:03}",
            milliseconds / 60_000,
            milliseconds / 1000 % 60,
            milliseconds % 1000,
        );
        let mut export = false;
        let mut cancel = false;
        daw_ui::popup("Export WAV", ctx)
            .id(egui::Id::new("export_options"))
            .default_width(360.0)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                daw_ui::text::body(ui, duration);
                ui.add_space(daw_ui::tokens::SPACE_SM);
                daw_ui::checkbox(ui, &mut self.export_include_effect_tails, "Include effect tails")
                    .on_hover_text("Keep instrument releases and effect tails after the arrangement. Turn off to end exactly at the arrangement boundary.");
                ui.add_space(daw_ui::tokens::SPACE_MD);
                ui.horizontal(|ui| {
                    export = daw_ui::primary_button(ui, "Export…").clicked();
                    cancel = daw_ui::button(ui, "Cancel").clicked();
                });
            });
        self.export_open = open && !cancel;
        if export {
            let include_effect_tails = self.export_include_effect_tails;
            self.close_popup();
            if let Some(path) = self.pick_document_path(DocumentDialog::Export) {
                self.export_to_with_tails(path, include_effect_tails);
            }
        }
    }
}
