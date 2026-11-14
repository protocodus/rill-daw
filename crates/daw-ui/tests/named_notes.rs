use daw_ui::{self as ui, theme, tokens};
use egui::{Context, Pos2, RawInput, Rect, Shape, Vec2};

#[test]
fn named_instrument_rows_are_uniform_readable_and_ellipsized_in_both_themes() {
    let label = "Bowed glass with a long resonant release 東京";
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        let context = Context::default();
        theme::apply(&context);
        theme::set_appearance(&context, appearance);
        for active in [false, true] {
            let bounds = Rect::from_min_size(
                Pos2::new(20.0, 20.0),
                Vec2::new(tokens::NAMED_NOTE_KEYS_WIDTH, 18.0),
            );
            let output = context.run(RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                ui::named_note_key(&painter, bounds, label, active);
            });
            let surfaces: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Rect(rect) => Some(rect),
                    _ => None,
                })
                .collect();
            assert_eq!(surfaces.len(), 1, "no inset black-key geometry");
            assert_eq!(surfaces[0].rect, bounds);
            let colors = theme::palette(&context);
            if active {
                assert_ne!(surfaces[0].fill, colors.raised, "audition is visible");
            } else {
                assert_eq!(surfaces[0].fill, colors.raised);
            }
            let (clip, text) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    Shape::Text(text) => Some((shape.clip_rect, text)),
                    _ => None,
                })
                .expect("named row retains a readable label at minimum height");
            assert_eq!(text.galley.text(), label, "keep complete Unicode source");
            assert!(text.galley.elided);
            assert_eq!(text.galley.rows.len(), 1);
            assert!(bounds.contains_rect(clip));
            assert!(clip.contains_rect(Rect::from_min_size(text.pos, text.galley.size())));
            assert_eq!(text.fallback_color, theme::contrast_text(surfaces[0].fill));
        }
    }
}

#[test]
fn scale_guidance_is_a_blue_mark_and_velocity_never_uses_warning_colors() {
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        let context = Context::default();
        theme::apply(&context);
        theme::set_appearance(&context, appearance);
        let note = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(48.0, 18.0));
        let output = context.run(RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            ui::paint_outside_scale_marker(&painter, note).unwrap();
        });
        let colors = theme::palette(&context);
        let marker = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Rect(rect) => Some(rect),
                _ => None,
            })
            .unwrap();
        assert!(note.contains_rect(marker.rect));
        assert_eq!(marker.fill, colors.action);
        assert_ne!(marker.fill, colors.warning);
        assert!(marker.fill.b() > marker.fill.r());

        for selected in [false, true] {
            let paint_velocity = |outside| {
                context
                    .run(RawInput::default(), |ctx| {
                        let painter = ctx.layer_painter(egui::LayerId::background());
                        ui::piano_velocity_marker(&painter, note, 96, selected, outside);
                        ui::piano_velocity_sibling_marker(&painter, note, 64, outside);
                    })
                    .shapes
            };
            assert_eq!(paint_velocity(false), paint_velocity(true));
        }
    }
}
