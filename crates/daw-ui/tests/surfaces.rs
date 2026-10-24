use daw_ui::{paint_ambient, soft_surface, theme};
use egui::{Color32, Context, Event, Frame, Modifiers, RawInput, Rect, Shape};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        events,
        ..Default::default()
    }
}

#[test]
fn ambient_wash_is_clipped_transparent_at_edges_and_has_bounded_geometry() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rect = Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(8000.0, 6000.0));
    let parent_clip = Rect::from_min_max(egui::pos2(10.0, 40.0), egui::pos2(750.0, 650.0));
    let output = ctx.run(input(vec![]), |ctx| {
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(parent_clip);
        paint_ambient(&painter, rect);
    });
    assert_eq!(output.shapes.len(), 1);
    let shape = &output.shapes[0];
    assert_eq!(shape.clip_rect, rect.intersect(parent_clip));
    let Shape::Mesh(mesh) = &shape.shape else {
        panic!("ambient surface should use one mesh");
    };
    assert!(mesh.is_valid());
    assert!(mesh.vertices.len() <= 425);
    assert!(mesh.indices.len() <= 2304);
    assert_eq!(mesh.calc_bounds(), rect);
    let mut interior_is_tinted = false;
    for vertex in &mesh.vertices {
        assert!(vertex.pos.is_finite());
        assert!(rect.contains(vertex.pos));
        if vertex.pos.x == rect.left()
            || vertex.pos.x == rect.right()
            || vertex.pos.y == rect.top()
            || vertex.pos.y == rect.bottom()
        {
            assert_eq!(vertex.color, Color32::TRANSPARENT);
        } else {
            interior_is_tinted |= vertex.color.a() > 0;
        }
        assert!(vertex.color.a() <= 14);
    }
    assert!(interior_is_tinted);
}

#[test]
fn ambient_wash_lifts_dark_surfaces_and_gently_tints_light_surfaces() {
    let ctx = Context::default();
    theme::apply(&ctx);
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        theme::set_appearance(&ctx, appearance);
        let output = ctx.run(input(vec![]), |ctx| {
            paint_ambient(
                &ctx.layer_painter(egui::LayerId::background()),
                Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(800.0, 600.0)),
            );
        });
        let Shape::Mesh(mesh) = &output.shapes[0].shape else {
            panic!("expected ambient mesh");
        };
        let strongest = mesh.vertices.iter().max_by_key(|v| v.color.a()).unwrap();
        let colors = theme::palette(&ctx);
        let composite = colors.panel.blend(strongest.color);
        let sum =
            |color: Color32| u16::from(color.r()) + u16::from(color.g()) + u16::from(color.b());
        if appearance == theme::Appearance::Dark {
            assert!(sum(composite) > sum(colors.panel));
            assert!(strongest.color.a() <= 14);
        } else {
            assert!(sum(composite) < sum(colors.panel));
            assert!(strongest.color.a() <= 10);
        }
        let rgba = strongest.color.to_srgba_unmultiplied();
        assert!(rgba[1] > rgba[0], "tint should remain quietly sage");
        for (after, before) in composite.to_array()[..3]
            .iter()
            .zip(&colors.panel.to_array()[..3])
        {
            assert!(after.abs_diff(*before) <= 16, "wash must stay subtle");
        }
    }
}

#[test]
fn ambient_wash_skips_tiny_offscreen_invisible_and_nonfinite_surfaces() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let output = ctx.run(input(vec![]), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        for rect in [
            Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(20.0, 24.0)),
            Rect::from_min_size(egui::pos2(0.0, 2000.0), egui::vec2(500.0, 500.0)),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(f32::INFINITY, 100.0)),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(100.0, f32::NAN)),
            Rect::NOTHING,
        ] {
            paint_ambient(&painter, rect);
        }
        let mut hidden = painter.clone();
        hidden.set_invisible();
        paint_ambient(
            &hidden,
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 500.0)),
        );
    });
    assert!(output.shapes.is_empty());
}

#[test]
fn soft_surface_preserves_native_frame_layout_focus_and_pointer_input() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut last = None;
    let mut paint = |tinted: bool, events| {
        let mut result = None;
        let output = ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let frame = Frame::new()
                    .fill(theme::palette(ctx).panel)
                    .inner_margin(12);
                let contents = |ui: &mut egui::Ui| {
                    ui.set_min_size(egui::vec2(300.0, 120.0));
                    ui.button("Visible and interactive")
                };
                let response = if tinted {
                    soft_surface(ui, frame, contents)
                } else {
                    frame.show(ui, contents)
                };
                result = Some((response.response.rect, response.inner));
            });
        });
        let result = result.unwrap();
        if let Some((rect, id, button_rect)) = last {
            assert_eq!(result.0, rect);
            assert_eq!(result.1.id, id);
            assert_eq!(result.1.rect, button_rect);
        }
        last = Some((result.0, result.1.id, result.1.rect));
        (output, result.1)
    };
    let (_, plain) = paint(false, vec![]);
    let (output, _) = paint(true, vec![]);
    let mesh_index = output
        .shapes
        .iter()
        .position(|shape| matches!(shape.shape, Shape::Mesh(_)))
        .unwrap();
    let text_index = output
        .shapes
        .iter()
        .position(|shape| matches!(shape.shape, Shape::Text(_)))
        .unwrap();
    assert!(mesh_index < text_index, "tint must stay behind controls");
    for pressed in [true, false] {
        let (_, button) = paint(
            true,
            vec![
                Event::PointerMoved(plain.rect.center()),
                Event::PointerButton {
                    pos: plain.rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(button.clicked(), !pressed);
        if !pressed {
            button.request_focus();
        }
    }
    let (_, button) = paint(
        true,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(button.clicked());
    assert!(button.has_focus());
}
