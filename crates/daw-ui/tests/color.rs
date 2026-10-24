use daw_ui::{ColorStripResponse, color_strip_at};
use egui::{Context, Event, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

fn frame(
    ctx: &Context,
    color: &mut [u8; 3],
    enabled: bool,
    events: Vec<Event>,
) -> ColorStripResponse {
    let mut response = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let position = ui.next_widget_position();
                    response = Some(color_strip_at(
                        ui,
                        Rect::from_min_size(pos2(20.0, 40.0), vec2(14.0, 72.0)),
                        "track",
                        "Color of synth",
                        color,
                    ));
                    assert_eq!(position, ui.next_widget_position());
                });
            });
        },
    );
    response.unwrap()
}

#[test]
fn strip_has_a_generous_hit_area_and_opening_does_not_change_saved_rgb() {
    for color in [[96, 177, 166], [128, 128, 128], [12, 34, 56]] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut value = color;
        for _ in 0..3 {
            frame(&ctx, &mut value, true, vec![]);
        }
        for pressed in [true, false] {
            // Within the hit rectangle, well outside the 3-pixel painted strip.
            let point = pos2(32.0, 60.0);
            let response = frame(
                &ctx,
                &mut value,
                true,
                vec![
                    Event::PointerMoved(point),
                    Event::PointerButton {
                        pos: point,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
            assert!(!response.response.changed());
        }
        assert!(frame(&ctx, &mut value, true, vec![]).picker.is_some());
        assert_eq!(value, color);
    }
}

#[test]
fn strip_supports_keyboard_activation_and_disabled_parent_blocks_opening() {
    for enabled in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut color = [96, 177, 166];
        let response = frame(&ctx, &mut color, enabled, vec![]);
        response.response.request_focus();
        let response = frame(
            &ctx,
            &mut color,
            enabled,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(response.picker.is_some(), enabled);
        assert!(!response.response.changed());
        assert_eq!(color, [96, 177, 166]);
    }
}
