use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

#[test]
fn immediate_pointer_text_stays_inside_its_canvas_at_every_edge_and_density() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            ctx.set_pixels_per_point(density);
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
            let canvas = Rect::from_min_max(Pos2::new(61.0, 240.0), Pos2::new(780.0, 510.0));
            let input = || RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let _ = ctx.run(input(), |_| {});
            for pointer in [
                canvas.left_top() + Vec2::splat(1.0),
                canvas.right_top() + Vec2::new(-1.0, 1.0),
                canvas.right_bottom() - Vec2::splat(1.0),
                canvas.left_bottom() + Vec2::new(1.0, -1.0),
                canvas.center(),
            ] {
                let id = egui::Id::new("bounded pitch hint");
                let mut bounds = Rect::NOTHING;
                let output = ctx.run(input(), |ctx| {
                    bounds = daw_ui::pointer_text_in_rect(ctx, id, pointer, canvas, "C♯4");
                });
                assert!(canvas.contains_rect(bounds));
                let (shape, text) = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "C♯4" => {
                            Some((shape, text))
                        }
                        _ => None,
                    })
                    .expect("Pitch text must be painted immediately");
                let painted = text.galley.rect.translate(text.pos.to_vec2());
                assert!(canvas.contains_rect(painted));
                assert!(bounds.contains_rect(painted));
                assert!(shape.clip_rect.contains_rect(painted));
                assert!(painted.center().distance(pointer) < 100.0);
                assert!(egui::AreaState::load(&ctx, id).is_none());
            }
        }
    }
}

#[test]
fn a_press_on_the_previous_pointer_hint_reaches_the_canvas_in_the_same_frame() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let canvas = Rect::from_min_max(Pos2::new(61.0, 240.0), Pos2::new(780.0, 510.0));
    let pointer = canvas.center();
    let mut time = 0.0;
    let mut draw = |events| {
        time += 0.1;
        let mut clicked = false;
        let mut hint = Rect::NOTHING;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    clicked = ui
                        .interact(canvas, ui.id().with("drawing canvas"), egui::Sense::click())
                        .clicked();
                    hint = daw_ui::pointer_text_in_rect(
                        ctx,
                        egui::Id::new("bounded pitch hint"),
                        pointer,
                        canvas,
                        "D♯4",
                    );
                });
            },
        );
        (clicked, hint)
    };
    draw(vec![]);
    let (_, hint) = draw(vec![Event::PointerMoved(pointer)]);
    let target = hint.center();
    assert!(canvas.contains(target));
    let press = |pressed| Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    assert!(!draw(vec![Event::PointerMoved(target), press(true)]).0);
    assert!(
        draw(vec![press(false)]).0,
        "The first click must reach the canvas"
    );
}
