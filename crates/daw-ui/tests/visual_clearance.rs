use daw_ui::{theme, tokens};
use egui::{Context, Pos2, RawInput, Rect, Shape, vec2};

#[test]
fn envelope_stage_labels_clear_the_curve_and_its_endpoint_handles() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            ctx.set_pixels_per_point(density);
            theme::apply(&ctx);
            for size in [vec2(233.0, 96.0), vec2(377.0, 144.0)] {
                let mut bounds = Rect::NOTHING;
                let output = ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            bounds = daw_ui::envelope_editor(ui, &Default::default(), size, &[])
                                .response
                                .rect;
                        });
                    },
                );
                let handle_bottom = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        Shape::Circle(circle) => Some(circle.center.y + circle.radius),
                        _ => None,
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                for label in ["Attack", "Decay", "Sustain", "Release"] {
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text) if text.galley.text() == label => Some(text),
                            _ => None,
                        })
                        .unwrap();
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    assert!(
                        rect.top() >= handle_bottom + 3.0,
                        "{label} overlaps the envelope: {rect:?}, handles end at {handle_bottom}"
                    );
                    assert!(bounds.contains_rect(rect));
                }
            }
        }
    }
}

#[test]
fn fallback_device_icon_stays_readable_in_short_preview_bands() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            ctx.set_pixels_per_point(density);
            theme::apply(&ctx);
            for size in [vec2(234.0, 34.0), vec2(34.0, 34.0), vec2(234.0, 144.0)] {
                let mut preview = Rect::NOTHING;
                let output = ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            preview =
                                daw_ui::device_preview(ui, None, daw_ui::Icon::Instrument, size)
                                    .rect;
                        });
                    },
                );
                let muted = theme::palette(&ctx).muted;
                let glyph = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        Shape::Rect(rect)
                            if rect.stroke.color == muted && rect.stroke.width > 0.0 =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .expect("The instrument icon outline is painted");
                assert!(
                    glyph.width() >= 17.0 && glyph.height() >= 14.0,
                    "Fallback glyph is too small: {glyph:?}"
                );
                assert!(preview.shrink(tokens::SPACE_XS).contains_rect(glyph));
                assert!(glyph.center().distance(preview.center()) < 0.1);
            }
        }
    }
}
