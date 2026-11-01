use daw_ui::{name_keyboard, name_keyboard_height, set_name_keyboard_open, theme, tokens};
use egui::{Context, Event, FullOutput, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

fn frame(ctx: &Context, size: Vec2, events: Vec<Event>) -> (FullOutput, Rect) {
    let mut bounds = Rect::NOTHING;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                bounds = ui
                    .allocate_ui_with_layout(
                        Vec2::new(460.0, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(460.0);
                            let field = Id::new("keyboard_spacing_field");
                            let mut value = String::new();
                            daw_ui::text_field(ui, &mut value, "Name", 220.0);
                            set_name_keyboard_open(ctx, field, true);
                            name_keyboard(ui, field, &mut value);
                        },
                    )
                    .response
                    .rect;
            });
        },
    );
    (output, bounds)
}

fn text_rect(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing keyboard label {label}"))
}

#[test]
fn lowercase_uppercase_and_command_rows_fit_the_shared_width_and_predicted_height() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            for size in [Vec2::new(800.0, 600.0), Vec2::new(1440.0, 1000.0)] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                for _ in 0..3 {
                    frame(&ctx, size, vec![]);
                }
                let (lowercase, bounds) = frame(&ctx, size, vec![]);
                let tolerance = 1.0 / density;
                assert!(bounds.width() <= 460.0 + tolerance, "{bounds:?}");
                assert!(
                    bounds.height()
                        <= tokens::CONTROL_HEIGHT + name_keyboard_height(&ctx) + tolerance,
                    "{bounds:?}"
                );
                let caps = text_rect(&lowercase, "Caps").center();
                for pressed in [true, false] {
                    frame(
                        &ctx,
                        size,
                        vec![
                            Event::PointerMoved(caps),
                            Event::PointerButton {
                                pos: caps,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: Modifiers::NONE,
                            },
                        ],
                    );
                }
                let (uppercase, bounds) = frame(&ctx, size, vec![]);
                assert!(
                    bounds.width() <= 460.0 + tolerance,
                    "Uppercase row: {bounds:?}"
                );
                assert!(
                    bounds.height()
                        <= tokens::CONTROL_HEIGHT + name_keyboard_height(&ctx) + tolerance
                );
                for label in [
                    "Q",
                    "W",
                    "M",
                    "Caps",
                    "Space",
                    "Backspace",
                    "Select all",
                    "Clear",
                ] {
                    assert!(
                        bounds.contains_rect(text_rect(&uppercase, label)),
                        "{label}: {bounds:?}"
                    );
                }
                let q = text_rect(&uppercase, "Q");
                let w = text_rect(&uppercase, "W");
                assert!(
                    w.center().x - q.center().x
                        >= tokens::CONTROL_HEIGHT + tokens::SPACE_SM - tolerance
                );
            }
        }
    }
}
