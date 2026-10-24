//! Exercise the painted native close control, including the edges of its target.
use daw_ui::theme::{self, Appearance};
use egui::{Context, Event, FullOutput, PointerButton, Pos2, RawInput, Rect, Shape};

fn close_icon(output: &FullOutput) -> Rect {
    output
        .shapes
        .windows(2)
        .find_map(|pair| {
            let [
                Shape::LineSegment { points: a, .. },
                Shape::LineSegment { points: b, .. },
            ] = [&pair[0].shape, &pair[1].shape]
            else {
                return None;
            };
            let rect = Rect::from_two_pos(a[0], a[1]);
            (rect.width() > 0.0
                && rect.width() == rect.height()
                && b[0] == rect.right_top()
                && b[1] == rect.left_bottom())
            .then_some(rect)
        })
        .expect("native titlebar paints a close X")
}

#[test]
fn popup_close_target_is_clickable_after_text_edit_in_both_themes_and_sizes() {
    for appearance in [Appearance::Dark, Appearance::Light] {
        for large in [false, true] {
            for density in [1.0, 2.0] {
                // Check the center and all four corners. This catches partial
                // overlap with another titlebar interaction, not only a dead center.
                for fraction in [
                    egui::vec2(0.5, 0.5),
                    egui::vec2(0.05, 0.05),
                    egui::vec2(0.95, 0.05),
                    egui::vec2(0.05, 0.95),
                    egui::vec2(0.95, 0.95),
                ] {
                    let ctx = Context::default();
                    theme::apply(&ctx);
                    theme::set_appearance(&ctx, appearance);
                    ctx.set_pixels_per_point(density);
                    let mut open = true;
                    let mut value = String::from("Original name");
                    let mut field = Rect::NOTHING;
                    let mut time = 0.0;
                    let mut frame = |events| {
                        time += 1.0 / 60.0;
                        let output = ctx.run(
                            RawInput {
                                screen_rect: Some(Rect::from_min_size(
                                    Pos2::ZERO,
                                    egui::vec2(1100.0, 720.0),
                                )),
                                time: Some(time),
                                events,
                                ..Default::default()
                            },
                            |ctx| {
                                let window = if large {
                                    daw_ui::workspace_popup("Large editor", ctx)
                                } else {
                                    daw_ui::popup("Small properties", ctx)
                                };
                                window.open(&mut open).show(ctx, |ui| {
                                    field = daw_ui::text_field(ui, &mut value, "Name", 240.0).rect;
                                    if large {
                                        for index in 0..100 {
                                            ui.label(format!("Parameter {index}"));
                                        }
                                    }
                                });
                            },
                        );
                        (output, open, field, value.clone())
                    };
                    let mut initial = frame(vec![]);
                    for _ in 0..3 {
                        initial = frame(vec![]);
                    }
                    let field = initial.2.center();
                    for pressed in [true, false] {
                        frame(pointer(field, pressed));
                    }
                    let edited = frame(vec![Event::Text(" edited".into())]);
                    assert!(edited.3.contains("edited"));
                    assert!(ctx.wants_keyboard_input());
                    // egui paints the X two points inside its actual hit target.
                    let target = close_icon(&edited.0).expand(2.0);
                    let position = target.min + target.size() * fraction;
                    frame(pointer(position, true));
                    let (_, open, _, _) = frame(pointer(position, false));
                    assert!(
                        !open,
                        "Close must work: {appearance:?}, large={large}, density={density}, position={fraction:?}, target={target:?}"
                    );
                }
            }
        }
    }
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}
