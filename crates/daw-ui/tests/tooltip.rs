use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

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
