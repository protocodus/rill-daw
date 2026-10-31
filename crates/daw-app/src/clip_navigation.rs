//! Clip-to-clip inspection keeps the user's editing tool and scale.
use crate::app::{DawApp, WorkspaceView};
use daw_core::ClipContent;
use daw_ui::{ClipStep, SelectionDirection, SelectionItem};
use eframe::egui;

impl DawApp {
    fn adjacent_editor_clips(&self) -> [Option<(u64, u64)>; 2] {
        let Some(current) = self.selected() else {
            return [None, None];
        };
        let visible = crate::routing::visible_tracks(&self.project);
        let Some(row) = visible
            .iter()
            .position(|&(index, _)| Some(self.project.tracks[index].id) == self.selected_track)
        else {
            return [None, None];
        };
        // One linear scan avoids sorting and allocating every clip on each paint.
        type Cue = (usize, f64, u64, u64);
        let compare = |a: Cue, b: Cue| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2));
        let origin = (row, current.start, current.id, self.selected_track.unwrap());
        let mut previous: Option<Cue> = None;
        let mut next: Option<Cue> = None;
        for (row, (index, _)) in visible.into_iter().enumerate() {
            let track = &self.project.tracks[index];
            for clip in track.clips.iter().filter(|clip| !clip.is_take_source()) {
                let cue = (row, clip.start, clip.id, track.id);
                match compare(cue, origin) {
                    std::cmp::Ordering::Less
                        if previous.is_none_or(|best| compare(cue, best).is_gt()) =>
                    {
                        previous = Some(cue)
                    }
                    std::cmp::Ordering::Greater
                        if next.is_none_or(|best| compare(cue, best).is_lt()) =>
                    {
                        next = Some(cue)
                    }
                    _ => {}
                }
            }
        }
        [previous, next].map(|cue| cue.map(|(_, _, clip, track)| (track, clip)))
    }

    pub(crate) fn select_editor_clip(
        &mut self,
        ctx: &egui::Context,
        track: u64,
        clip: u64,
        extend: bool,
    ) {
        let Some(target) = self.project.clip(track, clip) else {
            return;
        };
        if target.is_take_source() {
            return;
        }
        let audio = matches!(target.content, ClipContent::Audio { .. });
        let start = target.start;
        let mut selected = if extend {
            self.arranger_selection_ids()
        } else {
            Vec::new()
        };
        if !selected.contains(&(track, clip)) {
            selected.push((track, clip));
        }
        self.selected_track = Some(track);
        self.selected_clip = Some(clip);
        self.selected_notes.clear();
        self.arranger_select_clips(selected, None);
        if audio {
            crate::audio_editor::select_audio_clip_preserving_view(ctx, clip, start);
        } else {
            self.midi_editor.select_clip_preserving_view(clip);
        }
        self.last_editor_clip = Some(clip);
        self.sync_workspace_tool();
    }

    pub(crate) fn clip_navigation_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.view != WorkspaceView::Clip
            || ctx.wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
        {
            return false;
        }
        let step = ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                Some(false)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                Some(true)
            } else {
                None
            }
        });
        if let Some(next) = step {
            // egui reads Tab's focus direction before application shortcuts.
            // Consuming its Key event alone would still focus a toolbar button
            // at end-of-frame and swallow the following Shift+Tab.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            if let Some((track, clip)) = self.adjacent_editor_clips()[usize::from(next)] {
                self.select_editor_clip(ctx, track, clip, false);
            }
            return true;
        }
        false
    }

    pub(crate) fn clip_navigation_controls(&self, ui: &mut egui::Ui) {
        let [previous, next] = self.adjacent_editor_clips();
        let step = daw_ui::clip_navigation_controls(ui, previous.is_some(), next.is_some());
        if let Some((track, clip)) = match step {
            Some(ClipStep::Previous) => previous,
            Some(ClipStep::Next) => next,
            None => None,
        } {
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("clip_navigation_request"), (track, clip))
            });
        }
    }

    pub(crate) fn clip_navigation_header(&self, ui: &mut egui::Ui) {
        let [previous, next] = self.adjacent_editor_clips();
        let step = daw_ui::clip_navigation_header_controls(ui, previous.is_some(), next.is_some());
        if let Some((track, clip)) = match step {
            Some(ClipStep::Previous) => previous,
            Some(ClipStep::Next) => next,
            None => None,
        } {
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("clip_navigation_request"), (track, clip))
            });
        }
    }

    pub(crate) fn finish_clip_navigation(&mut self, ctx: &egui::Context) {
        let target = ctx.data_mut(|data| {
            data.remove_temp::<(u64, u64)>(egui::Id::new("clip_navigation_request"))
        });
        if let Some((track, clip)) = target {
            self.select_editor_clip(ctx, track, clip, false);
            ctx.request_repaint();
        }
    }

    pub(crate) fn audio_clip_selection_navigation(&mut self, ui: &egui::Ui) {
        if !matches!(
            self.selected().map(|clip| &clip.content),
            Some(ClipContent::Audio { .. })
        ) {
            return;
        }
        let Some((direction, extend)) = daw_ui::selection_navigation_input(ui, None) else {
            return;
        };
        let horizontal = matches!(
            direction,
            SelectionDirection::Left | SelectionDirection::Right
        );
        let selected_track = self.selected_track;
        let candidates: Vec<_> = crate::routing::visible_tracks(&self.project)
            .into_iter()
            .enumerate()
            .flat_map(|(row, (index, _))| {
                let track = &self.project.tracks[index];
                track
                    .clips
                    .iter()
                    .filter(move |clip| {
                        !clip.is_take_source() && (!horizontal || Some(track.id) == selected_track)
                    })
                    .map(move |clip| {
                        (
                            track.id,
                            SelectionItem {
                                id: clip.id,
                                start: clip.start,
                                end: clip.end(),
                                row: row as f64,
                            },
                        )
                    })
            })
            .collect();
        let items: Vec<_> = candidates.iter().map(|(_, item)| *item).collect();
        if let Some(target) = daw_ui::adjacent_selection_item(&items, self.selected_clip, direction)
            && let Some((track, _)) = candidates.iter().find(|(_, item)| item.id == target)
        {
            self.select_editor_clip(ui.ctx(), *track, target, extend);
        }
    }
}
