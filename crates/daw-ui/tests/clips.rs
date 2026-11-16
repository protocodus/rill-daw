use daw_ui::{Icon, clip_label, clip_title, text::TextRole, theme};
use egui::{Context, RawInput, Rect, Shape};

#[test]
fn collapsed_intervals_keep_gaps_and_mark_tiny_clips_and_base_points() {
    for scale in [1.0, 1.25, 2.0] {
        let ctx = Context::default();
        ctx.set_pixels_per_point(scale);
        let lane = Rect::from_min_max(egui::pos2(20.0, 30.0), egui::pos2(220.0, 50.0));
        let visible = Rect::from_min_max(egui::pos2(30.0, 30.0), egui::pos2(200.0, 48.0));
        let color = egui::Color32::from_rgb(70, 120, 180);
        let mut marks = Vec::new();
        let output = ctx.run(RawInput::default(), |ctx| {
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(visible);
            let view = daw_ui::TimelineViewport {
                start_beat: 2.0,
                pixels_per_beat: 20.0,
                ..Default::default()
            };
            let span =
                |start, end| daw_ui::collapsed_track_span(&painter, lane, view, start, end, color);
            assert_eq!(span(0.0, 1.0), None);
            assert_eq!(span(12.0, 13.0), None);
            assert_eq!(span(8.0, 7.0), None);
            marks = [(1.0, 4.0), (6.0, 6.001), (8.0, 8.0), (10.0, 15.0)]
                .map(|(start, end)| span(start, end).unwrap())
                .to_vec();
        });
        assert_eq!(marks[0].left(), visible.left());
        assert_eq!(marks[0].right(), 60.0);
        assert_eq!(marks[3].left(), 180.0);
        assert_eq!(marks[3].right(), visible.right());
        for (mark, expected_x) in [(marks[1], 100.0), (marks[2], 140.0)] {
            assert!((mark.center().x - expected_x).abs() <= 1.0 / scale);
            assert!((mark.width() - daw_ui::tokens::SPACE_XS).abs() <= 1.0 / scale);
        }
        let actual: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Rect(shape) if shape.fill == color => Some(shape.rect),
                _ => None,
            })
            .collect();
        assert_eq!(actual, marks);
        for rect in &actual {
            assert!(visible.contains_rect(*rect));
            assert!((rect.center().y - lane.center().y).abs() <= 1.0 / scale);
            assert!((rect.height() - daw_ui::tokens::SPACE_XS).abs() <= 1.0 / scale);
        }
        assert!(
            actual
                .windows(2)
                .all(|pair| pair[0].right() < pair[1].left())
        );
    }
}

#[test]
fn unnamed_clips_recover_the_title_space_and_keep_a_thin_restorable_border() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for scale in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(scale);
            let mut empty_region = Rect::NOTHING;
            let output = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for height in [32.0, 48.0, 128.0] {
                        let rect =
                            Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(320.0, height));
                        let named = daw_ui::clip_surface_named(
                            ui.painter(),
                            rect,
                            egui::Color32::LIGHT_BLUE,
                            false,
                            "Phrase",
                        );
                        for name in ["", " \t "] {
                            let blank = daw_ui::clip_surface_named(
                                ui.painter(),
                                rect,
                                egui::Color32::LIGHT_BLUE,
                                true,
                                name,
                            );
                            assert!(blank.title.height() <= 3.5);
                            assert!(blank.content.height() >= named.content.height() + 16.0);
                            assert!(rect.contains_rect(blank.name_hit));
                            assert!(
                                !blank.name_hit.contains(rect.center()),
                                "Ordinary body double-click stays outside rename area"
                            );
                            assert!(
                                blank.name_hit.width() >= 280.0
                                    && blank.name_hit.height() == blank.title.height()
                            );
                            let title = clip_title(
                                ui.painter(),
                                blank.title,
                                TextRole::Label.font(),
                                blank.title_color,
                                name,
                                true,
                            );
                            assert!(
                                title.icon.is_none(),
                                "A hidden name has no cramped chain icon or fallback text"
                            );
                            empty_region = rect;
                        }
                    }
                });
            });
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, Shape::Text(text) if empty_region.intersects(text.galley.rect.translate(text.pos.to_vec2())))),
                "Hidden names must not paint a generic replacement title");
        }
    }
}

#[test]
fn linked_clone_and_independent_copy_actions_have_distinct_vector_icons() {
    assert_eq!(Icon::for_label("Ghost copy"), Some(Icon::Link));
    assert_eq!(Icon::for_label("Make independent"), Some(Icon::Unlink));
    assert_eq!(Icon::for_label("Duplicate"), Some(Icon::Duplicate));
}

#[test]
fn clip_titles_reserve_chain_space_and_elide_names_in_both_themes() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut color = egui::Color32::TRANSPARENT;
        let mut layouts = Vec::new();
        let output = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                color = theme::palette(ctx).text;
                for (index, width) in [150.0, 50.0, 16.0, 8.0, 0.0].into_iter().enumerate() {
                    let rect = Rect::from_min_size(
                        egui::pos2(20.0, 20.0 + index as f32 * 30.0),
                        egui::vec2(width, 24.0),
                    );
                    let layout = clip_title(
                        ui.painter(),
                        rect,
                        TextRole::Label.font(),
                        color,
                        "Žluťoučký 🎹 東京 — a long linked clip name",
                        true,
                    );
                    assert!(rect.contains_rect(layout.text));
                    if let Some(icon) = layout.icon {
                        assert!(rect.contains_rect(icon));
                        assert!(icon.right() <= layout.text.left());
                    }
                    layouts.push((rect, layout));
                }
            });
        });
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some((shape.clip_rect, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts.len(),
            2,
            "icon-only widths must not paint cut-off text"
        );
        for ((clip, text), (rect, layout)) in texts.iter().zip(&layouts) {
            assert!(text.galley.elided);
            assert_eq!(text.galley.rows.len(), 1);
            assert!(text.pos.x >= layout.text.left());
            assert!(clip.left() >= layout.text.left());
            assert!(rect.contains_rect(*clip));
            assert_eq!(text.fallback_color, color);
        }
        let icons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Path(path) => Some((shape.clip_rect, path)),
                _ => None,
            })
            .collect();
        assert_eq!(
            icons.len(),
            12,
            "four nonempty chain icons have three paths each"
        );
        for (clip, path) in icons {
            assert_eq!(path.stroke.color, egui::epaint::ColorMode::Solid(color));
            assert!(path.points.iter().all(|point| clip.contains(*point)));
        }
    }
}

#[test]
fn standalone_clip_names_keep_full_width_and_painting_respects_parent_clip() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rect = Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(150.0, 24.0));
    let parent_clip = rect.shrink2(egui::vec2(0.0, 6.0));
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let layout = clip_title(
                &ui.painter().with_clip_rect(parent_clip),
                rect,
                TextRole::Label.font(),
                theme::palette(ctx).text,
                "Independent clip",
                false,
            );
            assert!(layout.icon.is_none());
            assert_eq!(layout.text, rect);
        });
    });
    let texts: Vec<_> = output
        .shapes
        .iter()
        .filter(|shape| matches!(shape.shape, Shape::Text(_)))
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].clip_rect, parent_clip);
    assert!(
        !output
            .shapes
            .iter()
            .any(|shape| matches!(shape.shape, Shape::Path(_)))
    );
}

#[test]
fn properties_clip_label_preserves_unicode_name_and_clear_icon_spacing() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(156.0, 200.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(140.0);
            let linked = clip_label(ui, "Tokyo 東京 🎹 linked melody", true);
            let standalone = clip_label(ui, "Independent clip", false);
            assert!(linked.rect.bottom() < standalone.rect.top());
            assert!(linked.rect.right() <= ui.max_rect().right());
        });
    });
    let mut icon_right = f32::NEG_INFINITY;
    let mut names = Vec::new();
    for shape in &output.shapes {
        match &shape.shape {
            Shape::Path(path) => {
                for point in &path.points {
                    icon_right = icon_right.max(point.x);
                }
            }
            Shape::Text(text) => names.push(text),
            _ => {}
        }
    }
    assert_eq!(names.len(), 2);
    assert_eq!(names[0].galley.text(), "Tokyo 東京 🎹 linked melody");
    assert!(names[0].galley.elided);
    assert!(icon_right + 4.0 <= names[0].pos.x);
    assert_eq!(names[1].galley.text(), "Independent clip");
}
