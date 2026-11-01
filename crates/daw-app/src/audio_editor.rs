//! Non-destructive audio editing in the shared clip workspace.
use crate::{app::DawApp, theme::*};
use daw_core::*;
use daw_ui::{EditorTool, TimelineBounds, TimelinePane, TimelineViewport, WaveformColumn};
use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const TOOLS: &[EditorTool] = &[
    EditorTool::Select,
    EditorTool::Cursor,
    EditorTool::Cut,
    EditorTool::Stretch,
    EditorTool::Slip,
    EditorTool::FadeIn,
    EditorTool::FadeOut,
];
// A new project or editor may discard its result, but it must not multiply
// whole-file analysis workers while a previous file is still being decoded.
static ANALYSIS_RUNNING: AtomicBool = AtomicBool::new(false);
type AnalysisResult = Arc<Mutex<Option<Result<Analysis, String>>>>;
#[derive(Clone)]
enum Analysis {
    Gain(f32),
    Transients(Vec<f64>),
}
#[derive(Clone)]
struct Pending {
    clip: Clip,
    tempo: f64,
    settings: TransientSettings,
    result: AnalysisResult,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum AudioDragMode {
    Move,
    TrimStart,
    TrimEnd,
    Stretch,
    Range,
    Slip,
}
#[derive(Clone)]
struct AudioDrag {
    original: Clip,
    mode: AudioDragMode,
    origin: f64,
    markers: Vec<f64>,
}
#[derive(Clone)]
struct AudioView {
    owner: u64,
    view: TimelineViewport,
    fitted: bool,
    tool: EditorTool,
    clipboard: Option<Clip>,
    transients_open: bool,
    settings: TransientSettings,
    preview: Vec<f64>,
    preview_source: Option<(Clip, f64)>,
    pending: Option<Pending>,
    error: String,
    amplitude_reference: f32,
    drag: Option<AudioDrag>,
    range: Option<(f64, f64)>,
    snap_edges: bool,
    snap_transients: bool,
    snap_zero_crossings: bool,
    show_markers: bool,
    show_neighbours: bool,
    grid_slice_open: bool,
    slice_grid: f64,
    reveal_owner: bool,
}
impl Default for AudioView {
    fn default() -> Self {
        Self {
            owner: 0,
            view: TimelineViewport::default(),
            fitted: false,
            tool: EditorTool::Cursor,
            clipboard: None,
            transients_open: false,
            settings: TransientSettings::default(),
            preview: Vec::new(),
            preview_source: None,
            pending: None,
            error: String::new(),
            amplitude_reference: 1.0,
            drag: None,
            range: None,
            snap_edges: true,
            snap_transients: true,
            snap_zero_crossings: false,
            show_markers: true,
            show_neighbours: true,
            grid_slice_open: false,
            slice_grid: 0.25,
            reveal_owner: false,
        }
    }
}
#[derive(Clone)]
enum Edit {
    Clone,
    Unlink,
    Gain(f32),
    Processing(AudioProcessing),
    Fades(f64, f64, FadeCurve, FadeCurve),
    Crossfade(Option<AudioCrossfade>),
    Reverse,
    Split(f64),
    Trim(f64, f64),
    Slip(Clip, f64, f64),
    Transients(Vec<f64>),
    GridSlices(Vec<f64>),
    Bounds(Clip, f64, f64, bool),
    Move(f64, bool),
    SplitRange(f64, f64),
    RippleRange(f64, f64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioClipboardAction {
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
}

pub(crate) fn has_audio_clipboard(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<AudioView>(egui::Id::new("audio_editor_state")))
        .is_some_and(|state| state.clipboard.is_some())
}

pub(crate) fn clear_audio_clipboard(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let id = egui::Id::new("audio_editor_state");
        if let Some(mut state) = data.get_temp::<AudioView>(id) {
            state.clipboard = None;
            data.insert_temp(id, state);
        }
    });
}

pub(crate) fn reset_audio_editor(ctx: &egui::Context) {
    crate::audio_silence::reset(ctx);
    ctx.data_mut(|data| {
        data.remove::<AudioView>(egui::Id::new("audio_editor_state"));
        data.remove::<serde_json::Value>(egui::Id::new("audio_editor_diagnostics"));
    });
}

pub(crate) fn audio_fade_bounds(clip: &Clip) -> daw_ui::ClipFadeBounds {
    let ClipContent::Audio {
        fade_in,
        fade_out,
        envelope,
        ..
    } = clip.content
    else {
        unreachable!("audio fade handles require an audio clip")
    };
    let region = envelope.unwrap_or(AudioEnvelope {
        offset: 0.0,
        length: clip.source_length(),
    });
    daw_ui::ClipFadeBounds {
        start: clip.start,
        end: clip.end(),
        fade_in_end: clip.start + (fade_in - region.offset) / clip.source_rate,
        fade_out_start: clip.start + (region.length - region.offset - fade_out) / clip.source_rate,
    }
}

/// Match the existing fade tools: independent clips reanchor explicitly edited
/// fades, while ghost clips keep the source envelope shared across instances.
pub(crate) fn audio_fade_handle_edit(
    clip: &Clip,
    edge: daw_ui::ClipFadeEdge,
    beat: f64,
) -> (f64, f64, FadeCurve, FadeCurve) {
    let ClipContent::Audio {
        fade_in,
        fade_out,
        fade_in_curve,
        fade_out_curve,
        envelope,
        ..
    } = clip.content
    else {
        unreachable!("audio fade handles require an audio clip")
    };
    let local = AudioEnvelope {
        offset: 0.0,
        length: clip.source_length(),
    };
    let region = if clip.link_group.is_some() {
        envelope.unwrap_or(local)
    } else {
        local
    };
    let position = region.offset + (beat - clip.start) * clip.source_rate;
    let (attack, release) = match edge {
        daw_ui::ClipFadeEdge::In => (
            position.clamp(0.0, region.length),
            fade_out.min(region.length),
        ),
        daw_ui::ClipFadeEdge::Out => (
            fade_in.min(region.length),
            (region.length - position).clamp(0.0, region.length),
        ),
    };
    (attack, release, fade_in_curve, fade_out_curve)
}

/// Explicit cue navigation keeps the current scale and tool. Analysis remains
/// tied to its original clip and is discarded by the usual identity checks.
pub(crate) fn select_audio_clip_preserving_view(ctx: &egui::Context, clip: u64, _start: f64) {
    ctx.data_mut(|data| {
        let id = egui::Id::new("audio_editor_state");
        let mut state = data.get_temp::<AudioView>(id).unwrap_or_default();
        if state.owner != clip {
            state.owner = clip;
            state.preview.clear();
            state.preview_source = None;
            state.range = None;
            state.drag = None;
            state.error.clear();
            state.reveal_owner = true;
        }
        data.insert_temp(id, state);
    });
}

/// Arranger and editor fade gestures share the same source-envelope mapping.
pub(crate) fn fade_parameters_at(
    clip: &Clip,
    beat: f64,
    incoming: bool,
) -> Option<(f64, f64, FadeCurve, FadeCurve)> {
    let ClipContent::Audio {
        fade_in,
        fade_out,
        fade_in_curve,
        fade_out_curve,
        envelope,
        ..
    } = clip.content
    else {
        return None;
    };
    let local = AudioEnvelope {
        offset: 0.0,
        length: clip.source_length(),
    };
    let region = if clip.link_group.is_some() {
        envelope.unwrap_or(local)
    } else {
        local
    };
    let at = (beat - clip.start) * clip.source_rate;
    let (fi, fo) = if incoming {
        (
            (region.offset + at).clamp(0.0, region.length),
            fade_out.min(region.length),
        )
    } else {
        (
            fade_in.min(region.length),
            (region.length - region.offset - at).clamp(0.0, region.length),
        )
    };
    Some((fi, fo, fade_in_curve, fade_out_curve))
}

fn grid_slice_points(clip: &Clip, range: Option<(f64, f64)>, grid: f64) -> Vec<f64> {
    if !grid.is_finite() || grid <= 0.0 {
        return Vec::new();
    }
    let (start, end) = range.unwrap_or((clip.start, clip.end()));
    let start = start.max(clip.start);
    let end = end.min(clip.end());
    let mut index = (start / grid).floor() + 1.0;
    let mut points = Vec::new();
    while points.len() <= 2048 {
        let beat = index * grid;
        if beat >= end - 1e-8 {
            break;
        }
        if beat > clip.start + 1e-8 && beat < clip.end() - 1e-8 {
            points.push(beat - clip.start);
        }
        index += 1.0;
    }
    points
}

fn selected_transients<'a>(state: &'a AudioView, clip: &Clip) -> impl Iterator<Item = f64> + 'a {
    let clip_start = clip.start;
    let range = state.range;
    state.preview.iter().copied().filter(move |offset| {
        range.is_none_or(|(start, end)| {
            let beat = clip_start + offset;
            beat >= start && beat <= end
        })
    })
}

fn adjacent_transient(points: &[f64], start: f64, cursor: f64, next: bool) -> Option<f64> {
    let beats = points.iter().map(|point| start + point);
    if next {
        beats
            .filter(|beat| *beat > cursor + 1e-7)
            .min_by(f64::total_cmp)
    } else {
        beats
            .filter(|beat| *beat < cursor - 1e-7)
            .max_by(f64::total_cmp)
    }
}

fn reveal_beat(view: &mut TimelineViewport, beat: f64, width: f32) {
    let visible = width.max(100.0) as f64 / view.pixels_per_beat as f64;
    if beat < view.start_beat || beat > view.start_beat + visible {
        view.start_beat = (beat - visible * 0.25).max(0.0);
    }
}

/// A magnetic target is measured in screen pixels, so zooming in never traps
/// an edit in an unexpectedly wide region. Shift is shared with the arranger.
fn snap_audio_beat(
    raw: f64,
    grid: f64,
    pixels_per_beat: f32,
    targets: &[f64],
    bypass: bool,
) -> (f64, Option<f64>) {
    if bypass {
        return (raw, None);
    }
    let nearest = targets
        .iter()
        .copied()
        .filter(|target| ((target - raw).abs() * pixels_per_beat as f64) <= 8.0)
        .min_by(|a, b| (a - raw).abs().total_cmp(&(b - raw).abs()));
    if let Some(target) = nearest {
        (target, Some(target))
    } else if grid > 0.0 {
        ((raw / grid).round() * grid, None)
    } else {
        (raw, None)
    }
}

fn snap_audio_move(
    raw: f64,
    length: f64,
    grid: f64,
    pixels_per_beat: f32,
    targets: &[f64],
    bypass: bool,
) -> (f64, Option<f64>) {
    let (start, start_target) = snap_audio_beat(raw, grid, pixels_per_beat, targets, bypass);
    let (end, end_target) = snap_audio_beat(raw + length, 0.0, pixels_per_beat, targets, bypass);
    if end_target.is_some()
        && (start_target.is_none() || (end - length - raw).abs() < (start - raw).abs())
    {
        (end - length, end_target)
    } else {
        (start, start_target)
    }
}

impl DawApp {
    pub(crate) fn audio_source_beats(&self, clip: &Clip) -> Option<f64> {
        let ClipContent::Audio { path, .. } = &clip.content else {
            return None;
        };
        self.waveforms.get(path).map(|audio| {
            audio.frames() as f64 / f64::from(audio.sample_rate) * self.project.tempo / 60.0
        })
    }
    fn audio_clipboard_edit(
        &mut self,
        ctx: &egui::Context,
        state: &mut AudioView,
        action: AudioClipboardAction,
    ) -> anyhow::Result<()> {
        let track = self
            .selected_track
            .ok_or_else(|| anyhow::anyhow!("Select an audio track"))?;
        if matches!(action, AudioClipboardAction::Paste) {
            let source = state
                .clipboard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Copy an audio selection first"))?;
            self.begin_edit();
            let id = self.project.paste_audio_clip(track, source, self.cursor)?;
            self.selected_clip = Some(id);
            self.edit_label = Some("Paste audio selection".into());
            state.range = None;
            self.changed();
            return Ok(());
        }
        let clip = self
            .selected()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Select an audio clip"))?;
        let (start, end) = state
            .range
            .filter(|_| state.owner == clip.id)
            .unwrap_or((clip.start, clip.end()));
        let region = self.project.copy_audio_range(track, clip.id, start, end)?;
        if matches!(action, AudioClipboardAction::Copy) {
            // egui-winit only emits native Paste when the system clipboard is
            // nonempty. Publish a human-readable marker; audio stays internal.
            self.clear_arranger_clipboard();
            ctx.copy_text(format!("RILL audio selection · {}", region.name));
            state.clipboard = Some(region);
            return Ok(());
        }
        self.begin_edit();
        if matches!(action, AudioClipboardAction::Duplicate) {
            let id = self.project.paste_audio_clip(track, &region, end)?;
            self.selected_clip = Some(id);
            self.edit_label = Some("Duplicate audio selection".into());
        } else {
            let remaining = self
                .project
                .delete_audio_range(track, clip.id, start, end)?;
            if matches!(action, AudioClipboardAction::Cut) {
                self.clear_arranger_clipboard();
                ctx.copy_text(format!("RILL audio selection · {}", region.name));
                state.clipboard = Some(region);
            }
            self.selected_clip = remaining.first().copied();
            self.edit_label = Some(
                if matches!(action, AudioClipboardAction::Cut) {
                    "Cut audio selection"
                } else {
                    "Delete audio selection"
                }
                .into(),
            );
        }
        state.range = None;
        self.changed();
        Ok(())
    }

    /// Route menus and keys through the same selection-aware transaction.
    pub(crate) fn audio_clipboard_action(
        &mut self,
        ctx: &egui::Context,
        action: AudioClipboardAction,
    ) -> bool {
        let paste_in_arranger = self.view == crate::app::WorkspaceView::Arrangement
            && matches!(action, AudioClipboardAction::Paste);
        if (self.view != crate::app::WorkspaceView::Clip && !paste_in_arranger)
            || !self
                .selected_track
                .and_then(|id| self.project.track(id))
                .is_some_and(|track| track.kind == TrackKind::Audio)
        {
            return false;
        }
        let id = egui::Id::new("audio_editor_state");
        let mut state = ctx.data_mut(|data| data.get_temp::<AudioView>(id).unwrap_or_default());
        match self.audio_clipboard_edit(ctx, &mut state, action) {
            Ok(()) => state.error.clear(),
            Err(error) => state.error = error.to_string(),
        }
        ctx.data_mut(|data| data.insert_temp(id, state));
        true
    }

    pub(crate) fn nudge_selected_clip(&mut self, ctx: &egui::Context, direction: i32) {
        let Some((track, clip)) = self.selected_track.zip(self.selected().cloned()) else {
            return;
        };
        let step = daw_ui::NudgeUnit::load(ctx).beats(
            self.snap,
            self.project.tempo,
            self.settings.sample_rate,
        );
        let start = (clip.start + step * f64::from(direction)).clamp(0.0, MAX_BEAT - clip.length);
        if start == clip.start {
            return;
        }
        self.begin_edit();
        match self.project.move_clip(track, clip.id, start) {
            Ok(()) => {
                self.edit_label = Some("Nudge clip".into());
                self.changed();
                ctx.data_mut(|data| {
                    let id = egui::Id::new("audio_editor_state");
                    if let Some(mut state) = data.get_temp::<AudioView>(id) {
                        state.range = None;
                        data.insert_temp(id, state);
                    }
                });
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    /// Called after modal and text-input guards, before global clip shortcuts.
    pub(crate) fn audio_edit_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        use crate::app::WorkspaceView;
        if egui::Popup::is_any_open(ctx) {
            return false;
        }
        let audio = self.view == WorkspaceView::Clip
            && self
                .selected_track
                .and_then(|id| self.project.track(id))
                .is_some_and(|track| track.kind == TrackKind::Audio);
        let nudge = audio || self.view == WorkspaceView::Arrangement;
        let paste = audio
            || (self.view == WorkspaceView::Arrangement
                && self
                    .selected_track
                    .and_then(|id| self.project.track(id))
                    .is_some_and(|track| track.kind == TrackKind::Audio));
        let commands = ctx.input_mut(|input| {
            let mut commands = Vec::new();
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
            input.events.retain(|event| {
                let command = match event {
                    egui::Event::Copy if audio => Some((Some(AudioClipboardAction::Copy), 0)),
                    egui::Event::Cut if audio => Some((Some(AudioClipboardAction::Cut), 0)),
                    egui::Event::Paste(_) if paste => Some((Some(AudioClipboardAction::Paste), 0)),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if (audio || paste || nudge)
                        && modifiers.command
                        && !modifiers.shift
                        && !modifiers.alt =>
                    {
                        match key {
                            egui::Key::C if audio && !native_copy => {
                                Some((Some(AudioClipboardAction::Copy), 0))
                            }
                            egui::Key::X if audio && !native_cut => {
                                Some((Some(AudioClipboardAction::Cut), 0))
                            }
                            egui::Key::V if paste && !native_paste => {
                                Some((Some(AudioClipboardAction::Paste), 0))
                            }
                            egui::Key::D if audio => {
                                Some((Some(AudioClipboardAction::Duplicate), 0))
                            }
                            egui::Key::ArrowLeft if nudge => Some((None, -1)),
                            egui::Key::ArrowRight if nudge => Some((None, 1)),
                            _ => None,
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.any() => match key {
                        egui::Key::Delete | egui::Key::Backspace if audio => {
                            Some((Some(AudioClipboardAction::Delete), 0))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(command) = command {
                    commands.push(command);
                    false
                } else {
                    true
                }
            });
            commands
        });
        let handled = !commands.is_empty();
        for (action, direction) in commands {
            if let Some(action) = action {
                self.audio_clipboard_action(ctx, action);
            } else {
                self.nudge_selected_clip(ctx, direction);
            }
        }
        handled
    }

    pub(crate) fn audio_editor(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self.selected().cloned() else {
            return;
        };
        let Some(track) = self.selected_track else {
            return;
        };
        let ClipContent::Audio {
            path,
            gain,
            fade_in,
            fade_out,
            fade_in_curve,
            fade_out_curve,
            reverse,
            envelope,
            processing,
            crossfade,
            ..
        } = &clip.content
        else {
            return;
        };
        let id = egui::Id::new("audio_editor_state");
        let mut state = ui
            .ctx()
            .data_mut(|d| d.get_temp::<AudioView>(id).unwrap_or_default());
        if state.owner != clip.id {
            state.owner = clip.id;
            state.fitted = false;
            state.preview.clear();
            state.preview_source = None;
            // Keep the sole in-flight worker until completion, even when its
            // result belongs to another clip. The identity check below drops it.
            state.drag = None;
            state.range = None;
            state.error.clear();
        }
        if state
            .preview_source
            .as_ref()
            .is_some_and(|(source, tempo)| source != &clip || *tempo != self.project.tempo)
        {
            state.preview.clear();
            state.preview_source = None;
        }
        let mut edit = None;
        let mut open_crossfade = false;
        let mut clipboard_action = None;
        let mut nudge = 0;
        if let Some(pending) = state.pending.clone() {
            let finished = pending
                .result
                .lock()
                .ok()
                .and_then(|mut result| result.take());
            if let Some(result) = finished {
                state.pending = None;
                if pending.clip == clip
                    && pending.tempo == self.project.tempo
                    && pending.settings == state.settings
                {
                    match result {
                        Ok(Analysis::Gain(gain)) => edit = Some(Edit::Gain(gain)),
                        Ok(Analysis::Transients(points)) => {
                            state.preview = points;
                            state.preview_source = Some((clip.clone(), self.project.tempo));
                        }
                        Err(error) => state.error = error,
                    }
                }
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
        daw_ui::editor_tool_shortcuts(ui, &mut state.tool, TOOLS);
        if !ui.input(|input| input.pointer.primary_down()) {
            state.drag = None;
        }
        if let Some((start, end)) = state.range {
            state.range = (end.min(clip.end()) > start.max(clip.start))
                .then_some((start.max(clip.start), end.min(clip.end())));
        }
        let view_before = state.view;
        let had_view = state.fitted;
        let mut view_action = daw_ui::TimelineViewAction::None;
        let mut fit = false;
        let mut analyze = None;
        let mut gain_rect = Rect::NOTHING;
        let mut sensitivity_rect = Rect::NOTHING;
        let mut fi = *fade_in / clip.source_rate;
        let mut fo = *fade_out / clip.source_rate;
        let mut fic = *fade_in_curve;
        let mut foc = *fade_out_curve;
        // Linked fades retain source coordinates when this instance is trimmed.
        // Independent fade edits keep their established local reanchor behavior.
        let local_region = AudioEnvelope {
            offset: 0.0,
            length: clip.source_length(),
        };
        let fade_region = if clip.link_group.is_some() {
            envelope.unwrap_or(local_region)
        } else {
            local_region
        };
        daw_ui::toolbar(ui, |ui| {
            daw_ui::editor_tools_with_context(
                ui,
                &mut state.tool,
                TOOLS,
                daw_ui::ToolHintContext::Audio,
            );
            ui.add_space(daw_ui::tokens::SPACE_SM);
            nudge = daw_ui::nudge_menu(ui, true);
            daw_ui::quiet_action_menu(ui, "Clip", |ui| {
                daw_ui::command_menu_contents(ui, |ui| {
                    if daw_ui::button(ui, "Copy").clicked() {
                        clipboard_action = Some(AudioClipboardAction::Copy);
                        ui.close();
                    }
                    if daw_ui::button(ui, "Cut selection")
                        .on_hover_text("Cut selected time, leaving a gap · Ctrl/Cmd+X")
                        .clicked()
                    {
                        clipboard_action = Some(AudioClipboardAction::Cut);
                        ui.close();
                    }
                    if ui
                        .add_enabled(state.clipboard.is_some(), daw_ui::button_widget("Paste"))
                        .clicked()
                    {
                        clipboard_action = Some(AudioClipboardAction::Paste);
                        ui.close();
                    }
                    if daw_ui::button(
                        ui,
                        if state.range.is_some() {
                            "Duplicate selection"
                        } else {
                            "Duplicate clip"
                        },
                    )
                    .on_hover_text(if state.range.is_some() {
                        "Copy selected time immediately after the selection · Ctrl/Cmd+D"
                    } else {
                        "Create an independent copy immediately after this clip · Ctrl/Cmd+D"
                    })
                    .clicked()
                    {
                        clipboard_action = Some(AudioClipboardAction::Duplicate);
                        ui.close();
                    }
                    if daw_ui::button(ui, "Delete selection")
                        .on_hover_text("Remove selected time, leaving a gap · Delete")
                        .clicked()
                    {
                        clipboard_action = Some(AudioClipboardAction::Delete);
                        ui.close();
                    }
                    ui.separator();
                    if daw_ui::button(ui, "Clip start").clicked() {
                        self.seek(clip.start);
                        ui.close();
                    }
                    if daw_ui::button(ui, "Clip end").clicked() {
                        self.seek(clip.end());
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.cursor < clip.end(), daw_ui::button_widget("Set start"))
                        .on_hover_text("Trim the clip start to the cursor")
                        .clicked()
                    {
                        edit = Some(Edit::Trim(self.cursor, clip.end() - self.cursor));
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.cursor > clip.start, daw_ui::button_widget("Set end"))
                        .on_hover_text("Trim the clip end to the cursor")
                        .clicked()
                    {
                        edit = Some(Edit::Trim(clip.start, self.cursor - clip.start));
                        ui.close();
                    }
                    if daw_ui::button(ui, "Snap end")
                        .on_hover_text("Move the clip end to the nearest grid division")
                        .clicked()
                    {
                        let end = self
                            .snapped(clip.end())
                            .max(clip.start + self.snap.max(0.001));
                        edit = Some(Edit::Trim(clip.start, end - clip.start));
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.cursor > clip.start && self.cursor < clip.end(),
                            daw_ui::button_widget("Split at cursor"),
                        )
                        .clicked()
                    {
                        edit = Some(Edit::Split(self.cursor));
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            state.range.is_some(),
                            daw_ui::button_widget("Split selection"),
                        )
                        .clicked()
                    {
                        let (start, end) = state.range.unwrap();
                        edit = Some(Edit::SplitRange(start, end));
                        ui.close();
                    }
                    if daw_ui::button(ui, "Slice on grid…")
                        .on_hover_text("Preview cuts at grid positions, then split as one edit")
                        .clicked()
                    {
                        state.grid_slice_open = true;
                        state.transients_open = false;
                        state.slice_grid = if self.snap > 0.0 { self.snap } else { 0.25 };
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            state.range.is_some(),
                            daw_ui::button_widget("Trim to selection"),
                        )
                        .clicked()
                    {
                        let (start, end) = state.range.unwrap();
                        edit = Some(Edit::Trim(start, end - start));
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            state.range.is_some(),
                            daw_ui::button_widget("Clear selection"),
                        )
                        .clicked()
                    {
                        state.range = None;
                        ui.close();
                    }
                    if daw_ui::button(ui, "Takes…").clicked() {
                        self.open_audio_takes(&clip, state.range);
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(state.range.is_some(), daw_ui::button_widget("Ripple delete selection"))
                    .on_hover_text("Remove this time from the selected audio track and close the gap. Other tracks stay in place.")
                    .clicked() {
                    let (start, end) = state.range.unwrap();
                    edit = Some(Edit::RippleRange(start, end));
                    ui.close();
                }
                    if daw_ui::button(ui, "Ghost copy")
                        .on_hover_text("Create another clip sharing its name and edits")
                        .clicked()
                    {
                        edit = Some(Edit::Clone);
                        ui.close();
                    }
                    if clip.link_group.is_some()
                        && daw_ui::button(ui, "Make independent")
                            .on_hover_text(
                                "Disconnect this ghost while keeping its current name and edits",
                            )
                            .clicked()
                    {
                        edit = Some(Edit::Unlink);
                        ui.close();
                    }
                });
            });
            daw_ui::quiet_action_menu(ui, "Process", |ui| {
                let mut reversed = *reverse;
                if daw_ui::checkbox(ui, &mut reversed, "Reverse").changed() {
                    edit = Some(Edit::Reverse);
                    ui.close();
                }
                if ui
                    .add_enabled(state.pending.is_none(), daw_ui::button_widget("Normalize"))
                    .on_hover_text("Set the source peak to -1 dBFS before fades")
                    .clicked()
                {
                    analyze = Some(false);
                    ui.close();
                }
                if daw_ui::button(ui, "Transients").clicked() {
                    state.transients_open = true;
                    state.grid_slice_open = false;
                    ui.close();
                }
                if ui
                    .add_enabled(
                        state.pending.is_none(),
                        daw_ui::button_widget("Strip silence…"),
                    )
                    .clicked()
                {
                    self.open_audio_silence(ui.ctx(), track, clip.id, state.range);
                    ui.close();
                }
            });
            daw_ui::toolbar_group_sized(ui, 145.0, |ui| {
                let mut db = 20.0 * gain.max(0.00001).log10();
                let label = daw_ui::text::label(ui, "Gain");
                let response = ui
                    .add(
                        daw_ui::number(&mut db)
                            .range(-100.0..=36.0)
                            .clamp_existing_to_range(false)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" dB"),
                    )
                    .labelled_by(label.id);
                gain_rect = response.rect;
                if response.changed() {
                    edit = Some(Edit::Gain(10.0_f32.powf(db / 20.0)));
                }
            });
            let mut processing = *processing;
            if audio_processing_menu(ui, &mut processing) {
                edit = Some(Edit::Processing(processing));
            }
            daw_ui::toolbar_trailing(ui, 530.0, |ui| {
                daw_ui::editor_tool_help(ui, state.tool, daw_ui::ToolHintContext::Audio);
                let zoom_action = daw_ui::timeline_zoom_controls(ui, true);
                if zoom_action != daw_ui::TimelineViewAction::None {
                    view_action = zoom_action;
                    fit = zoom_action == daw_ui::TimelineViewAction::FitAll;
                }
                daw_ui::quiet_view_menu(ui, "View", |ui| {
                    self.clip_navigation_controls(ui);
                    ui.separator();
                    view_action = daw_ui::timeline_selection_view_actions(ui, "audio", true);
                    ui.separator();
                    daw_ui::checkbox(ui, &mut state.show_markers, "Show transient markers");
                    daw_ui::checkbox(ui, &mut state.show_neighbours, "Show neighbouring clips");
                    crate::spectrogram::view_options(ui);
                });
                daw_ui::choice_menu(
                    ui,
                    if self.snap > 0.0 {
                        format!("Grid {}", self.snap)
                    } else {
                        "Grid off".into()
                    },
                    |ui| {
                        daw_ui::text::label(ui, "Grid");
                        for snap in [0.0, 0.125, 0.25, 0.5, 1.0] {
                            if daw_ui::selectable_value(
                                ui,
                                &mut self.snap,
                                snap,
                                if snap == 0.0 {
                                    "Off".into()
                                } else {
                                    format!("{snap} beat")
                                },
                            )
                            .clicked()
                            {
                                ui.close();
                            }
                        }
                        ui.separator();
                        let mut changed =
                            daw_ui::checkbox(ui, &mut state.snap_edges, "Clip edges").changed();
                        changed |=
                            daw_ui::checkbox(ui, &mut state.snap_transients, "Transient markers")
                                .changed();
                        changed |= daw_ui::checkbox(ui, &mut state.snap_zero_crossings, "Source zero crossings")
                    .on_hover_text("Use a quiet source crossing within 5 ms after grid snap. All channels stay aligned; pitch processing may change the final waveform.")
                    .changed();
                        if changed {
                            ui.close();
                        }
                        daw_ui::text::note(ui, "Hold Shift to bypass snapping");
                    },
                );
                daw_ui::editor_tool_caption(ui, state.tool, daw_ui::ToolHintContext::Audio);
            });
        });
        if matches!(state.tool, EditorTool::FadeIn | EditorTool::FadeOut) {
            daw_ui::toolbar(ui, |ui| {
                let (duration, curve, label) = if state.tool == EditorTool::FadeIn {
                    (&mut fi, &mut fic, "Fade in")
                } else {
                    (&mut fo, &mut foc, "Fade out")
                };
                let mut changed = false;
                daw_ui::toolbar_group_sized(ui, 340.0, |ui| {
                    let label = daw_ui::text::label(ui, label);
                    changed |= ui
                        .add(
                            daw_ui::number(duration)
                                .range(0.0..=fade_region.length / clip.source_rate)
                                .clamp_existing_to_range(false)
                                .speed(0.05)
                                .max_decimals(3)
                                .suffix(" beats"),
                        )
                        .labelled_by(label.id)
                        .changed();
                    daw_ui::select("audio_fade_curve", curve.name())
                        .width(105.0)
                        .show_ui(ui, |ui| {
                            for value in FadeCurve::ALL {
                                changed |= daw_ui::selectable_value(ui, curve, value, value.name())
                                    .changed();
                            }
                        });
                });
                if changed {
                    // Only an intentional independent edit reanchors both
                    // fades to this visible window; merely viewing never does.
                    edit = Some(Edit::Fades(
                        (fi * clip.source_rate).min(fade_region.length),
                        (fo * clip.source_rate).min(fade_region.length),
                        fic,
                        foc,
                    ));
                }
            });
        }
        let mut grid_points = Vec::new();
        if state.grid_slice_open {
            grid_points = grid_slice_points(&clip, state.range, state.slice_grid);
            daw_ui::toolbar(ui, |ui| {
                if daw_ui::audio_grid_slice_controls(ui, &mut state.slice_grid, grid_points.len()) {
                    edit = Some(Edit::GridSlices(grid_points.clone()));
                }
                if grid_points.len() > 2048 {
                    ui.label("Choose a wider grid or a shorter time selection");
                }
                if daw_ui::button(ui, "Close grid slicing").clicked() {
                    state.grid_slice_open = false;
                    grid_points.clear();
                }
            });
        }
        if state.transients_open {
            daw_ui::parameter_toolbar(ui, |ui| {
                let previous = state.settings;
                daw_ui::select("transient_algorithm", state.settings.algorithm.name())
                    .width(135.0)
                    .show_ui(ui, |ui| {
                        for algorithm in TransientAlgorithm::ALL {
                            daw_ui::selectable_value(
                                ui,
                                &mut state.settings.algorithm,
                                algorithm,
                                algorithm.name(),
                            );
                        }
                    });
                let response = ui.add(
                    daw_ui::slider(&mut state.settings.sensitivity, 0.0..=1.0).text("Sensitivity"),
                );
                sensitivity_rect = Rect::from_min_size(
                    response.rect.min,
                    Vec2::new(ui.spacing().slider_width, response.rect.height()),
                );
                ui.add(
                    daw_ui::number(&mut state.settings.minimum_gap_ms)
                        .range(20.0..=2000.0)
                        .suffix(" ms gap"),
                );
                if previous != state.settings {
                    state.preview.clear();
                    state.preview_source = None;
                }
                if ui
                    .add_enabled(state.pending.is_none(), daw_ui::button_widget("Analyze"))
                    .clicked()
                {
                    analyze = Some(true);
                }
                if ui
                    .add_enabled(
                        selected_transients(&state, &clip).next().is_some(),
                        daw_ui::button_widget("Split transients"),
                    )
                    .on_hover_text(if state.range.is_some() {
                        "Split only the markers inside the selected time range"
                    } else {
                        "Split at every detected marker"
                    })
                    .clicked()
                {
                    edit = Some(Edit::Transients(
                        selected_transients(&state, &clip).collect(),
                    ));
                }
                if ui
                    .add_enabled(
                        !state.preview.is_empty(),
                        daw_ui::button_widget("Previous transient"),
                    )
                    .clicked()
                    && let Some(beat) =
                        adjacent_transient(&state.preview, clip.start, self.cursor, false)
                {
                    self.seek(beat);
                    reveal_beat(&mut state.view, beat, ui.max_rect().width());
                }
                if ui
                    .add_enabled(
                        !state.preview.is_empty(),
                        daw_ui::button_widget("Next transient"),
                    )
                    .clicked()
                    && let Some(beat) =
                        adjacent_transient(&state.preview, clip.start, self.cursor, true)
                {
                    self.seek(beat);
                    reveal_beat(&mut state.view, beat, ui.max_rect().width());
                }
                ui.label(if state.range.is_some() {
                    format!(
                        "{} of {} markers",
                        selected_transients(&state, &clip).count(),
                        state.preview.len()
                    )
                } else {
                    format!("{} markers", state.preview.len())
                });
                if daw_ui::button(ui, "Close transients").clicked() {
                    state.transients_open = false;
                }
            });
        }
        // Async feedback occupies a stable row below all actions, so completion
        // can never move a pressed button before the pointer is released.
        let feedback = if state.pending.is_some() {
            "Analyzing…".into()
        } else if !state.error.is_empty() {
            state.error.clone()
        } else if let Some((start, end)) = state.range {
            format!("{start:.2}–{end:.2} beats selected")
        } else {
            String::new()
        };
        ui.label(
            egui::RichText::new(feedback)
                .small()
                .color(palette(ui.ctx()).muted),
        );
        if let Some(transients) = analyze.filter(|_| state.pending.is_none()) {
            if ANALYSIS_RUNNING
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                state.error = "Previous analysis is finishing. Try again shortly.".into();
            } else {
                state.error.clear();
                let result = Arc::new(Mutex::new(None));
                let destination = result.clone();
                let source = clip.clone();
                let tempo = self.project.tempo;
                let settings = state.settings;
                let path = path.clone();
                let spawned = std::thread::Builder::new()
                    .name("audio-analysis".into())
                    .spawn(move || {
                        struct ReleaseWorker;
                        impl Drop for ReleaseWorker {
                            fn drop(&mut self) {
                                ANALYSIS_RUNNING.store(false, Ordering::Release);
                            }
                        }
                        let _release = ReleaseWorker;
                        let value = AudioData::load_wav(path)
                            .and_then(|data| {
                                if transients {
                                    detect_transients(&source, &data, tempo, settings)
                                        .map(Analysis::Transients)
                                } else {
                                    normalization_gain(&source, &data, tempo, -1.0)
                                        .map(Analysis::Gain)
                                }
                            })
                            .map_err(|e| e.to_string());
                        if let Ok(mut slot) = destination.lock() {
                            *slot = Some(value);
                        }
                    });
                if let Err(error) = spawned {
                    ANALYSIS_RUNNING.store(false, Ordering::Release);
                    state.error = error.to_string();
                } else {
                    state.pending = Some(Pending {
                        clip: clip.clone(),
                        tempo,
                        settings,
                        result,
                    });
                }
            }
        }
        let width = ui.available_width().max(100.0);
        let (whole, _) = ui.allocate_exact_size(
            Vec2::new(width, ui.available_height().max(150.0)),
            Sense::hover(),
        );
        let ruler = Rect::from_min_max(
            whole.min,
            Pos2::new(
                whole.right(),
                whole.top() + daw_ui::tokens::TIMELINE_RULER_HEIGHT,
            ),
        );
        let loop_lane = Rect::from_min_max(
            Pos2::new(whole.left(), ruler.bottom()),
            Pos2::new(
                whole.right(),
                ruler.bottom() + daw_ui::tokens::TIMELINE_LOOP_HEIGHT,
            ),
        );
        let canvas = Rect::from_min_max(Pos2::new(whole.left(), loop_lane.bottom()), whole.max);
        let data = self.waveforms.get(path);
        crate::spectrogram::prioritize(ui.ctx(), path, data);
        let channels = data.map_or(1, |audio| audio.channels.min(2));
        let bounds = TimelineBounds {
            start_beat: clip.start,
            end_beat: clip.end(),
            start_row: 0.0,
            end_row: channels as f32 * 2.0,
        };
        if fit || !state.fitted {
            state.amplitude_reference =
                data.map_or(1.0, |data| visible_peak(data, &clip, self.project.tempo));
            state.view.fit_bounds(canvas, bounds);
            state.fitted = true;
        }
        if state.reveal_owner {
            reveal_beat(&mut state.view, clip.start, canvas.width());
            state.reveal_owner = false;
        }
        let previous_view = view_action == daw_ui::TimelineViewAction::Previous
            && daw_ui::restore_previous_timeline_view(ui.ctx(), "audio", &mut state.view);
        if view_action == daw_ui::TimelineViewAction::FitSelection {
            let (start, end) = state.range.unwrap_or_else(|| {
                self.arranger_selection_ids()
                    .into_iter()
                    .filter(|(owner, _)| *owner == track)
                    .filter_map(|(owner, id)| self.project.clip(owner, id))
                    .fold((clip.start, clip.end()), |(start, end), cue| {
                        (start.min(cue.start), end.max(cue.end()))
                    })
            });
            state.view.fit_bounds(
                canvas,
                TimelineBounds {
                    start_beat: start,
                    end_beat: end,
                    ..bounds
                },
            );
        }
        view_action.apply_zoom(&mut state.view, canvas);
        let navigation = TimelinePane::new("audio_timeline", canvas)
            .keyboard_active()
            .navigate(ui, &mut state.view, bounds);
        if navigation.fit {
            state.amplitude_reference =
                data.map_or(1.0, |data| visible_peak(data, &clip, self.project.tempo));
        }
        if had_view && !previous_view {
            daw_ui::remember_timeline_view(
                ui.ctx(),
                "audio",
                view_before,
                state.view,
                fit || navigation.fit
                    || view_action == daw_ui::TimelineViewAction::FitSelection
                    || view_action.is_zoom(),
            );
        }
        let headroom = state.amplitude_reference;
        let painter = ui.painter_at(whole);
        painter.rect_filled(whole, 0.0, palette(ui.ctx()).bg);
        daw_ui::paint_timeline_ruler(
            &painter,
            ruler,
            state.view,
            self.project.tempo,
            self.project.beats_per_bar,
        );
        let neighbours: Vec<_> = self
            .project
            .track(track)
            .into_iter()
            .flat_map(|track| &track.clips)
            .filter(|other| {
                other.id != clip.id
                    && !other.is_take_source()
                    && matches!(other.content, ClipContent::Audio { .. })
            })
            .collect();
        if state.show_neighbours {
            let visible_end = state.view.beat_at(canvas, canvas.right());
            for other in neighbours
                .iter()
                .filter(|other| other.end() > state.view.start_beat && other.start < visible_end)
            {
                if let ClipContent::Audio { path, .. } = &other.content
                    && let Some(data) = self.waveforms.get(path)
                {
                    draw_waveform(
                        &painter,
                        canvas,
                        state.view,
                        data,
                        other,
                        self.project.tempo,
                        headroom,
                        palette(ui.ctx()).muted.gamma_multiply(0.45),
                        "audio_neighbour",
                        daw_ui::WaveformAmplitudeScale::Default,
                    );
                }
            }
        }
        if let Some(data) = data {
            draw_waveform(
                &painter,
                canvas,
                state.view,
                data,
                &clip,
                self.project.tempo,
                headroom,
                palette(ui.ctx()).accent,
                "audio_editor",
                daw_ui::WaveformAmplitudeScale::Default,
            );
        }
        crate::audio_silence::paint_preview(&painter, canvas, state.view, &clip);
        let strip_open = crate::audio_silence::is_open(ui.ctx());
        for beat in state
            .preview
            .iter()
            .filter(|_| state.show_markers)
            .chain(grid_points.iter())
            .map(|beat| clip.start + beat)
        {
            let x = state.view.x_at(canvas, beat);
            painter.with_clip_rect(canvas).line_segment(
                [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                Stroke::new(1.0_f32, palette(ui.ctx()).guide),
            );
        }
        if let Some((start, end)) = state.range {
            daw_ui::paint_timeline_selection(&painter, canvas, state.view, start, end);
        }
        let edges = daw_ui::paint_audio_clip_edges(
            &painter,
            canvas,
            state.view,
            clip.start,
            clip.end(),
            palette(ui.ctx()).accent,
        );
        daw_ui::paint_clip_fades(
            &painter,
            canvas,
            state.view,
            clip.start,
            clip.end(),
            |beat| clip.audio_envelope_gain(beat - clip.start),
        );
        let fades = daw_ui::clip_fade_handles(
            ui,
            ui.id().with(("audio_fades", clip.id)),
            canvas,
            state.view,
            audio_fade_bounds(&clip),
            !navigation.navigating && !strip_open,
        );
        if let Some((edge, beat)) = fades.edit {
            let beat = beat.clamp(clip.start, clip.end());
            let (fi, fo, fic, foc) = audio_fade_handle_edit(&clip, edge, beat);
            edit = Some(Edit::Fades(fi, fo, fic, foc));
        }
        let overlap = self.project.track(track).and_then(|channel| {
            let schedule = AudioSchedule::new(&channel.clips).ok()?;
            schedule
                .overlaps
                .iter()
                .find(|overlap| channel.clips[overlap.incoming].id == clip.id)
                .map(|overlap| (overlap.start, overlap.end))
        });
        let blend = overlap.map(|(start, end)| {
            if let Some(config) = crossfade {
                let rect = Rect::from_min_max(
                    Pos2::new(state.view.x_at(canvas, start), canvas.top() + 24.0),
                    Pos2::new(
                        state.view.x_at(canvas, (start + config.duration).min(end)),
                        canvas.bottom(),
                    ),
                );
                daw_ui::paint_crossfade(
                    &painter.with_clip_rect(canvas),
                    rect,
                    daw_ui::CrossfadeCurve {
                        control1: config.curve.control1,
                        control2: config.curve.control2,
                    },
                );
            }
            let handle = daw_ui::crossfade_duration_handle(
                ui,
                ui.id().with(("audio_crossfade", clip.id)),
                canvas,
                state.view,
                daw_ui::CrossfadeBounds {
                    start,
                    end,
                    duration: crossfade.map_or(0.0, |config| config.duration),
                },
                !navigation.navigating && !strip_open,
            );
            open_crossfade = handle.open_curve;
            if let Some(duration) = handle.duration {
                let end_beat = if ui.input(|input| input.modifiers.shift) {
                    start + duration
                } else {
                    self.snapped(start + duration)
                };
                let duration = (end_beat - start).clamp(0.0, end - start);
                edit = Some(Edit::Crossfade((duration > 0.0).then_some(
                    AudioCrossfade {
                        duration,
                        curve: crossfade.map_or(CrossfadeCurve::LINEAR, |config| config.curve),
                    },
                )));
            }
            handle
        });
        let response = ui
            .interact(
                canvas,
                ui.id().with("audio_edit_canvas"),
                if navigation.navigating
                    || strip_open
                    || fades.interacting
                    || blend.as_ref().is_some_and(|handle| handle.interacting)
                {
                    Sense::hover()
                } else {
                    Sense::click_and_drag()
                },
            )
            .on_hover_text(if state.tool == EditorTool::Cursor {
                "Click to place the cursor · Drag to select time · Shift bypasses snap"
            } else {
                state.tool.hint()
            });
        let shift = ui.input(|input| input.modifiers.shift);
        let mut snap_target = None;
        let marker_source = state
            .drag
            .as_ref()
            .map_or(&state.preview, |drag| &drag.markers);
        let marker_start = state
            .drag
            .as_ref()
            .map_or(clip.start, |drag| drag.original.start);
        let edge_targets: Vec<_> = neighbours
            .iter()
            .filter(|_| state.snap_edges)
            .flat_map(|other| [other.start, other.end()])
            .collect();
        let mut targets = edge_targets.clone();
        let moving = state
            .drag
            .as_ref()
            .is_some_and(|drag| drag.mode == AudioDragMode::Move);
        if !moving {
            if state.snap_edges {
                targets.extend([clip.start, clip.end()]);
            }
            if state.snap_transients {
                targets.extend(marker_source.iter().map(|beat| marker_start + beat));
            }
        }
        let grid = self.snap;
        let pixels_per_beat = state.view.pixels_per_beat;
        let tempo = self.project.tempo;
        let source_beats = self.audio_source_beats(&clip);
        let snap = |raw| {
            let (beat, target) = snap_audio_beat(raw, grid, pixels_per_beat, &targets, shift);
            if state.snap_zero_crossings
                && !shift
                && let Some(crossing) =
                    data.and_then(|data| nearest_audio_zero_crossing(&clip, data, tempo, beat))
            {
                (crossing, Some(crossing))
            } else {
                (beat, target)
            }
        };
        let mut new_cursor = None;
        if !navigation.navigating
            && !strip_open
            && response.hovered()
            && let Some(pointer) = response.hover_pos()
        {
            ui.ctx().set_cursor_icon(match state.tool {
                EditorTool::Select
                    if edges.start.is_some_and(|rect| rect.contains(pointer))
                        || edges.end.is_some_and(|rect| rect.contains(pointer)) =>
                {
                    egui::CursorIcon::ResizeHorizontal
                }
                EditorTool::Select | EditorTool::Slip => egui::CursorIcon::Grab,
                EditorTool::Stretch => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::Crosshair,
            });
        }
        if !navigation.navigating
            && !strip_open
            && response.drag_started()
            && matches!(
                state.tool,
                EditorTool::Select | EditorTool::Stretch | EditorTool::Cursor | EditorTool::Slip
            )
            && ui
                .input(|input| input.pointer.press_origin())
                .is_some_and(|pressed| {
                    state.tool == EditorTool::Cursor
                        || (pressed.x >= state.view.x_at(canvas, clip.start) - 4.0
                            && pressed.x <= state.view.x_at(canvas, clip.end()) + 4.0)
                })
        {
            let pressed = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(canvas.center());
            let mode = if state.tool == EditorTool::Cursor {
                AudioDragMode::Range
            } else if state.tool == EditorTool::Slip {
                AudioDragMode::Slip
            } else if state.tool == EditorTool::Stretch {
                AudioDragMode::Stretch
            } else if edges.start.is_some_and(|rect| rect.contains(pressed)) {
                AudioDragMode::TrimStart
            } else if edges.end.is_some_and(|rect| rect.contains(pressed)) {
                AudioDragMode::TrimEnd
            } else {
                AudioDragMode::Move
            };
            state.drag = Some(AudioDrag {
                original: clip.clone(),
                mode,
                origin: state.view.beat_at(canvas, pressed.x),
                markers: state.preview.clone(),
            });
            if mode != AudioDragMode::Range {
                state.range = None;
            }
        }
        if !navigation.navigating
            && !strip_open
            && (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let raw = state.view.beat_at(canvas, pointer.x);
            let (snapped, target) = snap(raw);
            let beat = snapped.clamp(clip.start, clip.end());
            snap_target = target;
            match state.tool {
                EditorTool::Select
                    if response.clicked()
                        && state.show_neighbours
                        && (raw < clip.start || raw >= clip.end()) =>
                {
                    if let Some(other) = neighbours
                        .iter()
                        .filter(|other| raw >= other.start && raw < other.end())
                        .max_by(|a, b| a.start.total_cmp(&b.start))
                    {
                        self.selected_clip = Some(other.id);
                        state.owner = other.id;
                        state.preview.clear();
                        state.preview_source = None;
                        state.range = None;
                        state.error.clear();
                    }
                }
                EditorTool::Cursor if response.clicked() => {
                    state.range = None;
                    new_cursor = Some(beat);
                }
                EditorTool::Cut if response.clicked() && beat > clip.start && beat < clip.end() => {
                    edit = Some(Edit::Split(beat))
                }
                EditorTool::FadeIn => {
                    if let Some((fi, fo, fic, foc)) = fade_parameters_at(&clip, beat, true) {
                        edit = Some(Edit::Fades(fi, fo, fic, foc));
                    }
                }
                EditorTool::FadeOut => {
                    if let Some((fi, fo, fic, foc)) = fade_parameters_at(&clip, beat, false) {
                        edit = Some(Edit::Fades(fi, fo, fic, foc));
                    }
                }
                _ => {}
            }
            if response.dragged()
                && let Some(drag) = &state.drag
            {
                let delta = raw - drag.origin;
                let original = &drag.original;
                let minimum = original.length.min(0.0625);
                match drag.mode {
                    AudioDragMode::Range => {
                        let origin = snap(drag.origin).0.clamp(clip.start, clip.end());
                        let (start, end) = (origin.min(beat), origin.max(beat));
                        state.range = (end > start).then_some((start, end));
                        if response.drag_started() && !self.playing() {
                            new_cursor = Some(origin);
                        }
                    }
                    AudioDragMode::Move => {
                        // Shift copies on the first frame; subsequent frames
                        // move the independent copy using the original anchor.
                        let (start, target) = snap_audio_move(
                            original.start + delta,
                            original.length,
                            grid,
                            pixels_per_beat,
                            &edge_targets,
                            shift,
                        );
                        snap_target = target;
                        edit = Some(Edit::Move(start.max(0.0), response.drag_started() && shift));
                    }
                    AudioDragMode::Slip => {
                        if let Some(source_beats) = source_beats {
                            let movement = if grid > 0.0 && !shift {
                                (delta / grid).round() * grid
                            } else {
                                delta
                            };
                            edit = Some(Edit::Slip(original.clone(), movement, source_beats));
                        }
                    }
                    AudioDragMode::TrimStart => {
                        let earliest =
                            if matches!(original.content, ClipContent::Audio { reverse: true, .. })
                            {
                                0.0
                            } else {
                                (original.start - original.offset / original.source_rate).max(0.0)
                            };
                        let (start, target) = snap(original.start + delta);
                        snap_target = target;
                        let start = start.clamp(earliest, (original.end() - minimum).max(earliest));
                        edit = Some(Edit::Bounds(
                            original.clone(),
                            start,
                            original.end() - start,
                            false,
                        ));
                    }
                    AudioDragMode::TrimEnd | AudioDragMode::Stretch => {
                        let (end, target) = snap(original.end() + delta);
                        snap_target = target;
                        let stretch = drag.mode == AudioDragMode::Stretch;
                        let mut length = (end - original.start).max(minimum);
                        if stretch {
                            length = length.clamp(
                                original.source_length() / MAX_SOURCE_RATE,
                                original.source_length() / MIN_SOURCE_RATE,
                            );
                        } else if matches!(
                            original.content,
                            ClipContent::Audio { reverse: true, .. }
                        ) {
                            length = length
                                .min(original.length + original.offset / original.source_rate);
                        }
                        edit = Some(Edit::Bounds(
                            original.clone(),
                            original.start,
                            length,
                            stretch,
                        ));
                    }
                }
            }
        }
        if let Some(beat) = new_cursor {
            self.seek(beat);
        }
        if let Some(beat) = snap_target {
            let x = state.view.x_at(canvas, beat);
            painter.with_clip_rect(canvas).line_segment(
                [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                Stroke::new(2.0_f32, palette(ui.ctx()).guide),
            );
        }
        daw_ui::paint_timeline_cursor(
            &painter,
            ruler,
            canvas,
            state.view.x_at(canvas, self.cursor),
        );
        let edit = edit.filter(|edit| match edit {
            Edit::Fades(fi, fo, fic, foc) => {
                (*fi, *fo, *fic, *foc) != (*fade_in, *fade_out, *fade_in_curve, *fade_out_curve)
                    || (clip.link_group.is_none() && envelope.is_some())
            }
            Edit::Crossfade(config) => config != crossfade,
            Edit::Bounds(original, start, length, stretch) => {
                clip.start != *start
                    || clip.length != *length
                    || (*stretch
                        && clip.source_rate != original.source_rate * original.length / length)
            }
            Edit::Move(start, copy) => *copy || clip.start != *start,
            Edit::Slip(original, movement, source_beats)
                if *source_beats >= original.source_length() =>
            {
                let direction =
                    if matches!(original.content, ClipContent::Audio { reverse: true, .. }) {
                        1.0
                    } else {
                        -1.0
                    };
                let offset = (original.offset + movement * original.source_rate * direction)
                    .clamp(0.0, source_beats - original.source_length());
                clip.offset != offset
            }
            _ => true,
        });
        if let Some(edit) = edit {
            self.begin_edit();
            let grid_slice = matches!(&edit, Edit::GridSlices(_));
            let result = match edit {
                Edit::Clone => self.project.clone_clip(track, clip.id).map(|id| {
                    self.selected_clip = Some(id);
                    self.edit_label = Some("Create ghost copy".into());
                }),
                Edit::Unlink => self
                    .project
                    .make_clip_independent(track, clip.id)
                    .map(|()| {
                        self.edit_label = Some("Make clip independent".into());
                    }),
                Edit::Gain(gain) => self.project.set_audio_gain(track, clip.id, gain),
                Edit::Processing(processing) => {
                    self.edit_label = Some("Change audio pitch and algorithm".into());
                    self.project
                        .set_audio_processing(track, clip.id, processing)
                }
                Edit::Fades(fi, fo, fic, foc) => {
                    self.edit_label = Some("Edit clip fades".into());
                    self.project
                        .set_audio_fades(track, clip.id, fi, fo, fic, foc)
                }
                Edit::Crossfade(config) => {
                    self.edit_label = Some("Edit crossfade".into());
                    self.project.set_audio_crossfade(track, clip.id, config)
                }
                Edit::Reverse => self.project.reverse_audio_clip(track, clip.id),
                Edit::Trim(start, length) => self.project.trim_clip(track, clip.id, start, length),
                Edit::Split(beat) => self.project.split_clip(track, clip.id, beat).map(|id| {
                    self.arranger_select_clips([(track, clip.id), (track, id)], None);
                    self.edit_label = Some("Split audio clip".into());
                }),
                Edit::Slip(original, movement, source_beats) => {
                    self.edit_label = Some("Slip audio contents".into());
                    if let Some(current) = self.project.clip_mut(track, clip.id) {
                        current.clone_from(&original);
                    }
                    let result =
                        self.project
                            .slip_audio_clip(track, clip.id, movement, source_beats);
                    if result.is_err()
                        && let Some(current) = self.project.clip_mut(track, clip.id)
                    {
                        current.clone_from(&clip);
                    }
                    result
                }
                Edit::Transients(points) | Edit::GridSlices(points) => self
                    .project
                    .split_audio_transients(track, clip.id, &points)
                    .map(|ids| {
                        self.arranger_select_clips(ids.into_iter().map(|id| (track, id)), None);
                        self.edit_label = Some(
                            if grid_slice {
                                "Slice audio on grid"
                            } else {
                                "Split audio transients"
                            }
                            .into(),
                        );
                        state.grid_slice_open = false;
                        state.transients_open = false;
                        state.range = None;
                    }),
                Edit::Move(start, copy) => {
                    self.edit_label =
                        Some(if copy { "Duplicate clip" } else { "Move clip" }.into());
                    if copy {
                        let result =
                            self.project
                                .duplicate_clip_at(track, clip.id, start)
                                .map(|id| {
                                    self.selected_clip = Some(id);
                                    state.owner = id;
                                });
                        if result.is_err() {
                            state.drag = None;
                        }
                        result
                    } else {
                        self.project.move_clip(track, clip.id, start)
                    }
                }
                Edit::Bounds(original, start, length, stretch) => {
                    self.edit_label =
                        Some(if stretch { "Stretch clip" } else { "Trim clip" }.into());
                    if let Some(current) = self.project.clip_mut(track, clip.id) {
                        current.clone_from(&original);
                    }
                    let result = if stretch {
                        self.project.stretch_clip(track, clip.id, length)
                    } else {
                        self.project.trim_clip(track, clip.id, start, length)
                    };
                    if result.is_err()
                        && let Some(current) = self.project.clip_mut(track, clip.id)
                    {
                        current.clone_from(&clip);
                    }
                    result
                }
                Edit::SplitRange(start, end) => {
                    self.edit_label = Some("Split audio selection".into());
                    // Split from the right, retaining the original ID until
                    // the left boundary is cut, then select the isolated piece.
                    let result = if end < clip.end() {
                        self.project.split_clip(track, clip.id, end).map(|_| ())
                    } else {
                        Ok(())
                    };
                    result.and_then(|()| {
                        if start > clip.start {
                            self.project
                                .split_clip(track, clip.id, start)
                                .map(|id| self.selected_clip = Some(id))
                        } else {
                            Ok(())
                        }
                    })
                }
                Edit::RippleRange(start, end) => {
                    self.edit_label = Some("Ripple delete audio selection".into());
                    self.project
                        .ripple_delete_audio_range(track, start, end)
                        .map(|ids| {
                            self.selected_clip = ids.first().copied();
                            state.range = None;
                        })
                }
            };
            match result {
                Ok(()) => {
                    if let Some(id) = self.selected_clip
                        && self.arranger_selection_ids().is_empty()
                    {
                        self.arranger_select_clips([(track, id)], None);
                    }
                    self.changed();
                    state.error.clear();
                    state.preview.clear();
                    state.preview_source = None;
                }
                Err(error) => state.error = error.to_string(),
            }
        }
        if nudge != 0 {
            self.nudge_selected_clip(ui.ctx(), nudge);
            state.range = None;
        }
        if let Some(action) = clipboard_action {
            match self.audio_clipboard_edit(ui.ctx(), &mut state, action) {
                Ok(()) => state.error.clear(),
                Err(error) => state.error = error.to_string(),
            }
        }
        let nudge_unit = format!("{:?}", daw_ui::NudgeUnit::load(ui.ctx()));
        let loop_response = self.timeline_loop_ui(
            ui,
            "audio_editor_loop",
            loop_lane,
            state.view,
            !navigation.navigating && !strip_open,
        );
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("audio_editor_diagnostics"), serde_json::json!({"rect":[canvas.left(),canvas.top(),canvas.right(),canvas.bottom()],"start_beat":state.view.start_beat,"pixels_per_beat":state.view.pixels_per_beat,"row_offset":state.view.row_offset,"row_height":state.view.row_height,"rows":bounds.end_row,"clip_start":clip.start,"clip_end":clip.end(),"cursor":self.cursor,"preview":state.preview,"pending":state.pending.is_some(),"tool":state.tool.label(),"error":state.error,"gain_rect":[gain_rect.left(),gain_rect.top(),gain_rect.right(),gain_rect.bottom()],"sensitivity_rect":[sensitivity_rect.left(),sensitivity_rect.top(),sensitivity_rect.right(),sensitivity_rect.bottom()],"sensitivity":state.settings.sensitivity,"amplitude_reference":state.amplitude_reference,
                "ruler":{"min":[ruler.min.x,ruler.min.y],"max":[ruler.max.x,ruler.max.y]},
                "fade_handles":fades.handles.map(|handle|handle.map(|r|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]}))),
                "crossfade_handle":blend.as_ref().and_then(|handle|handle.handle).map(|r|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]})),
                "loop_lane":{"min":[loop_response.lane.min.x,loop_response.lane.min.y],"max":[loop_response.lane.max.x,loop_response.lane.max.y]},
                "loop_handles":loop_response.handles.map(|handle|handle.map(|r|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]}))),
                "range":state.range,"snap_target":snap_target,"snap_edges":state.snap_edges,"snap_transients":state.snap_transients,"snap_zero_crossings":state.snap_zero_crossings,"nudge_unit":nudge_unit,"show_markers":state.show_markers,"show_neighbours":state.show_neighbours,"grid_preview":grid_points,"grid_slice_open":state.grid_slice_open,
                "start_handle":edges.start.map(|r| [r.left(),r.top(),r.right(),r.bottom()]),"end_handle":edges.end.map(|r| [r.left(),r.top(),r.right(),r.bottom()])}));
            d.insert_temp(id, state);
        });
        if open_crossfade {
            self.open_popup(crate::app::PopupKind::Crossfade);
        }
    }
}

/// The same source-processing controls are available from the focused audio
/// editor and clip properties. All widgets come from the component library.
pub(crate) fn audio_processing_menu(ui: &mut egui::Ui, processing: &mut AudioProcessing) -> bool {
    let before = *processing;
    let label = if processing.transpose_semitones == 0.0 {
        "Pitch".into()
    } else {
        format!("Pitch {:+.2} st", processing.transpose_semitones)
    };
    daw_ui::choice_menu(ui, label, |ui| {
        daw_ui::form_grid(ui, "audio_processing", |ui| {
            let label = daw_ui::text::label(ui, "Transpose");
            let response = ui
                .add(
                    daw_ui::number(&mut processing.transpose_semitones)
                        .range(-24.0..=24.0)
                        .speed(0.05)
                        .max_decimals(2)
                        .suffix(" st"),
                )
                .labelled_by(label.id)
                .on_hover_text("Semitones · 0.01 st = 1 cent · duration stays unchanged");
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("audio_transpose_rect"), response.rect)
            });
            ui.end_row();
            daw_ui::text::label(ui, "Algorithm");
            daw_ui::choice_menu(ui, processing.algorithm.name(), |ui| {
                for algorithm in AudioProcessingAlgorithm::ALL {
                    daw_ui::selectable_value(
                        ui,
                        &mut processing.algorithm,
                        algorithm,
                        algorithm.name(),
                    )
                    .on_hover_text(match algorithm {
                        AudioProcessingAlgorithm::Signalsmith => {
                            "Signalsmith Stretch · default polyphonic pitch and time processing"
                        }
                        AudioProcessingAlgorithm::Bungee => {
                            "Bungee Basic · alternative pitch and time processing"
                        }
                    });
                }
            })
            .response
            .on_hover_text("Used for both pitch changes and pitch-preserving stretching");
            ui.end_row();
        });
        if ui
            .add_enabled(
                processing.transpose_semitones != 0.0,
                daw_ui::button_widget("Reset pitch"),
            )
            .clicked()
        {
            processing.transpose_semitones = 0.0;
        }
    });
    *processing != before
}

/// Measure only when fitting, using the displayed source window and envelope.
/// Gain/fade edits retain this reference until the next explicit Fit.
fn visible_peak(data: &AudioData, clip: &Clip, tempo: f64) -> f32 {
    let ClipContent::Audio { gain, reverse, .. } = clip.content else {
        return 1.0;
    };
    let frames_per_beat = data.sample_rate as f64 * 60.0 / tempo;
    let source_start = clip.offset * frames_per_beat;
    let start = source_start.ceil().max(0.0) as usize;
    let end = (clip.source_end() * frames_per_beat).ceil().max(0.0) as usize;
    let mut peak = 0.0_f32;
    for frame in start..end.min(data.frames()) {
        let beat = if reverse {
            clip.length - (frame as f64 - source_start + 1.0) / frames_per_beat / clip.source_rate
        } else {
            (frame as f64 - source_start) / frames_per_beat / clip.source_rate
        };
        let envelope = gain * clip.audio_envelope_gain(beat);
        for channel in 0..data.channels.min(2) {
            peak = peak.max(data.samples[frame * data.channels + channel].abs() * envelope);
        }
    }
    if peak > 0.0 { peak.max(0.001) } else { 1.0 }
}

// A cut-heavy arrangement can expose hundreds of short regions at once. Keep
// their inexpensive meshes warm instead of evicting an entire frame in order;
// the byte budget still bounds dense or highly zoomed geometry.
const WAVEFORM_VIEW_ENTRIES: usize = 512;
const WAVEFORM_VIEW_BYTES: usize = 8 * 1024 * 1024;
const WAVEFORM_MAX_COLUMNS: usize = 4096;
const WAVEFORM_RAW_COLUMNS: usize = 1024;
const WAVEFORM_FIT_REFERENCES: usize = 512;
const WAVEFORM_FIT_FADE_BINS: usize = 128;

#[derive(PartialEq)]
struct WaveformAmplitudeKey {
    generation: u64,
    content: ClipContent,
    samples: usize,
    frames: usize,
    format: (u32, usize),
    peaks: usize,
    tempo: f64,
    source_start: f64,
    source_end: f64,
}

#[derive(PartialEq)]
struct WaveformViewKey {
    generation: u64,
    rect: Rect,
    visible: Rect,
    view: TimelineViewport,
    tempo: f64,
    headroom: f32,
    color: egui::Color32,
    center_color: egui::Color32,
    density: f32,
    samples: usize,
    frames: usize,
    source_format: (u32, usize),
    peaks: usize,
    amplitude: daw_ui::WaveformAmplitudeScale,
}

enum WaveformGeometry {
    Raw(Vec<WaveformColumn>),
    Refined(daw_ui::PreparedAudioWaveform),
}

struct CachedWaveform {
    geometry: WaveformGeometry,
    path: Arc<str>,
    columns: usize,
    minimum: f32,
    maximum: f32,
    source_left: f64,
    source_right: f64,
    sample_reads: usize,
    bytes: usize,
    amplitude_reference: f64,
}

#[derive(Default)]
struct WaveformViews {
    generation: u64,
    entries: std::collections::VecDeque<(Clip, WaveformViewKey, Arc<CachedWaveform>)>,
    bytes: usize,
    amplitudes: std::collections::VecDeque<(WaveformAmplitudeKey, f64)>,
}

fn source_frame_ceil(frame: f64) -> f64 {
    if (frame - frame.round()).abs() < 1e-7 {
        frame.round()
    } else {
        frame.ceil()
    }
}

/// Compute exclusive source bounds once, then reuse them for every column.
/// Both Fit and drawing include interpolation at fractional slip edges.
struct WaveformSourceWindow {
    start: f64,
    end: f64,
    last_edge: f64,
    first: usize,
    last: usize,
}

impl WaveformSourceWindow {
    fn new(data: &AudioData, clip: &Clip, tempo: f64) -> Self {
        let frames_per_beat = f64::from(data.sample_rate) * 60.0 / tempo;
        let start = clip.offset * frames_per_beat;
        let end = clip.source_end() * frames_per_beat;
        Self {
            start,
            end,
            last_edge: (source_frame_ceil(end) - 1.0).max(start),
            first: source_frame_ceil(start).clamp(0.0, data.frames() as f64) as usize,
            last: source_frame_ceil(end).clamp(0.0, data.frames() as f64) as usize,
        }
    }

    fn edge_frame(&self, frame: f64) -> f64 {
        frame.clamp(self.start, self.last_edge)
    }

    fn extrema(
        &self,
        data: &AudioData,
        a: f64,
        b: f64,
        peaks: &AudioWaveformPeaks,
        channel: usize,
    ) -> (f32, f32, usize) {
        let edge_a = audio_sample(data, self.edge_frame(a), channel);
        let edge_b = audio_sample(data, self.edge_frame(b), channel);
        let mut minimum = edge_a.min(edge_b);
        let mut maximum = edge_a.max(edge_b);
        let first = (a.min(b).ceil().clamp(0.0, data.frames() as f64) as usize).max(self.first);
        let last =
            ((a.max(b).floor() + 1.0).clamp(0.0, data.frames() as f64) as usize).min(self.last);
        let mut sample_reads = 2;
        if let Some((lo, hi)) = peaks.range(data, first, last, channel) {
            minimum = minimum.min(lo);
            maximum = maximum.max(hi);
            sample_reads += last.saturating_sub(first).min(128);
        }
        (minimum, maximum, sample_reads)
    }
}

/// One full-clip reference shared by stereo channels and independent of zoom.
/// Unfaded clips use one exact source query; faded overviews use bounded global
/// bins with the same envelope approximation as drawing. Cache both stages so
/// neither navigation nor warm redraws repeat whole-clip analysis.
fn waveform_amplitude_reference(
    ctx: &egui::Context,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    peaks: Option<&Arc<AudioWaveformPeaks>>,
) -> (f64, usize) {
    let ClipContent::Audio {
        gain,
        fade_in,
        fade_out,
        boundary_fades,
        ..
    } = &clip.content
    else {
        return (1.0, 0);
    };
    let key = WaveformAmplitudeKey {
        generation: crate::spectrogram::waveform_generation(ctx),
        content: clip.content.clone(),
        samples: data.samples.as_ptr() as usize,
        frames: data.frames(),
        format: (data.sample_rate, data.channels),
        peaks: peaks.map_or(0, |peaks| Arc::as_ptr(peaks) as usize),
        tempo,
        source_start: clip.offset,
        source_end: clip.source_end(),
    };
    let shared = waveform_views(ctx);
    if let Ok(mut cache) = shared.lock()
        && let Some(index) = cache
            .amplitudes
            .iter()
            .position(|(candidate, _)| *candidate == key)
    {
        let entry = cache.amplitudes.remove(index).unwrap();
        let reference = entry.1;
        cache.amplitudes.push_front(entry);
        return (reference, 0);
    }
    let window = WaveformSourceWindow::new(data, clip, tempo);
    let mut peak = 0.0_f64;
    let mut sample_reads = 0;
    if let Some(peaks) = peaks {
        let bins = if *fade_in > 0.0 || *fade_out > 0.0 || boundary_fades.is_some() {
            WAVEFORM_FIT_FADE_BINS
        } else {
            1
        };
        for bin in 0..bins {
            let start = clip.length * bin as f64 / bins as f64;
            let end = clip.length * (bin + 1) as f64 / bins as f64;
            let a = clip.audio_source_frame(start, tempo, data.sample_rate);
            let b = clip.audio_source_frame(end, tempo, data.sample_rate);
            let scale =
                f64::from(gain.abs()) * f64::from(clip.audio_envelope_gain((start + end) * 0.5));
            for channel in 0..data.channels.min(2) {
                let (minimum, maximum, reads) = window.extrema(data, a, b, peaks, channel);
                peak = peak.max(f64::from(minimum.abs().max(maximum.abs())) * scale);
                sample_reads += reads;
            }
        }
    } else {
        let count = (window.end - window.start)
            .ceil()
            .clamp(2.0, WAVEFORM_RAW_COLUMNS as f64) as usize;
        for channel in 0..data.channels.min(2) {
            for index in 0..count {
                let local = clip.length * index as f64 / (count - 1) as f64;
                let frame =
                    window.edge_frame(clip.audio_source_frame(local, tempo, data.sample_rate));
                let scale = f64::from(gain.abs()) * f64::from(clip.audio_envelope_gain(local));
                peak = peak.max(f64::from(audio_sample(data, frame, channel).abs()) * scale);
            }
            sample_reads += count;
        }
    }
    let reference = peak;
    let reference = if reference.is_finite() && reference > 0.0 {
        reference
    } else {
        1.0
    };
    if let Ok(mut cache) = shared.lock() {
        cache
            .amplitudes
            .retain(|(candidate, _)| candidate.generation == key.generation);
        if cache.amplitudes.len() >= WAVEFORM_FIT_REFERENCES {
            cache.amplitudes.pop_back();
        }
        cache.amplitudes.push_front((key, reference));
    }
    (reference, sample_reads)
}

fn waveform_views(ctx: &egui::Context) -> Arc<Mutex<WaveformViews>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<WaveformViews>>>(egui::Id::new(
            "waveform_view_geometry",
        ))
        .clone()
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_waveform(
    painter: &egui::Painter,
    rect: Rect,
    view: TimelineViewport,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    headroom: f32,
    color: egui::Color32,
    spectrum_view: &'static str,
    amplitude: daw_ui::WaveformAmplitudeScale,
) {
    let ClipContent::Audio { path, gain, .. } = &clip.content else {
        return;
    };
    if !rect.is_finite()
        || !rect.is_positive()
        || !tempo.is_finite()
        || tempo <= 0.0
        || data.channels == 0
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
    {
        return;
    }
    let clipped = rect.intersect(painter.clip_rect());
    let visible = Rect::from_min_max(
        Pos2::new(
            view.x_at(rect, clip.start).max(clipped.left()),
            clipped.top(),
        ),
        Pos2::new(
            view.x_at(rect, clip.end()).min(clipped.right()),
            clipped.bottom(),
        ),
    );
    if !visible.is_positive() {
        return;
    }
    // Frequency placement keeps its full row height even when vertically clipped.
    let spectrum_rect = Rect::from_min_max(
        Pos2::new(visible.left(), rect.top()),
        Pos2::new(visible.right(), rect.bottom()),
    );
    crate::spectrogram::paint(
        painter,
        spectrum_rect,
        clip,
        tempo,
        (
            view.beat_at(rect, visible.left()),
            view.beat_at(rect, visible.right()),
        ),
        color,
        spectrum_view,
        Some(data),
    );
    let peaks = crate::spectrogram::waveform_peaks(
        painter.ctx(),
        path,
        Some(data),
        if spectrum_view == "audio_editor" {
            3
        } else {
            1
        },
    );
    let generation = crate::spectrogram::waveform_generation(painter.ctx());
    let density = painter.ctx().pixels_per_point();
    let channels = data.channels.min(2);
    let center_color = daw_ui::theme::palette(painter.ctx()).line;
    let key = WaveformViewKey {
        generation,
        rect,
        visible,
        view,
        tempo,
        headroom,
        color,
        center_color,
        density,
        samples: data.samples.as_ptr() as usize,
        frames: data.frames(),
        source_format: (data.sample_rate, data.channels),
        peaks: peaks
            .as_ref()
            .map_or(0, |peaks| Arc::as_ptr(peaks) as usize),
        amplitude,
    };
    let shared = waveform_views(painter.ctx());
    let cached = if let Ok(mut cache) = shared.lock() {
        if cache.generation != generation {
            cache.entries.clear();
            cache.bytes = 0;
            cache.generation = generation;
            cache.amplitudes.clear();
        }
        cache
            .entries
            .iter()
            .position(|(candidate_clip, candidate, _)| candidate_clip == clip && *candidate == key)
            .map(|index| {
                let entry = cache.entries.remove(index).unwrap();
                let value = Arc::clone(&entry.2);
                cache.entries.push_front(entry);
                value
            })
    } else {
        None
    };
    let was_cached = cached.is_some();
    let prepared = cached.unwrap_or_else(|| {
        let (amplitude_reference, reference_reads) =
            if amplitude == daw_ui::WaveformAmplitudeScale::Fit {
                waveform_amplitude_reference(painter.ctx(), data, clip, tempo, peaks.as_ref())
            } else {
                (1.0, 0)
            };
        let refined = peaks.is_some();
        let bins = (visible.width() * if refined { density } else { 1.0 })
            .ceil()
            .clamp(
                1.0,
                if refined {
                    WAVEFORM_MAX_COLUMNS
                } else {
                    WAVEFORM_RAW_COLUMNS
                } as f32,
            ) as usize;
        let mut columns = Vec::with_capacity(bins * channels);
        let mut sample_reads = reference_reads;
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        let window = WaveformSourceWindow::new(data, clip, tempo);
        for x in 0..bins {
            let left = visible.left() + visible.width() * x as f32 / bins as f32;
            let right = visible.left() + visible.width() * (x + 1) as f32 / bins as f32;
            let start = view.beat_at(rect, left).max(clip.start);
            let end = view.beat_at(rect, right).min(clip.end());
            let a = clip.audio_source_frame(start - clip.start, tempo, data.sample_rate);
            let b = clip.audio_source_frame(end - clip.start, tempo, data.sample_rate);
            for channel in 0..channels {
                let (minimum, maximum) = if let Some(peaks) = &peaks {
                    // A clip's end is exclusive. Do not expose the next sample
                    // through the final bin (or the preceding sample in reverse).
                    let (minimum, maximum, reads) = window.extrema(data, a, b, peaks, channel);
                    sample_reads += reads;
                    let envelope = clip.audio_envelope_gain((start + end) * 0.5 - clip.start);
                    let scale = f64::from(*gain) * f64::from(envelope);
                    (f64::from(minimum) * scale, f64::from(maximum) * scale)
                } else {
                    let frames = ((b - a).abs().ceil() as usize).clamp(1, 32);
                    let mut minimum = 0.0_f64;
                    let mut maximum = 0.0_f64;
                    for index in 0..frames {
                        let local =
                            start + (end - start) * index as f64 / frames as f64 - clip.start;
                        let scale = f64::from(*gain) * f64::from(clip.audio_envelope_gain(local));
                        let sample = f64::from(audio_sample(
                            data,
                            window.edge_frame(clip.audio_source_frame(
                                local,
                                tempo,
                                data.sample_rate,
                            )),
                            channel,
                        )) * scale;
                        minimum = minimum.min(sample);
                        maximum = maximum.max(sample);
                    }
                    sample_reads += frames;
                    (minimum, maximum)
                };
                let sample_f32 =
                    |value: f64| value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                low = low.min(sample_f32(minimum));
                high = high.max(sample_f32(maximum));
                let normalize = |value: f64| {
                    (value / amplitude_reference).clamp(-f64::from(f32::MAX), f64::from(f32::MAX))
                        as f32
                };
                columns.push(WaveformColumn {
                    beat: (start + end) * 0.5,
                    channel,
                    minimum: normalize(minimum),
                    maximum: normalize(maximum),
                });
            }
        }
        let count = columns.len();
        let geometry = if refined {
            WaveformGeometry::Refined(daw_ui::prepare_audio_waveform(
                rect,
                view,
                &columns,
                channels,
                headroom,
                color,
                density,
                center_color,
            ))
        } else {
            WaveformGeometry::Raw(columns)
        };
        let bytes = match &geometry {
            WaveformGeometry::Raw(columns) => {
                columns.capacity() * std::mem::size_of::<WaveformColumn>()
            }
            WaveformGeometry::Refined(prepared) => prepared.bytes(),
        };
        let value = Arc::new(CachedWaveform {
            geometry,
            path: Arc::from(path.as_str()),
            columns: count,
            minimum: low,
            maximum: high,
            source_left: clip.audio_source_frame(
                view.beat_at(rect, visible.left()) - clip.start,
                tempo,
                data.sample_rate,
            ),
            source_right: clip.audio_source_frame(
                view.beat_at(rect, visible.right()) - clip.start,
                tempo,
                data.sample_rate,
            ),
            sample_reads,
            bytes,
            amplitude_reference,
        });
        if let Ok(mut cache) = shared.lock() {
            while !cache.entries.is_empty()
                && (cache.entries.len() >= WAVEFORM_VIEW_ENTRIES
                    || cache.bytes + bytes > WAVEFORM_VIEW_BYTES)
            {
                if let Some((_, _, old)) = cache.entries.pop_back() {
                    cache.bytes -= old.bytes;
                }
            }
            if bytes <= WAVEFORM_VIEW_BYTES {
                cache.bytes += bytes;
                cache
                    .entries
                    .push_front((clip.clone(), key, Arc::clone(&value)));
            }
        }
        value
    });
    let (stage, stats) = match &prepared.geometry {
        WaveformGeometry::Raw(columns) => {
            daw_ui::paint_audio_waveform_raw(
                painter, rect, view, columns, channels, headroom, color,
            );
            ("raw", daw_ui::WaveformMeshStats::default())
        }
        WaveformGeometry::Refined(waveform) => {
            waveform.paint(painter);
            ("refined", waveform.stats)
        }
    };
    crate::spectrogram::record_waveform_paint(
        painter.ctx(),
        crate::spectrogram::WaveformPaint {
            view: spectrum_view,
            clip: clip.id,
            path: Arc::clone(&prepared.path),
            stage,
            columns: prepared.columns,
            channels,
            minimum: prepared.minimum,
            maximum: prepared.maximum,
            source_left: prepared.source_left,
            source_right: prepared.source_right,
            rect: visible,
            vertices: stats.vertices,
            indices: stats.indices,
            fringe_vertices: stats.fringe_vertices,
            sample_reads: if was_cached { 0 } else { prepared.sample_reads },
            cached: was_cached,
            amplitude_reference: prepared.amplitude_reference,
        },
    );
}

pub(crate) fn paint_audio_clip(
    painter: &egui::Painter,
    rect: Rect,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    color: egui::Color32,
    amplitude: daw_ui::WaveformAmplitudeScale,
) {
    paint_audio_clip_scaled_in_view(
        painter, rect, data, clip, tempo, color, "arranger", amplitude,
    );
}

pub(crate) fn paint_audio_clip_in_view(
    painter: &egui::Painter,
    rect: Rect,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    color: egui::Color32,
    view_name: &'static str,
) {
    paint_audio_clip_scaled_in_view(
        painter,
        rect,
        data,
        clip,
        tempo,
        color,
        view_name,
        daw_ui::WaveformAmplitudeScale::Default,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_audio_clip_scaled_in_view(
    painter: &egui::Painter,
    rect: Rect,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    color: egui::Color32,
    view_name: &'static str,
    amplitude: daw_ui::WaveformAmplitudeScale,
) {
    if !rect.is_positive() || data.channels == 0 {
        return;
    }
    let view = TimelineViewport {
        start_beat: clip.start,
        pixels_per_beat: rect.width() / clip.length as f32,
        row_offset: 0.0,
        row_height: rect.height() / (data.channels.min(2) * 2) as f32,
    };
    draw_waveform(
        painter, rect, view, data, clip, tempo, 1.0, color, view_name, amplitude,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_fit_matches_fractional_edges_and_reverse_source_mapping() {
        let ctx = egui::Context::default();
        let data = AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![1.0, 0.01, 0.01, 0.01, 1.0, 0.01],
        };
        let peaks = Arc::new(AudioWaveformPeaks::build(&data).unwrap());
        let mut clip = Clip::audio("Fractional source", 0.0, 4.0 / 24_000.0, "source.wav");
        clip.offset = 0.5 / 24_000.0;
        let (forward, _) = waveform_amplitude_reference(&ctx, &data, &clip, 120.0, Some(&peaks));
        assert!((forward - 1.0).abs() < 1e-6);
        if let ClipContent::Audio { reverse, .. } = &mut clip.content {
            *reverse = true;
        }
        let (reverse, _) = waveform_amplitude_reference(&ctx, &data, &clip, 120.0, Some(&peaks));
        assert!(
            (reverse - 0.505).abs() < 1e-6,
            "Reverse starts halfway before the last spike; only its interpolated edge contributes"
        );
        let (cached, reads) = waveform_amplitude_reference(&ctx, &data, &clip, 120.0, Some(&peaks));
        assert_eq!(cached, reverse);
        assert_eq!(reads, 0);
    }

    #[test]
    fn magnetic_snap_is_measured_in_pixels_and_aligns_either_move_edge() {
        assert_eq!(
            snap_audio_beat(1.38, 0.25, 100.0, &[1.37], false),
            (1.37, Some(1.37))
        );
        assert_eq!(
            snap_audio_beat(1.38, 0.25, 1000.0, &[1.37], false),
            (1.5, None)
        );
        assert_eq!(
            snap_audio_beat(1.38, 0.25, 100.0, &[1.37], true),
            (1.38, None)
        );
        let (start, target) = snap_audio_move(2.38, 4.0, 0.25, 100.0, &[6.37], false);
        assert!((start - 2.37).abs() < 1e-10);
        assert_eq!(target, Some(6.37));
    }

    #[test]
    fn fit_reference_uses_visible_window_envelope_channels_and_silence() {
        let mut samples = vec![0.0; 16000];
        for frame in 0..8000 {
            samples[frame * 2] = if frame < 4000 { 0.05 } else { 1.0 };
            samples[frame * 2 + 1] = if frame < 4000 { 0.1 } else { 1.0 };
        }
        let data = AudioData {
            sample_rate: 8000,
            channels: 2,
            samples,
        };
        let mut clip = Clip::audio("Window", 0.0, 1.0, "source.wav");
        assert!((visible_peak(&data, &clip, 120.0) - 0.1).abs() < 1e-6);
        if let ClipContent::Audio {
            gain,
            fade_in,
            fade_out,
            reverse,
            ..
        } = &mut clip.content
        {
            *gain = 2.0;
            *fade_in = 1.0;
            *fade_out = 1.0;
            *reverse = true;
        }
        assert!((visible_peak(&data, &clip, 120.0) - 0.05).abs() < 1e-6);
        let silence = AudioData {
            samples: vec![0.0; data.samples.len()],
            ..data
        };
        assert_eq!(visible_peak(&silence, &clip, 120.0), 1.0);
    }

    #[test]
    fn stretched_fit_measures_the_same_source_window_and_fade_envelope() {
        let data = AudioData {
            sample_rate: 8000,
            channels: 1,
            samples: (0..12000)
                .map(|frame| if frame < 8000 { 0.25 } else { 1.0 })
                .collect(),
        };
        for rate in [0.5, 1.0, 2.0] {
            let mut clip = Clip::audio("Stretched window", 2.0, 1.0 / rate, "source.wav");
            clip.offset = 1.0;
            clip.source_rate = rate;
            assert!((visible_peak(&data, &clip, 120.0) - 0.25).abs() < 1e-6);
            if let ClipContent::Audio {
                fade_in,
                fade_out,
                reverse,
                ..
            } = &mut clip.content
            {
                *fade_in = 1.0;
                *fade_out = 1.0;
                *reverse = true;
            }
            assert!((visible_peak(&data, &clip, 120.0) - 0.0625).abs() < 1e-6);
        }
    }

    fn linked_audio_editor(tool: EditorTool) -> (egui::Context, DawApp, u64, u64) {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let clip = Clip::audio("Source", 0.0, 4.0, "unused.wav");
        let original = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        project
            .set_audio_fades(
                track,
                original,
                3.0,
                3.0,
                FadeCurve::Linear,
                FadeCurve::Linear,
            )
            .unwrap();
        let clone = project.clone_clip(track, original).unwrap();
        project.trim_clip(track, clone, 5.0, 2.0).unwrap();
        let mut app = DawApp::with_context(&ctx, project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(clone);
        app.snap = 0.0;
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("audio_editor_state"),
                AudioView {
                    owner: clone,
                    tool,
                    ..Default::default()
                },
            )
        });
        (ctx, app, track, original)
    }

    fn audio_frame(ctx: &egui::Context, app: &mut DawApp, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.audio_editor(ui));
        });
    }

    #[test]
    fn native_clipboard_events_publish_paste_marker_and_cut_only_once() {
        let (ctx, mut app, track, _) = linked_audio_editor(EditorTool::Cursor);
        app.view = crate::app::WorkspaceView::Clip;
        ctx.data_mut(|data| {
            let id = egui::Id::new("audio_editor_state");
            let mut state = data.get_temp::<AudioView>(id).unwrap();
            state.range = Some((5.25, 5.75));
            data.insert_temp(id, state);
        });
        let before = app.project.clone();
        let copied = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                ..Default::default()
            },
            |ctx| {
                assert!(app.audio_edit_shortcuts(ctx));
            },
        );
        assert_eq!(app.project, before);
        let marker = copied
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .expect("Native Paste requires a nonempty system clipboard");
        assert!(!marker.is_empty());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::Cut,
                    egui::Event::Key {
                        key: egui::Key::X,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                assert!(app.audio_edit_shortcuts(ctx));
            },
        );
        assert_eq!(
            app.project.track(track).unwrap().clips.len(),
            3,
            "Native Cut and its key event must perform one deletion"
        );
        app.cursor = 8.0;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Paste(marker)],
                ..Default::default()
            },
            |ctx| {
                assert!(app.audio_edit_shortcuts(ctx));
            },
        );
        let pasted = app.selected().unwrap();
        assert_eq!((pasted.start, pasted.length), (8.0, 0.5));
        assert!(pasted.link_group.is_none());
        assert_eq!(app.project.track(track).unwrap().clips.len(), 4);
    }

    #[test]
    fn viewing_trimmed_linked_fades_does_not_clamp_shared_source_values() {
        for tool in [EditorTool::FadeIn, EditorTool::FadeOut] {
            let (ctx, mut app, _, _) = linked_audio_editor(tool);
            let before = app.project.clone();
            for _ in 0..3 {
                audio_frame(&ctx, &mut app, vec![]);
            }
            assert_eq!(app.project, before);
            assert!(!app.edit_dirty);
        }
    }

    #[test]
    fn viewing_independent_copies_preserves_long_source_fades() {
        for tool in [EditorTool::FadeIn, EditorTool::FadeOut] {
            for duplicate in [false, true] {
                let (ctx, mut app, track, _) = linked_audio_editor(tool);
                let selected = app.selected_clip.unwrap();
                if duplicate {
                    app.selected_clip = Some(app.project.duplicate_clip(track, selected).unwrap());
                } else {
                    app.project.make_clip_independent(track, selected).unwrap();
                }
                let before = app.project.clone();
                for _ in 0..3 {
                    audio_frame(&ctx, &mut app, vec![]);
                }
                assert_eq!(app.project, before);
                assert!(!app.edit_dirty);
                app.project.validate().unwrap();
            }
        }
    }

    #[test]
    fn linked_fade_pointer_edits_use_source_coordinates_and_preserve_opposite_fade() {
        for ((tool, source_beat, expected), rate) in [
            (EditorTool::FadeIn, 0.5, (1.5, 3.0)),
            (EditorTool::FadeOut, 1.5, (3.0, 1.5)),
        ]
        .into_iter()
        .flat_map(|case| [0.5, 1.0, 2.0].map(|rate| (case, rate)))
        {
            let (ctx, mut app, track, original) = linked_audio_editor(tool);
            let clip = app.selected_mut().unwrap();
            clip.source_rate = rate;
            clip.length /= rate;
            for _ in 0..3 {
                audio_frame(&ctx, &mut app, vec![]);
            }
            let state = ctx
                .data(|data| data.get_temp::<AudioView>(egui::Id::new("audio_editor_state")))
                .unwrap();
            let diagnostics = ctx
                .data(|data| {
                    data.get_temp::<serde_json::Value>(egui::Id::new("audio_editor_diagnostics"))
                })
                .unwrap();
            let values = diagnostics["rect"].as_array().unwrap();
            let canvas = Rect::from_min_max(
                Pos2::new(
                    values[0].as_f64().unwrap() as f32,
                    values[1].as_f64().unwrap() as f32,
                ),
                Pos2::new(
                    values[2].as_f64().unwrap() as f32,
                    values[3].as_f64().unwrap() as f32,
                ),
            );
            let position = Pos2::new(
                state.view.x_at(canvas, 5.0 + source_beat / rate),
                canvas.center().y,
            );
            for pressed in [true, false] {
                audio_frame(
                    &ctx,
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            let clone = app.selected_clip.unwrap();
            for (id, offset) in [(original, 0.0), (clone, 1.0)] {
                let ClipContent::Audio {
                    fade_in,
                    fade_out,
                    envelope: Some(envelope),
                    ..
                } = app.project.clip(track, id).unwrap().content
                else {
                    panic!("expected audio");
                };
                assert!((fade_in - expected.0).abs() < 1e-5, "{tool:?}: {fade_in}");
                assert!((fade_out - expected.1).abs() < 1e-5, "{tool:?}: {fade_out}");
                assert_eq!(
                    envelope,
                    AudioEnvelope {
                        offset,
                        length: 4.0
                    }
                );
            }
            app.project.validate().unwrap();
        }
    }

    #[test]
    fn completed_transient_preview_is_discarded_after_tempo_changes() {
        let (ctx, mut app, _, _) = linked_audio_editor(EditorTool::Cursor);
        let source = app.selected().unwrap().clone();
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("audio_editor_state"),
                AudioView {
                    owner: source.id,
                    transients_open: true,
                    preview: vec![0.5, 1.5],
                    preview_source: Some((source, app.project.tempo)),
                    ..Default::default()
                },
            )
        });
        app.project.tempo = 130.0;
        audio_frame(&ctx, &mut app, vec![]);
        let state = ctx
            .data(|data| data.get_temp::<AudioView>(egui::Id::new("audio_editor_state")))
            .unwrap();
        assert!(state.preview.is_empty());
        assert!(state.preview_source.is_none());
    }
}
