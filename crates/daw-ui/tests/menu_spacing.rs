use daw_ui::{self as controls, theme, tokens};
use egui::{Context, Event, FullOutput, Modifiers, RawInput, Rect, Response, Shape, Ui};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    }
}

fn click(pos: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn label(output: &FullOutput, name: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == name => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing rendered {name}"))
}

#[test]
fn choice_text_has_its_own_padding_inside_a_compact_menu_frame() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for scale in [1.0, 1.25, 2.0] {
            // Shared menus, standard combo fields, and unwrapped native menus
            // must all preserve real text padding on shared choice controls.
            for kind in 0..4 {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(scale);
                let mut current = 1;
                let mut frame = |events| {
                    let mut opener = None;
                    let mut rows = Vec::<(&str, Response)>::new();
                    let mut popup_id = None;
                    let output = ctx.run(input(events), |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let original = ui.style().clone();
                            let contents = |ui: &mut Ui| {
                                popup_id = Some(ui.layer_id().id);
                                for (value, name) in [
                                    (0, "Major pentatonic"),
                                    (1, "Natural minor"),
                                    (2, "Lydian"),
                                    (3, "Unavailable scale"),
                                ] {
                                    let response = if value == 2 {
                                        ui.add_enabled(
                                            false,
                                            controls::button_widget(name)
                                                .selected(false)
                                                .frame_when_inactive(false),
                                        )
                                    } else {
                                        controls::selectable_value(ui, &mut current, value, name)
                                    };
                                    if response.changed() {
                                        ui.close();
                                    }
                                    rows.push((name, response));
                                }
                                rows.push((
                                    "Effects",
                                    controls::menu_button(ui, "Effects", |ui| {
                                        controls::button(ui, "Chromatic");
                                    })
                                    .response,
                                ));
                            };
                            opener = Some(match kind {
                                0 => controls::choice_menu(ui, "Scale", contents).response,
                                1 => controls::menu_button(ui, "Scale", contents).response,
                                2 => {
                                    controls::select("scale", "Scale")
                                        .show_ui(ui, contents)
                                        .response
                                }
                                _ => ui.menu_button("Scale", contents).response,
                            });
                            assert_eq!(*ui.style(), original, "Menu styles stay inside the popup");
                        });
                    });
                    let popup = popup_id.and_then(|id| ctx.memory(|memory| memory.area_rect(id)));
                    (output, opener.unwrap(), rows, popup, current)
                };
                let opener = frame(vec![]).1.rect.center();
                frame(click(opener, true));
                frame(click(opener, false));
                for _ in 0..3 {
                    frame(vec![]);
                }
                let (output, _, rows, popup, selected) = frame(vec![]);
                assert_eq!(selected, 1);
                assert_eq!(rows.len(), 5);
                let tolerance = 1.0 / scale;
                for (name, response) in &rows {
                    let text = label(&output, name);
                    assert!(
                        text.left() - response.rect.left() >= tokens::SPACE_MD - tolerance
                            && response.rect.right() - text.right() >= tokens::SPACE_MD - tolerance,
                        "{name} needs padding inside its own hit/selection rectangle at {scale}×, kind {kind}: text {text:?}, row {:?}",
                        response.rect,
                    );
                    assert!((text.center().y - response.rect.center().y).abs() <= tolerance);
                    // Long labels may wrap at a bounded menu width, but must
                    // retain the same vertical padding rather than clipping.
                    let height =
                        tokens::CONTROL_HEIGHT.max(text.height() + 2.0 * tokens::BUTTON_PADDING_Y);
                    assert!(
                        (response.rect.height() - height).abs() <= tolerance,
                        "{name} must keep its text and padding at {scale}×, kind {kind}: text {text:?}, row {:?}",
                        response.rect,
                    );
                    if *name == "Natural minor" || *name == "Lydian" {
                        assert!(
                            (response.rect.height() - tokens::CONTROL_HEIGHT).abs() <= tolerance
                        );
                    }
                }
                let effects = &rows[4].1;
                let icon_center_x = effects.rect.left() + tokens::BUTTON_PADDING_X + 9.0;
                let circles: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        Shape::Circle(circle) if effects.rect.contains(circle.center) => {
                            Some((shape.clip_rect, circle))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    circles.len(),
                    2,
                    "The Effects icon must actually be painted"
                );
                for (clip, circle) in circles {
                    assert!(
                        (circle.center.x - icon_center_x).abs() <= tolerance,
                        "Effects icon must use the padded submenu atom at {scale}×, kind {kind}: {:?} in {:?}",
                        circle.center,
                        effects.rect,
                    );
                    assert!(clip.contains_rect(Rect::from_center_size(
                        circle.center,
                        egui::Vec2::splat(circle.radius * 2.0),
                    )));
                }
                let text = label(&output, "Effects");
                assert!(text.left() >= icon_center_x + 9.0 + tokens::SPACE_SM - tolerance);
                let popup = popup.expect("An open menu has an actual native frame");
                let border = ctx.style().visuals.window_stroke().width;
                assert!(
                    (rows[0].1.rect.left() - popup.left() - border - tokens::SPACE_SM).abs()
                        <= tolerance,
                    "Frame inset must be 8px plus its border: {popup:?}, {:?}",
                    rows[0].1.rect,
                );
                assert!(
                    (rows[0].1.rect.top() - popup.top() - border - tokens::SPACE_XS).abs()
                        <= tolerance,
                    "Vertical frame inset must be 5px plus its border: {popup:?}, {:?}",
                    rows[0].1.rect,
                );
                if kind < 3 {
                    for pair in rows.windows(2) {
                        assert!(
                            (pair[1].1.rect.top() - pair[0].1.rect.bottom() - tokens::SPACE_XS)
                                .abs()
                                <= tolerance,
                            "Shared menu rows retain compact 5px separation"
                        );
                    }
                }
                assert!(!rows[2].1.enabled());
                let target = rows[0].1.rect.center();
                frame(click(target, true));
                frame(click(target, false));
                let (_, _, rows, _, selected) = frame(vec![]);
                assert_eq!(selected, 0, "The padded choice remains clickable");
                assert!(rows.is_empty(), "Accepting a choice dismisses the menu");
            }
        }
    }
}

#[test]
fn menu_bar_and_menu_items_have_padding_and_no_blue_borders_or_action_tints() {
    fn visit_rects(shape: &Shape, rects: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            Shape::Rect(rect) => rects.push(rect.clone()),
            Shape::Vec(shapes) => {
                for shape in shapes {
                    visit_rects(shape, rects);
                }
            }
            _ => {}
        }
    }
    fn text_color(output: &FullOutput, name: &str) -> egui::Color32 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.text() == name => Some(
                    text.override_text_color
                        .unwrap_or(text.galley.job.sections[0].format.color),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing rendered text {name}"))
    }

    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        ctx.style_mut(|style| style.animation_time = 0.0);
        let colors = theme::palette(&ctx);
        let draw = |events| {
            let mut file_btn = None;
            let mut edit_btn = None;
            let mut ai_btn = None;
            let mut undo_btn = None;
            let mut redo_btn = None;
            let output = ctx.run(input(events), |ctx| {
                egui::TopBottomPanel::top("bar").show(ctx, |ui| {
                    controls::menu_bar(ui, |ui| {
                        file_btn = Some(
                            controls::command_menu_button(ui, "File", |ui| {
                                let _ = controls::button(ui, "New");
                            })
                            .response,
                        );
                        edit_btn = Some(
                            controls::command_menu_button(ui, "Edit", |ui| {
                                undo_btn =
                                    Some(ui.add_enabled(false, controls::button_widget("Undo")));
                                redo_btn = Some(controls::button(ui, "Redo"));
                            })
                            .response,
                        );
                        ai_btn = Some(controls::ai_button(ui, "AI"));
                    });
                });
                egui::CentralPanel::default().show(ctx, |_| {});
                controls::paint_popup_backdrop(ctx, false);
            });
            (
                output,
                file_btn.unwrap(),
                edit_btn.unwrap(),
                ai_btn.unwrap(),
                undo_btn,
                redo_btn,
            )
        };

        let (idle_out, file, edit, ai, _, _) = draw(vec![]);
        for (name, response) in [("File", &file), ("Edit", &edit), ("AI", &ai)] {
            let text = label(&idle_out, name);
            assert!(
                text.left() - response.rect.left() >= tokens::BUTTON_PADDING_X - 1.0
                    && response.rect.right() - text.right() >= tokens::BUTTON_PADDING_X - 1.0,
                "{name} in menu_bar must have horizontal padding: text {text:?}, button {:?}",
                response.rect
            );
        }

        let target = edit.rect.center();
        draw(click(target, true));
        draw(click(target, false));
        for _ in 0..3 {
            draw(vec![]);
        }
        let (open_out, _, _, _, undo, redo) = draw(vec![]);
        assert!(
            controls::backdrop_blur_info(&ctx).is_empty(),
            "Opening a menu must not blur the background"
        );
        let undo = undo.expect("Edit menu is open");
        let redo = redo.expect("Edit menu is open");
        assert_ne!(text_color(&open_out, "Undo"), colors.action);
        assert_eq!(text_color(&open_out, "Redo"), colors.text);

        let mut rects = Vec::new();
        for shape in &open_out.shapes {
            visit_rects(&shape.shape, &mut rects);
        }
        for item in [&undo, &redo] {
            assert!(
                !rects.iter().any(|rect| rect.rect == item.rect
                    && (rect.fill != egui::Color32::TRANSPARENT || !rect.stroke.is_empty())),
                "Idle menu items must not have boxes or borders: {:?}",
                item.rect
            );
        }

        let (hover_out, _, _, _, _, redo_hovered) =
            draw(vec![Event::PointerMoved(redo.rect.center())]);
        let redo_hovered = redo_hovered.unwrap();
        let mut hover_rects = Vec::new();
        for shape in &hover_out.shapes {
            visit_rects(&shape.shape, &mut hover_rects);
        }
        assert!(
            hover_rects
                .iter()
                .any(|rect| rect.rect == redo_hovered.rect && rect.fill == colors.control_hover),
            "Hovered menu item should use flat control_hover fill"
        );
        assert!(
            !hover_rects
                .iter()
                .any(|rect| rect.rect == redo_hovered.rect && !rect.stroke.is_empty()),
            "Hovered menu item must not have a blue or box border"
        );
    }
}
