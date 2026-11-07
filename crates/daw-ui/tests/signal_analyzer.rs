use daw_ui::{SignalAnalyzerData, SignalAnalyzerView};
use egui::{Context, RawInput, Rect, Vec2};

fn render(view: SignalAnalyzerView, phase: &[[f32; 2]]) -> egui::FullOutput {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let spectrum = vec![-60.0; 2049];
    ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(700.0, 500.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::signal_analyzer(
                    ui,
                    view,
                    Some(&SignalAnalyzerData {
                        sample_rate: 48_000,
                        fft_size: 4096,
                        spectrum_db: &spectrum,
                        lufs_momentary: None,
                        lufs_short_term: None,
                        lufs_integrated: None,
                        peak_dbfs: Some(-6.0),
                        rms_dbfs: Some(-9.0),
                        correlation: None,
                        phase,
                        ..Default::default()
                    }),
                    300.0,
                    "Play to analyze",
                );
            });
        },
    )
}

#[test]
fn phase_geometry_uses_mid_side_axes_and_rejects_nonfinite_samples() {
    for (samples, vertical) in [
        (vec![[0.2, 0.2], [0.7, 0.7], [-0.3, -0.3]], true),
        (vec![[0.2, -0.2], [0.7, -0.7], [-0.3, 0.3]], false),
    ] {
        let output = render(SignalAnalyzerView::Phase, &samples);
        let mesh = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.len() == samples.len() * 4 => Some(mesh),
                _ => None,
            })
            .unwrap();
        let centers: Vec<_> = mesh
            .vertices
            .chunks_exact(4)
            .map(|quad| (quad[0].pos + quad[2].pos.to_vec2()) * 0.5)
            .collect();
        let axis = if vertical {
            centers.iter().map(|p| p.x).collect::<Vec<_>>()
        } else {
            centers.iter().map(|p| p.y).collect()
        };
        assert!(
            axis.iter()
                .all(|position| (*position - axis[0]).abs() < 0.01)
        );
        assert!(mesh.is_valid());
    }
    let output = render(SignalAnalyzerView::Phase, &[[f32::NAN, 0.3]]);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| !v.pos.is_finite()))));
}

#[test]
fn levels_show_calibrated_stereo_and_estimates_without_overlapping_at_small_widths() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for width in [360.0, 640.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_theme(theme);
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 430.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        daw_ui::signal_analyzer(
                            ui,
                            SignalAnalyzerView::Levels,
                            Some(&SignalAnalyzerData {
                                channel_peak_dbfs: [Some(-1.4), Some(-7.4)],
                                channel_rms_300ms_dbfs: [Some(-6.0), Some(-12.0)],
                                peak_dbfs: Some(-1.4),
                                sample_peak_hold_dbfs: Some(-1.0),
                                true_peak_dbfs: Some(1.6),
                                true_peak_hold_dbfs: Some(2.1),
                                rms_300ms_dbfs: Some(-9.0),
                                true_peak_oversampling: 4,
                                overload_samples: [u64::MAX, 172_800_000],
                                ..Default::default()
                            }),
                            280.0,
                            "Waiting",
                        );
                    });
                },
            );
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            for caption in [
                "Sample peak",
                "Peak hold",
                "True peak",
                "TP hold",
                "RMS 300 ms",
                "Overs · L/R",
            ] {
                let label = texts
                    .iter()
                    .find(|text| text.galley.text() == caption)
                    .unwrap();
                let value = texts
                    .iter()
                    .find(|text| (text.pos.y - label.pos.y).abs() < 0.1 && text.pos.x > label.pos.x)
                    .unwrap();
                assert!(
                    label.pos.x + label.galley.size().x + 3.0 <= value.pos.x,
                    "width={width}: {} overlaps {}",
                    caption,
                    value.galley.text()
                );
            }
            assert!(texts.iter().any(|text| text.galley.text() == "1.6 dBTP"));
            assert!(texts.iter().all(|text| text.pos.is_finite()));
        }
    }
}
