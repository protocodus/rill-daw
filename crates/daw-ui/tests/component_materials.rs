use daw_ui::{ComponentMaterial, component_shape, material_overlay_shape, theme};
use egui::{Color32, Context, RawInput, Rect, Shape, Stroke};

fn input() -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    }
}

#[test]
fn rounded_material_reuses_its_texture_across_control_sizes_and_frames() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rects = [
        Rect::from_min_size(egui::pos2(21.0, 21.0), egui::vec2(55.0, 34.0)),
        Rect::from_min_size(egui::pos2(89.0, 21.0), egui::vec2(233.0, 55.0)),
    ];
    let mut last_texture = None;
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        theme::set_appearance(&ctx, appearance);
        for frame in 0..2 {
            let output = ctx.run(input(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                for rect in rects {
                    painter.add(material_overlay_shape(&painter, rect, 8));
                }
            });
            let texture_ids: Vec<_> = output
                .shapes
                .iter()
                .map(|shape| {
                    let Shape::Rect(shape) = &shape.shape else {
                        panic!("material must use a native rounded rectangle");
                    };
                    assert_eq!(shape.corner_radius, egui::CornerRadius::same(8));
                    assert_eq!(shape.round_to_pixels, Some(true));
                    shape.fill_texture_id()
                })
                .collect();
            assert_eq!(texture_ids.len(), rects.len());
            assert_eq!(texture_ids[0], texture_ids[1]);
            let texture = texture_ids[0];
            let upload = output
                .textures_delta
                .set
                .iter()
                .find(|(id, _)| *id == texture);
            if frame == 0 {
                assert_ne!(
                    Some(texture),
                    last_texture,
                    "appearances retain separate tints"
                );
                let image = &upload
                    .expect("first use uploads the tiny shared texture")
                    .1
                    .image;
                assert_eq!(image.size(), [55, 34]);
                let egui::ImageData::Color(image) = image;
                let max_alpha = if appearance == theme::Appearance::Dark {
                    13
                } else {
                    8
                };
                assert!(image.pixels.iter().all(|color| color.a() <= max_alpha));
                assert!(image.pixels.iter().any(|color| color.a() > 0));
                last_texture = Some(texture);
            } else {
                assert!(
                    upload.is_none(),
                    "steady frames must reuse the retained texture"
                );
            }
            let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(!primitives.is_empty());
            for primitive in primitives {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    panic!("materials must not need paint callbacks");
                };
                assert!(mesh.is_valid());
                assert_eq!(mesh.texture_id, texture);
                assert!(mesh.vertices.iter().all(|vertex| vertex.pos.is_finite()));
                assert!(
                    mesh.vertices.iter().any(|vertex| vertex.color.a() == 0),
                    "native edge feathering should retain transparent AA vertices"
                );
            }
        }
    }
}

#[test]
fn material_emphasis_is_bounded_and_keeps_the_callers_clip() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rect = Rect::from_min_size(egui::pos2(21.0, 34.0), egui::vec2(144.0, 34.0));
    let clip = rect.shrink(3.0);
    for material in [
        ComponentMaterial::default(),
        ComponentMaterial {
            elevated: true,
            active: true,
        },
    ] {
        let output = ctx.run(input(), |ctx| {
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(clip);
            painter.add(component_shape(
                &painter,
                rect,
                5,
                theme::palette(ctx).control_fill,
                Stroke::new(1.0_f32, theme::palette(ctx).line),
                material,
            ));
        });
        assert_eq!(output.shapes.len(), 1);
        assert_eq!(output.shapes[0].clip_rect, clip);
        let Shape::Vec(shapes) = &output.shapes[0].shape else {
            panic!("component background should contain native material layers");
        };
        for shape in shapes {
            let Shape::Rect(shape) = shape else {
                panic!("component layers should use native rounded rectangles");
            };
            assert!(rect.expand(8.0).contains_rect(shape.visual_bounding_rect()));
            if shape.blur_width == 0.0 {
                assert_eq!(shape.rect, rect, "material must preserve component bounds");
            } else {
                assert!(material.active || material.elevated);
            }
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(
            primitives
                .iter()
                .all(|primitive| primitive.clip_rect == clip)
        );
    }
}

#[test]
fn unavailable_materials_skip_texture_allocation() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let output = ctx.run(input(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        for rect in [
            Rect::NOTHING,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(f32::INFINITY, 55.0)),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(55.0, f32::NAN)),
            Rect::from_min_size(egui::pos2(0.0, 2000.0), egui::vec2(55.0, 34.0)),
        ] {
            assert!(matches!(
                material_overlay_shape(&painter, rect, 5),
                Shape::Noop
            ));
            assert!(matches!(
                component_shape(
                    &painter,
                    rect,
                    5,
                    Color32::WHITE,
                    Stroke::NONE,
                    ComponentMaterial::default(),
                ),
                Shape::Noop
            ));
        }
        let mut hidden = painter.clone();
        hidden.set_invisible();
        let rect = Rect::from_min_size(egui::pos2(21.0, 21.0), egui::vec2(55.0, 34.0));
        assert!(matches!(
            material_overlay_shape(&hidden, rect, 5),
            Shape::Noop
        ));
        // Quiet, outlined controls should keep their transparent interior and
        // avoid uploading a material they do not display.
        let outline = component_shape(
            &painter,
            rect,
            5,
            Color32::TRANSPARENT,
            Stroke::new(1.0_f32, Color32::GRAY),
            ComponentMaterial::default(),
        );
        let Shape::Vec(shapes) = outline else {
            panic!("outlined control should contain its native border");
        };
        assert_eq!(shapes.len(), 1);
        let Shape::Rect(border) = &shapes[0] else {
            panic!("outlined control should use a native rectangle");
        };
        assert_eq!(border.fill, Color32::TRANSPARENT);
        assert!(border.brush.is_none());
    });
    assert!(
        output
            .textures_delta
            .set
            .iter()
            .all(|(_, image)| image.image.size() != [55, 34])
    );
    assert!(output.shapes.is_empty());
}
