//! Names are edited beside their item; drafts never mutate the document.
use crate::app::{DawApp, WorkspaceView};
use eframe::egui;

#[derive(Clone, Copy)]
enum RenameTarget {
    Clip(u64, u64),
    Track(u64),
}

#[derive(Clone)]
struct RenameDraft {
    target: RenameTarget,
    view: WorkspaceView,
    document: u64,
    text: String,
    initial_focus: bool,
}

fn state_id() -> egui::Id {
    egui::Id::new("inline_item_rename")
}

pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<RenameDraft>(state_id()));
    ctx.memory_mut(|memory| memory.surrender_focus(state_id().with("field")));
}

fn rect(value: &serde_json::Value) -> Option<egui::Rect> {
    Some(egui::Rect::from_min_max(
        egui::pos2(
            value["min"][0].as_f64()? as f32,
            value["min"][1].as_f64()? as f32,
        ),
        egui::pos2(
            value["max"][0].as_f64()? as f32,
            value["max"][1].as_f64()? as f32,
        ),
    ))
}

impl DawApp {
    pub(crate) fn apply_precise_clip_timing(
        &mut self,
        edit: daw_ui::ClipTimingEdit,
    ) -> anyhow::Result<()> {
        use anyhow::{Context, ensure};
        use daw_core::{AudioData, ClipContent};
        let (track, clip) = self
            .selected_track
            .zip(self.selected_clip)
            .context("Select a clip")?;
        let mut candidate = self.project.clone();
        let mut label = "Edit clip timing";
        if let Some(start) = edit.start {
            candidate.move_clip(track, clip, start)?;
            label = "Move clip";
        }
        if let Some((duration, mode)) = edit.duration {
            match mode {
                daw_ui::DurationEditMode::Trim => {
                    let start = candidate.clip(track, clip).context("Clip not found")?.start;
                    candidate.trim_clip(track, clip, start, duration)?;
                    label = "Trim clip duration";
                }
                daw_ui::DurationEditMode::Stretch => {
                    candidate.stretch_clip(track, clip, duration)?;
                    label = "Stretch clip duration";
                }
            }
        }
        if let Some(offset) = edit.source_offset {
            let original = candidate.clip(track, clip).context("Clip not found")?;
            ensure!(
                !original.is_take_source(),
                "Choose a take before editing it"
            );
            ensure!(
                offset.is_finite() && offset >= 0.0,
                "Source offset must be finite and nonnegative"
            );
            if let ClipContent::Audio { path, reverse, .. } = &original.content {
                let seconds = if let Some(source) = self.waveforms.get(path) {
                    source.duration_seconds()
                } else {
                    AudioData::wav_duration_seconds(path)?
                };
                let source_beats = seconds * candidate.tempo / 60.0;
                ensure!(
                    offset + original.source_length() <= source_beats + 1e-9,
                    "Source offset would extend past the audio source"
                );
                let direction = if *reverse { 1.0 } else { -1.0 };
                let movement = (offset - original.offset) / (original.source_rate * direction);
                candidate.slip_audio_clip(track, clip, movement, source_beats)?;
            } else {
                candidate.clip_mut(track, clip).unwrap().offset = offset;
            }
            label = "Slip clip source";
        }
        candidate.synchronize_linked_clip(track, clip)?;
        candidate.validate()?;
        self.project = candidate;
        self.edit_label = Some(label.into());
        Ok(())
    }

    /// Active text editing owns Escape and document editing keys until its
    /// native field sees them. Call before the workspace's global shortcuts.
    pub(crate) fn rename_active(&self, ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<RenameDraft>(state_id()).is_some())
    }

    /// Document shortcuts finish the name before taking their project snapshot.
    pub(crate) fn finish_inline_rename(&mut self, ctx: &egui::Context, commit: bool) {
        let draft = ctx.data_mut(|data| {
            let draft = data.get_temp::<RenameDraft>(state_id());
            data.remove::<RenameDraft>(state_id());
            draft
        });
        ctx.memory_mut(|memory| memory.surrender_focus(state_id().with("field")));
        if let Some(draft) = draft
            && commit
            && draft.view == self.view
            && draft.document == crate::spectrogram::waveform_generation(ctx)
        {
            self.commit_inline_name(draft);
        }
    }

    fn commit_inline_name(&mut self, draft: RenameDraft) {
        if draft.text.trim().is_empty() {
            return;
        }
        self.flush_pending_edit();
        let unchanged = match draft.target {
            RenameTarget::Clip(track, clip) => self
                .project
                .clip(track, clip)
                .is_none_or(|clip| clip.name == draft.text),
            RenameTarget::Track(track) => self
                .project
                .track(track)
                .is_none_or(|track| track.name == draft.text),
        };
        if unchanged {
            return;
        }
        let mut candidate = self.project.clone();
        let (result, label) = match draft.target {
            RenameTarget::Clip(track, clip) => {
                candidate.clip_mut(track, clip).unwrap().name = draft.text;
                (
                    candidate.synchronize_linked_clip(track, clip),
                    "Rename clip",
                )
            }
            RenameTarget::Track(track) => {
                candidate.track_mut(track).unwrap().name = draft.text;
                (Ok(()), "Rename track")
            }
        };
        match result.and_then(|()| candidate.validate()) {
            Ok(()) => {
                self.begin_edit();
                self.project = candidate;
                self.edit_label = Some(label.into());
                self.changed();
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub(crate) fn rename_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if ctx.wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
            return false;
        }
        if !ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
            return false;
        }
        self.start_inline_rename(ctx);
        true
    }

    pub(crate) fn start_inline_rename(&mut self, ctx: &egui::Context) {
        let target = if self.view != WorkspaceView::Mixer {
            self.selected_track
                .zip(self.selected_clip)
                .and_then(|(track, clip)| {
                    self.project
                        .clip(track, clip)
                        .filter(|clip| !clip.is_take_source())
                        .map(|clip| (RenameTarget::Clip(track, clip.id), clip.name.clone()))
                })
        } else {
            None
        }
        .or_else(|| {
            self.selected_track.and_then(|id| {
                self.project
                    .track(id)
                    .map(|track| (RenameTarget::Track(id), track.name.clone()))
            })
        });
        if let Some((target, text)) = target {
            let document = crate::spectrogram::waveform_generation(ctx);
            ctx.data_mut(|data| {
                data.insert_temp(
                    state_id(),
                    RenameDraft {
                        target,
                        text,
                        view: self.view,
                        document,
                        initial_focus: true,
                    },
                )
            });
        }
    }

    pub(crate) fn rename_overlay(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = ctx.data_mut(|data| {
            let draft = data.get_temp::<RenameDraft>(state_id());
            data.remove::<RenameDraft>(state_id());
            draft
        }) else {
            return;
        };
        if draft.view != self.view
            || draft.document != crate::spectrogram::waveform_generation(ctx)
            || self.document_guard.is_some()
            || self.error_details.is_some()
            || self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
            || self.history_open
            || self.crossfade_open
            || self.join_open
            || self.audio_takes_open
            || self.recording_open
            || self.export_open
            || self.about_open
        {
            return;
        }
        let name = match draft.target {
            RenameTarget::Clip(track, clip) => {
                self.project.clip(track, clip).map(|clip| &clip.name)
            }
            RenameTarget::Track(track) => self.project.track(track).map(|track| &track.name),
        };
        if name.is_none() {
            return;
        }
        let anchor = match (draft.view, draft.target) {
            (WorkspaceView::Arrangement, target) => ctx.data(|data| {
                let timeline =
                    data.get_temp::<serde_json::Value>(egui::Id::new("timeline_diagnostics"))?;
                match target {
                    RenameTarget::Clip(_, id) => timeline["clips"]
                        .as_array()?
                        .iter()
                        .find(|clip| clip["id"] == id)
                        .and_then(|clip| rect(&clip["visible_rect"]))
                        .map(|rect| {
                            egui::Rect::from_min_size(
                                rect.min + egui::vec2(4.0, 0.0),
                                egui::vec2(rect.width().max(120.0), 20.0),
                            )
                        }),
                    RenameTarget::Track(id) => timeline["tracks"]
                        .as_array()?
                        .iter()
                        .find(|track| track["track"] == id)
                        .and_then(|track| rect(&track["name"])),
                }
            }),
            (WorkspaceView::Mixer, RenameTarget::Track(id)) => ctx.data(|data| {
                let mixer =
                    data.get_temp::<serde_json::Value>(egui::Id::new("mixer_diagnostics"))?;
                mixer["channels"]
                    .as_array()?
                    .iter()
                    .find(|strip| strip["track"] == id)
                    .and_then(|strip| rect(&strip["name"]))
            }),
            _ => {
                ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("workspace_title_rect")))
            }
        };
        let Some(anchor) = anchor.filter(|anchor| anchor.is_positive()) else {
            return;
        };
        let width = anchor.width().clamp(120.0, 360.0);
        let content = ctx.content_rect().shrink(4.0);
        let position = egui::pos2(
            anchor.left().clamp(
                content.left(),
                (content.right() - width - 172.0).max(content.left()),
            ),
            (anchor.center().y - daw_ui::tokens::CONTROL_HEIGHT * 0.5).clamp(
                content.top(),
                content.bottom() - daw_ui::tokens::CONTROL_HEIGHT,
            ),
        );
        let field_id = state_id().with("field");
        let response = egui::Area::new(state_id().with("area"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                daw_ui::inline_name_editor(
                    ui,
                    field_id,
                    &mut draft.text,
                    if matches!(draft.target, RenameTarget::Clip(..)) {
                        "Clip name"
                    } else {
                        "Track name"
                    },
                    width,
                    draft.initial_focus,
                )
            })
            .inner;
        match response.action {
            daw_ui::InlineNameAction::Editing => {
                draft.initial_focus = false;
                ctx.data_mut(|data| data.insert_temp(state_id(), draft));
            }
            action => {
                ctx.memory_mut(|memory| memory.surrender_focus(field_id));
                if action == daw_ui::InlineNameAction::Commit {
                    self.commit_inline_name(draft);
                }
            }
        }
    }
}
