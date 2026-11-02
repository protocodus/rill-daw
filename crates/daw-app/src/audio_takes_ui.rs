//! Retained takes and range comping. Import work is bounded and performed away
//! from the UI; the complete batch is published as one undoable transaction.
use crate::app::{DawApp, PopupKind, WorkspaceView};
use anyhow::{Context, Result, ensure};
use daw_core::{AudioData, Clip, ClipContent, Project, new_id};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

type ImportedTakes = Vec<(Clip, Option<AudioData>)>;

fn read_audio_takes(
    files: Vec<PathBuf>,
    start: f64,
    tempo: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<ImportedTakes> {
    ensure!(files.len() <= 32, "Add at most 32 takes at once");
    let mut remaining = budget;
    let mut imported = Vec::with_capacity(files.len());
    for path in files {
        ensure!(!cancel.load(Ordering::Acquire), "Take import cancelled");
        // Preview capacity is optional; a full cache must not prevent importing
        // valid media. Only one bounded decoder allocation exists outside it.
        let audio = AudioData::load_wav_with_limit(&path, crate::project_io::WAVEFORM_FILE_BYTES)
            .with_context(|| format!("Import take {}", path.display()))?;
        ensure!(!cancel.load(Ordering::Acquire), "Take import cancelled");
        ensure!(audio.frames() > 0, "Take {} is empty", path.display());
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let path = path
            .canonicalize()
            .with_context(|| format!("Locate take {}", path.display()))?;
        let clip = Clip::audio(
            name,
            start,
            audio.duration_seconds() * tempo / 60.0,
            path.to_string_lossy(),
        );
        let bytes = audio.samples.len() * size_of::<f32>();
        let preview = if bytes <= remaining {
            remaining -= bytes;
            Some(audio)
        } else {
            None
        }; // Release excess PCM here, away from the UI thread.
        imported.push((clip, preview));
    }
    Ok(imported)
}
struct TakeImport {
    receiver: mpsc::Receiver<Result<ImportedTakes>>,
    revision: u64,
    track: u64,
    anchor: Clip,
    epoch: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub(crate) struct AudioTakesUi {
    epoch: u64,
    track: Option<u64>,
    anchor: Option<u64>,
    source: Option<u64>,
    range: Option<(f64, f64)>,
    crossfade_ms: f64,
    pending: Option<TakeImport>,
    pub(crate) next_files: Option<Option<Vec<PathBuf>>>,
    rows: Vec<(u64, egui::Rect)>,
}

impl AudioTakesUi {
    pub(crate) fn invalidate_import(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
    }
}

impl DawApp {
    pub(crate) fn open_audio_takes(&mut self, clip: &Clip, range: Option<(f64, f64)>) {
        let Some(track) = self.selected_track else {
            return;
        };
        self.audio_takes_ui.invalidate_import();
        self.audio_takes_ui.track = Some(track);
        self.audio_takes_ui.anchor = Some(clip.id);
        self.audio_takes_ui.source = None;
        self.audio_takes_ui.range = range;
        self.audio_takes_ui.crossfade_ms = 10.0;
        self.open_popup(PopupKind::AudioTakes);
    }

    fn take_anchor(&self) -> Option<(u64, Clip)> {
        let state = &self.audio_takes_ui;
        let track = state.track?;
        let anchor = self.project.clip(track, state.anchor?)?;
        Some((track, anchor.clone()))
    }

    fn choose_take_files(&mut self) {
        if self.audio_takes_ui.pending.is_some() {
            return;
        }
        let Some((track, anchor)) = self.take_anchor() else {
            return;
        };
        let files = self.audio_takes_ui.next_files.take().unwrap_or_else(|| {
            daw_ui::with_native_dialog_menu(|| {
                rfd::FileDialog::new()
                    .set_title("Add audio takes")
                    .add_filter("Audio", &["wav"])
                    .pick_files()
            })
        });
        let Some(files) = files.filter(|files| !files.is_empty()) else {
            return;
        };
        let tempo = self.project.tempo;
        let start = anchor
            .take_group()
            .and_then(|group| {
                self.project
                    .audio_take_sources(track, group)
                    .into_iter()
                    .map(|clip| clip.start)
                    .reduce(f64::min)
            })
            .unwrap_or(anchor.start);
        let cache_bytes: usize = self
            .waveforms
            .values()
            .map(|audio| audio.samples.len() * size_of::<f32>())
            .sum();
        let budget = crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(cache_bytes);
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let _ = tx.send(read_audio_takes(
                files,
                start,
                tempo,
                budget,
                &worker_cancel,
            ));
        });
        self.audio_takes_ui.pending = Some(TakeImport {
            receiver: rx,
            revision: self.revision,
            track,
            anchor,
            epoch: self.audio_takes_ui.epoch,
            cancel,
        });
    }

    pub(crate) fn poll_audio_take_import(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.audio_takes_ui.pending else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Audio take import stopped unexpectedly"))
            }
        };
        let pending = self.audio_takes_ui.pending.take().unwrap();
        if !self.audio_takes_open
            || self.audio_takes_ui.epoch != pending.epoch
            || self.audio_takes_ui.anchor != Some(pending.anchor.id)
            || self.revision != pending.revision
            || self.audio_takes_ui.track != Some(pending.track)
            || self.project.clip(pending.track, pending.anchor.id) != Some(&pending.anchor)
        {
            if self.audio_takes_open {
                self.status = "Take import cancelled because the project changed".into();
            }
            return;
        }
        let result = result.and_then(|imported| {
            let mut project = self.project.clone();
            let group = match pending.anchor.take_group() {
                Some(group) => group,
                None => project.group_audio_takes(pending.track, &[pending.anchor.id])?,
            };
            let first = imported.first().map(|(clip, _)| clip.id);
            for (clip, _) in &imported {
                project
                    .track_mut(pending.track)
                    .context("Take channel disappeared")?
                    .clips
                    .push(clip.clone());
                project.add_audio_take(pending.track, group, clip.id)?;
            }
            project.validate()?;
            self.begin_edit();
            self.project = project;
            self.selected_clip = self
                .project
                .audio_take_regions(pending.track, group)
                .first()
                .map(|clip| clip.id);
            for (clip, audio) in imported {
                if let ClipContent::Audio { path, .. } = clip.content {
                    if let Some(audio) = audio { self.waveforms.insert(path, audio); }
                    else if !self.waveforms.contains_key(&path) {
                        self.waveform_warning = Some("Some take waveforms are unavailable because the preview cache is full · Audio remains linked".into());
                    }
                }
            }
            self.audio_takes_ui.source = first;
            self.edit_label = Some("Add audio takes".into());
            self.changed();
            Ok(())
        });
        self.message(result, "Audio takes ready");
    }

    fn commit_take_project(&mut self, project: Project, label: &str) {
        self.begin_edit();
        self.project = project;
        if let Some((track, clip)) = self.selected_track.zip(self.selected_clip)
            && let Some(clip) = self.project.clip(track, clip)
            && clip.is_take_source()
            && let Some(group) = clip.take_group()
        {
            self.selected_clip = self
                .project
                .audio_take_regions(track, group)
                .first()
                .map(|clip| clip.id);
        }
        self.edit_label = Some(label.into());
        self.changed();
    }

    fn use_audio_take(&mut self, track: u64, group: u64, source: u64, range: Option<(f64, f64)>) {
        let mut project = self.project.clone();
        let crossfade = self.audio_takes_ui.crossfade_ms / 1000.0 * project.tempo / 60.0;
        match project.select_audio_take(track, group, source, range, crossfade) {
            Ok(clip) => {
                self.commit_take_project(
                    project,
                    if range.is_some() {
                        "Comp audio selection"
                    } else {
                        "Choose audio take"
                    },
                );
                self.close_popup();
                self.selected_track = Some(track);
                self.selected_clip = Some(clip);
                self.view = WorkspaceView::Clip;
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    fn layer_audio_take(&mut self, track_id: u64, source: &Clip) {
        let Some(original) = self.project.track(track_id) else {
            return;
        };
        let (bus, folder) = (original.output_bus, original.parent_folder);
        let mut project = self.project.clone();
        let id = project.add_audio_track(format!("{} · overdub", source.name));
        let mut track = project.tracks.pop().expect("new overdub track");
        debug_assert_eq!(track.id, id);
        track.output_bus = bus;
        track.parent_folder = folder;
        let mut clip = source.clone();
        clip.id = new_id();
        clip.link_group = None;
        clip.audio_take = None;
        let selection = (track.id, clip.id);
        track.clips.push(clip);
        let index = project
            .tracks
            .iter()
            .position(|track| track.id == track_id)
            .unwrap()
            + 1;
        project.tracks.insert(index, track);
        if let Err(error) = project.validate() {
            self.message(Err(error), "");
            return;
        }
        self.commit_take_project(project, "Layer audio overdub");
        self.close_popup();
        self.selected_track = Some(selection.0);
        self.selected_clip = Some(selection.1);
        self.view = WorkspaceView::Clip;
    }

    pub(crate) fn audio_takes_popup(&mut self, ctx: &egui::Context) {
        if !self.audio_takes_open {
            return;
        }
        let Some((track_id, anchor)) = self.take_anchor() else {
            self.audio_takes_open = false;
            return;
        };
        let group = anchor.take_group();
        let candidates = group
            .map(|group| {
                self.project
                    .audio_take_sources(track_id, group)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![anchor.clone()]);
        let regions = group
            .map(|group| {
                self.project
                    .audio_take_regions(track_id, group)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !candidates
            .iter()
            .any(|source| Some(source.id) == self.audio_takes_ui.source)
        {
            self.audio_takes_ui.source = candidates.first().map(|source| source.id);
        }
        let selected = self
            .audio_takes_ui
            .source
            .and_then(|id| candidates.iter().find(|clip| clip.id == id))
            .cloned();
        let start = candidates
            .iter()
            .map(|clip| clip.start)
            .fold(f64::INFINITY, f64::min);
        let end = candidates
            .iter()
            .map(|clip| clip.end())
            .fold(start, f64::max);
        let span = (end - start).max(0.001);
        let track_name = self
            .project
            .track(track_id)
            .map(|track| track.name.clone())
            .unwrap_or_default();
        let color = self
            .project
            .track(track_id)
            .map(|track| egui::Color32::from_rgb(track.color[0], track.color[1], track.color[2]))
            .unwrap_or(egui::Color32::GRAY);
        let busy = self.audio_takes_ui.pending.is_some();
        let mut groups = Vec::new();
        if let Some(track) = self.project.track(track_id) {
            for clip in track.clips.iter().filter(|clip| clip.is_take_source()) {
                if !groups
                    .iter()
                    .any(|(id, _, _)| Some(*id) == clip.take_group())
                {
                    groups.push((clip.take_group().unwrap(), clip.id, clip.name.clone()));
                }
            }
        }
        let mut open = true;
        daw_ui::popup(format!("Audio takes · {track_name}"), ctx).id(egui::Id::new("audio_takes_popup")).open(&mut open).default_width(720.0).default_height(620.0).vscroll(false).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                daw_ui::view_menu(ui, "View", crate::spectrogram::view_options);
                if groups.len() > 1 {
                    daw_ui::picker_button(ui, "Take group", |ui| {
                        for (id, source, name) in &groups {
                            let mut choice = group;
                            if daw_ui::selectable_value(ui, &mut choice, Some(*id), name).clicked() {
                                self.audio_takes_ui.anchor = Some(*source);
                                self.audio_takes_ui.invalidate_import();
                                self.audio_takes_ui.source = Some(*source);
                                self.audio_takes_ui.range = None;
                                ui.close();
                            }
                        }
                    });
                }
                if ui.add_enabled(!busy, daw_ui::button_widget("Add takes…")).clicked() { self.choose_take_files(); }
                if group.is_none() {
                    let ids: Vec<_> = std::iter::once(anchor.id).chain(self.project.track(track_id).into_iter().flat_map(|track| &track.clips).filter(|clip| clip.id != anchor.id && !clip.is_take_source() && clip.take_group().is_none() && clip.start < anchor.end() && clip.end() > anchor.start).map(|clip| clip.id)).collect();
                    if ui.add_enabled(!busy && ids.len() > 1, daw_ui::button_widget("Group overlapping clips")).clicked() {
                        let mut project = self.project.clone();
                        match project.group_audio_takes(track_id, &ids) {
                            Ok(_) => self.commit_take_project(project, "Group audio takes"),
                            Err(error) => self.message(Err(error), ""),
                        }
                    }
                }
                if busy { ui.spinner(); daw_ui::text::note(ui, "Importing takes…"); }
                if let Some(group) = group {
                    daw_ui::menu_button(ui, "Takes", |ui| {
                        if ui.add_enabled(!busy && !regions.is_empty(), daw_ui::button_widget("Keep comp only"))
                            .on_hover_text("Remove retained alternatives from the project. Keep the audible comp and original files. Undo restores the takes.")
                            .clicked()
                        {
                            let mut project = self.project.clone();
                            match project.detach_audio_take_group(track_id, group) {
                                Ok(()) => {
                                    let clip = regions[0].id;
                                    self.commit_take_project(project, "Keep audio comp only");
                                    self.close_popup();
                                    self.selected_track = Some(track_id);
                                    self.selected_clip = Some(clip);
                                    self.view = WorkspaceView::Clip;
                                }
                                Err(error) => self.message(Err(error), ""),
                            }
                            ui.close();
                        }
                    });
                }
            });
            daw_ui::text::note(ui, "Choose a take, then use it for the whole clip or just your time selection.");
            ui.add_space((daw_ui::tokens::SPACE_LG - ui.spacing().item_spacing.y).max(0.0));
            self.audio_takes_ui.rows.clear();
            // Take lanes and their range/crossfade settings share the scroll
            // body. Only commit actions stay pinned, leaving a useful lane
            // viewport even beneath the wrapped transport at 800×600.
            let footer_height = daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_LG;
            let list_height = daw_ui::popup_scroll_height(ui, footer_height);
            egui::ScrollArea::vertical().id_salt("audio_take_lanes").max_height(list_height).min_scrolled_height(0.0).auto_shrink([false, false]).show(ui, |ui| {
                for clip in &candidates {
                    let in_comp = regions.iter().any(|region| region.audio_take.as_ref().is_some_and(|membership| matches!(membership.role, daw_core::AudioTakeRole::Comp { source } if source == clip.id)));
                    let detail = format!("{:.2} beats · {}", clip.length, if in_comp || group.is_none() { "In comp" } else { "Alternative" });
                    let response = ui.push_id(clip.id, |ui| daw_ui::audio_take_lane_with_waveform(ui, &daw_ui::TakeLane {
                        name: &clip.name, detail: &detail,
                        selected: Some(clip.id) == self.audio_takes_ui.source,
                        color, peaks: &[],
                        span: (((clip.start - start) / span) as f32, ((clip.end() - start) / span) as f32),
                        selection: self.audio_takes_ui.range.map(|(a, b)| (((a - start) / span) as f32, ((b - start) / span) as f32)),
                    }, |painter, plot| {
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(plot.left() + ((clip.start - start) / span) as f32 * plot.width(), plot.top()),
                            egui::pos2(plot.left() + ((clip.end() - start) / span) as f32 * plot.width(), plot.bottom()),
                        );
                        if let ClipContent::Audio { path, .. } = &clip.content
                            && let Some(audio) = self.waveforms.get(path)
                        {
                            crate::audio_editor::paint_audio_clip_in_view(
                                painter, rect, audio, clip, self.project.tempo, color, "takes",
                            );
                        } else {
                            crate::spectrogram::paint(
                                painter, rect, clip, self.project.tempo,
                                (clip.start, clip.end()), color, "takes", None,
                            );
                        }
                    })).inner;
                    self.audio_takes_ui.rows.push((clip.id, response.rect));
                    if response.clicked() { self.audio_takes_ui.source = Some(clip.id); }
                }
            ui.add(egui::Separator::default().spacing(daw_ui::tokens::SPACE_LG));
            ui.horizontal_wrapped(|ui| {
                let mut use_range = self.audio_takes_ui.range.is_some();
                if daw_ui::checkbox(ui, &mut use_range, "Time selection").changed() {
                    self.audio_takes_ui.range = use_range.then_some((anchor.start, anchor.end()));
                }
                if let Some((a, b)) = &mut self.audio_takes_ui.range {
                    let mut start_beat = *a + 1.0;
                    let mut end_beat = *b + 1.0;
                    ui.add(daw_ui::number(&mut start_beat).prefix("From beat ").range(1.0..=(end_beat - 0.001)).speed(0.25));
                    ui.add(daw_ui::number(&mut end_beat).prefix("To beat ").range((start_beat + 0.001)..=daw_core::MAX_BEAT).speed(0.25));
                    *a = start_beat - 1.0; *b = end_beat - 1.0;
                }
            });
            ui.horizontal_wrapped(|ui| {
                daw_ui::text::label(ui, "Boundary crossfade");
                ui.add(daw_ui::number(&mut self.audio_takes_ui.crossfade_ms).range(0.0..=500.0).speed(1.0).suffix(" ms"))
                    .on_hover_text("Smooth complementary fades, limited to available audio handles. Edit the spline in Crossfade after choosing a take.");
            });
            let selection_valid = selected.as_ref().is_some_and(|take| self.audio_takes_ui.range.is_none_or(|(start, end)| start >= take.start && end <= take.end() && end > start));
            // Feedback belongs to the scrolling options, so even a long
            // validation message cannot displace the pinned commit actions.
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(daw_ui::tokens::CONTROL_HEIGHT);
                if !selection_valid && let Some(take) = &selected {
                    daw_ui::action_row(ui, |ui| {
                        if daw_ui::button(ui, "Fit selection to take").clicked() {
                            let (start, end) = self.audio_takes_ui.range.unwrap_or((take.start, take.end()));
                            let overlap = (start.max(take.start), end.min(take.end()));
                            self.audio_takes_ui.range = Some(if overlap.1 > overlap.0 { overlap } else { (take.start, take.end()) });
                        }
                    }, |ui| {
                        daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, format!("Selection outside this take · Available beats {:.2}–{:.2}", take.start + 1.0, take.end() + 1.0), 2);
                    });
                    }
            });
            });
            ui.add_space((daw_ui::tokens::SPACE_LG - ui.spacing().item_spacing.y).max(0.0));
            let selection_valid = selected.as_ref().is_some_and(|take| self.audio_takes_ui.range.is_none_or(|(start, end)| start >= take.start && end <= take.end() && end > start));
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy && group.is_some() && selection_valid, |ui| {
                    if daw_ui::primary_button(ui, if self.audio_takes_ui.range.is_some() { "Use selection" } else { "Use whole take" }).clicked() {
                        self.use_audio_take(track_id, group.unwrap(), selected.as_ref().unwrap().id, self.audio_takes_ui.range);
                    }
                });
                if ui.add_enabled(!busy && selected.is_some(), daw_ui::button_widget("Layer as overdub")).on_hover_text("Copy this take to a separate audio track, aligned in time and routed to the same bus. Both tracks will play.").clicked() {
                    self.layer_audio_take(track_id, selected.as_ref().unwrap());
                }
                let return_clip = regions.first().map(|clip| clip.id).or_else(|| (!anchor.is_take_source()).then_some(anchor.id));
                if daw_ui::button(ui, if return_clip.is_some() { "Back to editor" } else { "Done" }).clicked() {
                    self.close_popup();
                    if let Some(clip) = return_clip {
                        self.selected_track = Some(track_id);
                        self.selected_clip = Some(clip);
                        self.view = WorkspaceView::Clip;
                    }
                }
            });
        });
        self.audio_takes_open &= open;
    }

    pub(crate) fn audio_takes_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "open": self.audio_takes_open, "busy": self.audio_takes_ui.pending.is_some(),
            "track": self.audio_takes_ui.track, "anchor": self.audio_takes_ui.anchor,
            "source": self.audio_takes_ui.source, "range": self.audio_takes_ui.range,
            "rows": self.audio_takes_ui.rows.iter().map(|(id, rect)| serde_json::json!({"id": id, "min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_import_with_full_preview_cache_still_imports_audio_and_honors_cancel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("take.wav");
        AudioData {
            sample_rate: 8000,
            channels: 1,
            samples: vec![0.2; 8000],
        }
        .write_wav(&path)
        .unwrap();
        let cancel = AtomicBool::new(false);
        let result = read_audio_takes(vec![path.clone()], 4.0, 120.0, 0, &cancel).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0.start, 4.0);
        assert_eq!(result[0].0.length, 2.0);
        assert!(result[0].1.is_none());
        cancel.store(true, Ordering::Release);
        assert!(read_audio_takes(vec![path], 4.0, 120.0, usize::MAX, &cancel).is_err());
    }

    #[test]
    fn delayed_take_import_does_not_resurrect_a_dismissed_or_changed_context() {
        for change in ["reopen", "anchor", "revision", "none"] {
            let ctx = egui::Context::default();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("take.wav");
            let audio = AudioData {
                sample_rate: 8000,
                channels: 1,
                samples: vec![0.2; 8000],
            };
            audio.write_wav(&path).unwrap();
            let mut project = Project::new();
            let track = project.add_audio_track("Takes");
            let anchor = Clip::audio("Original", 0.0, 2.0, path.to_string_lossy());
            project.track_mut(track).unwrap().clips.push(anchor.clone());
            let mut app = DawApp::with_context(&ctx, project.clone(), None);
            app.audio_device_enabled = false;
            app.open_audio_takes(&anchor, None);
            let (tx, receiver) = mpsc::channel();
            app.audio_takes_ui.pending = Some(TakeImport {
                receiver,
                track,
                anchor: anchor.clone(),
                revision: app.revision,
                epoch: app.audio_takes_ui.epoch,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            match change {
                "reopen" => {
                    app.close_popup();
                    app.open_audio_takes(&anchor, None);
                }
                "anchor" => app.audio_takes_ui.anchor = Some(new_id()),
                "revision" => app.revision += 1,
                _ => {}
            }
            tx.send(Ok(vec![(
                Clip::audio("Alternative", 0.0, 2.0, path.to_string_lossy()),
                Some(audio),
            )]))
            .unwrap();
            app.poll_audio_take_import(&ctx);
            assert!(app.audio_takes_ui.pending.is_none());
            if change == "none" {
                assert_eq!(
                    app.project.tracks[0]
                        .clips
                        .iter()
                        .filter(|clip| clip.is_take_source())
                        .count(),
                    2
                );
                assert!(app.selected().is_some());
            } else {
                assert_eq!(app.project, project, "stale {change} import committed");
                assert!(!app.modified);
            }
        }
    }
}
