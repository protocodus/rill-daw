use daw_ui::{self as ui, SignalAnalyzerData, SignalAnalyzerView, tokens};

pub struct State {
    spectrum: Vec<f32>,
    phase: Vec<[f32; 2]>,
}

impl Default for State {
    fn default() -> Self {
        let spectrum = (0..2049)
            .map(|bin| {
                let hz = bin as f32 * 48_000.0 / 4096.0;
                (-4.4 - ((hz - 1000.0) / 18.0).powi(2) * 6.0).max(-96.0)
            })
            .collect();
        let phase = (0..256)
            .map(|index| {
                let at = index as f32 / 256.0 * std::f32::consts::TAU;
                [
                    at.sin() * 0.6,
                    (at + std::f32::consts::FRAC_PI_3).sin() * 0.6,
                ]
            })
            .collect();
        Self { spectrum, phase }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State) {
    ui::text::note(
        ui,
        "Fixed demonstration readings; the application supplies live measured audio.",
    );
    ui.add_space(tokens::SPACE_MD);
    let width = (ui.available_width() - tokens::SPACE_LG) * 0.5;
    let data = SignalAnalyzerData {
        sample_rate: 48_000,
        fft_size: 4096,
        spectrum_db: &state.spectrum,
        lufs_momentary: Some(-4.4),
        lufs_short_term: Some(-4.4),
        lufs_integrated: Some(-4.4),
        peak_dbfs: Some(-4.4),
        rms_dbfs: Some(-7.4),
        rms_300ms_dbfs: Some(-7.4),
        channel_rms_300ms_dbfs: [Some(-7.4), Some(-8.0)],
        channel_peak_dbfs: [Some(-4.4), Some(-5.0)],
        sample_peak_hold_dbfs: Some(-4.0),
        true_peak_dbfs: Some(-4.2),
        true_peak_hold_dbfs: Some(-3.8),
        true_peak_oversampling: 4,
        overload_samples: [0; 2],
        correlation: Some(0.5),
        phase: &state.phase,
    };
    egui::Grid::new("signal_analysis_gallery")
        .num_columns(2)
        .spacing(egui::vec2(tokens::SPACE_LG, tokens::SPACE_LG))
        .show(ui, |ui| {
            for (index, view) in [
                SignalAnalyzerView::Spectrum,
                SignalAnalyzerView::Loudness,
                SignalAnalyzerView::Correlation,
                SignalAnalyzerView::Phase,
            ]
            .into_iter()
            .enumerate()
            {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui::text::subtitle(ui, view.label());
                    ui.add_space(tokens::SPACE_SM);
                    ui::signal_analyzer(ui, view, Some(&data), 230.0, "");
                });
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}
