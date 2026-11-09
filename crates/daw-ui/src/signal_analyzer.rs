//! Read-only live signal displays. Hosts supply measured audio, never demo data.
use egui::{Align2, FontId, Mesh, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SignalAnalyzerView {
    #[default]
    Spectrum,
    Levels,
    Loudness,
    Correlation,
    Phase,
}

impl SignalAnalyzerView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spectrum => "Spectrum",
            Self::Levels => "Levels",
            Self::Loudness => "Loudness",
            Self::Correlation => "Correlation",
            Self::Phase => "Phase",
        }
    }
}

/// Borrow one immutable engine snapshot for the current frame.
#[derive(Default)]
pub struct SignalAnalyzerData<'a> {
    pub sample_rate: u32,
    pub fft_size: usize,
    pub spectrum_db: &'a [f32],
    pub lufs_momentary: Option<f32>,
    pub lufs_short_term: Option<f32>,
    pub lufs_integrated: Option<f32>,
    pub peak_dbfs: Option<f32>,
    pub rms_dbfs: Option<f32>,
    pub rms_300ms_dbfs: Option<f32>,
    pub channel_rms_300ms_dbfs: [Option<f32>; 2],
    pub channel_peak_dbfs: [Option<f32>; 2],
    pub sample_peak_hold_dbfs: Option<f32>,
    pub true_peak_dbfs: Option<f32>,
    pub true_peak_hold_dbfs: Option<f32>,
    pub true_peak_oversampling: u8,
    pub overload_samples: [u64; 2],
    pub correlation: Option<f32>,
    pub phase: &'a [[f32; 2]],
}

pub struct SignalAnalyzerResponse {
    pub plot: Rect,
    pub trace_vertices: usize,
}

pub fn mixer_analysis_button(ui: &mut Ui, width: f32) -> Response {
    crate::hover_text(
        crate::mixer_row(
            ui,
            width,
            crate::tokens::COMPACT_HEIGHT,
            "Analyze",
            crate::MixerRowText::Caption,
            None,
            None,
        ),
        "Live spectrum, LUFS loudness, stereo correlation and phase",
    )
}

pub fn signal_analyzer_tabs(ui: &mut Ui, selected: &mut SignalAnalyzerView) {
    ui.horizontal_wrapped(|ui| {
        for view in [
            SignalAnalyzerView::Spectrum,
            SignalAnalyzerView::Levels,
            SignalAnalyzerView::Loudness,
            SignalAnalyzerView::Correlation,
            SignalAnalyzerView::Phase,
        ] {
            crate::selectable_value(ui, selected, view, view.label());
        }
    });
}

fn level(value: Option<f32>) -> String {
    finite(value)
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "−∞".into())
}

fn compact_count(value: u64) -> String {
    let mut number = value as f64;
    let mut unit = "";
    for suffix in ["k", "M", "G", "T", "P", "E"] {
        if number < 1_000.0 {
            break;
        }
        number /= 1_000.0;
        unit = suffix;
    }
    if unit.is_empty() {
        value.to_string()
    } else if number < 10.0 {
        format!("{number:.1}{unit}")
    } else {
        format!("{number:.0}{unit}")
    }
}

/// Stereo RMS bars and sample-peak markers use a shared calibrated dBFS axis.
/// Oversampled true peaks are reported separately, so peak bars do not imply
/// that a sample detector can observe inter-sample overloads.
fn levels(ui: &mut Ui, data: &SignalAnalyzerData<'_>, height: f32) -> SignalAnalyzerResponse {
    let rect = plot(ui, height);
    let colors = crate::theme::palette(ui.ctx());
    let meters = Rect::from_min_max(
        Pos2::new(rect.left() + 26.0, rect.top() + 20.0),
        Pos2::new(rect.left() + 78.0, rect.bottom()),
    );
    let y = |db: f32| meters.bottom() - ((db + 60.0) / 66.0).clamp(0.0, 1.0) * meters.height();
    for db in [-60, -48, -36, -24, -12, 0] {
        let color = if db == 0 { colors.warning } else { colors.line };
        ui.painter()
            .hline(meters.x_range(), y(db as f32), Stroke::new(1.0_f32, color));
        axis(
            ui,
            Pos2::new(meters.left() - 5.0, y(db as f32)),
            Align2::RIGHT_CENTER,
            &db.to_string(),
        );
    }
    for (channel, label) in ["L", "R"].into_iter().enumerate() {
        let x = meters.left() + channel as f32 * 28.0;
        let bar = Rect::from_min_max(
            Pos2::new(x, meters.top()),
            Pos2::new(x + 21.0, meters.bottom()),
        );
        axis(
            ui,
            Pos2::new(bar.center().x, rect.top()),
            Align2::CENTER_TOP,
            label,
        );
        ui.painter()
            .rect_filled(bar, 2.0, colors.line.gamma_multiply(0.5));
        if let Some(rms) = finite(data.channel_rms_300ms_dbfs[channel]) {
            ui.painter().rect_filled(
                Rect::from_min_max(Pos2::new(bar.left(), y(rms)), bar.max),
                1.0,
                colors.accent.gamma_multiply(0.65),
            );
        }
        if let Some(peak) = finite(data.channel_peak_dbfs[channel]) {
            ui.painter().hline(
                bar.x_range(),
                y(peak),
                Stroke::new(
                    2.0_f32,
                    if peak >= 0.0 {
                        colors.warning
                    } else {
                        colors.text
                    },
                ),
            );
        }
    }
    let entries = [
        ("Sample peak", format!("{} dBFS", level(data.peak_dbfs))),
        (
            "Peak hold",
            format!("{} dBFS", level(data.sample_peak_hold_dbfs)),
        ),
        ("True peak", format!("{} dBTP", level(data.true_peak_dbfs))),
        (
            "TP hold",
            format!("{} dBTP", level(data.true_peak_hold_dbfs)),
        ),
        ("RMS 300 ms", format!("{} dBFS", level(data.rms_300ms_dbfs))),
        (
            "Overs · L/R",
            format!(
                "{} / {}",
                compact_count(data.overload_samples[0]),
                compact_count(data.overload_samples[1])
            ),
        ),
    ];
    let left = meters.right() + 17.0;
    for (index, (caption, value)) in entries.into_iter().enumerate() {
        let at = rect.top() + index as f32 * rect.height() / 6.0;
        axis(ui, Pos2::new(left, at), Align2::LEFT_TOP, caption);
        ui.painter().text(
            Pos2::new(rect.right(), at),
            Align2::RIGHT_TOP,
            value,
            FontId::proportional(crate::tokens::FONT_CAPTION),
            colors.text,
        );
    }
    SignalAnalyzerResponse {
        plot: rect,
        trace_vertices: 0,
    }
}

fn finite(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite())
}

fn axis(ui: &Ui, at: Pos2, anchor: Align2, label: &str) {
    ui.painter().text(
        at,
        anchor,
        label,
        FontId::proportional(crate::tokens::FONT_CAPTION),
        crate::theme::palette(ui.ctx()).muted,
    );
}

fn plot(ui: &mut Ui, height: f32) -> Rect {
    let (outer, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(120.0), height),
        Sense::hover(),
    );
    ui.painter().rect_filled(
        outer,
        crate::tokens::RADIUS,
        crate::theme::palette(ui.ctx()).bg,
    );
    outer.shrink2(Vec2::new(crate::tokens::SPACE_XL, crate::tokens::SPACE_LG))
}

fn spectrum(ui: &mut Ui, data: &SignalAnalyzerData<'_>, height: f32) -> SignalAnalyzerResponse {
    let rect = plot(ui, height);
    let colors = crate::theme::palette(ui.ctx());
    let maximum = (data.sample_rate as f32 * 0.5).clamp(21.0, 24_000.0);
    let log_range = (maximum / 20.0).ln();
    let x = |hz: f32| rect.left() + ((hz / 20.0).ln() / log_range).clamp(0.0, 1.0) * rect.width();
    let y = |db: f32| rect.bottom() - ((db + 96.0) / 102.0).clamp(0.0, 1.0) * rect.height();
    for db in [-96, -72, -48, -24, 0] {
        ui.painter().hline(
            rect.x_range(),
            y(db as f32),
            Stroke::new(1.0_f32, colors.line),
        );
        axis(
            ui,
            Pos2::new(rect.left() - 5.0, y(db as f32)),
            Align2::RIGHT_CENTER,
            &db.to_string(),
        );
    }
    for (hz, label) in [
        (20.0, "20"),
        (100.0, "100"),
        (1000.0, "1k"),
        (10_000.0, "10k"),
    ] {
        if hz <= maximum {
            ui.painter()
                .vline(x(hz), rect.y_range(), Stroke::new(1.0_f32, colors.line));
            axis(
                ui,
                Pos2::new(x(hz), rect.bottom() + 5.0),
                Align2::CENTER_TOP,
                label,
            );
        }
    }
    // Max-pool in logarithmic frequency buckets so narrow spectral peaks survive
    // display reduction. Geometry remains bounded even for larger FFTs.
    let mut buckets = [-96.0_f32; 192];
    if data.fft_size > 0 {
        for (bin, level) in data.spectrum_db.iter().enumerate() {
            let hz = bin as f32 * data.sample_rate as f32 / data.fft_size as f32;
            if hz >= 20.0 && hz <= maximum && level.is_finite() {
                let bucket = (((hz / 20.0).ln() / log_range) * 191.0) as usize;
                buckets[bucket.min(191)] = buckets[bucket.min(191)].max(*level);
            }
        }
    }
    let mut mesh = Mesh::default();
    for (index, db) in buckets.iter().enumerate() {
        let at = rect.left() + index as f32 / 191.0 * rect.width();
        mesh.colored_vertex(Pos2::new(at, y(*db)), colors.accent.gamma_multiply(0.8));
        mesh.colored_vertex(
            Pos2::new(at, rect.bottom()),
            colors.accent.gamma_multiply(0.08),
        );
        if index > 0 {
            let n = index as u32 * 2;
            mesh.add_triangle(n - 2, n - 1, n);
            mesh.add_triangle(n, n - 1, n + 1);
        }
    }
    let trace_vertices = mesh.vertices.len();
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
    SignalAnalyzerResponse {
        plot: rect,
        trace_vertices,
    }
}

fn loudness(ui: &mut Ui, data: &SignalAnalyzerData<'_>, height: f32) -> SignalAnalyzerResponse {
    let rect = plot(ui, height);
    let colors = crate::theme::palette(ui.ctx());
    for (index, (label, value)) in [
        ("Momentary · 400 ms", data.lufs_momentary),
        ("Short term · 3 s", data.lufs_short_term),
        ("Integrated (approx.)", data.lufs_integrated),
    ]
    .into_iter()
    .enumerate()
    {
        let row_y = rect.top() + index as f32 * rect.height() / 3.0;
        axis(ui, Pos2::new(rect.left(), row_y), Align2::LEFT_TOP, label);
        let text = finite(value)
            .map(|value| format!("{value:.1} LUFS"))
            .unwrap_or_else(|| "— LUFS".into());
        ui.painter().text(
            Pos2::new(rect.right(), row_y),
            Align2::RIGHT_TOP,
            text,
            FontId::proportional(crate::tokens::FONT_SUBTITLE),
            colors.text,
        );
        let bar = Rect::from_min_size(
            Pos2::new(rect.left(), row_y + crate::tokens::CONTROL_HEIGHT),
            Vec2::new(rect.width(), crate::tokens::SPACE_SM),
        );
        ui.painter().rect_filled(bar, 2.0, colors.line);
        if let Some(value) = finite(value) {
            let end = bar.left() + ((value + 60.0) / 66.0).clamp(0.0, 1.0) * bar.width();
            ui.painter().rect_filled(
                Rect::from_min_max(bar.min, Pos2::new(end, bar.bottom())),
                2.0,
                colors.accent,
            );
        }
    }
    SignalAnalyzerResponse {
        plot: rect,
        trace_vertices: 0,
    }
}

fn correlation(ui: &mut Ui, data: &SignalAnalyzerData<'_>, height: f32) -> SignalAnalyzerResponse {
    let rect = plot(ui, height);
    let colors = crate::theme::palette(ui.ctx());
    let value = finite(data.correlation).map(|value| value.clamp(-1.0, 1.0));
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 30.0),
        Align2::CENTER_CENTER,
        value
            .map(|value| format!("{value:+.2}"))
            .unwrap_or_else(|| "—".into()),
        FontId::proportional(crate::tokens::FONT_HEADING * 2.0),
        colors.text,
    );
    let bar = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y + 15.0),
        Vec2::new(rect.width(), crate::tokens::CONTROL_HEIGHT),
    );
    ui.painter()
        .rect_filled(bar, crate::tokens::RADIUS, colors.line);
    if let Some(value) = value {
        let at = bar.center().x + value * bar.width() * 0.5;
        ui.painter().rect_filled(
            Rect::from_min_max(
                Pos2::new(at.min(bar.center().x), bar.top()),
                Pos2::new(at.max(bar.center().x), bar.bottom()),
            ),
            2.0,
            if value < 0.0 {
                colors.warning
            } else {
                colors.guide
            },
        );
        ui.painter()
            .vline(at, bar.y_range(), Stroke::new(2.0_f32, colors.text));
    }
    for (fraction, label) in [(0.0, "−1 · Opposite"), (0.5, "0"), (1.0, "+1 · Mono")] {
        axis(
            ui,
            Pos2::new(bar.left() + fraction * bar.width(), bar.bottom() + 8.0),
            if fraction == 0.0 {
                Align2::LEFT_TOP
            } else if fraction == 1.0 {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            },
            label,
        );
    }
    SignalAnalyzerResponse {
        plot: rect,
        trace_vertices: 0,
    }
}

fn phase(ui: &mut Ui, data: &SignalAnalyzerData<'_>, height: f32) -> SignalAnalyzerResponse {
    let available = plot(ui, height);
    let size = available.size().min_elem();
    let rect = Rect::from_center_size(available.center(), Vec2::splat(size));
    let colors = crate::theme::palette(ui.ctx());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0_f32, colors.line),
    );
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0_f32, colors.line),
    );
    axis(
        ui,
        Pos2::new(rect.center().x, rect.top() - 3.0),
        Align2::CENTER_BOTTOM,
        "Mid · L + R",
    );
    axis(
        ui,
        Pos2::new(rect.right() + 5.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Side",
    );
    let mut mesh = Mesh::default();
    for [left, right] in data
        .phase
        .iter()
        .copied()
        .take(256)
        .filter(|frame| frame.iter().all(|value| value.is_finite()))
    {
        let side = ((left - right) * 0.5).clamp(-1.0, 1.0);
        let mid = ((left + right) * 0.5).clamp(-1.0, 1.0);
        let at = Pos2::new(
            rect.center().x + side * rect.width() * 0.5,
            rect.center().y - mid * rect.height() * 0.5,
        );
        mesh.add_colored_rect(
            Rect::from_center_size(at, Vec2::splat(2.0)),
            colors.accent.gamma_multiply(0.75),
        );
    }
    let trace_vertices = mesh.vertices.len();
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
    SignalAnalyzerResponse {
        plot: rect,
        trace_vertices,
    }
}

/// Fixed-size plotting keeps tab changes and asynchronous measurements stable.
pub fn signal_analyzer(
    ui: &mut Ui,
    view: SignalAnalyzerView,
    data: Option<&SignalAnalyzerData<'_>>,
    height: f32,
    empty: &str,
) -> SignalAnalyzerResponse {
    let height = height.max(210.0);
    let Some(data) = data else {
        let rect = plot(ui, height);
        axis(ui, rect.center(), Align2::CENTER_CENTER, empty);
        return SignalAnalyzerResponse {
            plot: rect,
            trace_vertices: 0,
        };
    };
    if (view == SignalAnalyzerView::Spectrum && data.spectrum_db.is_empty())
        || (view == SignalAnalyzerView::Phase && data.phase.is_empty())
    {
        let rect = plot(ui, height);
        axis(
            ui,
            rect.center(),
            Align2::CENTER_CENTER,
            "Collecting audio…",
        );
        return SignalAnalyzerResponse {
            plot: rect,
            trace_vertices: 0,
        };
    }
    let response = match view {
        SignalAnalyzerView::Spectrum => spectrum(ui, data, height),
        SignalAnalyzerView::Levels => levels(ui, data, height),
        SignalAnalyzerView::Loudness => loudness(ui, data, height),
        SignalAnalyzerView::Correlation => correlation(ui, data, height),
        SignalAnalyzerView::Phase => phase(ui, data, height),
    };
    if view == SignalAnalyzerView::Loudness {
        crate::hover_text(
            crate::text::note(ui, "LUFS · K-weighted · Gated integrated estimate"),
            "Momentary uses 400 ms, short term uses 3 seconds. Integrated covers continuous playback since opening or resetting; seeking or restarting playback begins a fresh measurement. It uses absolute and relative gating with a bounded 0.01 LU histogram approximation. Levels shows a separate oversampled true-peak estimate.",
        );
    }
    if view == SignalAnalyzerView::Levels {
        crate::hover_text(
            crate::text::note(
                ui,
                format!(
                    "RMS · 300 ms exponential · True peak · {}× FIR estimate",
                    data.true_peak_oversampling
                ),
            ),
            format!(
                "RMS uses a 300 ms exponential mean-square time constant, not a VU calibration. White markers show sample peaks. True peak estimates inter-sample levels with FIR oversampling to at least 192 kHz; this is not a certified compliance meter. Peak holds and over-full-scale sample counts cover continuous audio since Reset; seeking or restarting resets the measurement. Overs counts samples above 0 dBFS, not reconstructed inter-sample peaks.\nOver-full-scale samples · Left {} · Right {}",
                data.overload_samples[0], data.overload_samples[1]
            ),
        );
    } else {
        let response = crate::text::note(
            ui,
            format!(
                "Sample peak {} dBFS · True peak {} dBTP · RMS {} dBFS",
                level(data.peak_dbfs),
                level(data.true_peak_dbfs),
                level(data.rms_dbfs),
            ),
        );
        crate::hover_text(
            response,
            format!(
                "Sample peak and RMS describe the most recent {:.0} ms of audio. Levels provides separate 300 ms exponential RMS and resettable peak holds. True peak is an oversampled inter-sample estimate.",
                if data.sample_rate > 0 {
                    data.fft_size as f32 / data.sample_rate as f32 * 1000.0
                } else {
                    0.0
                },
            ),
        );
    }
    response
}
