use daw_ui::{
    text::{self, TextRole},
    theme, tokens,
};
use egui::{Context, FontId, RawInput, Rect, Vec2};

#[test]
fn timeline_bar_and_clock_labels_have_separate_readable_rows() {
    let context = Context::default();
    theme::apply(&context);
    let output = context.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(600.0, tokens::TIMELINE_RULER_HEIGHT),
                egui::Sense::hover(),
            );
            daw_ui::paint_timeline_ruler(
                ui.painter(),
                rect,
                daw_ui::TimelineViewport::default(),
                120.0,
                4,
            );
        });
    });
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                ))
            } else {
                None
            }
        })
        .collect();
    let bar = labels.iter().find(|(text, _)| text == "1").unwrap().1;
    let clock = labels.iter().find(|(text, _)| text == "0:00.00").unwrap().1;
    assert!(
        bar.bottom() + 2.0 <= clock.top(),
        "bar {bar:?} overlaps clock {clock:?}"
    );
}

#[test]
fn timeline_ruler_omits_partial_edge_labels_after_panning_and_in_narrow_views() {
    for density in [1.0, 2.0] {
        let context = Context::default();
        theme::apply(&context);
        context.set_pixels_per_point(density);
        let input = || RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(800.0, 300.0),
            )),
            ..Default::default()
        };
        let _ = context.run(input(), |_| {});
        for (start, width, present, absent) in [
            (
                32.0,
                393.0,
                &["9", "10", "0:16.00", "0:18.00"][..],
                &["11", "0:20.00"][..],
            ),
            (
                32.2,
                389.0,
                &["10", "0:18.00"][..],
                &["9", "11", "0:16.00", "0:20.00"][..],
            ),
            (0.0, 44.0, &["1"][..], &["0:00.00"][..]),
            (0.2, 44.0, &[][..], &["1", "0:00.00"][..]),
        ] {
            let rect = Rect::from_min_size(
                egui::pos2(30.0, 30.0),
                Vec2::new(width, tokens::TIMELINE_RULER_HEIGHT),
            );
            let view = daw_ui::TimelineViewport {
                start_beat: start,
                ..Default::default()
            };
            let output = context.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    daw_ui::paint_timeline_ruler(ui.painter(), rect, view, 120.0, 4);
                });
            });
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => {
                        let painted = text
                            .galley
                            .rect
                            .union(text.galley.mesh_bounds)
                            .translate(text.pos.to_vec2());
                        assert!(
                            rect.intersect(shape.clip_rect).contains_rect(painted),
                            "partial {} at {density}x, start {start}, width {width}: {painted:?}",
                            text.galley.text()
                        );
                        if text.galley.text() == "10" {
                            assert!((text.pos.x - (view.x_at(rect, 36.0) + 4.0)).abs() < 0.01);
                        }
                        Some(text.galley.text())
                    }
                    _ => None,
                })
                .collect();
            for label in present {
                assert!(
                    labels.contains(label),
                    "missing whole label {label}: {labels:?}"
                );
            }
            for label in absent {
                assert!(!labels.contains(label), "partial label {label}: {labels:?}");
            }
        }
    }
}

#[test]
fn embedded_fonts_cover_names_and_emoji_in_every_text_role_and_numeric_digits_align() {
    let context = Context::default();
    theme::apply(&context);
    let _ = context.run(RawInput::default(), |ctx| {
        ctx.fonts_mut(|fonts| {
            for role in [
                TextRole::Title,
                TextRole::Subtitle,
                TextRole::Body,
                TextRole::Label,
                TextRole::Note,
            ] {
                for character in
                    "Žluťoučký příliš Ελληνικά Україна 東京 音楽 中文 한국어 🎹 🎸 😀 ♯ ♭".chars()
                {
                    assert!(
                        fonts.has_glyph(&role.font(), character),
                        "{role:?} missing {character}"
                    );
                }
            }
            let mono = FontId::monospace(tokens::FONT_BODY);
            let width = fonts.glyph_width(&mono, '0');
            for digit in '1'..='9' {
                assert_eq!(fonts.glyph_width(&mono, digit), width);
            }
        });
    });
}

#[test]
fn all_text_roles_wrap_with_full_height_at_standard_and_retina_density() {
    for density in [1.0, 2.0] {
        let context = Context::default();
        theme::apply(&context);
        context.set_pixels_per_point(density);
        // egui applies a density change between passes; measure the settled
        // viewport just as native and headless integrations do after resizing.
        let _ = context.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(176.0, 1200.0),
                )),
                ..Default::default()
            },
            |_| {},
        );
        let mut responses = Vec::new();
        let output = context.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(176.0, 1200.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_max_width(160.0);
                    for role in [
                        TextRole::Title,
                        TextRole::Subtitle,
                        TextRole::Body,
                        TextRole::Label,
                        TextRole::Note,
                    ] {
                        let value =
                            "Žluťoučký 🎹 東京 / a long instrument name that wraps completely";
                        let response = text::block(ui, role, value);
                        assert!(
                            response.rect.width() <= 160.5,
                            "{role:?} density{density} available{} max{:?}: {:?}",
                            ui.available_width(),
                            ui.max_rect(),
                            response.rect
                        );
                        assert!(response.rect.height() >= role.font().size * 2.0);
                        responses.push(response.rect);
                    }
                });
            },
        );
        assert!(
            responses
                .windows(2)
                .all(|rows| rows[0].bottom() < rows[1].top())
        );
        let text_shapes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((shape.clip_rect, text)),
                _ => None,
            })
            .collect();
        assert_eq!(text_shapes.len(), 5);
        for (clip, text) in text_shapes {
            assert!(text.galley.rows.len() >= 2);
            assert!(!text.galley.elided);
            let painted = Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                clip.contains_rect(painted),
                "text was clipped at {density}x: {painted:?} {clip:?}"
            );
        }
    }
}

#[test]
fn fixed_catalog_names_use_ellipsis_and_dense_piano_rows_hide_labels() {
    let context = Context::default();
    theme::apply(&context);
    let output = context.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::new(120.0, 30.0));
            text::paint_row(
                ui.painter(),
                rect,
                TextRole::Label,
                "Long instrument name 🎹 東京",
            );
            daw_ui::piano_key(
                ui.painter(),
                Rect::from_min_size(egui::pos2(20.0, 60.0), Vec2::new(60.0, 10.0)),
                "C4",
                false,
                1,
                false,
            );
            // A full-size key can still be only partly visible at a scrolled
            // viewport edge. Do not paint a cut-off fragment of its pitch name.
            daw_ui::piano_key(
                &ui.painter().with_clip_rect(Rect::from_min_size(
                    egui::pos2(20.0, 110.0),
                    Vec2::new(60.0, 20.0),
                )),
                Rect::from_min_size(egui::pos2(20.0, 100.0), Vec2::new(60.0, 18.0)),
                "C5",
                false,
                1,
                false,
            );
        });
    });
    let rows: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "tiny and partly clipped piano labels must be hidden"
    );
    assert!(rows[0].galley.elided);
    assert_eq!(rows[0].galley.rows.len(), 1);
    assert!(rows[0].galley.size().x <= 120.0);
}
