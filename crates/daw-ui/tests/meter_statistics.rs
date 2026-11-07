use egui::{Context, Pos2, RawInput, Rect, Vec2};

#[test]
fn compact_rms_is_read_only_calibrated_and_finite_at_display_scales() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for scale in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            ctx.set_pixels_per_point(scale);
            daw_ui::theme::apply(&ctx);
            ctx.set_theme(theme);
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(180.0, 150.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let stats = daw_ui::channel_meter_statistics(ui, 144.0, [0.1, 0.01]);
                        assert_eq!(stats.rect.height(), daw_ui::tokens::COMPACT_HEIGHT);
                        assert_eq!(stats.sense, egui::Sense::hover());
                        daw_ui::channel_meter_statistics(ui, 144.0, [f32::NAN, f32::INFINITY]);
                        daw_ui::channel_meter_statistics(ui, 48.0, [0.1, 0.01]);
                        daw_ui::channel_meter_statistics(ui, 48.0, [0.0; 2]);
                    });
                },
            );
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect();
            assert!(labels.contains(&"RMS -20.0"), "{labels:?}");
            assert!(labels.contains(&"RMS −∞"), "{labels:?}");
            assert!(
                labels.contains(&"-20.0"),
                "Folded strips keep the value legible: {labels:?}"
            );
            assert!(labels.contains(&"−∞"), "{labels:?}");
        }
    }
}
