use crate::app::DawApp;
use daw_core::{ClipContent, Project};
use eframe::egui;

impl DawApp {
    pub(crate) fn apply_history_limit(&mut self) {
        self.history.set_step_limit(self.settings.history_limit);
        self.selection_history
            .retain(self.history.retained_state_ids());
    }

    pub(crate) fn flush_pending_edit(&mut self) {
        // Save/Undo can explicitly commit while the pointer is still held.
        // End that gesture before overlap merging replaces any of its IDs.
        self.midi_editor.end_edit_gesture();
        self.finish_pending_midi_overlap_merges();
        let before = self.before_edit.take();
        let before_selection = self.before_selection.take();
        let preview_submitted = std::mem::take(&mut self.playback_preview_submitted);
        let preview_matches_current = std::mem::take(&mut self.playback_preview_matches_current);
        self.playback_preview_dirty = false;
        let label = self.edit_label.take();
        let group = self.edit_group.take();
        if !std::mem::take(&mut self.edit_dirty) {
            return;
        }
        let audio_changed = if preview_submitted {
            !preview_matches_current
        } else {
            before
                .as_ref()
                .is_none_or(|before| !crate::app::audio_equivalent(before, &self.project))
        };
        if let Some(before) = before
            && before != self.project
        {
            let before_state = self.history.state_id();
            self.song_session.bump();
            let label = label.unwrap_or_else(|| edit_name(&before, &self.project).into());
            if let Some(group) = group {
                self.history.commit_grouped(&before, label, group);
            } else {
                self.history.commit_named(&before, label);
            }
            if let Some(selection) = before_selection {
                let after = self.capture_selection();
                self.selection_history.record(
                    (before_state, selection),
                    (self.history.state_id(), after),
                    self.history.retained_state_ids(),
                );
            }
        }
        self.modified = !self.history.is_saved();
        self.checkpoint_committed_work();
        if audio_changed {
            self.revision = self.revision.wrapping_add(1);
            self.update_playback();
        }
    }

    pub(crate) fn history_popup(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let mut open = true;
        let mut jump = None;
        let undo: Vec<_> = self.history.undo_labels().map(str::to_owned).collect();
        let redo: Vec<_> = self.history.redo_labels().map(str::to_owned).collect();
        daw_ui::popup("History", ctx)
            .open(&mut open)
            .default_width(daw_ui::tokens::POPUP_WIDTH_SM)
            .vscroll(false)
            .show(ctx, |ui| {
                daw_ui::text::body(ui, format!("{} undo · {} redo", undo.len(), redo.len()));
                let mut footer = daw_ui::popup_footer(ui, "history_status");
                let window_ui = ui;
                let ui = footer.body();
                daw_ui::popup_scroll_body(ui, "history_steps", 0.0, |ui| {
                    // A list of steps, newest first: undone steps (muted) above
                    // the "Current" marker, applied steps below it. Clicking a
                    // row returns the song to the state before that step.
                    let muted = daw_ui::theme::palette(ui.ctx()).muted;
                    let row = |ui: &mut egui::Ui, label: &str, color: Option<egui::Color32>| {
                        let mut button = daw_ui::IconButton::tab(false, label).truncate();
                        if let Some(color) = color {
                            button = button.label_color(color);
                        }
                        ui.add_sized(
                            egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT),
                            button,
                        )
                    };
                    ui.add_enabled_ui(!self.state_sync_pending(), |ui| {
                        for (index, label) in redo.iter().enumerate().rev() {
                            if row(ui, label, Some(muted))
                                .on_hover_text(format!("Redo through {label}"))
                                .clicked()
                            {
                                jump = Some((true, index + 1));
                            }
                        }
                        if !undo.is_empty() || !redo.is_empty() {
                            ui.horizontal(|ui| {
                                daw_ui::text::note(ui, "Current");
                                ui.add(egui::Separator::default().horizontal());
                            });
                        }
                        for (index, label) in undo.iter().enumerate() {
                            if row(ui, label, None)
                                .on_hover_text(format!("Undo {label} and later steps"))
                                .clicked()
                            {
                                jump = Some((false, index + 1));
                            }
                        }
                    });
                    if undo.is_empty() && redo.is_empty() {
                        daw_ui::text::note(ui, "No edits yet");
                    }
                });
                footer.show(window_ui, |ui| {
                    daw_ui::text::note(
                        ui,
                        format!(
                            "Up to {} actions · {:.1} / 128 MiB",
                            self.history.step_limit(),
                            self.history.memory_bytes() as f64 / 1_048_576.0
                        ),
                    )
                    .on_hover_text("Manual and AI edits share this history. The 128 MiB safety cap may retain fewer actions, especially with large plug-in states. Change the action count in Setup → Storage.")
                });
            });
        self.history_open &= open;
        if let Some((redo, steps)) = jump {
            self.undo_redo_steps(redo, steps);
        }
    }
}

fn edit_name(before: &Project, after: &Project) -> &'static str {
    if before.tracks.len() < after.tracks.len() {
        return "Add track";
    }
    if before.tracks.len() > after.tracks.len() {
        return "Delete track";
    }
    if before.name != after.name {
        return "Rename project";
    }
    if before.tempo != after.tempo {
        return "Change tempo";
    }
    if before.loop_enabled != after.loop_enabled
        || before.loop_start != after.loop_start
        || before.loop_end != after.loop_end
    {
        return "Change loop";
    }
    for (before, after) in before.channels().zip(after.channels()) {
        if before.id != after.id {
            return "Move track";
        }
        if before.name != after.name {
            return "Rename track";
        }
        if before.color != after.color {
            return "Track color";
        }
        if before.parent_folder != after.parent_folder || before.collapsed != after.collapsed {
            return "Track folder";
        }
        if before.instrument != after.instrument {
            return "Change instrument";
        }
        if before.inserts.len() < after.inserts.len() {
            return "Add effect";
        }
        if before.inserts.len() > after.inserts.len() {
            return "Remove effect";
        }
        if before.inserts != after.inserts {
            return "Edit effect chain";
        }
        if before.output_bus != after.output_bus
            || before.sends != after.sends
            || before.instrument_inputs != after.instrument_inputs
            || before.instrument_output != after.instrument_output
        {
            return "Change routing";
        }
        if before.input_gain != after.input_gain {
            return "Channel input gain";
        }
        if before.gain != after.gain {
            return "Track volume";
        }
        if before.pan != after.pan {
            return "Track pan";
        }
        if before.mute != after.mute || before.solo != after.solo {
            return "Mute / solo";
        }
        if before.automation != after.automation {
            return "Edit automation";
        }
        if before.clips.len() < after.clips.len() {
            return "Add / split clip";
        }
        if before.clips.len() > after.clips.len() {
            return "Remove / glue clip";
        }
        for (before, after) in before.clips.iter().zip(&after.clips) {
            if before == after {
                continue;
            }
            if let (ClipContent::Midi { notes: a }, ClipContent::Midi { notes: b }) =
                (&before.content, &after.content)
            {
                if a.len() < b.len() {
                    return "Add notes";
                }
                if a.len() > b.len() {
                    return "Delete notes";
                }
                if a != b {
                    return "Edit notes";
                }
            }
            if before.content != after.content {
                return "Edit audio";
            }
            return "Edit clip";
        }
        if before.instrument_state != after.instrument_state
            || before.instrument_parameters != after.instrument_parameters
        {
            return "Instrument parameter";
        }
    }
    "Edit project"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_history_limit_prunes_selection_states_without_editing_the_song() {
        let context = egui::Context::default();
        let mut app = DawApp::with_context(&context, Project::new(), None);
        let mut states = Vec::new();
        for index in 0..3 {
            states.push(app.history.state_id());
            app.begin_edit();
            let track = app.project.add_audio_track(format!("Track {index}"));
            app.selected_track = Some(track);
            app.changed();
            app.flush_pending_edit();
        }
        app.history.mark_saved_at(app.history.state_id());
        app.modified = false;
        let project = app.project.clone();
        let state = app.history.state_id();
        let session = app.song_session;
        app.settings.history_limit = 1;
        app.apply_history_limit();
        assert_eq!(app.project, project);
        assert_eq!(app.history.state_id(), state);
        assert_eq!(app.song_session, session);
        assert!(app.history.is_saved());
        assert!(!app.modified);
        assert!(app.selection_history.get(states[0]).is_none());
        assert!(app.selection_history.get(states[1]).is_none());
        assert!(app.selection_history.get(states[2]).is_some());
        assert!(app.selection_history.get(state).is_some());
        app.undo_redo_steps(false, 1);
        assert_eq!(app.selected_track, Some(project.tracks[1].id));
        assert!(app.project.track(app.selected_track.unwrap()).is_some());
        app.undo_redo_steps(true, 1);
        assert_eq!(app.selected_track, Some(project.tracks[2].id));
        assert!(app.history.is_saved());
        assert!(!app.modified);
    }

    #[test]
    fn returning_a_gesture_to_its_original_value_keeps_saved_state_and_history() {
        let context = egui::Context::default();
        let mut app = DawApp::with_context(&context, Project::new(), None);
        let before = app.project.clone();
        let revision = app.revision;
        app.begin_edit();
        app.project.tempo = 130.0;
        app.changed();
        app.project.tempo = before.tempo;
        app.changed();
        app.flush_pending_edit();
        assert_eq!(app.project, before);
        assert!(app.history.is_saved());
        assert!(!app.modified);
        assert!(!app.history.can_undo());
        assert_eq!(app.revision, revision);
    }
}
