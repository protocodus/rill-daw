//! Exercise the painted native close control, including the edges of its target.
use daw_ui::theme::{self, Appearance};
use egui::{Context, Event, FullOutput, PointerButton, Pos2, RawInput, Rect, Shape};

fn close_icon(output: &FullOutput) -> Rect {
    let pixel = 1.0 / output.pixels_per_point;
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
                && (rect.width() - rect.height()).abs() <= pixel + 0.01
                && ((b[0].distance(rect.right_top()) < 0.1
                    && b[1].distance(rect.left_bottom()) < 0.1)
                    || (b[1].distance(rect.right_top()) < 0.1
                        && b[0].distance(rect.left_bottom()) < 0.1)))
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

#[test]
fn long_titles_keep_their_complete_semantic_text_distinct_ids_and_visible_native_close() {
    let prefix = "Properties of a long musical phrase 夜の音楽 🎹 ".repeat(20);
    for appearance in [Appearance::Dark, Appearance::Light] {
        for density in [1.0, 1.25, 2.0] {
            for size in [egui::vec2(800.0, 600.0), egui::vec2(1440.0, 1000.0)] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                let viewport = Rect::from_min_size(Pos2::ZERO, size);
                let mut previous_id = None;
                for suffix in ["first", "second"] {
                    let title = format!("{prefix}{suffix}");
                    let mut open = true;
                    let mut frame = |events| {
                        let mut response = None;
                        let output = ctx.run(
                            RawInput {
                                screen_rect: Some(viewport),
                                events,
                                ..Default::default()
                            },
                            |ctx| {
                                response = daw_ui::popup(&title, ctx)
                                    .open(&mut open)
                                    .show(ctx, |ui| {
                                        daw_ui::text::body(
                                            ui,
                                            "The property controls remain within the viewport.",
                                        );
                                    })
                                    .map(|shown| shown.response);
                            },
                        );
                        (output, response, open)
                    };
                    for _ in 0..3 {
                        frame(vec![]);
                    }
                    let (output, window, _) = frame(vec![]);
                    let window = window.unwrap();
                    assert!(
                        viewport.contains_rect(window.rect),
                        "{size:?}: {:?}",
                        window.rect
                    );
                    // Window's area retains the full-title ID; its returned
                    // interaction response uses a separate native move ID.
                    assert_eq!(window.layer_id.id, egui::Id::new(title.as_str()));
                    assert_ne!(
                        previous_id,
                        Some(window.id),
                        "elided suffixes must not alias window identity"
                    );
                    previous_id = Some(window.id);
                    let painted = output.shapes.iter().find_map(|shape| match &shape.shape {
                        Shape::Text(text) if text.galley.job.text == title => Some((shape.clip_rect, text)),
                        _ => None,
                    }).expect("Native window retains the full title as its accessibility/identity source");
                    assert!(painted.1.galley.elided);
                    assert_eq!(painted.1.galley.rows.len(), 1);
                    let text_rect = painted.1.galley.rect.translate(painted.1.pos.to_vec2());
                    assert!(painted.0.contains_rect(text_rect));
                    let close = close_icon(&output).expand(2.0);
                    assert!(viewport.contains_rect(close));
                    assert!(!close.intersects(text_rect));
                    frame(pointer(close.center(), true));
                    assert!(
                        !frame(pointer(close.center(), false)).2,
                        "native Close remains clickable"
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
