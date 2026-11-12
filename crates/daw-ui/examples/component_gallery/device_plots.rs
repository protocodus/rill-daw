use daw_ui::{
    self as ui, DelayHandle, DelayTail, DelayTap, DelayView, FrequencyCurve, FrequencyMarker,
    FrequencyNode, FrequencyPlot, HarmonicsView, LevelHistory, LfoMarker, LfoTrace, LfoView,
    ReductionMeter, SampleViewData, Spectrum, TransferAxis, TransferHandle, TransferPlot, tokens,
};

pub struct State {
    response: Vec<f32>,
    spectrum: Vec<f32>,
    nodes: Vec<FrequencyNode<'static>>,
    transfer: Vec<f32>,
    shaper: Vec<f32>,
    history: Vec<[f32; 3]>,
    threshold: f32,
    hold: f32,
    wave: Vec<f32>,
    frames: Vec<Vec<f32>>,
    peaks: Vec<(f32, f32)>,
    sample: (f32, f32),
    /// A comb's upper and lower edge, where its teeth are finer than a point.
    comb: (Vec<f32>, Vec<f32>),
    /// Echo time, echo level, pre-delay and decay of the delay view.
    echo: (f32, f32),
    tail: (f32, f32),
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
        let nodes = vec![
            FrequencyNode {
                gain_editable: false,
                label: Some("HP"),
                readout: Some("80 Hz  24 dB/oct"),
                ..FrequencyNode::new(80.0, 0.0)
            },
            FrequencyNode {
                selected: true,
                readout: Some("1.00 kHz  +6.0 dB  Q 1.00"),
                ..FrequencyNode::new(1000.0, 6.0)
            },
            FrequencyNode {
                enabled: false,
                ..FrequencyNode::new(9000.0, -2.0)
            },
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
        let shaper = (0..65)
            .map(|i| (3.0 * (i as f32 / 32.0 - 1.0)).tanh())
            .collect();
        // Hits that decay, squeezed 2:1 above −24 dB.
        let history = (0..240)
            .map(|i| {
                let level = 0.01 + 0.3 * (-((i % 40) as f32) / 9.0).exp();
                let over = (20.0 * level.log10() + 24.0).max(0.0);
                let reduction = over * 0.5;
                [level, level * 10f32.powf(-reduction / 20.0), reduction]
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
        let comb = (0..240)
            .map(|i| {
                let f = ui::log_grid_frequency(i, 240);
                // Teeth 500 Hz apart: single notches low down, a band on top.
                if f < 3000.0 {
                    let depth = 6.0 * (std::f32::consts::PI * f / 500.0).cos();
                    (depth, depth)
                } else {
                    (6.0, -6.0)
                }
            })
            .unzip();
        Self {
            comb,
            echo: (0.375, -6.0),
            tail: (0.02, 1.6),
            response,
            spectrum,
            nodes,
            transfer,
            shaper,
            history,
            threshold: -24.0,
            hold: 9.0,
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
            curves: &[
                FrequencyCurve {
                    db: &state.comb.0,
                    emphasis: false,
                    low: Some(&state.comb.1),
                },
                FrequencyCurve {
                    db: &state.response,
                    emphasis: true,
                    low: None,
                },
            ],
            spectrum: Some(Spectrum {
                tilt_db_per_octave: 4.5,
                smoothing_octaves: 1.0 / 6.0,
                ..Spectrum::new(&state.spectrum, 48_000.0)
            }),
            nodes: &state.nodes,
            range_db: 18.0,
            limit_db: 18.0,
            spans: &[(300.0, 2400.0)],
            markers: &[FrequencyMarker {
                frequency: 600.0,
                label: "L",
            }],
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
    if let Some(index) = output.double_clicked {
        state.nodes[index].enabled = !state.nodes[index].enabled;
    }
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        let readout = format!("Threshold {:.1} dB", state.threshold);
        let output = ui::transfer_plot(
            ui,
            &TransferPlot {
                curve: &state.transfer,
                x_range: (-60.0, 0.0),
                y_range: (-60.0, 0.0),
                axis: TransferAxis::Decibels,
                input: Some(-18.0),
                shade: Some((-27.0, -21.0)),
                handle: Some(TransferHandle {
                    x: state.threshold,
                    y: state.threshold,
                    label: "T",
                    readout: &readout,
                }),
                size: egui::vec2(170.0, 170.0),
                ..Default::default()
            },
        );
        if let Some((x, _)) = output.dragged {
            state.threshold = x;
        }
        if let Some((x, _)) = output.nudged {
            state.threshold = (state.threshold + x).clamp(-60.0, 0.0);
        }
        let output = ui::gain_reduction_history(
            ui,
            &LevelHistory {
                columns: &state.history,
                floor_db: -60.0,
                max_reduction_db: 24.0,
                threshold_db: Some(state.threshold),
                threshold_label: "Threshold",
                draggable: true,
                span_seconds: 8.0,
                size: egui::vec2(340.0, 170.0),
            },
        );
        if let Some(threshold) = output.threshold {
            state.threshold = threshold;
        }
        let meter = ui::reduction_meter(
            ui,
            &ReductionMeter {
                reduction_db: 6.0,
                hold_db: state.hold,
                max_reduction_db: 24.0,
                size: egui::vec2(48.0, 170.0),
            },
        );
        if meter.clicked() {
            state.hold = 6.0;
        }
    });
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        ui::transfer_plot(
            ui,
            &TransferPlot {
                curve: &state.shaper,
                input: Some(0.4),
                lit: Some((-0.4, 0.4)),
                size: egui::vec2(120.0, 120.0),
                ..Default::default()
            },
        );
        ui::harmonics_view(
            ui,
            &HarmonicsView {
                levels_db: &[
                    -3.0, -15.0, -9.0, -27.0, -21.0, -39.0, -33.0, -51.0, -63.0, -90.0,
                ],
                floor_db: -80.0,
                size: egui::vec2(335.0, 120.0),
            },
        );
        ui::scope(ui, &state.wave, true, egui::vec2(120.0, 120.0));
    });
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        let frames: Vec<&[f32]> = state.frames.iter().map(Vec::as_slice).collect();
        ui::wavetable_view(ui, &frames, 0.4, small);
        let second: Vec<f32> = (0..128).map(|i| state.wave[(i + 32) % 128] * 0.6).collect();
        ui::lfo_view(
            ui,
            &LfoView {
                traces: &[
                    LfoTrace {
                        samples: &state.wave[..128],
                        emphasis: true,
                    },
                    LfoTrace {
                        samples: &second,
                        emphasis: false,
                    },
                ],
                markers: &[
                    LfoMarker {
                        phase: 0.3,
                        label: "L",
                    },
                    LfoMarker {
                        phase: 0.55,
                        label: "R",
                    },
                ],
                draggable: false,
                size: small,
            },
        );
        ui::spectrum(ui, Spectrum::new(&state.spectrum, 48_000.0), small);
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
        let (time, level) = state.echo;
        let (pre_delay, decay) = state.tail;
        let gain = |db: f32| 10f32.powf(db / 20.0);
        let taps: Vec<DelayTap> = (1..=5)
            .map(|repeat| {
                let level = gain(level - 8.0 * (repeat - 1) as f32);
                DelayTap {
                    seconds: time * repeat as f32,
                    left: if repeat % 2 == 1 { level } else { 0.0 },
                    right: if repeat % 2 == 0 { level } else { 0.0 },
                    thin: false,
                    faint: false,
                }
            })
            .chain([0.012, 0.021, 0.034].map(|seconds| DelayTap {
                seconds: pre_delay + seconds,
                left: 0.2,
                right: 0.15,
                thin: true,
                faint: false,
            }))
            .collect();
        let output = ui::delay_view(
            ui,
            &DelayView {
                taps: &taps,
                tails: &[
                    DelayTail {
                        start_seconds: pre_delay,
                        start_db: -9.0,
                        decay_seconds: decay,
                        faint: false,
                    },
                    DelayTail {
                        start_seconds: time + pre_delay,
                        start_db: level - 9.0,
                        decay_seconds: decay,
                        faint: true,
                    },
                ],
                grid: &[(0.5, "2"), (1.0, "3"), (1.5, "4")],
                span_seconds: 2.0,
                floor_db: -60.0,
                handles: &[
                    DelayHandle {
                        seconds: time,
                        db: level,
                        below: false,
                        label: "T",
                        readout: &format!("Time {:.0} ms · Level {level:+.1} dB", time * 1000.0),
                        horizontal: true,
                        vertical: true,
                    },
                    DelayHandle {
                        seconds: pre_delay + decay * 51.0 / 60.0,
                        db: -60.0,
                        below: false,
                        label: "D",
                        readout: &format!("Decay {decay:.2} s"),
                        horizontal: true,
                        vertical: false,
                    },
                ],
                reduction_db: Some((6.0, 24.0)),
                size: egui::vec2(377.0, 144.0),
            },
        );
        for drag in output.drags {
            match drag.index {
                0 => state.echo = (drag.seconds.max(0.01), drag.db),
                _ => state.tail.1 = ((drag.seconds - pre_delay) * 60.0 / 51.0).max(0.1),
            }
        }
    });
}
