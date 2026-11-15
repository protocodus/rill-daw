//! The shared sample player editor is a workspace, retaining its instrument
//! and pad while the user visits the sample browser or returns with Back.
use crate::app::{DawApp, PopupKind};
use eframe::egui;

#[derive(Default)]
pub(crate) struct SamplePlayerEditor {
    pub open: bool,
    track: u64,
    group: u32,
    session: u64,
    device: String,
    palette: bool,
}

impl DawApp {
    /// Expanding the palette carries only an explicitly selected sample leaf.
    pub(crate) fn sample_player_browser_target(&self) -> Option<(u64, u32)> {
        let editor = &self.sample_player_editor;
        let instrument = &self.project.track(editor.track)?.instrument;
        if editor.session != self.song_session.id {
            return None;
        }
        let selected = self.instrument_ui.device_panel.player.selected?;
        // Without a stored tree the editor displays one implicit Sample atom
        // for the group. It is a real selected destination, not a fallback to
        // an unrelated child of a composite player.
        let Some(root) = instrument.sample_player(editor.group) else {
            return Some((editor.track, editor.group));
        };
        match root.find(selected)?.kind {
            daw_core::PlayerKind::Sample { slot, .. } => Some((editor.track, slot)),
            _ => None,
        }
    }

    pub(crate) fn open_sample_player(&mut self, track: u64, group: u32) {
        let Some(device) = self
            .project
            .track(track)
            .and_then(|track| track.instrument.builtin_device())
            .filter(|device| {
                daw_devices::descriptor(device)
                    .is_some_and(|descriptor| group < descriptor.sample_slots)
            })
            .map(str::to_owned)
        else {
            return;
        };
        if self.sample_player_editor.track != track
            || self.sample_player_editor.group != group
            || self.sample_player_editor.session != self.song_session.id
            || self.sample_player_editor.device != device
        {
            self.instrument_ui.device_panel.player = Default::default();
        }
        self.open_popup(PopupKind::SamplePlayer);
        self.selected_track = Some(track);
        self.sample_player_editor = SamplePlayerEditor {
            open: true,
            track,
            group,
            session: self.song_session.id,
            device,
            palette: true,
        };
    }

    pub(crate) fn sample_player_page(&mut self, ctx: &egui::Context) {
        if !self.sample_player_editor.open {
            return;
        }
        let track_id = self.sample_player_editor.track;
        let group = self.sample_player_editor.group;
        let Some(track) = self.project.track(track_id).filter(|track| {
            self.sample_player_editor.session == self.song_session.id
                && track.instrument.builtin_device()
                    == Some(self.sample_player_editor.device.as_str())
        }) else {
            self.close_popup();
            return;
        };
        let label = track
            .instrument
            .sample_name(group)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if daw_devices::descriptor(&self.sample_player_editor.device)
                    .is_some_and(|descriptor| descriptor.sample_slots > 1)
                {
                    format!("Pad {}", group + 1)
                } else {
                    "Sample player".into()
                }
            });
        let title = format!("{label} · {}", track.name);
        let mut open = true;
        daw_ui::page(title)
            .id(egui::Id::new("sample_player_editor"))
            .open(&mut open)
            .show(ctx, |ui| {
                daw_ui::toolbar(ui, |ui| {
                    daw_ui::checkbox(ui, &mut self.sample_player_editor.palette, "Samples");
                });
                if self.sample_player_editor.palette {
                    egui::SidePanel::left("player_samples")
                        .default_width(260.0)
                        .min_width(220.0)
                        .max_width(360.0)
                        .show_inside(ui, |ui| self.sample_palette(ui));
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| self.draw_sample_player_panel(ui, track_id, group));
            });
        if !open {
            self.close_popup();
        }
    }
}
