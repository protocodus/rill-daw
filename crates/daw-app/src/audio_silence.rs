//! Reviewed, source-preserving silence removal from the focused audio selection.
use crate::app::DawApp;
use anyhow::Context;
use daw_core::{AudioFile, AudioSilenceSettings, Clip, ClipContent, detect_audio_silence_file};
use eframe::egui;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const STATE: &str = "audio_strip_silence";
static ANALYZING: AtomicBool = AtomicBool::new(false);
type Analysis = Result<(Vec<(f64, f64)>, AudioFile), String>;
type SharedResult = Arc<Mutex<Option<Analysis>>>;
#[derive(Clone)]
struct Pending {
    settings: AudioSilenceSettings,
    result: SharedResult,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Dialog {
    session: u64,
    revision: u64,
    track: u64,
    clip: Clip,
    tempo: f64,
    range: (f64, f64),
    settings: AudioSilenceSettings,
    preview: Option<Vec<(f64, f64)>>,
    preview_file: Option<AudioFile>,
    pending: Option<Pending>,
    requested: Option<Instant>,
    error: Option<String>,
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE)).is_some())
}
pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(state) = data.get_temp::<Dialog>(egui::Id::new(STATE))
            && let Some(pending) = state.pending
        {
            pending.cancel.store(true, Ordering::Release);
        }
        data.remove::<Dialog>(egui::Id::new(STATE));
    });
}
pub(crate) fn poll_source_cancellation(app: &DawApp, ctx: &egui::Context) {
    if let Some(state) = ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE)))
        && (state.session != app.song_session.id
            || state.revision != app.song_session.revision
            || app.edit_dirty
            || app.project.clip(state.track, state.clip.id) != Some(&state.clip)
            || app.project.tempo != state.tempo)
    {
        reset(ctx);
    }
}
pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE))).map_or(serde_json::Value::Null, |state| {
        serde_json::json!({"range":state.range,"retained":state.preview,
            "pending":state.pending.is_some() || state.requested.is_some(),"error":state.error,
            "worker_started":state.pending.is_some(),
            "worker_running":ANALYZING.load(Ordering::Acquire),
            "cancel_requested":state.pending.as_ref().is_some_and(|pending|pending.cancel.load(Ordering::Acquire)),
            "threshold_db":state.settings.threshold_db,"minimum_silence_ms":state.settings.minimum_silence_ms,
            "padding_ms":state.settings.padding_ms,
            "controls":ctx.data(|data| data.get_temp::<Vec<[f32;4]>>(egui::Id::new("audio_silence_controls")))})
    })
}
pub(crate) fn paint_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: daw_ui::TimelineViewport,
    clip: &Clip,
) {
    let state = painter
        .ctx()
        .data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE)));
    if let Some(state) = state.filter(|state| state.clip == *clip)
        && let Some(preview) = state.preview
    {
        for (start, end) in preview {
            daw_ui::paint_timeline_selection(painter, rect, view, start, end);
        }
    }
}

impl DawApp {
    pub(crate) fn open_audio_silence(
        &mut self,
        ctx: &egui::Context,
        track: u64,
        clip: u64,
        range: Option<(f64, f64)>,
    ) {
        let Some(clip) = self
            .project
            .clip(track, clip)
            .filter(|clip| {
                matches!(clip.content, ClipContent::Audio { .. }) && !clip.is_take_source()
            })
            .cloned()
        else {
            return;
        };
        self.flush_pending_edit();
        self.history.seal_group();
        let range = range.unwrap_or((clip.start, clip.end()));
        if let Err(error) = clip.audio_region(range.0, range.1) {
            self.message(Err(error), "");
            return;
        }
        reset(ctx);
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(STATE),
                Dialog {
                    session: self.song_session.id,
                    revision: self.song_session.revision,
                    track,
                    clip,
                    tempo: self.project.tempo,
                    range,
                    settings: AudioSilenceSettings::default(),
                    preview: None,
                    preview_file: None,
                    pending: None,
                    requested: Some(Instant::now() - Duration::from_secs(1)),
                    error: None,
                },
            )
        });
    }

    pub(crate) fn audio_silence_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut state) = ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE))) else {
            return;
        };
        if self.project.clip(state.track, state.clip.id) != Some(&state.clip)
            || state.session != self.song_session.id
            || state.revision != self.song_session.revision
            || self.edit_dirty
            || self.project.tempo != state.tempo
            || self.selected_clip != Some(state.clip.id)
            || self.document_guard.is_some()
        {
            reset(ctx);
            return;
        }
        if let Some(pending) = &state.pending {
            let result = pending
                .result
                .lock()
                .ok()
                .and_then(|mut result| result.take());
            if let Some(result) = result {
                if pending.settings == state.settings && !pending.cancel.load(Ordering::Acquire) {
                    match result.and_then(|(regions, file)| {
                        file.check_unchanged().map_err(|error| error.to_string())?;
                        Ok((regions, file))
                    }) {
                        Ok((regions, file)) => {
                            state.preview = Some(regions);
                            state.preview_file = Some(file);
                            state.error = None;
                        }
                        Err(error) => {
                            state.preview = None;
                            state.preview_file = None;
                            state.error = Some(error);
                        }
                    }
                }
                state.pending = None;
            }
        }
        if state.pending.is_none()
            && state
                .requested
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(180))
            && ANALYZING
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            let result: SharedResult = Arc::new(Mutex::new(None));
            let target = Arc::clone(&result);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let source = state.clip.clone();
            let admitted = if let ClipContent::Audio { path, .. } = &source.content {
                self.audio_files.get(path.as_str()).cloned()
            } else {
                None
            };
            let (tempo, range, settings) = (state.tempo, state.range, state.settings);
            let wake = ctx.clone();
            let worker = std::thread::Builder::new()
                .name("rill-strip-silence".into())
                .spawn(move || {
                    struct Reset;
                    impl Drop for Reset {
                        fn drop(&mut self) {
                            ANALYZING.store(false, Ordering::Release);
                        }
                    }
                    let _reset = Reset;
                    let analysis = std::panic::catch_unwind(|| -> anyhow::Result<_> {
                        crate::audio_editor::analysis_test_delay(&worker_cancel)?;
                        let ClipContent::Audio { path, .. } = &source.content else {
                            unreachable!()
                        };
                        let file = admitted.map_or_else(|| AudioFile::open(path), Ok)?;
                        let regions = detect_audio_silence_file(
                            &source,
                            &file,
                            tempo,
                            range,
                            settings,
                            &worker_cancel,
                        )?;
                        Ok((regions, file))
                    })
                    .map_err(|_| {
                        "Silence analysis stopped unexpectedly; source files are unchanged"
                            .to_owned()
                    })
                    .and_then(|result| result.map_err(|error| format!("{error:#}")));
                    drop(_reset);
                    if let Ok(mut slot) = target.lock() {
                        *slot = Some(analysis);
                    }
                    wake.request_repaint();
                });
            state.requested = None;
            match worker {
                Ok(_) => {
                    state.pending = Some(Pending {
                        settings: state.settings,
                        result,
                        cancel,
                    })
                }
                Err(error) => {
                    ANALYZING.store(false, Ordering::Release);
                    state.error = Some(format!("Start silence analysis: {error}"));
                }
            }
        }
        let title = format!("Strip silence · {}", state.clip.name);
        let mut apply = false;
        let mut cancel = false;
        let mut controls = Vec::new();
        let mut open = true;
        daw_ui::popup(title.as_str(), ctx)
            .id(egui::Id::new("audio_silence_dialog"))
            .default_width(daw_ui::tokens::POPUP_WIDTH_SM)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
            let mut footer = daw_ui::popup_footer(ui, "audio_silence_actions");
            let window_ui = ui;
            let ui = footer.body();
            daw_ui::text::note(
                ui,
                format!(
                    "Beats {:.3}–{:.3} · gaps stay in place",
                    state.range.0 + 1.0,
                    state.range.1 + 1.0
                ),
            );
            let before = state.settings;
            daw_ui::form_grid(ui, "audio_silence_settings", |ui| {
                let label = daw_ui::form_label(ui, "Threshold");
                let response = ui.add(daw_ui::number(&mut state.settings.threshold_db).range(-96.0..=0.0).speed(0.5).suffix(" dBFS")).labelled_by(label.id)
                    .on_hover_text("Source level before clip gain, fades and pitch processing. Any channel above this level is retained.");
                controls.push(response.rect);
                ui.end_row();
                let label = daw_ui::form_label(ui, "Minimum silence");
                controls.push(
                    ui.add(
                        daw_ui::number(&mut state.settings.minimum_silence_ms)
                            .range(1.0..=10000.0)
                            .speed(1.0)
                            .suffix(" ms"),
                    )
                    .labelled_by(label.id)
                    .rect,
                );
                ui.end_row();
                let label = daw_ui::form_label(ui, "Padding");
                controls.push(
                    ui.add(
                        daw_ui::number(&mut state.settings.padding_ms)
                            .range(0.0..=1000.0)
                            .speed(1.0)
                            .suffix(" ms"),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Keep this much audio around every retained attack and release.")
                    .rect,
                );
                ui.end_row();
            });
            if before != state.settings {
                if let Some(pending) = &state.pending { pending.cancel.store(true, Ordering::Release); }
                state.preview = None;
                state.preview_file = None;
                state.error = None;
                state.requested = Some(Instant::now());
            }
            daw_ui::section_gap(ui);
            let busy = state.pending.is_some() || state.requested.is_some();
            // Analysis results and errors share a reserved feedback area so
            // the Apply/Cancel targets stay in place when the job completes.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT * 2.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(daw_ui::tokens::CONTROL_HEIGHT * 2.0);
                    if busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            daw_ui::text::note(ui, "Analyzing source…");
                        });
                    } else if let Some(error) = &state.error {
                        daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, error, 3)
                            .on_hover_text(error);
                    } else if let Some(preview) = &state.preview {
                        let retained: f64 = preview.iter().map(|(start, end)| end - start).sum();
                        let removed = ((state.range.1 - state.range.0 - retained) * 60.0
                            / state.tempo)
                            .max(0.0);
                        daw_ui::text::body(
                            ui,
                            format!("Keep {} · remove {removed:.2} s", daw_ui::text::count(preview.len(), "region", "regions")),
                        );
                        daw_ui::text::bounded_block(
                            ui,
                            daw_ui::text::TextRole::Note,
                            "Tinted regions stay. Source files and other occurrences stay intact.",
                            2,
                        );
                    }
                },
            );
            let changes = state
                .preview
                .as_ref()
                .is_some_and(|regions| regions.as_slice() != [state.range]);
            footer.show(window_ui, |ui| {
                daw_ui::dialog_actions(
                    ui,
                    |ui| {
                        apply = ui
                            .add_enabled(
                                !busy && changes && state.error.is_none(),
                                daw_ui::dialog_primary_widget(ui, "Apply strip silence"),
                            )
                            .clicked();
                        cancel = daw_ui::dialog_button(ui, "Cancel").clicked();
                    },
                    |_| {},
                )
            });
        });
        // Keyboard shortcuts are gated while this dialog is open, so it owns Escape.
        cancel |=
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("audio_silence_controls"),
                controls
                    .into_iter()
                    .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
                    .collect::<Vec<_>>(),
            )
        });
        if !open || cancel {
            if let Some(pending) = &state.pending {
                pending.cancel.store(true, Ordering::Release);
            }
            reset(ctx);
            return;
        }
        if apply {
            let mut project = self.project.clone();
            let result = state
                .preview_file
                .as_ref()
                .context("Analyze the source before applying silence edits")
                .and_then(AudioFile::check_unchanged)
                .and_then(|()| {
                    project.strip_audio_silence(
                        state.track,
                        state.clip.id,
                        state.range,
                        state.preview.as_deref().unwrap_or_default(),
                    )
                });
            match result {
                Ok(ids) => {
                    self.begin_edit();
                    self.project = project;
                    self.selected_clip = ids.first().copied();
                    if ids.is_empty() {
                        self.close_popups();
                    }
                    self.edit_label = Some("Strip audio silence".into());
                    self.changed();
                    self.flush_pending_edit();
                    reset(ctx);
                    return;
                }
                Err(error) => {
                    state.preview = None;
                    state.preview_file = None;
                    state.error = Some(format!("{error:#}"));
                }
            }
        }
        if state.pending.is_some() || state.requested.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(STATE), state));
    }
}
