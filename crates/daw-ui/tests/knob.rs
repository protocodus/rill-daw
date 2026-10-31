use daw_ui::{AutomationState, KnobGeometry};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Response, pos2, vec2};

#[derive(Clone, Copy)]
struct Options {
    diameter: f32,
    step: Option<f64>,
    enabled: bool,
    automation: Option<AutomationState>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            diameter: 96.0,
            step: None,
            enabled: true,
            automation: None,
        }
    }
}

fn frame(
    ctx: &Context,
    value: &mut f64,
    options: Options,
    modifiers: Modifiers,
    events: Vec<Event>,
) -> (Response, KnobGeometry, egui::FullOutput) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(440.0, 420.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.2),
            modifiers,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut knob = daw_ui::knob(value, 0.0..=100.0)
                    .text("Normalized value")
                    .suffix(" %")
                    .diameter(options.diameter)
                    .automation(options.automation);
                if let Some(step) = options.step {
                    knob = knob.step_by(step);
                }
                let response = ui.add_enabled(options.enabled, knob);
                let geometry = daw_ui::knob_geometry(ctx, response.id)
                    .expect("knob exposes the actual controls");
                result = Some((response, geometry));
            });
        },
    );
    let (response, geometry) = result.unwrap();
    (response, geometry, output)
}

fn pointer(
    position: Pos2,
    button: PointerButton,
    pressed: bool,
    modifiers: Modifiers,
) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers,
        },
    ]
}

fn labels(output: &egui::FullOutput) -> Vec<&str> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn arc_endpoints_midpoint_ticks_and_exact_number_agree_at_every_density() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            ctx.set_pixels_per_point(density);
            let mut value = 50.0;
            frame(
                &ctx,
                &mut value,
                Options::default(),
                Modifiers::NONE,
                vec![],
            );
            for (diameter, divisions) in [(48.0, 2), (84.0, 5), (140.0, 10)] {
                let options = Options {
                    diameter,
                    ..Default::default()
                };
                for actual in [0.0, 12.3456789, 50.0, 100.0] {
                    value = actual;
                    let (response, geometry, output) =
                        frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
                    assert_eq!(value, actual, "paint must not round the stored value");
                    assert_eq!(geometry.divisions, divisions);
                    assert_eq!(geometry.value, actual);
                    assert_eq!(geometry.range, [0.0, 100.0]);
                    let angle = std::f32::consts::PI * (0.75 + 1.5 * actual as f32 / 100.0);
                    let expected = geometry.center + egui::Vec2::angled(angle) * geometry.radius;
                    assert!(geometry.index.distance(expected) < 0.001);
                    let middle = geometry.center - vec2(0.0, geometry.radius);
                    assert!(
                        geometry
                            .ticks
                            .iter()
                            .any(|point| point.distance(middle) < 0.001)
                    );
                    assert!(geometry.dial.contains(geometry.minimum));
                    assert!(geometry.dial.contains(geometry.maximum));
                    for shape in &output.shapes {
                        if let egui::Shape::LineSegment { points, stroke } = &shape.shape
                            && stroke.width == 1.0
                        {
                            assert!(
                                points.iter().all(|point| geometry.dial.contains(*point)),
                                "scale ticks must fit inside the dial at compact sizes"
                            );
                        }
                    }
                    assert!(response.rect.contains_rect(geometry.number));
                    let viewport = Rect::from_min_size(Pos2::ZERO, vec2(440.0, 420.0));
                    assert!(viewport.contains_rect(response.rect));
                    assert!(
                        response.rect.width() <= diameter.max(180.0) + 0.1,
                        "rotary controls must not reserve the whole parent row: {:?}, numeric {:?}, diameter {}",
                        response.rect,
                        geometry.number,
                        diameter
                    );
                    let rendered = labels(&output);
                    assert!(rendered.contains(&"0 %"));
                    assert!(rendered.contains(&"100 %"));
                    if actual == 12.3456789 {
                        assert!(
                            rendered.iter().any(|label| label.contains("12.3456789")),
                            "{rendered:?}"
                        );
                    }
                    assert!(
                        output.shapes.iter().any(|shape| match &shape.shape {
                            egui::Shape::LineSegment { points, stroke } =>
                                stroke.width == 2.0 && points[1].distance(expected) < 0.001,
                            _ => false,
                        }),
                        "the visible index must point at the actual value"
                    );
                }
            }
        }
    }
}

#[test]
fn relative_drag_has_no_click_jump_and_fine_adjustment_does_not_rebase() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let options = Options::default();
    let mut value = 50.0;
    let (_, geometry, _) = frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
    let origin = geometry.minimum;
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(origin, PointerButton::Primary, true, Modifiers::NONE),
    );
    assert_eq!(
        value, 50.0,
        "pressing a different angle must not jump the value"
    );
    let moved = origin + vec2(20.0, 0.0);
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        vec![Event::PointerMoved(moved)],
    );
    assert!((value - 60.0).abs() < 1e-8, "{value}");
    let shift = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    let fine = moved + vec2(0.0, -20.0);
    frame(
        &ctx,
        &mut value,
        options,
        shift,
        vec![Event::PointerMoved(fine)],
    );
    assert!(
        (value - 61.0).abs() < 1e-8,
        "changing precision must continue from the current value: {value}"
    );
    frame(
        &ctx,
        &mut value,
        options,
        shift,
        pointer(fine, PointerButton::Primary, false, shift),
    );
    assert!((value - 61.0).abs() < 1e-8);
}

#[test]
fn exact_number_can_be_typed_and_committed_with_a_mouse_click() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 50.0;
    let options = Options::default();
    let (_, geometry, _) = frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
    let position = geometry.number.center();
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(position, PointerButton::Primary, true, Modifiers::NONE),
    );
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(position, PointerButton::Primary, false, Modifiers::NONE),
    );
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        vec![Event::Text("37.125".into())],
    );
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(
            pos2(350.0, 300.0),
            PointerButton::Primary,
            true,
            Modifiers::NONE,
        ),
    );
    let (_, geometry, _) = frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(
            pos2(350.0, 300.0),
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        ),
    );
    assert_eq!(value, 37.125);
    assert_eq!(geometry.value, 37.125);
}

#[test]
fn steps_disabled_controls_and_secondary_clicks_preserve_expected_values() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 37.1;
    let options = Options {
        step: Some(12.5),
        ..Default::default()
    };
    let (_, geometry, _) = frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
    assert_eq!(
        value, 37.1,
        "metadata step size cannot quantize during paint"
    );
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        frame(
            &ctx,
            &mut value,
            options,
            Modifiers::NONE,
            pointer(geometry.dial.center(), button, true, Modifiers::NONE),
        );
        frame(
            &ctx,
            &mut value,
            options,
            Modifiers::NONE,
            pointer(geometry.maximum, button, false, Modifiers::NONE),
        );
        assert_eq!(value, 37.1);
    }
    let disabled = Options {
        enabled: false,
        ..options
    };
    frame(
        &ctx,
        &mut value,
        disabled,
        Modifiers::NONE,
        pointer(
            geometry.dial.center(),
            PointerButton::Primary,
            true,
            Modifiers::NONE,
        ),
    );
    frame(
        &ctx,
        &mut value,
        disabled,
        Modifiers::NONE,
        vec![Event::PointerMoved(
            geometry.dial.center() + vec2(50.0, -50.0),
        )],
    );
    frame(
        &ctx,
        &mut value,
        disabled,
        Modifiers::NONE,
        pointer(
            geometry.maximum,
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        ),
    );
    assert_eq!(value, 37.1);
    let origin = geometry.dial.center();
    frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        pointer(origin, PointerButton::Primary, true, Modifiers::NONE),
    );
    frame(
        &ctx,
        &mut value,
        options,
        Modifiers::NONE,
        vec![Event::PointerMoved(origin + vec2(30.0, 0.0))],
    );
    assert_eq!(value, 50.0);
}

#[test]
fn automation_has_wording_and_color_and_outside_values_stay_exact() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    for state in [
        AutomationState::Read,
        AutomationState::Overridden,
        AutomationState::Bypassed,
    ] {
        let mut value = 140.25;
        let options = Options {
            automation: Some(state),
            ..Default::default()
        };
        let (response, geometry, output) =
            frame(&ctx, &mut value, options, Modifiers::NONE, vec![]);
        assert!(!response.changed());
        assert_eq!(value, 140.25);
        assert_eq!(geometry.value, 140.25);
        assert!(geometry.index.distance(geometry.maximum) < 0.001);
        let rendered = labels(&output);
        assert!(rendered.contains(&state.label()));
        assert!(rendered.contains(&"Outside range"));
        assert!(rendered.iter().any(|label| label.contains("140.25")));
        assert!(
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Path(path) =>
                    path.stroke.color == egui::epaint::ColorMode::Solid(state.color(&ctx)),
                _ => false,
            }),
            "automation must color the actual dial arc"
        );
    }
}

fn compact_frame(
    ctx: &Context,
    value: &mut f64,
    compact: bool,
    events: Vec<Event>,
) -> (Response, KnobGeometry, egui::FullOutput) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(440.0, 420.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.2),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut knob = daw_ui::knob(value, 0.0..=100.0)
                    .text("Normalized value")
                    .diameter(48.0)
                    .endpoint_labels("L", "R")
                    .automation(Some(AutomationState::Read));
                if compact {
                    knob = knob.compact();
                }
                let response = ui.add(knob);
                let geometry = daw_ui::knob_geometry(ctx, response.id).unwrap();
                result = Some((response, geometry));
            });
        },
    );
    let (response, geometry) = result.unwrap();
    (response, geometry, output)
}

#[test]
fn compact_knob_fits_the_dial_without_removing_the_default_numeric_editor() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 140.123_456_789;
    let (full_before, geometry_before, _) = compact_frame(&ctx, &mut value, false, vec![]);
    assert!(geometry_before.number.is_positive());
    assert!(full_before.rect.width() >= 112.0);
    assert!(full_before.rect.height() > 48.0);
    let (compact, geometry, output) = compact_frame(&ctx, &mut value, true, vec![]);
    assert_eq!(compact.rect.size(), vec2(48.0, 48.0));
    assert_eq!(geometry.dial, compact.rect);
    assert_eq!(geometry.number, Rect::NOTHING);
    assert_eq!(geometry.value, 140.123_456_789);
    assert_eq!(value, 140.123_456_789);
    assert!(geometry.index.distance(geometry.maximum) < 0.001);
    let rendered = labels(&output);
    assert!(rendered.contains(&"L") && rendered.contains(&"R"));
    for omitted in [
        "Normalized value",
        AutomationState::Read.label(),
        "Outside range",
    ] {
        assert!(
            !rendered.contains(&omitted),
            "compact presentation must not add {omitted} rows"
        );
    }
    let (full_after, geometry_after, output) = compact_frame(&ctx, &mut value, false, vec![]);
    assert_eq!(full_after.rect.size(), full_before.rect.size());
    assert_eq!(geometry_after.number.size(), geometry_before.number.size());
    let rendered = labels(&output);
    assert!(rendered.contains(&"Normalized value"));
    assert!(rendered.contains(&AutomationState::Read.label()));
    assert!(rendered.contains(&"Outside range"));
    assert!(rendered.iter().any(|label| label.contains("140.123456789")));
}

#[test]
fn compact_knob_leaves_unit_aware_hints_to_its_host() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let mut value = 37.125;
    let (_, geometry, _) = compact_frame(&ctx, &mut value, true, vec![]);
    compact_frame(
        &ctx,
        &mut value,
        true,
        vec![Event::PointerMoved(geometry.center)],
    );
    compact_frame(&ctx, &mut value, true, vec![]);
    let (_, _, output) = compact_frame(&ctx, &mut value, true, vec![]);
    let rendered = labels(&output);
    assert!(!rendered.iter().any(|text| text.contains("Drag up/right")));
    assert!(
        !rendered
            .iter()
            .any(|text| text.contains("Click the number"))
    );
    assert!(
        !rendered.iter().any(|text| text.contains("37.125")),
        "a generic normalized readout must not compete with the host's units"
    );
    assert_eq!(value, 37.125);
}

#[test]
fn secondary_or_middle_buttons_block_concurrent_primary_drag_through_release() {
    for compact in [false, true] {
        for button in [PointerButton::Secondary, PointerButton::Middle] {
            for concurrent_start in [false, true] {
                let ctx = Context::default();
                daw_ui::theme::apply(&ctx);
                let mut value = 50.0;
                let (_, geometry, _) = compact_frame(&ctx, &mut value, compact, vec![]);
                let origin = geometry.center;
                let mut events = pointer(origin, PointerButton::Primary, true, Modifiers::NONE);
                if concurrent_start {
                    events.extend(pointer(origin, button, true, Modifiers::NONE));
                }
                compact_frame(&ctx, &mut value, compact, events);
                let moved = origin + vec2(20.0, 0.0);
                compact_frame(&ctx, &mut value, compact, vec![Event::PointerMoved(moved)]);
                if concurrent_start {
                    assert_eq!(value, 50.0);
                } else {
                    assert_eq!(value, 60.0);
                    compact_frame(
                        &ctx,
                        &mut value,
                        compact,
                        pointer(moved, button, true, Modifiers::NONE),
                    );
                }
                let before = value;
                let held = moved + vec2(10.0, -10.0);
                let (response, _, _) =
                    compact_frame(&ctx, &mut value, compact, vec![Event::PointerMoved(held)]);
                assert_eq!(value, before);
                assert!(!response.changed());
                let released = held + vec2(10.0, -10.0);
                let (response, _, _) = compact_frame(
                    &ctx,
                    &mut value,
                    compact,
                    pointer(released, button, false, Modifiers::NONE),
                );
                assert_eq!(value, before, "the release frame must remain protected");
                assert!(!response.changed());
                compact_frame(
                    &ctx,
                    &mut value,
                    compact,
                    pointer(released, PointerButton::Primary, false, Modifiers::NONE),
                );
                assert_eq!(value, before);
            }
        }
    }
}
