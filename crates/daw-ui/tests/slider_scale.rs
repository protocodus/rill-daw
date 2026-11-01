use daw_ui::{SliderScaleGeometry, slider_scale_geometry};
use egui::{
    Context, Event, Modifiers, PointerButton, RawInput, Rect, Response, SliderOrientation, pos2,
    vec2,
};

#[test]
fn always_clamps_to_exact_boundaries_but_only_quantizes_intentional_edits() {
    let minimum = 0.123456789_f64;
    let maximum = 1.23456789_f64;
    for (stored, expected, changed) in [
        (0.23456789, 0.23456789, false),
        (0.0, minimum, true),
        (2.0, maximum, true),
    ] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut value = stored;
        let mut edited = false;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                edited = ui
                    .add(
                        daw_ui::slider(&mut value, minimum..=maximum)
                            .clamping(egui::SliderClamping::Always)
                            .step_by(0.1)
                            .max_decimals(3),
                    )
                    .changed();
            });
        });
        assert_eq!(
            value.to_bits(),
            expected.to_bits(),
            "paint must not quantize an existing fraction or an exact range endpoint"
        );
        assert_eq!(edited, changed);
    }
}

#[test]
fn tiny_ranges_keep_distinct_readable_endpoints_without_changing_values() {
    for (minimum, maximum, value, captions) in [
        (0.0001, 0.0009, 0.0005, ["0.0001", "0.0009"]),
        (1e-20, 9e-20, 5e-20, ["1e-20", "9e-20"]),
    ] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut current = value;
        let (_, output) = number_frame(&ctx, &mut current, minimum..=maximum, false, vec![]);
        assert_eq!(current, value);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        for caption in captions {
            assert!(
                labels.iter().any(|text| text == caption),
                "missing {caption}: {labels:?}"
            );
        }
    }
}

#[test]
fn endpoint_precision_trims_only_unnecessary_zeros() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 0.5123456_f64;
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(
                daw_ui::slider(&mut value, 0.0..=16.0)
                    .clamping(egui::SliderClamping::Edits)
                    .min_decimals(2)
                    .max_decimals(6)
                    .suffix("×"),
            );
        });
    });
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    assert!(labels.iter().any(|text| text == "0.00×"), "{labels:?}");
    assert!(labels.iter().any(|text| text == "16.00×"), "{labels:?}");
    assert_eq!(
        value, 0.5123456,
        "caption formatting must not round the parameter"
    );
}

#[test]
fn out_of_range_values_have_visible_direction_markers_and_remain_unchanged() {
    for vertical in [false, true] {
        for (stored, caption) in [(-2.0, "< -1"), (2.0, "> 1")] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            let mut value = stored;
            let (_, geometry, output) =
                frame(&ctx, &mut value, 200.0, vertical, false, true, vec![]);
            assert_eq!(value, stored);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == caption)));
            let orange = daw_ui::theme::palette(&ctx).orange;
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Path(path) if path.closed && path.points.len() == 3 && path.fill == orange
                    && path.points.iter().all(|point| geometry.rail.contains(*point)))));
        }
    }
}

#[test]
fn slider_automation_glyphs_are_distinct_without_covering_handle_index_or_captions() {
    for vertical in [false, true] {
        for (state, expected) in [
            (daw_ui::AutomationState::Read, "A"),
            (daw_ui::AutomationState::Overridden, "!"),
            (daw_ui::AutomationState::Bypassed, "–"),
        ] {
            for value in [-1.0_f32, 0.0, 1.0] {
                let ctx = Context::default();
                daw_ui::theme::apply(&ctx);
                let mut value = value;
                let mut geometry = None;
                let output = ctx.run(RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.spacing_mut().slider_width = 150.0;
                        let mut slider = daw_ui::slider(&mut value, -1.0..=1.0).show_value(false);
                        if vertical {
                            slider = slider.vertical();
                        }
                        let response = ui.add(slider);
                        geometry = daw_ui::slider_scale_geometry(ctx, response.id);
                        daw_ui::paint_automation_control(ui, &response, state);
                    });
                });
                let geometry = geometry.unwrap();
                let marker = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == expected => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .expect("automation must be identifiable without color");
                assert!(
                    geometry.rail.expand(0.1).contains_rect(marker),
                    "{marker:?} {:?}",
                    geometry.rail
                );
                assert!(
                    !marker.contains(geometry.handle),
                    "glyph must leave the index visible"
                );
            }
        }
    }
}

fn frame(
    ctx: &Context,
    value: &mut f32,
    width: f32,
    vertical: bool,
    compact: bool,
    enabled: bool,
    events: Vec<Event>,
) -> (Response, SliderScaleGeometry, egui::FullOutput) {
    let mut response = None;
    let mut geometry = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 800.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().slider_width = width;
                ui.add_enabled_ui(enabled, |ui| {
                    let mut slider = daw_ui::slider(value, -1.0..=1.0)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Edits)
                        .smart_aim(false);
                    if vertical {
                        slider = slider.vertical();
                    }
                    if compact {
                        slider = slider.compact();
                    }
                    let rendered = ui.add(slider);
                    geometry = slider_scale_geometry(ctx, rendered.id);
                    response = Some(rendered);
                });
            });
        },
    );
    (response.unwrap(), geometry.unwrap(), output)
}
fn pointer(position: egui::Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}
#[test]
fn adaptive_scale_uses_native_handle_centers_and_keeps_midpoint_at_every_density() {
    for dark in [false, true] {
        for vertical in [false, true] {
            for compact in [false, true] {
                let ctx = Context::default();
                daw_ui::theme::apply(&ctx);
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let mut value = 0.0;
                for (width, expected) in [(75.0, 2), (150.0, 5), (280.0, 10)] {
                    let (response, geometry, output) =
                        frame(&ctx, &mut value, width, vertical, compact, true, vec![]);
                    assert_eq!(geometry.divisions, expected);
                    if !vertical {
                        assert!(response.rect.width() <= width + 0.1);
                    }
                    assert_eq!(
                        geometry.orientation,
                        if vertical {
                            SliderOrientation::Vertical
                        } else {
                            SliderOrientation::Horizontal
                        }
                    );
                    let middle = geometry
                        .ticks
                        .iter()
                        .find(|tick| tick.fraction == 0.5)
                        .unwrap();
                    assert!(middle.center.distance(geometry.handle) < 0.001);
                    assert!(geometry.rail.contains(geometry.minimum));
                    assert!(geometry.rail.contains(geometry.maximum));
                    assert!(
                        geometry
                            .ticks
                            .iter()
                            .all(|tick| geometry.rail.contains(tick.center))
                    );
                    let zero = output.shapes.iter().any(|shape| match shape.shape {
                        egui::Shape::Circle(circle) => {
                            circle.fill == egui::Color32::TRANSPARENT
                                && circle.stroke.width > 0.0
                                && circle.center.distance(geometry.handle) < 0.001
                        }
                        _ => false,
                    });
                    assert!(zero, "zero circle must actually be painted at native value");
                    for shape in &output.shapes {
                        if let egui::Shape::LineSegment { points, stroke } = shape.shape
                            && stroke.width == 1.0
                        {
                            assert!(
                                points[0].distance(points[1]) >= 2.49,
                                "Ticks must remain visible lines at every rail thickness: {points:?}"
                            );
                        }
                    }
                    if !vertical {
                        assert!(
                            response.rect.height()
                                <= if compact {
                                    24.1
                                } else {
                                    daw_ui::tokens::PARAMETER_ROW_HEIGHT + 0.1
                                },
                            "{:?}",
                            response.rect
                        );
                    }
                    assert_eq!(value, 0.0);
                }
            }
        }
    }
}
#[test]
fn exact_tick_clicks_track_actual_values_and_secondary_buttons_do_not_edit() {
    for vertical in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut value = 0.0;
        let (_, geometry, _) = frame(&ctx, &mut value, 280.0, vertical, false, true, vec![]);
        for fraction in [0.0, 0.2, 0.5, 0.8, 1.0] {
            let position = geometry.minimum.lerp(geometry.maximum, fraction);
            frame(
                &ctx,
                &mut value,
                280.0,
                vertical,
                false,
                true,
                pointer(position, PointerButton::Primary, true),
            );
            let (_, after, _) = frame(
                &ctx,
                &mut value,
                280.0,
                vertical,
                false,
                true,
                pointer(position, PointerButton::Primary, false),
            );
            assert!(
                (value - (-1.0 + 2.0 * fraction)).abs() < 0.00001,
                "fraction{fraction},value{value}"
            );
            assert!(after.handle.distance(position) < 0.001);
        }
        for button in [PointerButton::Secondary, PointerButton::Middle] {
            let before = value;
            frame(
                &ctx,
                &mut value,
                280.0,
                vertical,
                false,
                true,
                pointer(geometry.minimum, button, true),
            );
            let (response, _, _) = frame(
                &ctx,
                &mut value,
                280.0,
                vertical,
                false,
                true,
                pointer(geometry.minimum, button, false),
            );
            assert_eq!(value, before);
            assert!(!response.changed());
            if button == PointerButton::Secondary {
                assert!(
                    response.secondary_clicked(),
                    "right click must reach automation menus"
                );
            }
        }
        let before = value;
        frame(
            &ctx,
            &mut value,
            280.0,
            vertical,
            false,
            false,
            pointer(geometry.minimum, PointerButton::Primary, true),
        );
        frame(
            &ctx,
            &mut value,
            280.0,
            vertical,
            false,
            false,
            pointer(geometry.minimum, PointerButton::Primary, false),
        );
        assert_eq!(value, before);
    }
}
#[test]
fn keyboard_and_out_of_range_readouts_keep_native_parameter_semantics() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 1.5;
    let (response, geometry, _) = frame(&ctx, &mut value, 280.0, false, false, true, vec![]);
    assert_eq!(value, 1.5);
    assert_eq!(geometry.handle, geometry.maximum);
    value = 0.0;
    response.request_focus();
    let event = Event::Key {
        key: egui::Key::ArrowRight,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let (response, _, _) = frame(&ctx, &mut value, 280.0, false, false, true, vec![event]);
    assert!(response.changed());
    assert!(value > 0.0 && value < 0.1);
}

#[test]
fn compact_captions_fit_and_the_entire_caption_is_a_mouse_target() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 0.0;
    let (response, geometry, _) = frame(&ctx, &mut value, 150.0, false, true, true, vec![]);
    let position = pos2(geometry.maximum.x, response.rect.bottom() - 2.0);
    assert!(position.y > geometry.rail.bottom());
    frame(
        &ctx,
        &mut value,
        150.0,
        false,
        true,
        true,
        pointer(position, PointerButton::Primary, true),
    );
    frame(
        &ctx,
        &mut value,
        150.0,
        false,
        true,
        true,
        pointer(position, PointerButton::Primary, false),
    );
    assert_eq!(
        value, 1.0,
        "clicking a caption edits at its horizontal coordinate"
    );

    for height in [20.0, 24.0] {
        let mut response = None;
        let output = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(74.0, height));
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Min)),
                );
                child.spacing_mut().slider_width = 74.0;
                response = Some(
                    child.add(
                        daw_ui::slider(&mut value, -1.0..=1.0)
                            .compact()
                            .show_value(false)
                            .endpoint_labels("−∞", "+3.5")
                            .current_label("−25"),
                    ),
                );
            });
        });
        let response = response.unwrap();
        for shape in &output.shapes {
            if let egui::Shape::LineSegment { points, stroke } = shape.shape
                && stroke.width == 1.0
            {
                assert!(
                    points[0].distance(points[1]) >= 2.49,
                    "Compact tick must not become a dot: {points:?}"
                );
            }
        }
        assert!(
            response.rect.height() <= height + 0.1,
            "height {height}: {:?}",
            response.rect
        );
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut bounds = Vec::new();
        for label in ["−∞", "−25", "+3.5"] {
            let rect = labels
                .iter()
                .find(|(text, _)| text == label)
                .unwrap_or_else(|| panic!("missing {label}: {labels:?}"))
                .1;
            assert!(
                response.rect.expand(0.1).contains_rect(rect),
                "{label}: {rect:?} in {:?}",
                response.rect
            );
            bounds.push(rect);
        }
        assert!(bounds[0].right() <= bounds[1].left());
        assert!(bounds[1].right() <= bounds[2].left());
    }
}

fn number_frame<T: egui::emath::Numeric>(
    ctx: &Context,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    slope: bool,
    events: Vec<Event>,
) -> (Response, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 300.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut slider =
                    daw_ui::slider(value, range.clone()).clamping(egui::SliderClamping::Edits);
                if slope {
                    slider = slider.step_by(0.01).fixed_decimals(2);
                }
                response = Some(ui.add(slider));
            });
        },
    );
    (response.unwrap(), output)
}

#[test]
fn exact_number_entry_rounds_once_before_updating_the_model() {
    for integral in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut fractional = 0.0_f64;
        let mut integer = 64_i32;
        let mut render = |events| {
            if integral {
                number_frame(&ctx, &mut integer, 1..=127, false, events)
            } else {
                number_frame(&ctx, &mut fractional, -1.0..=1.0, true, events)
            }
        };
        render(vec![]);
        let (_, output) = render(vec![]);
        let numeric = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    let label = text.galley.text();
                    ((integral && label == "64") || (!integral && label == "0.00"))
                        .then(|| text.galley.rect.translate(text.pos.to_vec2()).center())
                } else {
                    None
                }
            })
            .unwrap();
        render(pointer(numeric, PointerButton::Primary, true));
        render(pointer(numeric, PointerButton::Primary, false));
        let command = Modifiers {
            command: true,
            ctrl: true,
            ..Modifiers::NONE
        };
        render(vec![Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: command,
        }]);
        let (edited, _) = render(vec![Event::Text(
            if integral { "64.9" } else { "0.123" }.into(),
        )]);
        assert!(edited.changed());
        render(vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        let (settled, _) = render(vec![]);
        assert!(
            !settled.changed(),
            "rounding must not generate a second model edit on next paint"
        );
        if integral {
            assert_eq!(integer, 65);
        } else {
            assert_eq!(fractional, 0.12);
        }
    }
}
