//! Arrangement selection is presentation state; edits operate on one explicit set.
use crate::app::{DawApp, WorkspaceView};
use daw_core::{Clip, ClipContent, MAX_BEAT, TrackKind, new_id};
use eframe::egui::{self, Pos2, Rect};
use std::collections::HashSet;

type PhraseRegions = (f64, f64, Vec<(u64, Clip)>);

#[derive(Clone, Default)]
pub(crate) struct ArrangerSelection {
    pub(crate) ids: HashSet<u64>,
    pub(crate) span: Option<(f64, f64)>,
    pub(crate) gesture: Option<SelectionGesture>,
    pub(crate) audio_drag: Option<ArrangeAudioDrag>,
    pub(crate) drag: Option<GroupDrag>,
    pub(crate) reveal: Option<(u64, u64)>,
    pub(crate) press_shift: Option<(u64, bool)>,
    pub(crate) click_tracker: daw_ui::ItemClickTracker,
    pub(crate) background_click_tracker: daw_ui::ItemClickTracker,
    clipboard: Option<ClipClipboard>,
}

#[derive(Clone, Default)]
pub(crate) struct ArrangerSelectionSnapshot {
    pub(crate) clips: Vec<(u64, u64)>,
    pub(crate) span: Option<(f64, f64)>,
    pub(crate) primary: Option<(u64, u64)>,
}

#[derive(Clone)]
pub(crate) struct SelectionGesture {
    pub(crate) origin: Pos2,
    pub(crate) current: Pos2,
    pub(crate) phrase: bool,
    pub(crate) additive: HashSet<u64>,
}

#[derive(Clone, Copy)]
pub(crate) enum ArrangeAudioMode {
    Slip(f64),
    Fade(bool),
}

#[derive(Clone)]
pub(crate) struct ArrangeAudioDrag {
    track: u64,
    original: Clip,
    mode: ArrangeAudioMode,
    origin: f64,
}

#[derive(Clone)]
pub(crate) struct GroupDrag {
    pub(crate) originals: std::sync::Arc<Vec<(u64, Clip)>>,
    pub(crate) span: Option<(f64, f64)>,
    pub(crate) copied: bool,
    pub(crate) source_track: u64,
}

#[derive(Clone)]
struct ClipClipboard {
    clips: Vec<(u64, TrackKind, Clip)>,
    start: f64,
    end: f64,
    phrase: bool,
}

impl DawApp {
    /// Project order makes group commands and their primary selection stable.
    pub(crate) fn arranger_selection_ids(&self) -> Vec<(u64, u64)> {
        let explicit = self
            .selected_clip
            .is_some_and(|id| self.arranger_selection.ids.contains(&id));
        self.project
            .tracks
            .iter()
            .flat_map(|track| {
                track.clips.iter().filter_map(move |clip| {
                    (!clip.is_take_source()
                        && if explicit {
                            self.arranger_selection.ids.contains(&clip.id)
                        } else {
                            self.selected_track == Some(track.id)
                                && self.selected_clip == Some(clip.id)
                        })
                    .then_some((track.id, clip.id))
                })
            })
            .collect()
    }

    pub(crate) fn arranger_phrase_span(&self) -> Option<(f64, f64)> {
        self.selected_clip
            .filter(|id| self.arranger_selection.ids.contains(id))
            .and(self.arranger_selection.span)
    }

    pub(crate) fn arranger_select_clips(
        &mut self,
        clips: impl IntoIterator<Item = (u64, u64)>,
        span: Option<(f64, f64)>,
    ) {
        let previous = self.selected_track.zip(self.selected_clip);
        let clips: Vec<_> = clips
            .into_iter()
            .filter(|(track, clip)| {
                self.project
                    .clip(*track, *clip)
                    .is_some_and(|clip| !clip.is_take_source())
            })
            .collect();
        let primary = previous
            .filter(|primary| clips.contains(primary))
            .or_else(|| clips.first().copied());
        self.arranger_selection.ids = clips.iter().map(|(_, id)| *id).collect();
        self.arranger_selection.span = span.filter(|(start, end)| {
            start.is_finite() && end.is_finite() && *start >= 0.0 && end > start
        });
        self.selected_clip = primary.map(|(_, clip)| clip);
        if let Some((track, _)) = primary {
            self.selected_track = Some(track);
        }
        self.selected_notes.clear();
    }

    pub(crate) fn sync_arranger_selection(&mut self) {
        let clips = self.arranger_selection_ids();
        let span = self.arranger_phrase_span();
        self.arranger_selection.ids = clips.iter().map(|(_, id)| *id).collect();
        self.arranger_selection.span = span;
    }

    pub(crate) fn arranger_selection_snapshot(&self) -> ArrangerSelectionSnapshot {
        ArrangerSelectionSnapshot {
            clips: self.arranger_selection_ids(),
            span: self.arranger_phrase_span(),
            primary: self.selected_track.zip(self.selected_clip),
        }
    }

    pub(crate) fn restore_arranger_selection_snapshot(
        &mut self,
        snapshot: ArrangerSelectionSnapshot,
    ) {
        self.selected_track = snapshot
            .primary
            .map(|(track, _)| track)
            .or(self.selected_track);
        self.selected_clip = snapshot.primary.map(|(_, clip)| clip);
        self.arranger_select_clips(snapshot.clips, snapshot.span);
        self.arranger_selection.gesture = None;
        self.arranger_selection.drag = None;
        self.arranger_selection.audio_drag = None;
        self.arranger_selection.press_shift = None;
        self.arranger_selection.reveal = None;
        self.arranger_selection.click_tracker.clear();
        self.arranger_selection.background_click_tracker.clear();
        self.drag = None;
    }

    /// A click retains a selected group for dragging; Shift-click toggles membership.
    pub(crate) fn arranger_click_clip(
        &mut self,
        track: u64,
        clip: u64,
        shift: bool,
        dragging: bool,
    ) {
        self.sync_arranger_selection();
        let mut clips = self.arranger_selection_ids();
        // Starting a drag inside the selected phrase must carry its rests along
        // with the clips. A fresh click or membership change clears the span.
        let span = if dragging && clips.contains(&(track, clip)) {
            self.arranger_phrase_span()
        } else {
            None
        };
        if shift && !dragging {
            if clips.contains(&(track, clip)) {
                clips.retain(|item| *item != (track, clip));
            } else {
                clips.push((track, clip));
            }
        } else if !clips.contains(&(track, clip)) {
            clips = vec![(track, clip)];
        }
        self.arranger_select_clips(clips, span);
        if self.arranger_selection.ids.contains(&clip) {
            self.selected_track = Some(track);
            self.selected_clip = Some(clip);
        }
    }

    fn arranger_clipboard(&self) -> Option<ClipClipboard> {
        let clips = self.arranger_selection_ids();
        let (start, end) = self.arranger_phrase_span().unwrap_or_else(|| {
            clips
                .iter()
                .filter_map(|(track, clip)| self.project.clip(*track, *clip))
                .fold((f64::INFINITY, 0.0_f64), |(start, end), clip| {
                    (start.min(clip.start), end.max(clip.end()))
                })
        });
        if !start.is_finite() || end <= start {
            return None;
        }
        let clips: Vec<_> = clips
            .into_iter()
            .filter_map(|(track, clip)| {
                let source = self.project.clip(track, clip)?;
                let a = source.start.max(start);
                let b = source.end().min(end);
                if b <= a {
                    return None;
                }
                let mut region = if a == source.start && b == source.end() {
                    source.clone()
                } else if matches!(source.content, ClipContent::Audio { .. }) {
                    source.audio_region(a, b).ok()?
                } else {
                    let mut region = source.clone();
                    region.offset += (a - region.start) * region.source_rate;
                    region.start = a;
                    region.length = b - a;
                    region
                };
                region.link_group = None;
                region.audio_take = None;
                if let ClipContent::Audio { crossfade, .. } = &mut region.content {
                    *crossfade = None;
                }
                Some((track, self.project.track(track)?.kind, region))
            })
            .collect();
        (!clips.is_empty()).then_some(ClipClipboard {
            clips,
            start,
            end,
            phrase: self.arranger_phrase_span().is_some(),
        })
    }

    pub(crate) fn arranger_phrase_regions(&self) -> Option<PhraseRegions> {
        self.arranger_clipboard().map(|clipboard| {
            (
                clipboard.start,
                clipboard.end,
                clipboard
                    .clips
                    .into_iter()
                    .map(|(track, _, clip)| (track, clip))
                    .collect(),
            )
        })
    }

    pub(crate) fn clear_arranger_clipboard(&mut self) {
        self.arranger_selection.clipboard = None;
    }

    fn paste_arranger_clips(&mut self, clipboard: &ClipClipboard, at: f64) -> bool {
        let delta = at - clipboard.start;
        if !at.is_finite() || at < 0.0 || at + clipboard.end - clipboard.start > MAX_BEAT {
            self.message(
                Err(anyhow::anyhow!("The phrase exceeds the supported timeline")),
                "",
            );
            return false;
        }
        let primary_track = self.selected_track;
        let one_track = clipboard
            .clips
            .iter()
            .all(|(track, _, _)| *track == clipboard.clips[0].0);
        let mut additions = Vec::with_capacity(clipboard.clips.len());
        for (source_track, kind, original) in &clipboard.clips {
            let target = if one_track {
                primary_track
                    .filter(|track| {
                        self.project
                            .track(*track)
                            .is_some_and(|track| track.kind == *kind)
                    })
                    .unwrap_or(*source_track)
            } else {
                *source_track
            };
            if !self
                .project
                .track(target)
                .is_some_and(|track| track.kind == *kind)
            {
                self.message(
                    Err(anyhow::anyhow!(
                        "The phrase's destination track is unavailable"
                    )),
                    "",
                );
                return false;
            }
            let mut clip = original.clone();
            clip.id = new_id();
            clip.start += delta;
            if let ClipContent::Midi { notes } = &mut clip.content {
                for note in notes {
                    note.id = new_id();
                }
            }
            additions.push((target, clip));
        }
        let mut next = self.project.clone();
        let ids: Vec<_> = additions
            .iter()
            .map(|(track, clip)| (*track, clip.id))
            .collect();
        for (track, clip) in additions {
            next.track_mut(track).unwrap().clips.push(clip);
        }
        if let Err(error) = next.validate() {
            self.message(Err(error), "");
            return false;
        }
        let span = clipboard
            .phrase
            .then_some((at, at + clipboard.end - clipboard.start));
        self.begin_edit();
        self.project = next;
        self.arranger_select_clips(ids, span);
        self.edit_label = Some("Duplicate phrase".into());
        self.changed();
        true
    }

    pub(crate) fn arranger_select_all(&mut self) {
        let clips = crate::routing::visible_tracks(&self.project)
            .into_iter()
            .flat_map(|(index, _)| {
                let track = &self.project.tracks[index];
                track
                    .clips
                    .iter()
                    .filter(|clip| !clip.is_take_source())
                    .map(move |clip| (track.id, clip.id))
            })
            .collect::<Vec<_>>();
        self.arranger_select_clips(clips, None);
    }

    pub(crate) fn has_arranger_clipboard(&self) -> bool {
        self.arranger_selection.clipboard.is_some()
    }

    pub(crate) fn arranger_clipboard_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::audio_editor::AudioClipboardAction,
    ) -> bool {
        use crate::audio_editor::AudioClipboardAction;
        if self.view != WorkspaceView::Arrangement {
            return false;
        }
        match action {
            AudioClipboardAction::Copy | AudioClipboardAction::Cut => {
                self.arranger_selection.clipboard = self.arranger_clipboard();
                if self.arranger_selection.clipboard.is_some() {
                    crate::audio_editor::clear_audio_clipboard(ctx);
                    ctx.copy_text("Rill clip selection".into());
                    if matches!(action, AudioClipboardAction::Cut) {
                        self.arranger_delete_selection();
                    }
                }
                true
            }
            AudioClipboardAction::Paste => {
                let Some(clipboard) = self.arranger_selection.clipboard.clone() else {
                    return false;
                };
                if self.paste_arranger_clips(&clipboard, self.cursor) {
                    self.edit_label = Some("Paste selected clips".into());
                }
                true
            }
            AudioClipboardAction::Duplicate => self.arranger_duplicate_selection(),
            AudioClipboardAction::Delete => self.arranger_delete_selection(),
        }
    }

    pub(crate) fn arranger_duplicate_selection(&mut self) -> bool {
        if self.view != WorkspaceView::Arrangement {
            return false;
        }
        let Some(clipboard) = self.arranger_clipboard() else {
            return true;
        };
        self.paste_arranger_clips(&clipboard, clipboard.end);
        true
    }

    pub(crate) fn arranger_delete_selection(&mut self) -> bool {
        if self.view != WorkspaceView::Arrangement {
            return false;
        }
        let selected = self.arranger_selection_ids();
        if selected.is_empty() {
            return true;
        }
        let mut next = self.project.clone();
        for (track, clip) in &selected {
            let result = if let Some((start, end)) = self.arranger_phrase_span() {
                let source = next.clip(*track, *clip).unwrap().clone();
                let a = start.max(source.start);
                let b = end.min(source.end());
                if b <= a {
                    continue;
                }
                if matches!(source.content, ClipContent::Audio { .. }) {
                    next.delete_audio_range(*track, *clip, a, b).map(|_| ())
                } else {
                    (|| -> anyhow::Result<()> {
                        if b < source.end() {
                            next.split_clip(*track, *clip, b)?;
                        }
                        let removed = if a > source.start {
                            next.split_clip(*track, *clip, a)?
                        } else {
                            *clip
                        };
                        next.delete_clip(*track, removed)?;
                        Ok(())
                    })()
                }
            } else {
                next.delete_clip(*track, *clip).map(|_| ())
            };
            if let Err(error) = result {
                self.message(Err(error), "");
                return true;
            }
        }
        if let Err(error) = next.validate() {
            self.message(Err(error), "");
            return true;
        }
        self.begin_edit();
        self.project = next;
        self.arranger_select_clips([], None);
        self.edit_label = Some("Delete selected clips".into());
        self.changed();
        true
    }

    pub(crate) fn arranger_start_audio_drag(
        &mut self,
        track: u64,
        clip: u64,
        mode: ArrangeAudioMode,
        origin: f64,
    ) {
        let Some(original) = self.project.clip(track, clip).cloned() else {
            return;
        };
        self.arranger_select_clips([(track, clip)], None);
        self.arranger_selection.audio_drag = Some(ArrangeAudioDrag {
            track,
            original,
            mode,
            origin,
        });
    }

    pub(crate) fn arranger_edit_audio_drag(&mut self, beat: f64) {
        let Some(drag) = self.arranger_selection.audio_drag.clone() else {
            return;
        };
        let mut next = self.project.clone();
        let Some(clip) = next.clip_mut(drag.track, drag.original.id) else {
            return;
        };
        clip.clone_from(&drag.original);
        let result = match drag.mode {
            ArrangeAudioMode::Slip(source_beats) => next.slip_audio_clip(
                drag.track,
                drag.original.id,
                beat - drag.origin,
                source_beats,
            ),
            ArrangeAudioMode::Fade(incoming) => {
                let Some((fi, fo, fic, foc)) =
                    crate::audio_editor::fade_parameters_at(&drag.original, beat, incoming)
                else {
                    return;
                };
                next.set_audio_fades(drag.track, drag.original.id, fi, fo, fic, foc)
            }
        };
        if let Err(error) = result {
            self.message(Err(error), "");
            return;
        }
        if next == self.project {
            return;
        }
        self.begin_edit();
        self.project = next;
        self.edit_label = Some(
            match drag.mode {
                ArrangeAudioMode::Slip(_) => "Slip audio contents",
                ArrangeAudioMode::Fade(true) => "Fade in",
                ArrangeAudioMode::Fade(false) => "Fade out",
            }
            .into(),
        );
        self.changed();
    }

    pub(crate) fn arranger_start_group_drag(&mut self, handle: u64, duplicate: bool) {
        let originals: Vec<_> = self
            .arranger_selection_ids()
            .into_iter()
            .filter_map(|(track, clip)| {
                self.project
                    .clip(track, clip)
                    .cloned()
                    .map(|clip| (track, clip))
            })
            .collect();
        if !originals.iter().any(|(_, clip)| clip.id == handle) {
            return;
        }
        let source_track = self.selected_track.unwrap_or(0);
        self.arranger_selection.drag = Some(GroupDrag {
            originals: std::sync::Arc::new(originals),
            span: self.arranger_phrase_span(),
            copied: !duplicate,
            source_track,
        });
    }

    pub(crate) fn arranger_move_group(&mut self, delta: f64) {
        let Some(mut drag) = self.arranger_selection.drag.clone() else {
            return;
        };
        let minimum = drag
            .originals
            .iter()
            .map(|(_, clip)| clip.start)
            .reduce(f64::min)
            .unwrap_or(0.0)
            .min(drag.span.map_or(f64::INFINITY, |(start, _)| start));
        let maximum = drag
            .originals
            .iter()
            .map(|(_, clip)| clip.end())
            .reduce(f64::max)
            .unwrap_or(0.0)
            .max(drag.span.map_or(0.0, |(_, end)| end));
        if !drag.copied && (!delta.is_finite() || delta < -minimum || delta > MAX_BEAT - maximum) {
            // An invalid initial Shift-copy must be rejected, rather than
            // clamped into an overlapping copy or resumed as an original move.
            self.arranger_selection.drag = None;
            self.drag = None;
            self.message(
                Err(anyhow::anyhow!(
                    "Duplicate selection exceeds the supported timeline"
                )),
                "",
            );
            return;
        }
        let delta = delta.clamp(-minimum, MAX_BEAT - maximum);
        if !delta.is_finite() {
            return;
        }
        if !drag.copied {
            let mut additions = Vec::new();
            for (track, original) in drag.originals.iter() {
                let mut clip = original.clone();
                clip.id = new_id();
                clip.link_group = None;
                clip.audio_take = None;
                if let ClipContent::Audio { crossfade, .. } = &mut clip.content {
                    *crossfade = None;
                }
                if let ClipContent::Midi { notes } = &mut clip.content {
                    for note in notes {
                        note.id = new_id();
                    }
                }
                additions.push((*track, clip));
            }
            self.begin_edit();
            for (track, clip) in &additions {
                self.project
                    .track_mut(*track)
                    .unwrap()
                    .clips
                    .push(clip.clone());
            }
            let ids = additions
                .iter()
                .map(|(track, clip)| (*track, clip.id))
                .collect::<Vec<_>>();
            drag.originals = std::sync::Arc::new(additions);
            drag.copied = true;
            self.arranger_select_clips(ids, drag.span);
            self.edit_label = Some("Duplicate selected clips".into());
            self.changed();
        }
        let differs = drag.originals.iter().any(|(track, original)| {
            self.project
                .clip(*track, original.id)
                .is_some_and(|clip| clip.start != original.start + delta)
        });
        if differs {
            self.begin_edit();
            for (track, original) in drag.originals.iter() {
                if let Some(clip) = self.project.clip_mut(*track, original.id) {
                    clip.start = original.start + delta;
                }
            }
            self.arranger_selection.span =
                drag.span.map(|(start, end)| (start + delta, end + delta));
            if self.edit_label.is_none() {
                self.edit_label = Some("Move selected clips".into());
            }
            self.changed();
        }
        self.arranger_selection.drag = Some(drag);
    }

    pub(crate) fn arranger_transfer_group(&mut self, target: u64) {
        let Some(drag) = self.arranger_selection.drag.as_ref() else {
            return;
        };
        if drag.source_track == target {
            return;
        }
        let rows = crate::routing::visible_tracks(&self.project)
            .into_iter()
            .map(|(index, _)| &self.project.tracks[index])
            .filter(|track| matches!(track.kind, TrackKind::Audio | TrackKind::Midi))
            .map(|track| track.id)
            .collect::<Vec<_>>();
        let Some(source_row) = rows.iter().position(|id| *id == drag.source_track) else {
            return;
        };
        let Some(target_row) = rows.iter().position(|id| *id == target) else {
            return;
        };
        let shift = target_row as isize - source_row as isize;
        let mut moves = Vec::new();
        for (source, original) in drag.originals.iter() {
            let Some(row) = rows.iter().position(|id| id == source) else {
                return;
            };
            let row = row as isize + shift;
            if row < 0 || row as usize >= rows.len() {
                return;
            }
            let destination = rows[row as usize];
            if self.project.track(*source).map(|track| track.kind)
                != self.project.track(destination).map(|track| track.kind)
            {
                return;
            }
            moves.push((*source, destination, original.id));
        }
        let mut next = self.project.clone();
        for (source, destination, id) in &moves {
            let Ok(mut clip) = next.delete_clip(*source, *id) else {
                return;
            };
            clip.audio_take = None;
            next.track_mut(*destination).unwrap().clips.push(clip);
        }
        if let Err(error) = next.validate() {
            self.message(Err(error), "");
            return;
        }
        let span = self.arranger_phrase_span();
        self.begin_edit();
        self.project = next;
        self.arranger_select_clips(
            moves.into_iter().map(|(_, track, clip)| (track, clip)),
            span,
        );
        self.edit_label = Some("Move selected clips".into());
        self.changed();
    }

    pub(crate) fn arranger_nudge(&mut self, ctx: &egui::Context, direction: i32) {
        let Some(handle) = self.selected_clip else {
            return;
        };
        self.arranger_start_group_drag(handle, false);
        let delta = daw_ui::NudgeUnit::load(ctx).beats(
            self.snap,
            self.project.tempo,
            self.settings.sample_rate,
        ) * f64::from(direction);
        self.arranger_move_group(delta);
        self.arranger_selection.drag = None;
        self.edit_label = Some("Nudge selected clips".into());
    }

    fn arranger_navigate(&mut self, direction: daw_ui::SelectionDirection, extend: bool) {
        let visible = crate::routing::visible_tracks(&self.project);
        let items = visible
            .iter()
            .enumerate()
            .flat_map(|(row, (index, _))| {
                self.project.tracks[*index]
                    .clips
                    .iter()
                    .filter(|clip| !clip.is_take_source())
                    .map(move |clip| daw_ui::SelectionItem {
                        id: clip.id,
                        start: clip.start,
                        end: clip.end(),
                        row: row as f64,
                    })
            })
            .collect::<Vec<_>>();
        let Some(next) = daw_ui::adjacent_selection_item(&items, self.selected_clip, direction)
        else {
            return;
        };
        let Some(track) = self
            .project
            .tracks
            .iter()
            .find(|track| track.clips.iter().any(|clip| clip.id == next))
            .map(|track| track.id)
        else {
            return;
        };
        let mut selected = if extend {
            self.arranger_selection_ids()
        } else {
            Vec::new()
        };
        if !selected.contains(&(track, next)) {
            selected.push((track, next));
        }
        self.arranger_select_clips(selected, None);
        self.selected_track = Some(track);
        self.selected_clip = Some(next);
        self.arranger_selection.reveal = Some((track, next));
    }

    pub(crate) fn arranger_edit_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.view != WorkspaceView::Arrangement || egui::Popup::is_any_open(ctx) {
            return false;
        }
        self.sync_arranger_selection();
        let commands = ctx.input_mut(|input| {
            let native_copy = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy));
            let native_cut = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Cut));
            let native_paste = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)));
            let has_clipboard = self.arranger_selection.clipboard.is_some();
            let mut actions = Vec::new();
            input.events.retain(|event| {
                let action = match event {
                    egui::Event::Copy => Some(0),
                    egui::Event::Cut => Some(1),
                    egui::Event::Paste(_) if has_clipboard => Some(2),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command && !modifiers.alt && !modifiers.shift => match key {
                        egui::Key::C if !native_copy => Some(0),
                        egui::Key::X if !native_cut => Some(1),
                        egui::Key::V if has_clipboard && !native_paste => Some(2),
                        egui::Key::D => Some(3),
                        egui::Key::ArrowLeft => Some(14),
                        egui::Key::ArrowRight => Some(15),
                        _ => None,
                    },
                    egui::Event::Key {
                        key: egui::Key::A,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command && !modifiers.alt => Some(5),
                    egui::Event::Key {
                        key: egui::Key::Delete | egui::Key::Backspace,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.any() => Some(4),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                        let extend = if modifiers.shift { 4 } else { 0 };
                        match key {
                            egui::Key::ArrowLeft => Some(6 + extend),
                            egui::Key::ArrowRight => Some(7 + extend),
                            egui::Key::ArrowUp => Some(8 + extend),
                            egui::Key::ArrowDown => Some(9 + extend),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(action) = action {
                    actions.push(action);
                    false
                } else {
                    true
                }
            });
            actions
        });
        for action in &commands {
            match action {
                0..=4 => {
                    use crate::audio_editor::AudioClipboardAction;
                    let actions = [
                        AudioClipboardAction::Copy,
                        AudioClipboardAction::Cut,
                        AudioClipboardAction::Paste,
                        AudioClipboardAction::Duplicate,
                        AudioClipboardAction::Delete,
                    ];
                    self.arranger_clipboard_action(ctx, actions[*action as usize]);
                }
                5 => self.arranger_select_all(),
                6..=13 => {
                    let directions = [
                        daw_ui::SelectionDirection::Left,
                        daw_ui::SelectionDirection::Right,
                        daw_ui::SelectionDirection::Up,
                        daw_ui::SelectionDirection::Down,
                    ];
                    self.arranger_navigate(directions[(*action as usize - 6) % 4], *action >= 10);
                }
                14 | 15 => self.arranger_nudge(ctx, if *action == 14 { -1 } else { 1 }),
                _ => {}
            }
        }
        !commands.is_empty()
    }
}

/// Intersection includes the selected time but never a merely touching boundary.
pub(crate) fn selection_hits(rect: Rect, region: Rect) -> bool {
    rect.left() < region.right()
        && rect.right() > region.left()
        && rect.top() < region.bottom()
        && rect.bottom() > region.top()
}
