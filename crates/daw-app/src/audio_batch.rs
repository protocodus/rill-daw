//! Parameter-only batch smoothing with occurrence-local, source-safe fades.
use crate::app::DawApp;
use daw_core::{Clip, ClipContent};
use eframe::egui;

const DIALOG: &str = "audio_boundary_fades_dialog";
const SETTINGS: &str = "audio_boundary_fades_settings";

#[derive(Clone)]
struct Dialog {
    clips: Vec<(u64, Clip)>,
    tempo: f64,
    settings: daw_ui::BoundaryFadeSettings,
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(DIALOG)).is_some())
}

pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Dialog>(egui::Id::new(DIALOG));
    });
}

pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(DIALOG)))
        .map_or(serde_json::Value::Null, |state| serde_json::json!({
            "clips":state.clips.iter().map(|(track,clip)| (*track,clip.id)).collect::<Vec<_>>(),
            "fade_in_ms":state.settings.fade_in_ms,"fade_out_ms":state.settings.fade_out_ms,
            "controls":ctx.data(|data| data.get_temp::<Vec<[f32;4]>>(egui::Id::new("audio_boundary_fades_controls")))
        }))
}

impl DawApp {
    pub(crate) fn selected_audio_boundaries(&self) -> Vec<(u64, u64)> {
        self.arranger_selection_ids()
            .into_iter()
            .filter(|&(track, id)| {
                self.project.clip(track, id).is_some_and(|clip| {
                    matches!(clip.content, ClipContent::Audio { .. }) && !clip.is_take_source()
                })
            })
            .collect()
    }

    /// One action uses the last accepted settings; settings never create history.
    pub(crate) fn smooth_audio_boundaries(&mut self, ctx: &egui::Context) {
        let selected = self.selected_audio_boundaries();
        let settings = ctx
            .data(|data| data.get_temp::<daw_ui::BoundaryFadeSettings>(egui::Id::new(SETTINGS)))
            .unwrap_or_default();
        self.apply_audio_boundary_fades(&selected, Some(settings));
    }

    pub(crate) fn open_audio_batch_fades(&mut self, ctx: &egui::Context) {
        self.flush_pending_edit();
        self.history.seal_group();
        let clips: Vec<_> = self
            .selected_audio_boundaries()
            .into_iter()
            .filter_map(|(track, id)| {
                self.project
                    .clip(track, id)
                    .cloned()
                    .map(|clip| (track, clip))
            })
            .collect();
        if clips.is_empty() {
            return;
        }
        let settings = ctx
            .data(|data| data.get_temp::<daw_ui::BoundaryFadeSettings>(egui::Id::new(SETTINGS)))
            .unwrap_or_default();
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(DIALOG),
                Dialog {
                    clips,
                    tempo: self.project.tempo,
                    settings,
                },
            )
        });
    }

    fn apply_audio_boundary_fades(
        &mut self,
        clips: &[(u64, u64)],
        settings: Option<daw_ui::BoundaryFadeSettings>,
    ) {
        if clips.is_empty() {
            return;
        }
        self.flush_pending_edit();
        self.history.seal_group();
        self.begin_edit();
        let result = match settings {
            Some(settings) => self.project.smooth_audio_boundaries(
                clips,
                settings.fade_in_ms,
                settings.fade_out_ms,
            ),
            None => self.project.clear_audio_boundary_fades(clips),
        };
        match result {
            Ok(changed) => {
                if changed > 0 {
                    self.edit_label = Some(
                        if settings.is_some() {
                            "Smooth edit boundaries"
                        } else {
                            "Remove boundary fades"
                        }
                        .into(),
                    );
                    self.changed();
                }
                self.status = if changed == 0 {
                    "Selected boundaries are unchanged".into()
                } else if settings.is_some() {
                    format!(
                        "Smoothed {changed} audio {}",
                        if changed == 1 { "clip" } else { "clips" }
                    )
                } else {
                    format!(
                        "Removed boundary fades from {changed} audio {}",
                        if changed == 1 { "clip" } else { "clips" }
                    )
                };
            }
            Err(error) => {
                self.message(Err(error), "");
            }
        }
        self.flush_pending_edit();
    }

    pub(crate) fn audio_batch_fades_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut state) = ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(DIALOG)))
        else {
            return;
        };
        if self.document_guard.is_some()
            || self.project.tempo != state.tempo
            || state
                .clips
                .iter()
                .any(|(track, clip)| self.project.clip(*track, clip.id) != Some(clip))
        {
            reset(ctx);
            return;
        }
        let mut apply = false;
        let mut cancel = false;
        let mut remove = false;
        let response = daw_ui::modal(
            ctx,
            egui::Id::new("audio_boundary_fades_modal"),
            "Smooth edit boundaries",
            |ui| {
                daw_ui::text::body(
                    ui,
                    format!(
                        "{} selected audio {}",
                        state.clips.len(),
                        if state.clips.len() == 1 {
                            "clip"
                        } else {
                            "clips"
                        }
                    ),
                );
                let controls = daw_ui::boundary_fade_settings(ui, &mut state.settings);
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("audio_boundary_fades_controls"),
                        controls
                            .iter()
                            .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
                            .collect::<Vec<_>>(),
                    )
                });
                daw_ui::text::note(
                    ui,
                    "Existing musical fades stay intact. Only selected occurrences change.",
                );
                ui.add_space((daw_ui::tokens::SPACE_LG - ui.spacing().item_spacing.y).max(0.0));
                ui.horizontal_wrapped(|ui| {
                    cancel = daw_ui::button(ui, "Cancel").clicked();
                    remove = daw_ui::button(ui, "Remove boundary fades").clicked();
                    apply = daw_ui::button(ui, "Apply boundary fades").clicked();
                });
            },
        );
        if apply || remove {
            let clips = state
                .clips
                .iter()
                .map(|(track, clip)| (*track, clip.id))
                .collect::<Vec<_>>();
            if apply {
                ctx.data_mut(|data| data.insert_temp(egui::Id::new(SETTINGS), state.settings));
            }
            reset(ctx);
            self.apply_audio_boundary_fades(&clips, apply.then_some(state.settings));
        } else if cancel || response.should_close() {
            reset(ctx);
        } else {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new(DIALOG), state));
        }
    }
}
