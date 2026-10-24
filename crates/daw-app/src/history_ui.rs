use crate::app::DawApp;
use daw_core::{ClipContent, Project};
use eframe::egui;

impl DawApp {
    pub(crate) fn flush_pending_edit(&mut self) {
        let before = self.before_edit.take();
        let label = self.edit_label.take();
        let group = self.edit_group.take();
        if !std::mem::take(&mut self.edit_dirty) {
            return;
        }
        let audio_changed = before
            .as_ref()
            .is_none_or(|before| !crate::app::audio_equivalent(before, &self.project));
        if let Some(before) = before
            && before != self.project
        {
            let label = label.unwrap_or_else(|| edit_name(&before, &self.project).into());
            if let Some(group) = group {
                self.history.commit_grouped(&before, label, group);
            } else {
                self.history.commit_named(&before, label);
            }
        }
        self.modified = !self.history.is_saved();
        if audio_changed {
            self.revision = self.revision.wrapping_add(1);
            self.rebuild();
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
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.label(format!("{} undo · {} redo", undo.len(), redo.len()));
                egui::ScrollArea::vertical()
                    .max_height(440.0)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.state_sync_pending(), |ui| {
                            for (index, label) in redo.iter().enumerate().rev() {
                                if daw_ui::button(ui, format!("Redo · {label}")).clicked() {
                                    jump = Some((true, index + 1));
                                }
                            }
                            if !redo.is_empty() {
                                ui.separator();
                            }
                            for (index, label) in undo.iter().enumerate() {
                                if daw_ui::button(ui, format!("Undo · {label}")).clicked() {
                                    jump = Some((false, index + 1));
                                }
                            }
                        });
                        if undo.is_empty() && redo.is_empty() {
                            ui.label("No edits yet");
                        }
                    });
                ui.separator();
                ui.small(format!(
                    "Up to 128 steps · {:.1} / 128 MiB",
                    self.history.memory_bytes() as f64 / 1_048_576.0
                ));
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
    for (before, after) in before.tracks.iter().zip(&after.tracks) {
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
        if before.output_bus != after.output_bus || before.sends != after.sends {
            return "Change routing";
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
