use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape};

fn context(dark: bool, density: f32) -> Context {
    let ctx = Context::default();
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_pixels_per_point(density);
    daw_ui::theme::apply(&ctx);
    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 480.0))),
            ..Default::default()
        },
        |_| {},
    );
    ctx
}

fn frame(
    ctx: &Context,
    time: &mut f64,
    events: Vec<Event>,
    mut contents: impl FnMut(&mut egui::Ui),
) -> FullOutput {
    *time += 0.1;
    ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 480.0))),
            time: Some(*time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| contents(ui));
        },
    )
}

fn text_rect(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing painted text: {label}"))
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn hinted_tabs_keep_layout_and_mouse_navigation_in_both_themes_and_densities() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = context(dark, density);
            let mut time = 0.0;
            let mut page = 0;
            let draw = |ui: &mut egui::Ui, page: &mut usize, hints| {
                if hints {
                    daw_ui::popup_tabs_with_hints(
                        ui,
                        page,
                        &[
                            (
                                0,
                                "Instruments",
                                "Browse sound generators to play with MIDI notes",
                            ),
                            (1, "Effects", "Browse processors for channel audio"),
                        ],
                    )
                } else {
                    daw_ui::popup_tabs(ui, page, &[(0, "Instruments"), (1, "Effects")])
                }
                daw_ui::quiet_button(ui, "Refresh");
                ui.label("Page body");
            };
            let ordinary = frame(&ctx, &mut time, vec![], |ui| draw(ui, &mut page, false));
            let hinted = frame(&ctx, &mut time, vec![], |ui| draw(ui, &mut page, true));
            for label in ["Instruments", "Effects", "Refresh", "Page body"] {
                assert_eq!(text_rect(&ordinary, label), text_rect(&hinted, label));
            }
            assert!(text_rect(&hinted, "Refresh").top() > text_rect(&hinted, "Effects").bottom());
            for (label, hint, value) in [
                ("Effects", "Browse processors for channel audio", 1),
                (
                    "Instruments",
                    "Browse sound generators to play with MIDI notes",
                    0,
                ),
            ] {
                let pos = text_rect(&hinted, label).center();
                frame(&ctx, &mut time, vec![Event::PointerMoved(pos)], |ui| {
                    draw(ui, &mut page, true)
                });
                frame(&ctx, &mut time, vec![], |ui| draw(ui, &mut page, true));
                let hover = frame(&ctx, &mut time, vec![], |ui| draw(ui, &mut page, true));
                assert!(text_rect(&hover, hint).is_positive());
                frame(&ctx, &mut time, pointer(pos, true), |ui| {
                    draw(ui, &mut page, true)
                });
                frame(&ctx, &mut time, pointer(pos, false), |ui| {
                    draw(ui, &mut page, true)
                });
                assert_eq!(page, value, "Hover hints must not intercept the tab click");
            }
        }
    }
}

#[test]
fn unavailable_view_actions_explain_prerequisites_and_remain_inactive() {
    for dark in [false, true] {
        let ctx = context(dark, 1.0);
        let mut time = 0.0;
        let mut draw = |ui: &mut egui::Ui| {
            assert!(
                daw_ui::timeline_selection_view_actions(ui, "hint_test", false)
                    == daw_ui::TimelineViewAction::None
            );
        };
        let initial = frame(&ctx, &mut time, vec![], &mut draw);
        for (label, hint) in [
            (
                "Fit selection",
                "Select clips, notes or a time range to fit the selection",
            ),
            (
                "Previous view",
                "Pan, zoom or use Fit before returning to a previous view",
            ),
        ] {
            let pos = text_rect(&initial, label).center();
            frame(&ctx, &mut time, vec![Event::PointerMoved(pos)], &mut draw);
            frame(&ctx, &mut time, vec![], &mut draw);
            let hover = frame(&ctx, &mut time, vec![], &mut draw);
            assert!(text_rect(&hover, hint).is_positive());
            frame(&ctx, &mut time, pointer(pos, true), &mut draw);
            frame(&ctx, &mut time, pointer(pos, false), &mut draw);
        }
    }
}

#[test]
fn numeric_boundary_hint_explains_the_limit_without_changing_the_value() {
    let ctx = context(true, 1.0);
    let mut time = 0.0;
    let mut value = 30.0;
    let mut draw = |ui: &mut egui::Ui| {
        daw_ui::number_stepper(ui, &mut value, 1.0, 30.0..=300.0, "Tempo", |ui, value| {
            ui.add(daw_ui::number(value).range(30.0..=300.0))
        });
    };
    let initial = frame(&ctx, &mut time, vec![], &mut draw);
    let pos = text_rect(&initial, "−").center();
    frame(&ctx, &mut time, vec![Event::PointerMoved(pos)], &mut draw);
    frame(&ctx, &mut time, vec![], &mut draw);
    let hover = frame(&ctx, &mut time, vec![], &mut draw);
    assert!(
        text_rect(
            &hover,
            "Cannot decrease Tempo by 1; allowed range is 30–300"
        )
        .is_positive()
    );
    frame(&ctx, &mut time, pointer(pos, true), &mut draw);
    frame(&ctx, &mut time, pointer(pos, false), &mut draw);
    assert_eq!(value, 30.0);
}

#[test]
fn toggle_group_help_does_not_collide_with_option_ids_or_block_selection() {
    let ctx = context(true, 1.0);
    let mut time = 0.0;
    let mut selected = 0;
    let mut draw = |ui: &mut egui::Ui| {
        daw_ui::toggle_group(ui, &mut selected, &[(0, "Linear"), (1, "Step")])
            .on_hover_text("Choose how values change between automation points");
    };
    let initial = frame(&ctx, &mut time, vec![], &mut draw);
    let first = text_rect(&initial, "Linear").center();
    frame(&ctx, &mut time, vec![Event::PointerMoved(first)], &mut draw);
    frame(&ctx, &mut time, vec![], &mut draw);
    let hover = frame(&ctx, &mut time, vec![], &mut draw);
    assert!(text_rect(&hover, "Choose how values change between automation points").is_positive());
    let second = text_rect(&initial, "Step").center();
    frame(&ctx, &mut time, pointer(second, true), &mut draw);
    frame(&ctx, &mut time, pointer(second, false), &mut draw);
    assert_eq!(selected, 1);
}
