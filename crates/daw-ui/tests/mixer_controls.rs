use daw_ui::{KnobGeometry, MixerMuteSoloResponse, SliderScaleGeometry};
use egui::{
    Context, Event, Key, Modifiers, PointerButton, RawInput, Rect, Response, SliderOrientation,
    pos2, vec2,
};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 640.0))),
        events,
        ..Default::default()
    }
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

fn key(key: Key, pressed: bool) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn fader_frame(
    ctx: &Context,
    value: &mut f32,
    enabled: bool,
    events: Vec<Event>,
) -> (Response, SliderScaleGeometry, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                response = Some(daw_ui::mixer_fader(ui, value, 240.0));
            });
        });
    });
    let response = response.unwrap();
    let geometry = daw_ui::slider_scale_geometry(ctx, response.id)
        .expect("mixer fader must retain the native slider's calibrated identity");
    (response, geometry, output)
}

fn pan_frame(
    ctx: &Context,
    value: &mut f32,
    enabled: bool,
    modifiers: Modifiers,
    events: Vec<Event>,
) -> (Response, KnobGeometry, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            modifiers,
            ..input(events)
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    response = Some(daw_ui::mixer_pan(ui, value, 180.0));
                });
            });
        },
    );
    let response = response.unwrap();
    let geometry = daw_ui::knob_geometry(ctx, response.id)
        .expect("pan row must retain the compact knob's calibrated identity");
    (response, geometry, output)
}

fn text_labels(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn mixer_fader_preserves_precise_and_out_of_range_values_when_inspected() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        for original in [0.0_f32, 0.000_000_000_123, 0.123_456_79, 1.0, 2.5, -1.25] {
            let mut value = original;
            for _ in 0..2 {
                let (response, geometry, _) = fader_frame(&ctx, &mut value, true, vec![]);
                assert_eq!(value.to_bits(), original.to_bits());
                assert!(!response.changed());
                assert_eq!(geometry.value, f64::from(original));
                assert_eq!(geometry.range, [0.0, 1.5]);
                assert_eq!(geometry.zero_reference, 1.0);
                assert_eq!(geometry.orientation, SliderOrientation::Vertical);
                assert!(geometry.rail.contains(geometry.minimum));
                assert!(geometry.rail.contains(geometry.maximum));
                assert!(geometry.rail.contains(geometry.handle));
                let fraction = (original / 1.5).clamp(0.0, 1.0);
                assert!(
                    geometry
                        .handle
                        .distance(geometry.minimum.lerp(geometry.maximum, fraction))
                        < 0.001
                );
            }
        }
    }
}

#[test]
fn mixer_fader_pointer_and_keyboard_edits_follow_native_handle_travel() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 0.2;
    let (_, initial, _) = fader_frame(&ctx, &mut value, true, vec![]);
    for fraction in [0.0, 0.5, 1.0] {
        let at = initial.minimum.lerp(initial.maximum, fraction);
        fader_frame(
            &ctx,
            &mut value,
            true,
            pointer(at, PointerButton::Primary, true),
        );
        let (_, geometry, _) = fader_frame(
            &ctx,
            &mut value,
            true,
            pointer(at, PointerButton::Primary, false),
        );
        assert!((value - fraction * 1.5).abs() < 0.00001);
        assert!(geometry.handle.distance(at) < 0.001);
    }
    value = 0.5;
    let (response, _, _) = fader_frame(&ctx, &mut value, true, vec![]);
    response.request_focus();
    let before = value;
    let (response, _, _) = fader_frame(&ctx, &mut value, true, key(Key::ArrowUp, true));
    assert!(response.changed());
    assert!(value > before && value < before + 0.1);
}

#[test]
fn mixer_fader_secondary_middle_and_disabled_input_never_edit_values() {
    for button in [
        PointerButton::Secondary,
        PointerButton::Middle,
        PointerButton::Primary,
    ] {
        let enabled = button != PointerButton::Primary;
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut value = 0.123_456_79_f32;
        let before = value;
        let (_, geometry, _) = fader_frame(&ctx, &mut value, enabled, vec![]);
        fader_frame(
            &ctx,
            &mut value,
            enabled,
            pointer(geometry.maximum, button, true),
        );
        let (response, _, _) = fader_frame(
            &ctx,
            &mut value,
            enabled,
            pointer(geometry.maximum, button, false),
        );
        assert_eq!(value.to_bits(), before.to_bits());
        assert!(!response.changed());
        if button == PointerButton::Secondary {
            assert!(
                response.secondary_clicked(),
                "native context menus must remain reachable"
            );
        }
        if !enabled {
            response.request_focus();
            let (response, _, _) = fader_frame(&ctx, &mut value, false, key(Key::ArrowRight, true));
            assert_eq!(value.to_bits(), before.to_bits());
            assert!(!response.changed());
        }
    }
}

#[test]
fn compact_pan_preserves_values_and_paints_its_real_index_in_a_bounded_row() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        for original in [
            -1.25_f32,
            -1.0,
            -0.123_456_79,
            0.0,
            0.000_000_000_123,
            0.123_456_79,
            1.0,
            1.25,
        ] {
            let mut value = original;
            let (response, geometry, output) =
                pan_frame(&ctx, &mut value, true, Modifiers::NONE, vec![]);
            assert_eq!(value.to_bits(), original.to_bits());
            assert!(!response.changed());
            assert_eq!(geometry.value, f64::from(original));
            assert_eq!(geometry.range, [-1.0, 1.0]);
            assert_eq!(geometry.number, Rect::NOTHING);
            assert_eq!(geometry.dial.size(), vec2(48.0, 48.0));
            assert!((response.rect.width() - 180.0).abs() < 0.1);
            assert!(
                (response.rect.height() - daw_ui::Knob::compact_height(48.0)).abs() < 0.1,
                "compact pan row must include its endpoint caption clearance: {:?}, dial {:?}",
                response.rect,
                geometry.dial
            );
            assert!(response.rect.contains_rect(geometry.dial));
            let fraction = ((original + 1.0) * 0.5).clamp(0.0, 1.0);
            let expected = geometry.center
                + egui::Vec2::angled(std::f32::consts::PI * (0.75 + 1.5 * fraction))
                    * geometry.radius;
            assert!(geometry.index.distance(expected) < 0.001);
            let center_reference = geometry.center - vec2(0.0, geometry.radius);
            assert!(
                geometry
                    .ticks
                    .iter()
                    .any(|tick| tick.distance(center_reference) < 0.001)
            );
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 2.0 && points[1].distance(geometry.index) < 0.001)),
                "visible index must end at the actual stored pan"
            );
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 1.0
                        && (points[0].x - geometry.center.x).abs() < 0.001
                        && (points[1].x - geometry.center.x).abs() < 0.001
                        && points.iter().all(|point| point.y < center_reference.y))),
                "fixed center tick stays visible away from center pan"
            );
            let labels = text_labels(&output);
            for caption in [
                "L".to_owned(),
                "R".to_owned(),
                format!("Pan {}", daw_ui::channel_pan_label(original)),
            ] {
                assert!(labels.contains(&caption), "missing {caption}: {labels:?}");
            }
        }
    }
}

#[test]
fn compact_pan_is_relative_with_no_click_jump_fine_drag_and_keyboard_steps() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 0.123_456_79_f32;
    let before = value;
    let (_, geometry, _) = pan_frame(&ctx, &mut value, true, Modifiers::NONE, vec![]);
    let origin = geometry.minimum;
    for pressed in [true, false] {
        let (response, _, _) = pan_frame(
            &ctx,
            &mut value,
            true,
            Modifiers::NONE,
            pointer(origin, PointerButton::Primary, pressed),
        );
        assert_eq!(
            value.to_bits(),
            before.to_bits(),
            "clicking a different angle must not jump pan"
        );
        assert!(!response.changed());
    }
    pan_frame(
        &ctx,
        &mut value,
        true,
        Modifiers::NONE,
        pointer(origin, PointerButton::Primary, true),
    );
    let moved = origin + vec2(20.0, 0.0);
    let (response, _, _) = pan_frame(
        &ctx,
        &mut value,
        true,
        Modifiers::NONE,
        vec![Event::PointerMoved(moved)],
    );
    assert!(response.changed());
    assert!((value - before - 0.2).abs() < 0.000001);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    let fine = moved + vec2(0.0, -20.0);
    pan_frame(
        &ctx,
        &mut value,
        true,
        shift,
        vec![Event::PointerMoved(fine)],
    );
    assert!(
        (value - before - 0.22).abs() < 0.000001,
        "fine movement must continue from the current pan"
    );
    let (response, _, _) = pan_frame(
        &ctx,
        &mut value,
        true,
        Modifiers::NONE,
        pointer(fine, PointerButton::Primary, false),
    );
    response.request_focus();
    let before = value;
    let (response, _, _) = pan_frame(
        &ctx,
        &mut value,
        true,
        Modifiers::NONE,
        key(Key::ArrowRight, true),
    );
    assert!(response.changed());
    assert!((value - before - 0.01).abs() < 0.000001);
}

#[test]
fn compact_pan_context_clicks_cover_the_caption_and_disabled_input_cannot_edit() {
    for enabled in [false, true] {
        for button in [
            PointerButton::Secondary,
            PointerButton::Middle,
            PointerButton::Primary,
        ] {
            if enabled && button == PointerButton::Primary {
                continue;
            }
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            let mut value = -0.123_456_79_f32;
            let before = value;
            let (response, geometry, output) =
                pan_frame(&ctx, &mut value, enabled, Modifiers::NONE, vec![]);
            let caption = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with("Pan ") => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .expect("pan paints a current caption");
            assert!(caption.left() > geometry.dial.right());
            let gap = pos2(
                (geometry.dial.right() + caption.left()) * 0.5,
                response.rect.center().y,
            );
            let edge = pos2(response.rect.right() - 2.0, response.rect.center().y);
            for target in [
                geometry.center,
                caption.center(),
                gap,
                edge,
                response.rect.center(),
            ] {
                pan_frame(
                    &ctx,
                    &mut value,
                    enabled,
                    Modifiers::NONE,
                    pointer(target, button, true),
                );
                let (response, _, _) = pan_frame(
                    &ctx,
                    &mut value,
                    enabled,
                    Modifiers::NONE,
                    pointer(target, button, false),
                );
                assert_eq!(value.to_bits(), before.to_bits());
                assert!(!response.changed());
                if enabled && button == PointerButton::Secondary {
                    assert!(
                        response.secondary_clicked(),
                        "whole row must expose the same pan context menu at {target:?}; caption {caption:?}"
                    );
                }
            }
            if !enabled {
                pan_frame(
                    &ctx,
                    &mut value,
                    false,
                    Modifiers::NONE,
                    pointer(geometry.center, PointerButton::Primary, true),
                );
                pan_frame(
                    &ctx,
                    &mut value,
                    false,
                    Modifiers::NONE,
                    vec![Event::PointerMoved(geometry.center + vec2(20.0, -20.0))],
                );
                let (response, _, _) = pan_frame(
                    &ctx,
                    &mut value,
                    false,
                    Modifiers::NONE,
                    pointer(
                        geometry.center + vec2(20.0, -20.0),
                        PointerButton::Primary,
                        false,
                    ),
                );
                response.request_focus();
                pan_frame(
                    &ctx,
                    &mut value,
                    false,
                    Modifiers::NONE,
                    key(Key::ArrowRight, true),
                );
                assert_eq!(value.to_bits(), before.to_bits());
            }
        }
    }
}

#[test]
fn compact_pan_hint_uses_pan_units_and_only_offers_visible_edit_paths() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let mut value = 0.25;
    let (_, geometry, _) = pan_frame(&ctx, &mut value, true, Modifiers::NONE, vec![]);
    pan_frame(
        &ctx,
        &mut value,
        true,
        Modifiers::NONE,
        vec![Event::PointerMoved(geometry.center)],
    );
    pan_frame(&ctx, &mut value, true, Modifiers::NONE, vec![]);
    let (_, _, output) = pan_frame(&ctx, &mut value, true, Modifiers::NONE, vec![]);
    let rendered = text_labels(&output);
    let hint = rendered
        .iter()
        .find(|text| text.contains("Stereo balance"))
        .expect("pan wrapper supplies a unit-aware editing hint");
    for wording in ["R25", "25%", "Drag up/right", "Shift", "Right-click"] {
        assert!(hint.contains(wording), "missing {wording}: {hint}");
    }
    assert!(!hint.contains("Click the number"));
    assert_eq!(value, 0.25);
}

fn mute_solo_frame(
    ctx: &Context,
    mute: &mut bool,
    solo: &mut bool,
    enabled: bool,
    events: Vec<Event>,
) -> (MixerMuteSoloResponse, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                response = Some(daw_ui::mixer_mute_solo(ui, mute, solo, 180.0));
            });
        });
    });
    (response.unwrap(), output)
}

#[test]
fn mute_and_solo_are_equal_sized_independent_native_buttons() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let (mut mute, mut solo) = (false, false);
    let (initial, output) = mute_solo_frame(&ctx, &mut mute, &mut solo, true, vec![]);
    assert!((initial.mute.rect.width() - initial.solo.rect.width()).abs() < 0.1);
    assert_eq!(initial.mute.rect.height(), daw_ui::tokens::CONTROL_HEIGHT);
    assert_eq!(initial.solo.rect.height(), daw_ui::tokens::CONTROL_HEIGHT);
    assert_eq!(initial.mute.rect.center().y, initial.solo.rect.center().y);
    assert!(initial.mute.rect.right() < initial.solo.rect.left());
    assert!(initial.solo.rect.right() - initial.mute.rect.left() <= 180.1);
    let labels = text_labels(&output);
    assert!(labels.iter().any(|label| label == "Mute"));
    assert!(labels.iter().any(|label| label == "Solo"));
    let at = initial.mute.rect.center();
    mute_solo_frame(
        &ctx,
        &mut mute,
        &mut solo,
        true,
        pointer(at, PointerButton::Primary, true),
    );
    let (response, _) = mute_solo_frame(
        &ctx,
        &mut mute,
        &mut solo,
        true,
        pointer(at, PointerButton::Primary, false),
    );
    assert_eq!((mute, solo), (true, false));
    assert!(response.mute.changed() && !response.solo.changed());
    response.solo.request_focus();
    let (response, _) = mute_solo_frame(&ctx, &mut mute, &mut solo, true, key(Key::Space, true));
    assert_eq!((mute, solo), (true, true));
    assert!(!response.mute.changed() && response.solo.changed());
    mute_solo_frame(&ctx, &mut mute, &mut solo, true, key(Key::Space, false));
    for target in [initial.mute.rect.center(), initial.solo.rect.center()] {
        for button in [
            PointerButton::Primary,
            PointerButton::Secondary,
            PointerButton::Middle,
        ] {
            mute_solo_frame(
                &ctx,
                &mut mute,
                &mut solo,
                false,
                pointer(target, button, true),
            );
            let (response, _) = mute_solo_frame(
                &ctx,
                &mut mute,
                &mut solo,
                false,
                pointer(target, button, false),
            );
            assert_eq!((mute, solo), (true, true));
            assert!(!response.mute.changed() && !response.solo.changed());
        }
    }
}

#[test]
fn level_meter_has_a_bounded_captioned_footprint_and_sanitizes_nonfinite_input() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let _ = ctx.run(input(vec![]), |_| {});
        for peaks in [
            [0.0, 0.0],
            [f32::NAN, f32::INFINITY],
            [f32::NEG_INFINITY, -1.0],
            [0.01, 1.2],
        ] {
            let mut response = None;
            let output = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(daw_ui::mixer_level_meter(ui, peaks, 180.0));
                });
            });
            let response = response.unwrap();
            assert_eq!(
                response.rect.size(),
                vec2(180.0, daw_ui::MIXER_LEVEL_METER_HEIGHT)
            );
            assert_eq!(response.sense, egui::Sense::hover());
            assert!(!response.changed());
            let labels = text_labels(&output);
            for caption in ["Level", "dBFS"] {
                assert!(
                    labels.iter().any(|label| label == caption),
                    "missing {caption}: {labels:?}"
                );
            }
            let caption_rects: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((
                            text.galley.text(),
                            text.galley.rect.translate(text.pos.to_vec2()),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            for (index, (caption, bounds)) in caption_rects.iter().enumerate() {
                assert!(
                    response.rect.expand(1.0 / density).contains_rect(*bounds),
                    "Meter caption {caption} escaped its allocated height: {bounds:?}"
                );
                for (other, other_bounds) in &caption_rects[..index] {
                    assert!(
                        !bounds.intersects(*other_bounds),
                        "Meter captions {caption} and {other} overlap"
                    );
                }
            }
            for shape in &output.shapes {
                if !matches!(shape.shape, egui::Shape::Noop) {
                    assert!(shape.shape.visual_bounding_rect().is_finite());
                }
            }
        }
    }
}

#[test]
fn level_meter_uses_decibel_travel_and_never_fabricates_or_overflows_signal() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let colors = daw_ui::theme::palette(&ctx);
        for (peaks, expected) in [
            ([10.0_f32.powf(-30.0 / 20.0), 1.0], [0.5, 1.0]),
            ([10.0_f32.powf(-45.0 / 20.0), 1.2], [0.25, 1.0]),
            ([0.0, 0.0], [0.0, 0.0]),
            ([f32::NAN, f32::INFINITY], [0.0, 0.0]),
            ([f32::NEG_INFINITY, -1.0], [0.0, 0.0]),
        ] {
            let mut response = None;
            let output = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(daw_ui::mixer_level_meter(ui, peaks, 180.0));
                });
            });
            let bounds = response.unwrap().rect;
            let rails: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == colors.bg
                            && rect.rect.height() <= 8.0
                            && bounds.contains_rect(rect.rect) =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                rails.len(),
                2,
                "stereo channels need separate calibrated rails"
            );
            let fills: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == colors.accent || rect.fill == colors.orange =>
                    {
                        Some(rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                fills.len(),
                expected.into_iter().filter(|level| *level > 0.0).count()
            );
            for (index, rail) in rails.into_iter().enumerate() {
                let fill = fills
                    .iter()
                    .find(|fill| fill.rect.center().y == rail.center().y);
                if expected[index] == 0.0 {
                    assert!(
                        fill.is_none(),
                        "silence and invalid readings must paint no signal"
                    );
                } else {
                    let fill = fill.expect("a finite audible peak needs a signal fill");
                    assert!(rail.contains_rect(fill.rect));
                    assert_eq!(fill.rect.left(), rail.left());
                    assert!((fill.rect.width() / rail.width() - expected[index]).abs() < 0.0001);
                    assert_eq!(
                        fill.fill,
                        if peaks[index] >= 1.0 {
                            colors.orange
                        } else {
                            colors.accent
                        }
                    );
                }
            }
            let labels = text_labels(&output);
            for caption in ["−60", "−30", "0"] {
                assert!(
                    labels.iter().any(|label| label == caption),
                    "missing scale reference {caption}"
                );
            }
        }
    }
}

#[test]
fn mixer_skin_does_not_change_neighboring_shared_slider_geometry() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut geometries = Vec::new();
    let mut values = [0.25_f32; 3];
    let _ = ctx.run(input(vec![]), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().slider_width = 180.0;
            for (index, value) in values.iter_mut().enumerate() {
                let mut slider = daw_ui::slider(value, -1.0..=1.0)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Edits);
                if index == 1 {
                    slider = slider.mixer_style();
                }
                let response = ui.add(slider);
                geometries.push(daw_ui::slider_scale_geometry(ctx, response.id).unwrap());
            }
        });
    });
    assert_eq!(values, [0.25; 3]);
    assert_eq!(geometries[0].rail.size(), geometries[2].rail.size());
    assert_eq!(
        geometries[0].handle_half_extent,
        geometries[2].handle_half_extent
    );
    assert_eq!(
        geometries[0].maximum - geometries[0].minimum,
        geometries[2].maximum - geometries[2].minimum,
    );
    assert!(
        geometries
            .iter()
            .all(|geometry| geometry.rail.contains(geometry.handle))
    );
}
