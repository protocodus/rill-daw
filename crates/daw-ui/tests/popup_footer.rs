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
            let mut stable = None;
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
                if let Some((window, action)) = stable {
                    assert_eq!(state.0, window, "body growth must not resize the window");
                    assert_eq!(state.1, action, "body growth must not move actions");
                }
                stable = Some((state.0, state.1));
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
