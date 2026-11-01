use daw_ui::{ViewBackdrop, theme};
use egui::{Context, Event, FullOutput, PointerButton, Pos2, RawInput, Rect, vec2};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events,
        ..Default::default()
    }
}

fn pointer(point: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(point),
        Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}

fn dim_index(output: &FullOutput, ctx: &Context, scrim: egui::Color32) -> usize {
    let indices: Vec<_> = output
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == scrim && rect.rect == ctx.content_rect() => {
                Some(index)
            }
            _ => None,
        })
        .collect();
    assert_eq!(indices.len(), 1, "exactly one full-viewport dim layer");
    indices[0]
}

fn text(output: &FullOutput, label: &str) -> (usize, Rect) {
    output
        .shapes
        .iter()
        .enumerate()
        .find_map(|(index, shape)| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some((index, Rect::from_min_size(text.pos, text.galley.size())))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible text: {label}"))
}

#[test]
fn utility_dimming_preserves_foreground_and_never_enters_retained_snapshots() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            ctx.style_mut(|style| style.animation_time = 0.0);
            let mut retained = ViewBackdrop::default();
            for frame in 0..4 {
                let output = ctx.run(input(vec![]), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.label("Caller context");
                    });
                    daw_ui::popup("Active utility", ctx).show(ctx, |ui| {
                        daw_ui::button(ui, "Foreground action");
                    });
                    let original = ViewBackdrop::capture(ctx, ctx.content_rect());
                    let nested = ViewBackdrop::capture_overlay(ctx, ctx.content_rect(), &retained);
                    daw_ui::paint_popup_backdrop(ctx, true);
                    let captured = ViewBackdrop::capture(ctx, ctx.content_rect());
                    let captured_nested =
                        ViewBackdrop::capture_overlay(ctx, ctx.content_rect(), &retained);
                    assert_eq!(captured.shape_count(), original.shape_count());
                    assert_eq!(captured_nested.shape_count(), nested.shape_count());
                    retained = captured_nested;
                });
                if frame > 1 {
                    let dim = dim_index(&output, &ctx, daw_ui::popup_backdrop_color(&ctx));
                    assert!(text(&output, "Caller context").0 < dim);
                    assert!(text(&output, "Foreground action").0 > dim);
                }
            }
            let output = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Restored workspace");
                });
                daw_ui::paint_popup_backdrop(ctx, false);
            });
            assert!(!output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(rect)
                    if rect.fill == daw_ui::popup_backdrop_color(&ctx))
            }));
        }
    }
}

#[test]
fn native_menu_hierarchy_stays_above_one_backdrop_and_accepts_clicks() {
    let ctx = Context::default();
    theme::apply(&ctx);
    ctx.style_mut(|style| style.animation_time = 0.0);
    let mut chosen = false;
    let mut parent_clicked = false;
    let mut draw = |events| {
        ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Caller context");
            });
            daw_ui::popup("Active utility", ctx).show(ctx, |ui| {
                parent_clicked |= daw_ui::button(ui, "Parent action").clicked();
                daw_ui::menu_button(ui, "Options", |ui| {
                    ui.label("Visible menu");
                    daw_ui::menu_button(ui, "Nested", |ui| {
                        if daw_ui::button(ui, "Apply selection").clicked() {
                            chosen = true;
                            ui.close();
                        }
                    });
                });
            });
            daw_ui::paint_popup_backdrop(ctx, true);
        })
    };
    let mut output = draw(vec![]);
    for _ in 0..3 {
        output = draw(vec![]);
    }
    let options = text(&output, "Options").1.center();
    for pressed in [true, false] {
        draw(pointer(options, pressed));
    }
    output = draw(vec![]);
    let dim = dim_index(&output, &ctx, daw_ui::popup_backdrop_color(&ctx));
    assert!(text(&output, "Parent action").0 < dim);
    assert!(text(&output, "Visible menu").0 > dim);
    let nested = text(&output, "Nested").1.center();
    for pressed in [true, false] {
        draw(pointer(nested, pressed));
    }
    for _ in 0..3 {
        output = draw(vec![]);
    }
    let dim = dim_index(&output, &ctx, daw_ui::popup_backdrop_color(&ctx));
    assert!(text(&output, "Visible menu").0 > dim);
    assert!(text(&output, "Apply selection").0 > dim);
    let selection = text(&output, "Apply selection").1.center();
    for pressed in [true, false] {
        draw(pointer(selection, pressed));
    }
    output = draw(vec![]);
    assert!(!egui::Popup::is_any_open(&ctx));
    let dim = dim_index(&output, &ctx, daw_ui::popup_backdrop_color(&ctx));
    assert!(text(&output, "Parent action").0 > dim);
    let parent = text(&output, "Parent action").1.center();
    for pressed in [true, false] {
        draw(pointer(parent, pressed));
    }
    assert!(chosen, "dim paint must not intercept the submenu action");
    assert!(parent_clicked, "closing a menu must restore parent input");
}

#[test]
fn native_confirmation_uses_the_shared_dim_color_and_keeps_its_buttons_active() {
    let ctx = Context::default();
    theme::apply(&ctx);
    ctx.style_mut(|style| style.animation_time = 0.0);
    let mut accepted = false;
    let mut draw = |events| {
        ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Caller context");
            });
            daw_ui::modal(ctx, egui::Id::new("confirm"), "Confirm", |ui| {
                accepted |= daw_ui::button(ui, "Continue").clicked();
            });
        })
    };
    let mut output = draw(vec![]);
    for _ in 0..3 {
        output = draw(vec![]);
    }
    let dim = dim_index(&output, &ctx, theme::palette(&ctx).popup_scrim);
    assert!(text(&output, "Caller context").0 < dim);
    assert!(text(&output, "Continue").0 > dim);
    let point = text(&output, "Continue").1.center();
    for pressed in [true, false] {
        draw(pointer(point, pressed));
    }
    assert!(accepted);
}
