use daw_ui::{clip_surface, mixer_strip, paint_selection_glow, theme, tokens};
use egui::{Color32, Context, Pos2, RawInput, Rect, Shape, Vec2};

fn meshes(output: &egui::FullOutput) -> Vec<&egui::Mesh> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Mesh(mesh) => Some(mesh.as_ref()),
            _ => None,
        })
        .collect()
}

#[test]
fn selection_halo_has_constant_geometry_transparent_edges_and_no_texture() {
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        for (width, height, density) in [(4.0, 1.0, 1.0), (250.0, 64.0, 2.0), (180.0, 900.0, 1.0)] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let rect = Rect::from_min_size(egui::pos2(30.0, 30.0), Vec2::new(width, height));
            let output = ctx.run(RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                paint_selection_glow(
                    &painter,
                    rect,
                    tokens::RADIUS,
                    Color32::from_rgb(80, 170, 220),
                );
            });
            let meshes = meshes(&output);
            assert_eq!(meshes.len(), 1);
            let mesh = meshes[0];
            assert!(mesh.is_valid());
            assert_eq!(mesh.texture_id, egui::TextureId::default());
            assert_eq!(mesh.vertices.len(), 128);
            assert_eq!(mesh.indices.len(), 576);
            assert_eq!(
                mesh.vertices
                    .iter()
                    .filter(|vertex| vertex.color.a() == 0)
                    .count(),
                64
            );
            let bounds = mesh.calc_bounds();
            let expected = rect.expand(tokens::SELECTION_GLOW_EXTENT);
            assert!((bounds.min - expected.min).length() < 0.01);
            assert!((bounds.max - expected.max).length() < 0.01);
            assert!(mesh.vertices.iter().all(|vertex| vertex.pos.is_finite()));
            let peak = mesh
                .vertices
                .iter()
                .map(|vertex| vertex.color.a())
                .max()
                .unwrap();
            assert_eq!(
                peak,
                if appearance == theme::Appearance::Dark {
                    64
                } else {
                    48
                }
            );
        }
    }
}

#[test]
fn hidden_or_invalid_halos_submit_no_geometry_and_visible_halos_keep_parent_clip() {
    let ctx = Context::default();
    let clip = Rect::from_min_max(Pos2::ZERO, egui::pos2(200.0, 200.0));
    let output = ctx.run(RawInput::default(), |ctx| {
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(clip);
        for rect in [
            Rect::NOTHING,
            Rect::EVERYTHING,
            Rect::from_min_size(egui::pos2(400.0, 400.0), Vec2::splat(20.0)),
            Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::ZERO),
        ] {
            paint_selection_glow(&painter, rect, tokens::RADIUS, Color32::RED);
        }
        assert!(painter.clip_rect() == clip);
        paint_selection_glow(
            &painter,
            Rect::from_min_max(egui::pos2(180.0, 40.0), egui::pos2(240.0, 80.0)),
            tokens::RADIUS,
            Color32::RED,
        );
    });
    assert_eq!(meshes(&output).len(), 1);
    assert_eq!(output.shapes.len(), 1);
    assert_eq!(output.shapes[0].clip_rect, clip);
}

#[test]
fn clip_and_channel_selection_add_halos_without_changing_geometry_or_controls() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let clip = Rect::from_min_size(egui::pos2(350.0, 20.0), Vec2::new(220.0, 64.0));
    let mut previous = None;
    for selected in [false, true, false] {
        let mut bounds = None;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let surface = clip_surface(ui.painter(), clip, Color32::LIGHT_BLUE, selected);
                    let strip = mixer_strip(
                        ui,
                        "channel",
                        Vec2::new(180.0, 400.0),
                        Some(Color32::LIGHT_BLUE),
                        selected,
                        |ui| daw_ui::button(ui, "Effects").rect,
                    );
                    bounds = Some((
                        surface.title,
                        surface.content,
                        strip.inner,
                        strip.response.rect,
                    ));
                });
            },
        );
        let halos = meshes(&output)
            .into_iter()
            .filter(|mesh| mesh.vertices.len() == 128 && mesh.indices.len() == 576)
            .count();
        assert_eq!(halos, if selected { 2 } else { 0 });
        if let Some(previous) = previous {
            assert_eq!(bounds, Some(previous));
        }
        previous = bounds;
    }
}

#[test]
fn tool_and_page_selection_outline_the_full_target_without_moving_labels() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            for role in ["tool", "choice", "tab"] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                let mut original = None;
                for (selected, enabled) in
                    [(false, true), (true, true), (true, false), (false, true)]
                {
                    let mut target = Rect::NOTHING;
                    let output = ctx.run(RawInput::default(), |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    ui.add(match role {
                                        "tool" => daw_ui::IconButton::tool(selected, "Duplicate"),
                                        "choice" => daw_ui::IconButton::tool_group_choice(
                                            "Duplicate",
                                            daw_ui::Icon::Duplicate,
                                            selected.then_some("Duplicate"),
                                        ),
                                        _ => daw_ui::IconButton::tab(selected, "Duplicate"),
                                    })
                                })
                                .inner;
                            target = response.rect;
                            assert_eq!(response.enabled(), enabled);
                        });
                    });
                    let label = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text) if text.galley.text() == "Duplicate" => {
                                Some(text.galley.rect.translate(text.pos.to_vec2()))
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(*original.get_or_insert((target, label)), (target, label));
                    assert!(target.shrink(tokens::SPACE_XS).contains_rect(label));
                    let halos: Vec<_> = meshes(&output)
                        .into_iter()
                        .filter(|mesh| mesh.vertices.len() == 128 && mesh.indices.len() == 576)
                        .collect();
                    assert_eq!(halos.len(), usize::from(selected && enabled));
                    if let Some(halo) = halos.first() {
                        let expected = target.expand(tokens::SELECTION_GLOW_EXTENT);
                        assert!((halo.calc_bounds().min - expected.min).length() < 0.1);
                        assert!((halo.calc_bounds().max - expected.max).length() < 0.1);
                    }
                    let borders: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            Shape::Rect(shape)
                                if shape.rect == target
                                    && shape.stroke.width == tokens::FOCUS_WIDTH =>
                            {
                                Some(shape)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(borders.len(), usize::from(selected));
                    if let Some(border) = borders.first() {
                        assert_eq!(border.stroke_kind, egui::StrokeKind::Inside);
                        assert_eq!(border.fill, Color32::TRANSPARENT);
                        if enabled {
                            assert_eq!(border.stroke.color, theme::palette(&ctx).accent);
                        }
                    }
                }
            }
        }
    }
}
