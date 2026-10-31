use daw_ui::{theme, tokens};
use egui::{Context, RawInput, Rect, Response, Shape};

struct InputFrame {
    response: Response,
    following: Rect,
    text: Vec<egui::epaint::TextShape>,
}

fn frame(ctx: &Context, shared: bool, numeric: bool, enabled: bool) -> InputFrame {
    let mut text = "Glass Current".to_owned();
    let mut number = -6.5;
    let mut controls = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            time: Some(1.0),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let style = ui.style().clone();
                    let response = match (shared, numeric) {
                        (true, true) => {
                            ui.add(daw_ui::number(&mut number).fixed_decimals(2).suffix(" dB"))
                        }
                        (false, true) => ui.add(
                            egui::DragValue::new(&mut number)
                                .fixed_decimals(2)
                                .suffix(" dB"),
                        ),
                        (true, false) => daw_ui::text_field(ui, &mut text, "Project", 200.0),
                        (false, false) => ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .hint_text("Project")
                                .desired_width(200.0)
                                .margin(egui::vec2(
                                    tokens::BUTTON_PADDING_X,
                                    tokens::BUTTON_PADDING_Y,
                                ))
                                .vertical_align(egui::Align::Center)
                                .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
                        ),
                    };
                    assert_eq!(*ui.style(), style, "input material styles must stay local");
                    controls = Some((response, ui.button("Following control").rect));
                });
            });
        },
    );
    let (response, following) = controls.unwrap();
    InputFrame {
        response,
        following,
        text: output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                Shape::Text(text) => Some(text),
                _ => None,
            })
            .collect(),
    }
}

#[test]
fn input_materials_preserve_native_geometry_glyphs_and_selected_text() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            for numeric in [false, true] {
                for enabled in [false, true] {
                    let native = Context::default();
                    let shared = Context::default();
                    for ctx in [&native, &shared] {
                        theme::apply(ctx);
                        theme::set_appearance(ctx, appearance);
                        ctx.set_pixels_per_point(density);
                    }
                    for focused in [false, true] {
                        let expected = frame(&native, false, numeric, enabled);
                        let actual = frame(&shared, true, numeric, enabled);
                        assert_eq!(actual.response.id, expected.response.id);
                        assert_eq!(actual.response.rect, expected.response.rect);
                        assert_eq!(actual.response.has_focus(), expected.response.has_focus());
                        assert_eq!(actual.following, expected.following);
                        assert_eq!(
                            actual.text, expected.text,
                            "material must not tint native glyphs or selection"
                        );
                        if !focused {
                            expected.response.request_focus();
                            actual.response.request_focus();
                        }
                    }
                }
            }
        }
    }
}
