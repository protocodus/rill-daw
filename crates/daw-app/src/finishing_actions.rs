//! Small selection commands that reuse the existing document and routing paths.
use crate::app::{DawApp, WorkspaceView};
use anyhow::{Context, Result, ensure};
use daw_core::{Clip, ClipContent, Project, TrackKind, new_id};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

const MAX_REPEAT_CLIPS: usize = 4096;
type NamedBuses = Vec<(u64, String)>;
const MAX_REPEAT_BYTES: usize = 32 * 1024 * 1024;
const MAX_RECENT_EXPORTS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ExportKind {
    Wav,
    Midi,
}

impl ExportKind {
    fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Midi => "MIDI",
        }
    }
}

struct ExportReceipt {
    kind: ExportKind,
    path: PathBuf,
}

#[derive(Clone)]
struct ExportTarget {
    path: PathBuf,
    include_effect_tails: bool,
}

#[derive(Default)]
pub(crate) struct FinishingActions {
    channels: HashSet<u64>,
    last_export: Option<ExportTarget>,
    pending_export: Option<ExportTarget>,
    recent_exports: Vec<ExportReceipt>,
}

impl FinishingActions {
    pub(crate) fn clear_channels(&mut self) {
        self.channels.clear();
    }

    pub(crate) fn export_started(&mut self, path: PathBuf, include_effect_tails: bool) {
        self.pending_export = Some(ExportTarget {
            path,
            include_effect_tails,
        });
    }

    pub(crate) fn export_finished(&mut self, path: &Path) {
        if let Some(target) = self
            .pending_export
            .take()
            .filter(|target| target.path == path)
        {
            self.last_export = Some(target);
            self.record_export(ExportKind::Wav, path);
        }
    }

    pub(crate) fn midi_export_finished(&mut self, path: &Path) {
        self.record_export(ExportKind::Midi, path);
    }

    fn record_export(&mut self, kind: ExportKind, path: &Path) {
        self.recent_exports
            .retain(|receipt| receipt.kind != kind || receipt.path != path);
        self.recent_exports.insert(
            0,
            ExportReceipt {
                kind,
                path: path.to_owned(),
            },
        );
        self.recent_exports.truncate(MAX_RECENT_EXPORTS);
    }

    pub(crate) fn export_failed(&mut self) {
        self.pending_export = None;
    }
}

impl DawApp {
    pub(crate) fn finishing_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "selected_channels": self.finishing_selected_channels(),
            "last_export": self.finishing.last_export.as_ref().map(|target| serde_json::json!({
                "path": target.path, "include_effect_tails": target.include_effect_tails,
            })),
            "recent_exports": self.finishing.recent_exports.iter().map(|receipt| serde_json::json!({
                "kind": receipt.kind.label(), "path": receipt.path,
            })).collect::<Vec<_>>(),
        })
    }

    /// Receipts describe completed exports, not the current existence of files.
    /// FinishingActions resets with the document; copying never changes status.
    pub(crate) fn recent_exports_menu(&self, ui: &mut egui::Ui) {
        daw_ui::menu_button(ui, "Recent exports", |ui| {
            if self.finishing.recent_exports.is_empty() {
                daw_ui::text::note(ui, "No completed exports in this project");
            } else {
                daw_ui::text::note(ui, "Completed exports");
                for receipt in &self.finishing.recent_exports {
                    let path = receipt.path.to_string_lossy();
                    let name = receipt.path.file_name().unwrap_or(receipt.path.as_os_str());
                    let label = format!("{} · {}", receipt.kind.label(), name.to_string_lossy());
                    ui.push_id((receipt.kind, &receipt.path), |ui| {
                        daw_ui::menu_button(ui, label, |ui| {
                            if daw_ui::button(ui, "Copy path")
                                .on_hover_text(path.as_ref())
                                .clicked()
                            {
                                ui.ctx().copy_text(path.to_string());
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text(path.as_ref());
                    });
                }
            }
        });
    }

    pub(crate) fn finishing_selected_channels(&self) -> Vec<u64> {
        let explicit = self
            .selected_track
            .is_some_and(|id| self.finishing.channels.contains(&id));
        let clips: HashSet<_> = if explicit || self.selected_clip.is_none() {
            HashSet::new()
        } else {
            self.arranger_selection_ids()
                .into_iter()
                .map(|(track, _)| track)
                .collect()
        };
        self.project
            .tracks
            .iter()
            .filter(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                ) && if explicit {
                    self.finishing.channels.contains(&track.id)
                } else if !clips.is_empty() {
                    clips.contains(&track.id)
                } else {
                    self.selected_track == Some(track.id)
                }
            })
            .map(|track| track.id)
            .collect()
    }

    /// Shift-click uses the same additive selection rule as the arrangers.
    pub(crate) fn mixer_toggle_channel_selection(&mut self, track: u64) {
        let mut selected: HashSet<_> = self.finishing_selected_channels().into_iter().collect();
        if !selected.insert(track) {
            selected.remove(&track);
        }
        self.finishing.channels = selected;
        self.selected_track = if self.finishing.channels.contains(&track) {
            Some(track)
        } else {
            self.project
                .tracks
                .iter()
                .find(|track| self.finishing.channels.contains(&track.id))
                .map(|track| track.id)
        };
        self.selected_clip = None;
        self.selected_notes.clear();
    }

    pub(crate) fn mixer_select_channel(&mut self, track: u64, extend: bool) {
        let mut selected = if extend {
            self.finishing_selected_channels().into_iter().collect()
        } else {
            HashSet::new()
        };
        selected.insert(track);
        self.finishing.channels = selected;
        self.selected_track = Some(track);
        self.selected_clip = None;
        self.selected_notes.clear();
    }

    pub(crate) fn mixer_channel_selection_action(
        &mut self,
        track: u64,
        choice: daw_ui::ChannelSelectionChoice,
    ) {
        use daw_ui::ChannelSelectionChoice as Choice;
        match choice {
            Choice::SelectOnly => self.mixer_select_channel(track, false),
            Choice::Add => self.mixer_select_channel(track, true),
            Choice::Remove => {
                if self.finishing_selected_channels().contains(&track) {
                    self.mixer_toggle_channel_selection(track);
                }
            }
            Choice::All => {
                self.finishing.channels = self
                    .project
                    .tracks
                    .iter()
                    .filter(|track| {
                        matches!(
                            track.kind,
                            TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                        )
                    })
                    .map(|track| track.id)
                    .collect();
                self.selected_track = Some(track);
                self.selected_clip = None;
                self.selected_notes.clear();
            }
            Choice::Clear => {
                self.finishing.channels.clear();
                self.selected_track = None;
                self.selected_clip = None;
                self.selected_notes.clear();
            }
        }
    }

    pub(crate) fn finishing_actions_menu(&mut self, ui: &mut egui::Ui) {
        let selection = self.arranger_selection_ids();
        let span = self.arranger_phrase_span().or_else(|| {
            selection
                .iter()
                .filter_map(|(track, clip)| self.project.clip(*track, *clip))
                .map(|clip| (clip.start, clip.end()))
                .reduce(|(start, end), (a, b)| (start.min(a), end.max(b)))
        });
        let can_repeat =
            !selection.is_empty() && span.is_some_and(|(_, end)| self.project.loop_end > end);
        if ui.add_enabled(can_repeat, daw_ui::button_widget("Repeat to loop end"))
            .on_hover_text("Create independent copies of the selected phrase, including rests, up to the loop end. The last copy is trimmed to fit.")
            .clicked()
        {
            self.repeat_selection_to_loop_end();
            ui.close();
        }
        let channels = self.finishing_selected_channels();
        if !channels.is_empty() {
            let mut choice = None;
            daw_ui::menu_button(ui, "Route selected channels", |ui| {
                let (outputs, sends) = self.batch_route_destinations(&channels);
                daw_ui::text::note(ui, format!("{} selected", channels.len()));
                choice = daw_ui::batch_route_choices(ui, &outputs, &sends);
            });
            if let Some(choice) = choice {
                self.route_selected_channels(&channels, choice);
            }
        }
    }

    pub(crate) fn batch_route_destinations(&self, channels: &[u64]) -> (NamedBuses, NamedBuses) {
        let mut outputs = Vec::new();
        let mut sends = Vec::new();
        // Eligibility is calculated as an intersection, while final application
        // still validates the complete batch on a temporary document.
        let eligible: HashSet<_> = self
            .project
            .eligible_output_buses_for(channels)
            .unwrap_or_default()
            .into_iter()
            .map(|track| track.id)
            .collect();
        let selected: HashSet<_> = channels.iter().copied().collect();
        let mut send_counts: HashMap<u64, usize> = HashMap::new();
        for track in self
            .project
            .tracks
            .iter()
            .filter(|track| selected.contains(&track.id))
        {
            for send in &track.sends {
                *send_counts.entry(send.target_bus).or_default() += 1;
            }
        }
        for bus in self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Bus)
        {
            if eligible.contains(&bus.id) {
                outputs.push((bus.id, bus.name.clone()));
                if send_counts.get(&bus.id).copied().unwrap_or(0) < selected.len() {
                    sends.push((bus.id, bus.name.clone()));
                }
            }
        }
        (outputs, sends)
    }

    pub(crate) fn route_selected_channels(
        &mut self,
        channels: &[u64],
        choice: daw_ui::BatchRouteChoice,
    ) {
        let mut candidate = self.project.clone();
        let result = apply_batch_route(&mut candidate, channels, choice);
        match result {
            Ok(()) if candidate != self.project => {
                self.begin_edit();
                self.project = candidate;
                self.edit_label = Some(
                    match choice {
                        daw_ui::BatchRouteChoice::Output(_) => "Route selected channels",
                        daw_ui::BatchRouteChoice::Send { .. } => "Send selected channels",
                    }
                    .into(),
                );
                self.changed();
                self.message(Ok(()), "Selected channels routed");
            }
            Ok(()) => {}
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub(crate) fn repeat_selection_to_loop_end(&mut self) {
        let Some((start, end, regions)) = self.arranger_phrase_regions() else {
            return;
        };
        let mut candidate = self.project.clone();
        match repeat_regions(&mut candidate, &regions, start, end, self.project.loop_end) {
            Ok(ids) if !ids.is_empty() => {
                self.begin_edit();
                self.project = candidate;
                self.arranger_select_clips(ids, Some((end, self.project.loop_end)));
                self.edit_label = Some("Repeat to loop end".into());
                self.changed();
                self.message(Ok(()), "Phrase repeated to loop end");
            }
            Ok(_) => {}
            Err(error) => self.message(Err(error), ""),
        }
    }

    /// This command is explicitly labelled as replacing the previous mixdown.
    /// It uses the regular snapshot queue and atomic, source-guarded exporter.
    pub(crate) fn reexport_menu(&mut self, ui: &mut egui::Ui) {
        let target = self.finishing.last_export.as_ref();
        let enabled = target.is_some() && self.export_job.is_none();
        let response = ui.add_enabled(
            enabled,
            daw_ui::button_widget("Re-export last WAV (replace)"),
        );
        let response = if let Some(target) = target {
            response.on_hover_text(target.path.display().to_string())
        } else {
            response
        };
        if response.clicked() {
            if let Some(target) = self.finishing.last_export.clone() {
                self.export_to_with_tails(target.path, target.include_effect_tails);
            }
            ui.close();
        }
    }

    pub(crate) fn mixer_batch_routing_menu(&mut self, ui: &mut egui::Ui) {
        let channels = self.finishing_selected_channels();
        if self.view != WorkspaceView::Mixer || channels.len() < 2 {
            return;
        }
        let mut choice = None;
        daw_ui::toolbar(ui, |ui| {
            daw_ui::text::note(ui, format!("{} channels selected", channels.len()));
            daw_ui::action_menu(ui, "Route selected channels", |ui| {
                let (outputs, sends) = self.batch_route_destinations(&channels);
                choice = daw_ui::batch_route_choices(ui, &outputs, &sends);
            });
        });
        if let Some(choice) = choice {
            self.route_selected_channels(&channels, choice);
        }
    }
}

fn apply_batch_route(
    project: &mut Project,
    channels: &[u64],
    choice: daw_ui::BatchRouteChoice,
) -> Result<()> {
    ensure!(!channels.is_empty(), "Select at least one channel");
    let selected: HashSet<_> = channels.iter().copied().collect();
    let mut found = 0;
    for track in project
        .tracks
        .iter_mut()
        .filter(|track| selected.contains(&track.id))
    {
        found += 1;
        ensure!(
            matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            "Select audio, MIDI, or bus channels"
        );
        match choice {
            daw_ui::BatchRouteChoice::Output(target) => track.output_bus = target,
            daw_ui::BatchRouteChoice::Send { bus, pre_fader } => {
                if !track.sends.iter().any(|send| send.target_bus == bus) {
                    track.sends.push(daw_core::Send {
                        target_bus: bus,
                        gain: 0.25,
                        pre_fader,
                    });
                }
            }
        }
    }
    ensure!(found == selected.len(), "Source channel not found");
    project.routing_order().map(|_| ())
}

fn repeat_regions(
    project: &mut Project,
    regions: &[(u64, Clip)],
    start: f64,
    end: f64,
    loop_end: f64,
) -> Result<Vec<(u64, u64)>> {
    ensure!(!regions.is_empty(), "Select at least one clip");
    ensure!(
        start.is_finite() && end.is_finite() && loop_end.is_finite() && end > start,
        "Select a valid phrase"
    );
    if loop_end <= end {
        return Ok(Vec::new());
    }
    let length = end - start;
    let repeats = ((loop_end - end) / length).ceil() as usize;
    ensure!(
        repeats
            .checked_mul(regions.len())
            .is_some_and(|count| count <= MAX_REPEAT_CLIPS),
        "Repeat would create more than {MAX_REPEAT_CLIPS} clips; select a shorter destination"
    );
    let source_bytes = regions.iter().fold(0usize, |bytes, (_, clip)| {
        let content = match &clip.content {
            ClipContent::Midi { notes } => notes
                .len()
                .saturating_mul(std::mem::size_of::<daw_core::Note>()),
            ClipContent::Audio { path, .. } => path.len(),
        };
        bytes
            .saturating_add(std::mem::size_of::<Clip>())
            .saturating_add(clip.name.len())
            .saturating_add(content)
    });
    ensure!(
        source_bytes.saturating_mul(repeats) <= MAX_REPEAT_BYTES,
        "Repeat would create more than 32 MiB of clip data; select a shorter destination"
    );
    let mut ids = Vec::new();
    for index in 1..=repeats {
        for (track, source) in regions {
            let position = source.start + index as f64 * length;
            if position >= loop_end {
                continue;
            }
            let mut clip = source.clone();
            clip.id = new_id();
            clip.link_group = None;
            clip.audio_take = None;
            if let ClipContent::Midi { notes } = &mut clip.content {
                for note in notes {
                    note.id = new_id();
                }
            }
            clip.start = position;
            let id = clip.id;
            project
                .track_mut(*track)
                .context("Source track not found")?
                .clips
                .push(clip);
            let duration = source.length.min(loop_end - position);
            if duration < source.length {
                project.trim_clip(*track, id, position, duration)?;
            }
            ids.push((*track, id));
        }
    }
    project.validate()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_batch_route_keeps_the_entire_document_and_history_unchanged() {
        let context = egui::Context::default();
        let mut project = Project::new();
        let audio = project.add_audio_track("Audio");
        let first = project.add_bus("First");
        let second = project.add_bus("Second");
        project.set_output(second, Some(first)).unwrap();
        let mut app = DawApp::with_context(&context, project.clone(), None);
        // Audio alone may target Second, but First would close a feedback path.
        // The preceding valid assignment in the same batch must not leak.
        app.route_selected_channels(
            &[audio, first],
            daw_ui::BatchRouteChoice::Output(Some(second)),
        );
        assert_eq!(app.project, project);
        assert!(app.error);
        assert!(!app.edit_dirty);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn repeat_limit_fails_before_allocating_or_changing_a_clip() {
        let context = egui::Context::default();
        let mut project = Project::new();
        let midi = project.add_midi_track("Tiny phrase");
        let clip = Clip::midi("Tiny", 0.0, 0.001, Vec::new());
        let id = clip.id;
        project.track_mut(midi).unwrap().clips.push(clip);
        project.loop_end = 100.0;
        let mut app = DawApp::with_context(&context, project.clone(), None);
        app.arranger_select_clips([(midi, id)], None);
        app.repeat_selection_to_loop_end();
        assert_eq!(app.project, project);
        assert!(app.error);
        assert!(!app.edit_dirty);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn dense_midi_repeat_is_bounded_even_when_the_clip_count_is_small() {
        let mut project = Project::new();
        let midi = project.add_midi_track("Dense phrase");
        let clip = Clip::midi(
            "Dense",
            0.0,
            1.0,
            (0..20_000)
                .map(|_| daw_core::Note::new(60, 0.0, 0.125, 100))
                .collect(),
        );
        project.track_mut(midi).unwrap().clips.push(clip.clone());
        let before = project.clone();
        assert!(repeat_regions(&mut project, &[(midi, clip)], 0.0, 1.0, 101.0).is_err());
        assert_eq!(project, before);
    }

    #[test]
    fn final_repeat_trim_keeps_reversed_source_mapping_and_the_original_envelope() {
        let mut project = Project::new();
        let audio = project.add_audio_track("Reverse");
        let mut original = Clip::audio("Reverse", 0.5, 1.0, "original.wav");
        original.source_rate = 2.0;
        if let ClipContent::Audio {
            reverse, fade_out, ..
        } = &mut original.content
        {
            *reverse = true;
            *fade_out = 0.5;
        }
        project
            .track_mut(audio)
            .unwrap()
            .clips
            .push(original.clone());
        let region = original
            .audio_region(original.start, original.end())
            .unwrap();
        let ids = repeat_regions(&mut project, &[(audio, region)], 0.0, 2.0, 3.0).unwrap();
        assert_eq!(ids.len(), 1);
        let result = project.clip(ids[0].0, ids[0].1).unwrap();
        assert_eq!((result.start, result.length), (2.5, 0.5));
        for local in [0.0, 0.125, 0.499] {
            assert!(
                (result.audio_source_frame(local, 120.0, 48_000)
                    - original.audio_source_frame(local, 120.0, 48_000))
                .abs()
                    < 1e-8
            );
            assert!(
                (result.audio_envelope_gain(local) - original.audio_envelope_gain(local)).abs()
                    < 1e-7
            );
        }
        assert_eq!(project.clip(audio, original.id), Some(&original));
    }
}
