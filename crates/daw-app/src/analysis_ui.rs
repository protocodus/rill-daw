//! Focused live mixer measurements. The engine owns audio analysis.
use crate::app::{DawApp, PopupKind};
use daw_engine::SignalMonitorTarget;
use eframe::egui;
use serde_json::{Value, json};

pub(crate) struct SignalAnalysis {
    pub open: bool,
    target: SignalMonitorTarget,
    view: daw_ui::SignalAnalyzerView,
    engine_id: Option<u64>,
    latest: Option<std::sync::Arc<daw_engine::SignalMonitorSnapshot>>,
}

impl Default for SignalAnalysis {
    fn default() -> Self {
        Self {
            open: false,
            target: SignalMonitorTarget::Master,
            view: Default::default(),
            engine_id: None,
            latest: None,
        }
    }
}

impl DawApp {
    pub(crate) fn analysis_title(&self) -> String {
        let subject = match self.analysis.target {
            SignalMonitorTarget::Master => "Master",
            SignalMonitorTarget::Channel(id) => self
                .project
                .track(id)
                .map(|track| track.name.as_str())
                .unwrap_or("Channel"),
        };
        format!("Signal analysis · {subject}")
    }

    pub(crate) fn open_signal_analysis(&mut self, target: SignalMonitorTarget) {
        self.open_popup(PopupKind::Analysis);
        self.analysis.target = target;
        self.analysis.view = Default::default();
        self.analysis.engine_id = None;
        self.analysis.latest = None;
    }

    pub(crate) fn stop_signal_analysis(&mut self) {
        self.analysis.open = false;
        self.analysis.engine_id = None;
        self.analysis.latest = None;
        if let Some(engine) = &self.engine {
            let _ = engine.set_signal_monitor(None);
        }
    }

    pub(crate) fn sync_signal_analysis(&mut self) {
        if !self.analysis.open {
            return;
        }
        if let SignalMonitorTarget::Channel(id) = self.analysis.target
            && self.project.track(id).is_none()
        {
            self.close_popup();
            return;
        }
        if let Some(engine) = &self.engine {
            let id = engine.snapshot().engine_id;
            if self.analysis.engine_id != Some(id) {
                self.analysis.latest = None;
                match engine.set_signal_monitor(Some(self.analysis.target)) {
                    Ok(()) => self.analysis.engine_id = Some(id),
                    Err(error) => self.message(Err(error), ""),
                }
            }
            if self.playing() {
                self.analysis.latest = self
                    .engine
                    .as_ref()
                    .and_then(|engine| engine.signal_monitor_snapshot())
                    .filter(|snapshot| snapshot.target == self.analysis.target);
            }
        } else {
            self.analysis.engine_id = None;
        }
    }

    pub(crate) fn signal_analysis_popup(&mut self, ctx: &egui::Context) {
        if !self.analysis.open {
            return;
        }
        let (mut open, mut done, mut reset) = (true, false, false);
        let playing = self.playing();
        let snapshot = self.analysis.latest.as_ref().filter(|snapshot| {
            snapshot.target == self.analysis.target && snapshot.observed_frames > 0
        });
        let data = snapshot
            .as_ref()
            .map(|snapshot| daw_ui::SignalAnalyzerData {
                sample_rate: snapshot.sample_rate,
                fft_size: snapshot.fft_size,
                spectrum_db: &snapshot.spectrum_db,
                lufs_momentary: snapshot.lufs_momentary,
                lufs_short_term: snapshot.lufs_short_term,
                lufs_integrated: snapshot.lufs_integrated,
                peak_dbfs: snapshot.peak_dbfs,
                rms_dbfs: snapshot.rms_dbfs,
                rms_300ms_dbfs: snapshot.rms_300ms_dbfs,
                channel_rms_300ms_dbfs: snapshot.channel_rms_300ms_dbfs,
                channel_peak_dbfs: snapshot.channel_peak_dbfs,
                sample_peak_hold_dbfs: snapshot.sample_peak_hold_dbfs,
                true_peak_dbfs: snapshot.true_peak_dbfs,
                true_peak_hold_dbfs: snapshot.true_peak_hold_dbfs,
                true_peak_oversampling: snapshot.true_peak_oversampling,
                overload_samples: snapshot.overload_samples,
                correlation: snapshot.correlation,
                phase: &snapshot.phase,
            });
        let status = if playing {
            if data.is_some() {
                "Live · Post-fader signal"
            } else {
                "Waiting for audio…"
            }
        } else if data.is_some() {
            "Paused / stopped · Readings held"
        } else {
            "Play to analyze this signal"
        };
        daw_ui::popup(self.analysis_title(), ctx).id(egui::Id::new("signal_analysis_popup"))
            .open(&mut open).default_width(daw_ui::tokens::POPUP_WIDTH_MD)
            .default_height(470.0).min_width(360.0).vscroll(false)
            .show(ctx, |ui| {
                daw_ui::signal_analyzer_tabs(ui, &mut self.analysis.view);
                ui.add_space(daw_ui::tokens::SPACE_SM);
                let mut footer = daw_ui::popup_footer(ui, "signal_analysis_footer");
                let body = footer.body();
                let height = (body.available_height() - 65.0).clamp(210.0, 360.0);
                let response = daw_ui::signal_analyzer(body, self.analysis.view, data.as_ref(), height, status);
                ctx.data_mut(|values| values.insert_temp(egui::Id::new("signal_analysis_plot"), json!({
                    "plot":{"min":[response.plot.left(),response.plot.top()],"max":[response.plot.right(),response.plot.bottom()]},
                    "trace_vertices":response.trace_vertices,
                })));
                footer.show(ui, |ui| {
                    daw_ui::dialog_actions(ui, |ui| {
                        done = daw_ui::dialog_button(ui, "Done").clicked();
                        reset = daw_ui::dialog_button(ui, "Reset measurement").clicked();
                    }, |ui| { daw_ui::hover_text(daw_ui::text::note(ui, status), match self.analysis.target {
                        SignalMonitorTarget::Master => "Post-insert Master output, after volume and pan; before device clipping, dry input monitoring and metronome",
                        SignalMonitorTarget::Channel(_) => "Channel after inserts, volume, pan and mute; before output/send destination gain",
                    }); });
                });
            });
        if reset {
            if let Some(engine) = &self.engine {
                engine.reset_signal_monitor();
            }
            self.analysis.latest = None;
        }
        self.analysis.open &= open && !done;
        if playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// The headless driver observes the same immutable snapshot used for plots.
    pub(crate) fn signal_analysis_diagnostics(&self, ctx: &egui::Context) -> Value {
        if !self.analysis.open {
            return Value::Null;
        }
        let snapshot = self
            .analysis
            .latest
            .as_ref()
            .filter(|snapshot| snapshot.target == self.analysis.target);
        let mut result = ctx
            .data(|values| values.get_temp::<Value>(egui::Id::new("signal_analysis_plot")))
            .unwrap_or_else(|| json!({}));
        result["view"] = json!(self.analysis.view.label());
        result["target"] = json!(match self.analysis.target {
            SignalMonitorTarget::Master => "Master",
            SignalMonitorTarget::Channel(_) => "Channel",
        });
        result["track"] = json!(match self.analysis.target {
            SignalMonitorTarget::Channel(id) => Some(id),
            _ => None,
        });
        result["playing"] = json!(self.playing());
        result["snapshot"] = snapshot.map(|snapshot| json!({
            "sample_rate":snapshot.sample_rate,"fft_size":snapshot.fft_size,"observed_frames":snapshot.observed_frames,
            "generation":snapshot.generation,"dropped_blocks":snapshot.dropped_blocks,
            "spectrum_db":snapshot.spectrum_db,"lufs_momentary":snapshot.lufs_momentary,
            "lufs_short_term":snapshot.lufs_short_term,"lufs_integrated":snapshot.lufs_integrated,
            "peak_dbfs":snapshot.peak_dbfs,"rms_dbfs":snapshot.rms_dbfs,
            "rms_300ms_dbfs":snapshot.rms_300ms_dbfs,
            "channel_rms_300ms_dbfs":snapshot.channel_rms_300ms_dbfs,
            "channel_peak_dbfs":snapshot.channel_peak_dbfs,
            "sample_peak_hold_dbfs":snapshot.sample_peak_hold_dbfs,
            "true_peak_dbfs":snapshot.true_peak_dbfs,
            "true_peak_hold_dbfs":snapshot.true_peak_hold_dbfs,
            "true_peak_oversampling":snapshot.true_peak_oversampling,
            "overload_samples":snapshot.overload_samples,
            "correlation":snapshot.correlation,"phase":snapshot.phase,
        })).unwrap_or(Value::Null);
        result
    }
}
