use daw_ui::{self as controls, theme};
use egui::{Context, FullOutput, RawInput, Rect};

fn frame(ctx: &Context, width: f32, strength: &mut f64, swing: &mut f64) -> FullOutput {
    ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 400.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                controls::parameter_row_wrapped(ui, |ui| {
                    ui.label("Quantize preview");
                    ui.label("Selected notes");
                    controls::select("quantize_grid", "0.25 beat")
                        .width(90.0)
                        .show_ui(ui, |_| {});
                    ui.add(
                        controls::slider(strength, 0.0..=100.0)
                            .text("Strength")
                            .suffix("%"),
                    );
                    ui.add(
                        controls::slider(swing, 50.0..=75.0)
                            .text("Swing")
                            .suffix("%"),
                    );
                    controls::button(ui, "Apply quantize");
                });
            });
        },
    )
}

fn assert_visible(output: &FullOutput, width: f32) {
    for caption in ["Strength", "Swing", "Apply quantize"] {
        let shape = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == caption => Some((shape, text)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {caption}"));
        let bounds = shape.1.galley.rect.translate(shape.1.pos.to_vec2());
        assert!(
            shape.0.clip_rect.contains_rect(bounds),
            "{caption} was clipped at {width}px: {bounds:?}"
        );
        assert!(bounds.right() <= width && bounds.left() >= 0.0);
    }
}

#[test]
fn wrapped_sliders_keep_their_value_and_label_together_and_settle_in_one_pass() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let mut strength = 100.0;
            let mut swing = 50.0;
            for width in [1440.0, 800.0, 1440.0, 800.0] {
                let output = frame(&ctx, width, &mut strength, &mut swing);
                assert_visible(&output, width);
                let settled = frame(&ctx, width, &mut strength, &mut swing);
                assert_visible(&settled, width);
                assert_eq!(settled.platform_output.num_completed_passes, 1);
                assert_eq!((strength, swing), (100.0, 50.0));
            }
        }
    }
}
