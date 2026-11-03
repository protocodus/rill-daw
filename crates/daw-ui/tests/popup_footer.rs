//! Footer placement and hit targets survive body growth and action wrapping.
use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

fn draw(ctx: &Context, width: f32, count: usize, events: Vec<Event>) -> (Rect, Rect, bool) {
    let mut window = Rect::NOTHING;
    let mut action = Rect::NOTHING;
    let mut clicked = false;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 760.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            window = daw_ui::popup("Footer", ctx)
                .fixed_size([width, 320.0])
                .vscroll(false)
                .show(ctx, |ui| {
                    let mut footer = daw_ui::popup_footer(ui, "actions");
                    daw_ui::scroll_body(footer.body(), "options", |ui| {
                        for index in 0..count {
                            ui.label(format!("Parameter {index}"));
                        }
                    });
                    footer.show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            daw_ui::button(ui, "A wider secondary action");
                            let apply = daw_ui::primary_button(ui, "Apply");
                            action = apply.rect;
                            clicked = apply.clicked();
                        })
                    });
                })
                .unwrap()
                .response
                .rect;
        },
    );
    (window, action, clicked)
}

#[test]
fn footer_stays_at_the_bottom_and_clickable_through_growth_wrapping_and_resize() {
    for appearance in [
        daw_ui::theme::Appearance::Light,
        daw_ui::theme::Appearance::Dark,
    ] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        daw_ui::theme::set_appearance(&ctx, appearance);
        for width in [480.0, 280.0, 480.0] {
            let mut stable = std::collections::HashMap::new();
            for count in [1, 100, 1] {
                let mut state = draw(&ctx, width, count, vec![]);
                for _ in 0..5 {
                    state = draw(&ctx, width, count, vec![]);
                }
                let gap = state.0.bottom() - state.1.bottom();
                assert!(
                    (12.0..=18.0).contains(&gap),
                    "footer gap: {gap}, state: {state:?}"
                );
                assert!(state.0.contains_rect(state.1));
                if count == 1 {
                    assert!(state.0.height() < 250.0, "short body must hug: {state:?}");
                }
                // The window hugs short content and returns to the same
                // geometry once the long body shrinks again.
                // A centered window may round its top by one point.
                if let Some((window, action)) = stable.insert(count, (state.0, state.1)) {
                    assert!(
                        (state.0.height() - window.height()).abs() < 1.0
                            && (state.0.top() - window.top()).abs() <= 1.0,
                        "the same body must give the same window: {state:?} {window:?}"
                    );
                    assert!(
                        ((state.1.top() - state.0.top()) - (action.top() - window.top())).abs()
                            < 0.5,
                        "the same body must give the same actions"
                    );
                }
                let at = state.1.center();
                for pressed in [true, false] {
                    let state = draw(
                        &ctx,
                        width,
                        count,
                        vec![
                            Event::PointerMoved(at),
                            Event::PointerButton {
                                pos: at,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    assert_eq!(state.2, !pressed);
                }
            }
            // An asynchronous body change between press and release must keep
            // the footer's hit target and identity intact.
            let state = draw(&ctx, width, 1, vec![]);
            let at = state.1.center();
            for (pressed, count) in [(true, 1), (false, 100)] {
                let result = draw(
                    &ctx,
                    width,
                    count,
                    vec![
                        Event::PointerMoved(at),
                        Event::PointerButton {
                            pos: at,
                            button: PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
                assert_eq!(result.1, state.1);
                assert_eq!(result.2, !pressed);
            }
        }
    }
}
