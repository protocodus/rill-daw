//! Reviewed, source-preserving silence removal from the focused audio selection.
use crate::app::DawApp;
use daw_core::{AudioData, AudioSilenceSettings, Clip, ClipContent, detect_audio_silence};
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
type Analysis = Result<Vec<(f64, f64)>, String>;
type SharedResult = Arc<Mutex<Option<Analysis>>>;
#[derive(Clone)]
struct Pending {
    settings: AudioSilenceSettings,
    result: SharedResult,
}
#[derive(Clone)]
struct Dialog {
    track: u64,
    clip: Clip,
    tempo: f64,
    range: (f64, f64),
    settings: AudioSilenceSettings,
    preview: Option<Vec<(f64, f64)>>,
    pending: Option<Pending>,
    requested: Option<Instant>,
    error: Option<String>,
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE)).is_some())
}
pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Dialog>(egui::Id::new(STATE));
    });
}
pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new(STATE))).map_or(serde_json::Value::Null, |state| {
        serde_json::json!({"range":state.range,"retained":state.preview,
            "pending":state.pending.is_some() || state.requested.is_some(),"error":state.error,
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
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(STATE),
                Dialog {
                    track,
                    clip,
                    tempo: self.project.tempo,
                    range,
                    settings: AudioSilenceSettings::default(),
                    preview: None,
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
                if pending.settings == state.settings {
                    match result {
                        Ok(regions) => {
                            state.preview = Some(regions);
                            state.error = None;
                        }
                        Err(error) => state.error = Some(error),
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
            let source = state.clip.clone();
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
                        let ClipContent::Audio { path, .. } = &source.content else {
                            unreachable!()
                        };
                        let data = AudioData::load_wav_with_limit(path, 512 * 1024 * 1024)?;
                        detect_audio_silence(&source, &data, tempo, range, settings)
                    })
                    .map_err(|_| {
                        "Silence analysis stopped unexpectedly; source files are unchanged"
                            .to_owned()
                    })
                    .and_then(|result| result.map_err(|error| format!("{error:#}")));
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
        let response = daw_ui::modal(ctx, egui::Id::new("audio_silence_dialog"), &title, |ui| {
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
                let label = daw_ui::text::label(ui, "Threshold");
                let response = ui.add(daw_ui::number(&mut state.settings.threshold_db).range(-96.0..=0.0).speed(0.5).suffix(" dBFS")).labelled_by(label.id)
                    .on_hover_text("Source level before clip gain, fades and pitch processing. Any channel above this level is retained.");
                controls.push(response.rect);
                ui.end_row();
                let label = daw_ui::text::label(ui, "Minimum silence");
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
                let label = daw_ui::text::label(ui, "Padding");
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
                state.preview = None;
                state.error = None;
                state.requested = Some(Instant::now());
            }
            ui.add_space(daw_ui::tokens::SPACE_MD);
            let busy = state.pending.is_some() || state.requested.is_some();
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    daw_ui::text::note(ui, "Analyzing source…");
                });
            }
            if let Some(preview) = &state.preview {
                let retained: f64 = preview.iter().map(|(start, end)| end - start).sum();
                let removed =
                    ((state.range.1 - state.range.0 - retained) * 60.0 / state.tempo).max(0.0);
                daw_ui::text::body(
                    ui,
                    format!("Keep {} region(s) · remove {removed:.2} s", preview.len()),
                );
                daw_ui::text::note(
                    ui,
                    "Tinted regions stay. Source files and other occurrences stay intact.",
                );
            }
            if let Some(error) = &state.error {
                daw_ui::text::note(ui, error);
            }
            ui.horizontal(|ui| {
                let changes = state
                    .preview
                    .as_ref()
                    .is_some_and(|regions| regions.as_slice() != [state.range]);
                apply = ui
                    .add_enabled(
                        !busy && changes && state.error.is_none(),
                        daw_ui::button_widget("Apply strip silence"),
                    )
                    .clicked();
                cancel = daw_ui::button(ui, "Cancel").clicked();
            });
        });
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("audio_silence_controls"),
                controls
                    .into_iter()
                    .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
                    .collect::<Vec<_>>(),
            )
        });
        if response.should_close() || cancel {
            reset(ctx);
            return;
        }
        if apply {
            let mut project = self.project.clone();
            match project.strip_audio_silence(
                state.track,
                state.clip.id,
                state.range,
                state.preview.as_deref().unwrap_or_default(),
            ) {
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
                Err(error) => state.error = Some(format!("{error:#}")),
            }
        }
        if state.pending.is_some() || state.requested.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(STATE), state));
    }
}
