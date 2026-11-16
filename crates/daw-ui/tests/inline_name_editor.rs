use daw_ui::{theme, tokens};
use egui::{Context, Event, FullOutput, Id, Modifiers, Pos2, RawInput, Rect, Vec2};

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
        .unwrap_or_else(|| panic!("Missing label {label}"))
}

#[test]
fn inline_editor_surface_matches_its_measured_bounds_and_covers_every_row() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let field = Id::new("opaque_inline_name");
            let mut value = "Long existing phrase name".to_owned();
            let mut measured = Vec2::ZERO;
            let mut render = || {
                ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        measured = daw_ui::inline_name_editor_size(ctx, 220.0);
                        egui::Area::new(Id::new("opaque_inline_area"))
                            .fade_in(false)
                            .fixed_pos(egui::pos2(8.0, 8.0))
                            .show(ctx, |ui| {
                                daw_ui::inline_name_editor(
                                    ui, field, &mut value, "Name", 220.0, false,
                                );
                            });
                    },
                )
            };
            render();
            render();
            let output = render();
            let apply = text_rect(&output, "Apply name");
            let cancel = text_rect(&output, "Cancel");
            let surface = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(shape)
                        if shape.fill == theme::palette(&ctx).raised
                            && shape.rect.contains_rect(apply)
                            && shape.rect.contains_rect(cancel) =>
                    {
                        Some(shape.rect)
                    }
                    _ => None,
                })
                .expect("The inline controls share one opaque surface");
            let tolerance = 1.0 / density;
            assert!(
                surface.width() <= measured.x + tolerance,
                "{surface:?} > {measured:?}"
            );
            assert!(
                surface.height() <= measured.y + tolerance,
                "{surface:?} > {measured:?}"
            );
            assert!(surface.shrink(tokens::SPACE_MD).contains_rect(apply));
            assert!(surface.shrink(tokens::SPACE_MD).contains_rect(cancel));
        }
    }
}

#[test]
fn inline_editor_surface_catches_pointer_gaps_without_adding_an_unnamed_tab_stop() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let field = Id::new("inline_name_focus");
    let mut value = "Keep this name".to_owned();
    let mut draw = |events, modifiers| {
        let mut before = None;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    before = Some(daw_ui::button(ui, "Previous control"));
                    daw_ui::inline_name_editor(ui, field, &mut value, "Name", 220.0, false);
                });
            },
        );
        before.unwrap()
    };
    draw(vec![], Modifiers::NONE);
    let before = draw(vec![], Modifiers::NONE);
    let surface = ctx.read_response(field.with("surface")).unwrap();
    assert!(
        surface.sense.senses_click(),
        "The surface still intercepts pointer gaps"
    );
    assert!(
        !surface.sense.is_focusable(),
        "A decorative surface is not a keyboard action"
    );
    before.request_focus();
    draw(vec![], Modifiers::NONE);
    let tab = |pressed, modifiers| {
        vec![Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }]
    };
    draw(tab(true, Modifiers::NONE), Modifiers::NONE);
    draw(tab(false, Modifiers::NONE), Modifiers::NONE);
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(field),
        "Tab goes directly to the named text field"
    );
    let shift = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    draw(tab(true, shift), shift);
    draw(tab(false, shift), shift);
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(before.id),
        "Shift-Tab skips the unnamed surface"
    );
    assert_eq!(value, "Keep this name");
}
