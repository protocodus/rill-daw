use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

#[test]
fn pointer_feedback_is_immediate_and_keeps_its_text_inside_the_viewport() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        for width in [320.0, 800.0, 1440.0] {
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0));
            for pointer in [
                Pos2::new(1.0, 1.0),
                Pos2::new(width - 1.0, 1.0),
                Pos2::new(width - 1.0, 599.0),
                Pos2::new(1.0, 599.0),
            ] {
                let id = egui::Id::new("pointer feedback");
                let text = "End 30.000 s · Beat 57.00\nDuration 4.286 s · 8.00 beats";
                let mut bounds = Rect::NOTHING;
                let output = ctx.run(
                    RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ctx| bounds = daw_ui::pointer_text(ctx, id, pointer, text),
                );
                assert!(screen.contains_rect(bounds));
                let painted = output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(painted) = &shape.shape {
                            (painted.galley.text() == text).then_some((shape, painted))
                        } else {
                            None
                        }
                    })
                    .expect("Text is painted on its first frame without a hover delay");
                let text_bounds = painted.1.galley.rect.translate(painted.1.pos.to_vec2());
                assert!(screen.contains_rect(text_bounds));
                assert!(painted.0.clip_rect.contains_rect(text_bounds));
                assert!(egui::AreaState::load(&ctx, id).is_none());
            }
        }
    }
}

#[test]
fn stacked_text_hints_allow_an_immediate_click_on_the_control_beneath_them() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let source_rect = Rect::from_min_size(Pos2::new(30.0, 20.0), Vec2::new(160.0, 34.0));
    let target_rect = Rect::from_min_size(Pos2::new(20.0, 64.0), Vec2::new(360.0, 180.0));
    let mut time = 0.0;
    let mut draw = |events| {
        time += 0.1;
        let mut clicked = false;
        let mut source_id = egui::Id::NULL;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 300.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let source = ui.put(source_rect, egui::Button::new("Volume"));
                    source_id = source.id;
                    daw_ui::hover_text(
                        source.clone(),
                        "Current value and complete range\nDrag the center marker to adjust.",
                    );
                    daw_ui::hover_text(source, "Right-click to create automation");
                    clicked = ui
                        .put(target_rect, egui::Button::new("Open routing"))
                        .clicked();
                });
            },
        );
        (source_id, clicked, output)
    };
    draw(vec![]);
    draw(vec![Event::PointerMoved(source_rect.center())]);
    draw(vec![]);
    let (source_id, _, output) = draw(vec![]);
    let text_rect = |prefix: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    text.galley
                        .text()
                        .starts_with(prefix)
                        .then(|| text.galley.rect.translate(text.pos.to_vec2()))
                } else {
                    None
                }
            })
            .unwrap()
    };
    let first = text_rect("Current value");
    let second = text_rect("Right-click");
    assert!(first.bottom() <= second.top());
    assert!(
        egui::AreaState::load(&ctx, source_id.with(("read_only_tooltip", 0_usize))).is_none(),
        "Read-only hints must not register an input-occluding Area"
    );
    let covered = first.intersect(target_rect);
    assert!(
        covered.is_positive(),
        "The regression requires a hint over the destination"
    );
    let position = covered.center();
    let event = |pressed| Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    // The mouse arrives and presses in the same input frame, while last frame's
    // tooltip was painted over the destination control.
    let (_, clicked, _) = draw(vec![Event::PointerMoved(position), event(true)]);
    assert!(!clicked);
    let (_, clicked, _) = draw(vec![event(false)]);
    assert!(
        clicked,
        "Read-only hints must never swallow the first click"
    );
}
