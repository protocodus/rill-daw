use daw_ui::{MixerMuteSoloResponse, SliderScaleGeometry};
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
    events: Vec<Event>,
) -> (Response, SliderScaleGeometry, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                response = Some(daw_ui::mixer_pan(ui, value, 180.0));
            });
        });
    });
    let response = response.unwrap();
    let geometry = daw_ui::slider_scale_geometry(ctx, response.id)
        .expect("pan row must retain the rail's calibrated identity");
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
                let travel = daw_ui::gain_taper(original);
                assert_eq!(geometry.value, f64::from(travel));
                assert_eq!(geometry.range, [0.0, 1.0]);
                assert_eq!(geometry.zero_reference, 0.75);
                assert!(geometry.custom_scale);
                assert_eq!(geometry.divisions, 0);
                assert_eq!(geometry.orientation, SliderOrientation::Vertical);
                assert!(geometry.rail.contains(geometry.minimum));
                assert!(geometry.rail.contains(geometry.maximum));
                assert!(geometry.rail.contains(geometry.handle));
                let fraction = travel.clamp(0.0, 1.0);
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
        assert!((value - daw_ui::gain_untaper(fraction)).abs() < 0.00001);
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
fn pan_rail_preserves_values_and_shows_detents_and_a_short_value() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        for original in [-1.25_f32, -1.0, -0.123_456_79, 0.0, 0.004, 0.5, 1.0, 1.25] {
            let mut value = original;
            let (response, geometry, output) = pan_frame(&ctx, &mut value, true, vec![]);
            assert_eq!(value.to_bits(), original.to_bits());
            assert!(!response.changed());
            assert_eq!(geometry.value, f64::from(original));
            assert_eq!(geometry.range, [-1.0, 1.0]);
            assert_eq!(geometry.orientation, SliderOrientation::Horizontal);
            assert!(geometry.custom_scale);
            assert_eq!(
                geometry
                    .ticks
                    .iter()
                    .map(|tick| tick.fraction)
                    .collect::<Vec<_>>(),
                [0.5]
            );
            assert!((response.rect.width() - 180.0).abs() < 0.1);
            assert!(
                (response.rect.height() - daw_ui::tokens::COMPACT_HEIGHT).abs() < 0.1,
                "the pan row is one compact row: {:?}",
                response.rect
            );
            let caption = daw_ui::mixer_pan_caption(original);
            let labels = text_labels(&output);
            assert!(labels.contains(&caption), "missing {caption}: {labels:?}");
            let caption_rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == caption => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap();
            assert!(caption_rect.left() >= geometry.rail.right());
            assert!(
                response.rect.expand(1.0).contains_rect(caption_rect),
                "{caption_rect:?} escapes {:?}",
                response.rect
            );
        }
    }
    assert_eq!(daw_ui::mixer_pan_caption(0.004), "C");
    assert_eq!(daw_ui::mixer_pan_caption(-0.15), "L15");
    assert_eq!(daw_ui::mixer_pan_caption(1.0), "R100");
}

#[test]
fn pan_rail_drags_and_context_clicks_never_jump_from_secondary_input() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 0.0_f32;
    let (_, geometry, _) = pan_frame(&ctx, &mut value, true, vec![]);
    let quarter = geometry.minimum.lerp(geometry.maximum, 0.25);
    pan_frame(
        &ctx,
        &mut value,
        true,
        pointer(geometry.handle, PointerButton::Primary, true),
    );
    pan_frame(&ctx, &mut value, true, vec![Event::PointerMoved(quarter)]);
    let (response, _, _) = pan_frame(
        &ctx,
        &mut value,
        true,
        pointer(quarter, PointerButton::Primary, false),
    );
    assert!((value + 0.5).abs() < 0.02, "{value}");
    let _ = response;
    let before = value;
    for enabled in [true, false] {
        let (response, geometry, _) = pan_frame(&ctx, &mut value, enabled, vec![]);
        let caption = pos2(response.rect.right() - 4.0, response.rect.center().y);
        for target in [geometry.maximum, caption] {
            pan_frame(
                &ctx,
                &mut value,
                enabled,
                pointer(target, PointerButton::Secondary, true),
            );
            let (response, _, _) = pan_frame(
                &ctx,
                &mut value,
                enabled,
                pointer(target, PointerButton::Secondary, false),
            );
            assert_eq!(value.to_bits(), before.to_bits());
            if enabled {
                assert!(
                    response.secondary_clicked(),
                    "the whole row exposes the pan context menu at {target:?}"
                );
            }
        }
    }
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

fn meter_frame(ctx: &Context, peaks: [f32; 2], events: Vec<Event>) -> (Response, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            response = Some(daw_ui::channel_meter(
                ui,
                egui::Id::new("meter"),
                peaks,
                240.0,
                daw_ui::MIXER_FADER_INSET,
            ));
        });
    });
    (response.unwrap(), output)
}

#[test]
fn gain_taper_puts_unity_at_three_quarters_and_spaces_decades_evenly() {
    assert_eq!(daw_ui::gain_taper(0.0), 0.0);
    assert_eq!(daw_ui::gain_taper(-1.0), 0.0);
    assert_eq!(daw_ui::gain_taper(f32::NAN), 0.0);
    assert_eq!(daw_ui::gain_taper(1.0), 0.75);
    assert!((daw_ui::gain_taper(1.5) - 1.0).abs() < 1e-5);
    let at = |db: f32| daw_ui::gain_taper(10.0_f32.powf(db / 20.0));
    let steps = [
        at(0.0) - at(-10.0),
        at(-10.0) - at(-20.0),
        at(-20.0) - at(-30.0),
    ];
    for step in steps {
        assert!((step - steps[0]).abs() < 1e-5, "{steps:?}");
    }
    let mut previous = -1.0;
    for index in 0..=300 {
        let gain = index as f32 * 0.005;
        let fraction = daw_ui::gain_taper(gain);
        assert!(fraction > previous, "the taper must be monotonic");
        previous = fraction;
        assert!((daw_ui::gain_untaper(fraction) - gain).abs() < 1e-4);
    }
}

#[test]
fn channel_meter_follows_the_fader_scale_and_sanitizes_input() {
    for dark in [false, true] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let colors = daw_ui::theme::palette(&ctx);
        for peaks in [
            [0.0, 0.0],
            [f32::NAN, f32::INFINITY],
            [f32::NEG_INFINITY, -1.0],
            [0.5, 1.0],
        ] {
            let (response, output) = meter_frame(&ctx, peaks, vec![]);
            assert_eq!(
                response.rect.size(),
                vec2(daw_ui::tokens::MIXER_METER_WIDTH, 240.0)
            );
            assert!(!response.changed());
            for shape in &output.shapes {
                if !matches!(shape.shape, egui::Shape::Noop) {
                    assert!(shape.shape.visual_bounding_rect().is_finite());
                }
            }
            let fills: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == colors.guide => Some(rect.rect),
                    _ => None,
                })
                .collect();
            let audible = peaks
                .iter()
                .filter(|peak| peak.is_finite() && **peak > 0.0)
                .count();
            assert_eq!(fills.len(), audible, "silence paints no signal");
            let bottom = response.rect.bottom() - daw_ui::MIXER_FADER_INSET;
            let top = response.rect.top() + daw_ui::MIXER_FADER_INSET;
            for (fill, peak) in fills.iter().zip(peaks.iter().filter(|peak| **peak > 0.0)) {
                assert!(response.rect.contains_rect(*fill));
                let expected = bottom + (top - bottom) * daw_ui::gain_taper(*peak);
                assert!((fill.top() - expected).abs() < 0.01);
            }
        }
    }
}

#[test]
fn channel_meter_latches_clipping_until_clicked() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let colors = daw_ui::theme::palette(&ctx);
    let warning = |output: &egui::FullOutput| {
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == colors.warning),
        )
    };
    let (response, output) = meter_frame(&ctx, [1.2, 0.1], vec![]);
    assert!(warning(&output));
    let (_, output) = meter_frame(&ctx, [0.1, 0.1], vec![]);
    assert!(
        warning(&output),
        "a clip stays latched after the peak falls"
    );
    let center = response.rect.center();
    meter_frame(
        &ctx,
        [0.1, 0.1],
        pointer(center, PointerButton::Primary, true),
    );
    meter_frame(
        &ctx,
        [0.1, 0.1],
        pointer(center, PointerButton::Primary, false),
    );
    let (_, output) = meter_frame(&ctx, [0.0, 0.0], vec![]);
    assert!(!output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == colors.warning
            && rect.rect.height() <= daw_ui::tokens::SPACE_XS)
    }));
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
