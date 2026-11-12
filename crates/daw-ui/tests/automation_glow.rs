use daw_ui::{AutomationState, paint_automation_control, theme};
use egui::{Context, RawInput, Rect, Sense, Shape, pos2, vec2};

#[test]
fn manual_override_palette_is_red_in_both_themes() {
    for color in [
        theme::DARK.automation_override,
        theme::LIGHT.automation_override,
    ] {
        let (r, g, b) = (
            i32::from(color.r()),
            i32::from(color.g()),
            i32::from(color.b()),
        );
        assert!(
            r > g && r > b && (g - b).abs() * 4 < r - g.min(b),
            "Manual takeover must be visibly red rather than another orange: {color:?}"
        );
    }
}

#[test]
fn automation_ownership_glows_orange_and_manual_takeover_glows_red() {
    for dark in [false, true] {
        for density in [1.0, 1.25, 2.0] {
            for state in [
                AutomationState::Read,
                AutomationState::Overridden,
                AutomationState::Bypassed,
            ] {
                let ctx = Context::default();
                theme::apply(&ctx);
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                ctx.set_pixels_per_point(density);
                let mut target = Rect::NOTHING;
                let mut orange = egui::Color32::TRANSPARENT;
                let output = ctx.run(RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let colors = theme::palette(ctx);
                        orange = if state == AutomationState::Overridden {
                            colors.automation_override
                        } else {
                            colors.automation
                        };
                        target = Rect::from_min_size(pos2(30.0, 30.0), vec2(140.0, 30.0));
                        let response =
                            ui.interact(target, ui.id().with("automated"), Sense::click_and_drag());
                        paint_automation_control(ui, &response, state);
                    });
                });
                let outlines: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        Shape::Rect(rect)
                            if rect.stroke.width > 0.0
                                && rect.rect.expand(0.6).contains_rect(target) =>
                        {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert!(
                    outlines.iter().any(|rect| rect.stroke.color == orange),
                    "Missing ownership border in {state:?}"
                );
                assert!(
                    outlines
                        .iter()
                        .filter(
                            |rect| rect.stroke.color.a() > 0 && rect.stroke.color.a() < orange.a()
                        )
                        .count()
                        >= 2,
                    "The glow needs two low-opacity rings, not a solid block"
                );
            }
        }
    }
}

fn shape_colors(shape: &Shape, out: &mut Vec<egui::Color32>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_colors(shape, out)),
        Shape::Rect(rect) => out.extend([rect.fill, rect.stroke.color]),
        Shape::LineSegment { stroke, .. } => out.push(stroke.color),
        Shape::Path(path) => out.push(path.fill),
        Shape::Circle(circle) => out.extend([circle.fill, circle.stroke.color]),
        _ => {}
    }
}

#[test]
fn an_overridden_choice_turns_red_and_ghosts_its_lane_in_orange() {
    for dark in [false, true] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let mut leaked = true;
        let output = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mark = daw_ui::AutomationMark {
                    state: AutomationState::Overridden,
                    lane: Some(0.0),
                };
                let mut current = 1;
                daw_ui::automated(ui, Some(mark), |ui| {
                    daw_ui::toggle_group(ui, &mut current, &[(0, "Clean"), (1, "Tube")]);
                });
                leaked = daw_ui::automation_mark(ui).is_some();
            });
        });
        assert!(!leaked, "the mark stays inside its scope");
        let mut colors = Vec::new();
        for shape in &output.shapes {
            shape_colors(&shape.shape, &mut colors);
        }
        let palette = if dark { theme::DARK } else { theme::LIGHT };
        assert!(
            colors.contains(&palette.automation_override),
            "the chosen option is red"
        );
        assert!(
            colors.contains(&palette.automation),
            "the lane's option is outlined in orange"
        );
    }
}

#[test]
fn an_automated_variation_stepper_shows_its_state_on_the_track() {
    let ctx = Context::default();
    theme::apply(&ctx);
    ctx.set_theme(egui::Theme::Dark);
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mark = daw_ui::AutomationMark {
                state: AutomationState::Read,
                lane: Some(0.5),
            };
            let mut seed = 7;
            daw_ui::automated(ui, Some(mark), |ui| {
                daw_ui::variation_stepper(ui, &mut seed, 1..=32, "seed");
            });
        });
    });
    let mut colors = Vec::new();
    for shape in &output.shapes {
        shape_colors(&shape.shape, &mut colors);
    }
    assert!(
        colors.contains(&theme::DARK.automation),
        "the stepper's track is outlined in the lane's color"
    );
}
