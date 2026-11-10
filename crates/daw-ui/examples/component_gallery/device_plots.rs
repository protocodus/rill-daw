use daw_ui::{
    self as ui, DelayViewData, FrequencyCurve, FrequencyNode, FrequencyPlot, SampleViewData,
    Spectrum, TransferPlot, tokens,
};

pub struct State {
    response: Vec<f32>,
    spectrum: Vec<f32>,
    nodes: Vec<FrequencyNode>,
    transfer: Vec<f32>,
    history: Vec<(f32, f32)>,
    wave: Vec<f32>,
    frames: Vec<Vec<f32>>,
    peaks: Vec<(f32, f32)>,
    sample: (f32, f32),
}

impl Default for State {
    fn default() -> Self {
        let response = (0..240)
            .map(|i| {
                let f = ui::log_grid_frequency(i, 240);
                6.0 * (-((f.ln() - 1000f32.ln()) / 0.7).powi(2)).exp()
                    - 3.0 * (-((f.ln() - 100f32.ln()) / 0.5).powi(2)).exp()
            })
            .collect();
        let spectrum = (0..2049)
            .map(|bin| -20.0 - 40.0 * (bin as f32 / 2049.0).sqrt() - (bin % 7) as f32)
            .collect();
        let node = |frequency, gain_db, selected, gain_editable| FrequencyNode {
            frequency,
            gain_db,
            selected,
            color: None,
            gain_editable,
        };
        let nodes = vec![
            node(80.0, 0.0, false, false),
            node(1000.0, 6.0, true, true),
            node(9000.0, -2.0, false, true),
        ];
        let transfer = (0..65)
            .map(|i| {
                let x = -60.0 + i as f32 * 60.0 / 64.0;
                if x < -24.0 {
                    x
                } else {
                    -24.0 + (x + 24.0) / 4.0
                }
            })
            .collect();
        let history = (0..120)
            .map(|i| {
                (
                    0.2 + 0.6 * ((i as f32 / 9.0).sin().abs()),
                    8.0 * ((i as f32 / 9.0).sin().abs()),
                )
            })
            .collect();
        let wave = (0..256)
            .map(|i| (i as f32 / 256.0 * std::f32::consts::TAU * 2.0).sin() * 0.8)
            .collect();
        let frames = (1..=8)
            .map(|h| {
                (0..128)
                    .map(|i| {
                        (0..h)
                            .map(|k| {
                                (i as f32 / 128.0 * std::f32::consts::TAU * (k + 1) as f32).sin()
                                    / (k + 1) as f32
                            })
                            .sum::<f32>()
                            * 0.6
                    })
                    .collect()
            })
            .collect();
        let peaks = (0..200)
            .map(|i| {
                let a = ((i as f32 / 12.0).sin().abs() * (1.0 - i as f32 / 220.0)).min(1.0);
                (-a, a)
            })
            .collect();
        Self {
            response,
            spectrum,
            nodes,
            transfer,
            history,
            wave,
            frames,
            peaks,
            sample: (0.1, 0.9),
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State) {
    ui::text::note(
        ui,
        "Fixed demonstration data; devices supply curves from the same code as their audio path.",
    );
    ui.add_space(tokens::SPACE_MD);
    let small = egui::vec2(233.0, 120.0);
    let output = ui::frequency_plot(
        ui,
        &FrequencyPlot {
            curves: &[FrequencyCurve {
                db: &state.response,
                emphasis: true,
            }],
            spectrum: Some(Spectrum {
                bins_db: &state.spectrum,
                sample_rate: 48_000.0,
            }),
            nodes: &state.nodes,
            range_db: 18.0,
            size: egui::vec2(610.0, 170.0),
        },
    );
    for drag in output.drags {
        state.nodes[drag.index].frequency = drag.frequency;
        state.nodes[drag.index].gain_db = drag.gain_db;
    }
    if let Some(index) = output.clicked {
        for (i, node) in state.nodes.iter_mut().enumerate() {
            node.selected = i == index;
        }
    }
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        ui::transfer_plot(
            ui,
            &TransferPlot {
                curve: &state.transfer,
                x_range: (-60.0, 0.0),
                y_range: (-60.0, 0.0),
                input: Some(-18.0),
                size: small,
            },
        );
        ui::gain_reduction_history(ui, &state.history, egui::vec2(233.0, 120.0), 24.0);
        ui::reduction_meter(ui, 6.0, 9.0, 24.0, egui::vec2(13.0, 120.0));
        ui::scope(ui, &state.wave, true, small);
    });
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        let frames: Vec<&[f32]> = state.frames.iter().map(Vec::as_slice).collect();
        ui::wavetable_view(ui, &frames, 0.4, small);
        ui::lfo_view(ui, &state.wave[..128], Some(0.3), small);
        ui::spectrum(
            ui,
            Spectrum {
                bins_db: &state.spectrum,
                sample_rate: 48_000.0,
            },
            small,
        );
    });
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        let output = ui::sample_view(
            ui,
            &SampleViewData {
                peaks: &state.peaks,
                start: state.sample.0,
                end: state.sample.1,
                loop_range: Some((0.3, 0.7)),
                playheads: &[0.45],
            },
            egui::vec2(377.0, 120.0),
        );
        for (marker, value) in output.drags {
            match marker {
                ui::SampleMarker::Start => state.sample.0 = value,
                ui::SampleMarker::End => state.sample.1 = value,
                _ => {}
            }
        }
        ui::delay_view(
            ui,
            &DelayViewData {
                taps: &[
                    (0.25, 0.9, 0.0),
                    (0.5, 0.0, 0.6),
                    (0.75, 0.4, 0.0),
                    (1.0, 0.0, 0.25),
                ],
                decay_seconds: 1.6,
                pre_delay_seconds: 0.02,
                span_seconds: 1.5,
            },
            small,
        );
        ui::harmonics_view(
            ui,
            &[
                0.0, -12.0, -6.0, -24.0, -18.0, -36.0, -30.0, -48.0, -60.0, -70.0,
            ],
            small,
        );
    });
}
