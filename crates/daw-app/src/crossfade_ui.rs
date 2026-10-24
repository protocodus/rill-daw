use crate::app::DawApp;
use daw_core::{AudioCrossfade, AudioSchedule, ClipContent, CrossfadeCurve, MAX_BEAT};
use eframe::egui;

impl DawApp {
    pub(crate) fn crossfade_popup(&mut self, ctx: &egui::Context) {
        if !self.crossfade_open {
            return;
        }
        let selection = self
            .selected_track
            .zip(self.selected_clip)
            .and_then(|(track, id)| {
                let channel = self.project.track(track)?;
                let index = channel.clips.iter().position(|clip| clip.id == id)?;
                let ClipContent::Audio { crossfade, .. } = channel.clips[index].content else {
                    return None;
                };
                let schedule = AudioSchedule::new(&channel.clips).ok()?;
                let overlap = schedule
                    .overlaps
                    .iter()
                    .find(|overlap| overlap.incoming == index);
                Some((
                    track,
                    id,
                    crossfade,
                    channel.clips[index].name.clone(),
                    overlap.map(|overlap| {
                        (
                            channel.clips[overlap.outgoing].name.clone(),
                            overlap.start,
                            overlap.end,
                        )
                    }),
                ))
            });
        let mut open = true;
        daw_ui::popup("Crossfade", ctx).open(&mut open).default_width(460.0).show(ctx, |ui| {
            let Some((track, clip, original, name, overlap)) = &selection else {
                daw_ui::text::note(ui, "Select an audio clip.");
                return;
            };
            daw_ui::text::subtitle(ui, name);
            if let Some((outgoing, _, _)) = overlap {
                daw_ui::text::note(ui, format!("{outgoing} → {name}"));
            } else {
                daw_ui::text::note(ui, "No earlier clip overlaps this start.");
            }
            let mut enabled = original.is_some();
            daw_ui::switch(ui, &mut enabled, "Enable crossfade")
                .on_hover_text("Blend from the preceding clip at this clip’s start. Disabled: switch immediately.");
            let mut config = original.unwrap_or(AudioCrossfade {
                duration: overlap.as_ref().map_or(1.0, |(_, start, end)| end - start),
                curve: CrossfadeCurve::LINEAR,
            });
            ui.add_enabled_ui(enabled, |ui| {
                let duration = ui.horizontal(|ui| {
                    daw_ui::text::label(ui, "Duration");
                    ui.add(daw_ui::number(&mut config.duration).speed(0.05).range(0.001..=MAX_BEAT).suffix(" beats"))
                        .on_hover_text("Automatically limited to the available overlap and next clip takeover.")
                }).inner;
                ui.horizontal(|ui| {
                    if daw_ui::button(ui, "Linear").clicked() { config.curve = CrossfadeCurve::LINEAR; }
                    if daw_ui::button(ui, "Smooth").clicked() { config.curve = CrossfadeCurve::SMOOTH; }
                });
                let mut curve = daw_ui::CrossfadeCurve { control1: config.curve.control1, control2: config.curve.control2 };
                let editor = daw_ui::crossfade_editor(ui, egui::Id::new(("crossfade", track, clip)), &mut curve);
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("crossfade_curve_focus"), editor.focus_ids));
                config.curve = CrossfadeCurve { control1: curve.control1, control2: curve.control2 };
                let rect = |rect: egui::Rect| serde_json::json!({"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]});
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("crossfade_diagnostics"), serde_json::json!({
                    "plot": rect(editor.plot_rect), "handles": editor.handles.map(rect), "duration": rect(duration.rect),
                    "effective": overlap.as_ref().map(|(_, start, end)| [*start, (*start + config.duration).min(*end)]),
                })));
            });
            if enabled && let Some((_, start, end)) = overlap {
                daw_ui::text::note(ui, format!("Effective overlap: {:.2} beats", config.duration.min(end - start)));
            }
            let next = enabled.then_some(config);
            if next != *original {
                self.begin_edit();
                match self.project.set_audio_crossfade(*track, *clip, next) {
                    Ok(()) => { self.edit_label = Some("Edit crossfade".into()); self.changed(); }
                    Err(error) => self.message(Err(error), ""),
                }
            }
        });
        self.crossfade_open &= open;
    }
}
