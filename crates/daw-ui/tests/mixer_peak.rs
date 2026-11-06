use daw_ui::{self as controls, theme};
use egui::{
    Context, Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, Response, Shape, pos2,
    vec2,
};

fn click(pos: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn frame(ctx: &Context, held: [f32; 2], time: f64, events: Vec<Event>) -> (Response, FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(controls::channel_peak_indicator(
                    ui,
                    egui::Id::new("peak"),
                    held,
                    180.0,
                    Rect::from_min_size(pos2(80.0, 60.0), vec2(128.0, 34.0)),
                ));
                controls::channel_meter_with_peak_hold(
                    ui,
                    egui::Id::new("peak"),
                    [0.0; 2],
                    held,
                    200.0,
                    8.0,
                );
            });
        },
    );
    (response.unwrap(), output)
}

fn visit(shape: &Shape, f: &mut impl FnMut(&Shape)) {
    f(shape);
    if let Shape::Vec(shapes) = shape {
        for shape in shapes {
            visit(shape, f);
        }
    }
}

#[test]
fn peak_badge_and_meter_share_a_persistent_clip_hold_with_mouse_reset() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let colors = theme::palette(&ctx);
        let (response, output) = frame(&ctx, [1.25, 0.5], 0.0, vec![]);
        assert_eq!(response.rect.height(), 34.0);
        assert!(response.rect.width() >= 34.0);
        assert!(response.rect.right() <= 208.0);
        let text_color = |output: &FullOutput, text: &str| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.galley.job.sections[0].format.color)
                }
                _ => None,
            })
        };
        assert_eq!(text_color(&output, "1.9"), Some(colors.warning));
        let (_, held) = frame(&ctx, [1.25, 0.5], 30.0, vec![]);
        assert_eq!(
            text_color(&held, "1.9"),
            Some(colors.warning),
            "Silence cannot expire an engine-owned peak"
        );
        frame(&ctx, [1.25, 0.5], 30.1, click(response.rect.center(), true));
        let (reset, _) = frame(
            &ctx,
            [1.25, 0.5],
            30.2,
            click(response.rect.center(), false),
        );
        assert!(
            reset.clicked(),
            "The badge exposes a real reset action to its caller"
        );
        let (_, clear) = frame(&ctx, [0.0; 2], 30.3, vec![]);
        assert_eq!(text_color(&clear, "−∞"), Some(colors.muted));
        assert!(!clear.shapes.iter().any(|shape| matches!(&shape.shape,
            Shape::Rect(rect) if rect.fill == colors.warning && rect.rect.height() <= 5.0)));
        let (_, clean) = frame(&ctx, [f32::NAN, f32::INFINITY], 30.4, vec![]);
        assert!(
            clean
                .shapes
                .iter()
                .all(|shape| matches!(shape.shape, Shape::Noop)
                    || shape.shape.visual_bounding_rect().is_finite())
        );
    }
}

#[test]
fn mute_and_solo_keep_distinct_idle_colors_and_solid_active_fills() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let colors = theme::palette(&ctx);
        for active in [false, true] {
            let mut mute = active;
            let mut solo = active;
            let mut responses = None;
            let output = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    responses = Some(controls::mixer_mute_solo(ui, &mut mute, &mut solo, 180.0));
                });
            });
            let responses = responses.unwrap();
            let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
            for (label, accent, rect) in [
                ("Mute", colors.mute, responses.mute.rect),
                ("Solo", colors.solo, responses.solo.rect),
            ] {
                let mut found_text = false;
                let mut found_fill = false;
                for clipped in &output.shapes {
                    visit(&clipped.shape, &mut |shape| match shape {
                        Shape::Text(text) if text.galley.text() == label => {
                            assert_eq!(
                                text.galley.job.sections[0].format.color,
                                if active {
                                    theme::contrast_text(accent)
                                } else {
                                    accent
                                }
                            );
                            found_text = true;
                        }
                        Shape::Rect(shape) if shape.rect == rect && shape.fill == accent => {
                            found_fill = true
                        }
                        _ => {}
                    });
                }
                assert!(found_text);
                assert_eq!(
                    found_fill, active,
                    "Only an active {label} has a solid operation-color fill"
                );
                assert!(
                    primitives
                        .iter()
                        .any(|primitive| match &primitive.primitive {
                            egui::epaint::Primitive::Mesh(mesh) => mesh
                                .vertices
                                .iter()
                                .any(|vertex| rect.contains(vertex.pos) && vertex.color == accent),
                            _ => false,
                        }),
                    "{label}'s operation color must survive into its rendered mesh"
                );
            }
        }
    }
}
