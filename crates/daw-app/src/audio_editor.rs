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
    show_markers: bool,
    show_neighbours: bool,
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
            show_markers: true,
            show_neighbours: true,
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
    Reverse,
    Split(f64),
    Trim(f64, f64),
    Paste(Clip, f64),
    Duplicate,
    Transients(Vec<f64>),
    Bounds(Clip, f64, f64, bool),
    Move(f64, bool),
    SplitRange(f64, f64),
}

pub(crate) fn reset_audio_editor(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<AudioView>(egui::Id::new("audio_editor_state"));
        data.remove::<serde_json::Value>(egui::Id::new("audio_editor_diagnostics"));
    });
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
            daw_ui::editor_tools(ui, &mut state.tool, TOOLS);
            fit = daw_ui::timeline_fit_button(ui).clicked();
            daw_ui::select(
                "audio_snap",
                if self.snap > 0.0 {
                    format!("Grid {}", self.snap)
                } else {
                    "Grid off".into()
                },
            )
            .width(90.0)
            .height(380.0)
            .show_ui(ui, |ui| {
                daw_ui::text::label(ui, "Grid");
                for snap in [0.0, 0.125, 0.25, 0.5, 1.0] {
                    daw_ui::selectable_value(
                        ui,
                        &mut self.snap,
                        snap,
                        if snap == 0.0 {
                            "Off".into()
                        } else {
                            format!("{snap} beat")
                        },
                    );
                }
                ui.separator();
                daw_ui::checkbox(ui, &mut state.snap_edges, "Clip edges");
                daw_ui::checkbox(ui, &mut state.snap_transients, "Transient markers");
                daw_ui::text::note(ui, "Hold Shift to bypass snapping");
            });
            daw_ui::view_menu(ui, "View", |ui| {
                daw_ui::checkbox(ui, &mut state.show_markers, "Show transient markers");
                daw_ui::checkbox(ui, &mut state.show_neighbours, "Show neighbouring clips");
            });
            daw_ui::action_menu(ui, "Clip", |ui| {
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
                if daw_ui::button(ui, "Copy").clicked() {
                    state.clipboard = Some(clip.clone());
                    ui.close();
                }
                if ui
                    .add_enabled(state.clipboard.is_some(), daw_ui::button_widget("Paste"))
                    .clicked()
                {
                    edit = Some(Edit::Paste(state.clipboard.clone().unwrap(), self.cursor));
                    ui.close();
                }
                if daw_ui::button(ui, "Duplicate clip")
                    .on_hover_text("Create an independent copy of this clip")
                    .clicked()
                {
                    edit = Some(Edit::Duplicate);
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
            daw_ui::action_menu(ui, "Process", |ui| {
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
                    ui.close();
                }
            });
            let mut db = 20.0 * gain.max(0.00001).log10();
            ui.label("Gain");
            let response = ui.add(
                daw_ui::number(&mut db)
                    .range(-100.0..=36.0)
                    .clamp_existing_to_range(false)
                    .speed(0.1)
                    .suffix(" dB"),
            );
            gain_rect = response.rect;
            if response.changed() {
                edit = Some(Edit::Gain(10.0_f32.powf(db / 20.0)));
            }
            let mut processing = *processing;
            if audio_processing_menu(ui, &mut processing) {
                edit = Some(Edit::Processing(processing));
            }
        });
        if matches!(state.tool, EditorTool::FadeIn | EditorTool::FadeOut) {
            daw_ui::toolbar(ui, |ui| {
                let (duration, curve, label) = if state.tool == EditorTool::FadeIn {
                    (&mut fi, &mut fic, "Fade in")
                } else {
                    (&mut fo, &mut foc, "Fade out")
                };
                ui.label(label);
                let mut changed = ui
                    .add(
                        daw_ui::number(duration)
                            .range(0.0..=fade_region.length / clip.source_rate)
                            .clamp_existing_to_range(false)
                            .speed(0.05)
                            .suffix(" beats"),
                    )
                    .changed();
                daw_ui::select("audio_fade_curve", curve.name())
                    .width(105.0)
                    .show_ui(ui, |ui| {
                        for value in FadeCurve::ALL {
                            changed |=
                                daw_ui::selectable_value(ui, curve, value, value.name()).changed();
                        }
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
        if state.transients_open {
            daw_ui::toolbar(ui, |ui| {
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
        let canvas = Rect::from_min_max(Pos2::new(whole.left(), ruler.bottom()), whole.max);
        let data = self.waveforms.get(path);
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
        let navigation =
            TimelinePane::new("audio_timeline", canvas).navigate(ui, &mut state.view, bounds);
        if navigation.fit {
            state.amplitude_reference =
                data.map_or(1.0, |data| visible_peak(data, &clip, self.project.tempo));
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
            );
        }
        for beat in state
            .preview
            .iter()
            .filter(|_| state.show_markers)
            .map(|beat| clip.start + beat)
        {
            let x = state.view.x_at(canvas, beat);
            painter.with_clip_rect(canvas).line_segment(
                [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                Stroke::new(1.0_f32, palette(ui.ctx()).orange),
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
        let cursor_x = state.view.x_at(canvas, self.cursor);
        painter.with_clip_rect(canvas).line_segment(
            [
                Pos2::new(cursor_x, canvas.top()),
                Pos2::new(cursor_x, canvas.bottom()),
            ],
            Stroke::new(1.0_f32, palette(ui.ctx()).text),
        );
        let response = ui
            .interact(
                canvas,
                ui.id().with("audio_edit_canvas"),
                if navigation.navigating {
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
        let snap = |raw| snap_audio_beat(raw, grid, pixels_per_beat, &targets, shift);
        if !navigation.navigating
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
                EditorTool::Select => egui::CursorIcon::Grab,
                EditorTool::Stretch => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::Crosshair,
            });
        }
        if !navigation.navigating
            && response.drag_started()
            && matches!(
                state.tool,
                EditorTool::Select | EditorTool::Stretch | EditorTool::Cursor
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
                    self.seek(beat);
                }
                EditorTool::Cut if response.clicked() && beat > clip.start && beat < clip.end() => {
                    edit = Some(Edit::Split(beat))
                }
                EditorTool::FadeIn => {
                    edit = Some(Edit::Fades(
                        (fade_region.offset + (beat - clip.start) * clip.source_rate)
                            .clamp(0.0, fade_region.length),
                        (fo * clip.source_rate).min(fade_region.length),
                        fic,
                        foc,
                    ))
                }
                EditorTool::FadeOut => {
                    edit = Some(Edit::Fades(
                        (fi * clip.source_rate).min(fade_region.length),
                        (fade_region.length
                            - fade_region.offset
                            - (beat - clip.start) * clip.source_rate)
                            .clamp(0.0, fade_region.length),
                        fic,
                        foc,
                    ))
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
                        if response.drag_started() {
                            self.seek(origin);
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
        if let Some(beat) = snap_target {
            let x = state.view.x_at(canvas, beat);
            painter.with_clip_rect(canvas).line_segment(
                [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                Stroke::new(2.0_f32, palette(ui.ctx()).orange),
            );
        }
        let edit = edit.filter(|edit| match edit {
            Edit::Bounds(original, start, length, stretch) => {
                clip.start != *start
                    || clip.length != *length
                    || (*stretch
                        && clip.source_rate != original.source_rate * original.length / length)
            }
            Edit::Move(start, copy) => *copy || clip.start != *start,
            _ => true,
        });
        if let Some(edit) = edit {
            self.begin_edit();
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
                Edit::Fades(fi, fo, fic, foc) => self
                    .project
                    .set_audio_fades(track, clip.id, fi, fo, fic, foc),
                Edit::Reverse => self.project.reverse_audio_clip(track, clip.id),
                Edit::Trim(start, length) => self.project.trim_clip(track, clip.id, start, length),
                Edit::Split(beat) => self.project.split_clip(track, clip.id, beat).map(|_| ()),
                Edit::Paste(source, start) => self
                    .project
                    .paste_audio_clip(track, &source, start)
                    .map(|id| {
                        self.selected_clip = Some(id);
                    }),
                Edit::Duplicate => self.project.duplicate_clip(track, clip.id).map(|id| {
                    self.selected_clip = Some(id);
                }),
                Edit::Transients(points) => self
                    .project
                    .split_audio_transients(track, clip.id, &points)
                    .map(|_| ()),
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
            };
            match result {
                Ok(()) => {
                    self.changed();
                    state.error.clear();
                    state.preview.clear();
                    state.preview_source = None;
                }
                Err(error) => state.error = error.to_string(),
            }
        }
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("audio_editor_diagnostics"), serde_json::json!({"rect":[canvas.left(),canvas.top(),canvas.right(),canvas.bottom()],"start_beat":state.view.start_beat,"pixels_per_beat":state.view.pixels_per_beat,"row_offset":state.view.row_offset,"row_height":state.view.row_height,"rows":bounds.end_row,"clip_start":clip.start,"clip_end":clip.end(),"cursor":self.cursor,"preview":state.preview,"pending":state.pending.is_some(),"tool":state.tool.label(),"error":state.error,"gain_rect":[gain_rect.left(),gain_rect.top(),gain_rect.right(),gain_rect.bottom()],"sensitivity_rect":[sensitivity_rect.left(),sensitivity_rect.top(),sensitivity_rect.right(),sensitivity_rect.bottom()],"sensitivity":state.settings.sensitivity,"amplitude_reference":state.amplitude_reference,
                "range":state.range,"snap_target":snap_target,"snap_edges":state.snap_edges,"snap_transients":state.snap_transients,"show_markers":state.show_markers,"show_neighbours":state.show_neighbours,
                "start_handle":edges.start.map(|r| [r.left(),r.top(),r.right(),r.bottom()]),"end_handle":edges.end.map(|r| [r.left(),r.top(),r.right(),r.bottom()])}));
            d.insert_temp(id, state);
        });
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
) {
    let ClipContent::Audio { gain, .. } = clip.content else {
        return;
    };
    let channels = data.channels.min(2);
    let width = rect.width().ceil().max(0.0) as usize;
    let mut columns = Vec::with_capacity(width * channels);
    for x in 0..width {
        let beat = view.beat_at(rect, rect.left() + x as f32);
        let next = view.beat_at(rect, rect.left() + x as f32 + 1.0);
        if next <= clip.start || beat >= clip.end() {
            continue;
        }
        let start = beat.max(clip.start);
        let end = next.min(clip.end());
        let frames = ((end - start) * clip.source_rate * 60.0 / tempo * data.sample_rate as f64)
            .ceil()
            .clamp(1.0, 32.0) as usize;
        for channel in 0..channels {
            let mut minimum = 0.0_f32;
            let mut maximum = 0.0_f32;
            for index in 0..frames {
                let local = start + (end - start) * index as f64 / frames as f64 - clip.start;
                let sample = audio_sample(
                    data,
                    clip.audio_source_frame(local, tempo, data.sample_rate),
                    channel,
                ) * gain
                    * clip.audio_envelope_gain(local);
                minimum = minimum.min(sample);
                maximum = maximum.max(sample);
            }
            columns.push(WaveformColumn {
                beat,
                channel,
                minimum,
                maximum,
            });
        }
    }
    daw_ui::paint_audio_waveform(painter, rect, view, &columns, channels, headroom, color);
}

pub(crate) fn paint_audio_clip(
    painter: &egui::Painter,
    rect: Rect,
    data: &AudioData,
    clip: &Clip,
    tempo: f64,
    color: egui::Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let view = TimelineViewport {
        start_beat: clip.start,
        pixels_per_beat: rect.width() / clip.length as f32,
        row_offset: 0.0,
        row_height: rect.height() / (data.channels.min(2) * 2) as f32,
    };
    draw_waveform(painter, rect, view, data, clip, tempo, 1.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

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
