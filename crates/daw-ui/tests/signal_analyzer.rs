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
