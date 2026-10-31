use daw_ui::{TrackMixResponse, theme, track_mix_controls_at};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect};

fn frame(
    ctx: &Context,
    size: egui::Vec2,
    gain: &mut f32,
    pan: &mut f32,
    enabled: bool,
    events: Vec<Event>,
) -> (TrackMixResponse, FullOutput) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.interact(
                        Rect::from_min_max(egui::pos2(0.0, 34.0), egui::pos2(610.0, 200.0)),
                        ui.id().with("track_row"),
                        egui::Sense::click(),
                    );
                    let cursor = ui.next_widget_position();
                    result = Some(track_mix_controls_at(
                        ui,
                        Rect::from_min_size(egui::pos2(55.0, 55.0), size),
                        "track",
                        gain,
                        pan,
                        false,
                        true,
                    ));
                    // The host attaches automation to the same response. Its
                    // context popup must not close the left-click exact editor.
                    result.as_ref().unwrap().gain.context_menu(|ui| {
                        ui.label("Gain context");
                    });
                    result.as_ref().unwrap().pan.context_menu(|ui| {
                        ui.label("Pan context");
                    });
                    assert_eq!(
                        cursor,
                        ui.next_widget_position(),
                        "Canvas mix must not move its parent layout"
                    );
                });
            });
        },
    );
    (result.unwrap(), output)
}

fn pointer(at: egui::Pos2, pressed: bool, button: PointerButton) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            pressed,
            button,
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

#[test]
fn track_mix_is_bounded_without_tiny_scale_labels_at_supported_sizes_and_densities() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.5, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let (mut gain, mut pan) = (0.5, 0.12);
            frame(
                &ctx,
                egui::vec2(235.0, 42.0),
                &mut gain,
                &mut pan,
                true,
                vec![],
            );
            for size in [
                egui::vec2(179.0, 21.0),
                egui::vec2(179.0, 42.0),
                egui::vec2(235.0, 42.0),
            ] {
                let (mix, output) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
                let bounds = Rect::from_min_size(egui::pos2(55.0, 55.0), size);
                let rects = [
                    mix.gain_slider,
                    mix.pan_slider,
                    mix.gain_readout,
                    mix.pan_readout,
                    mix.mute.rect,
                    mix.solo.rect,
                ];
                for (index, rect) in rects.iter().enumerate() {
                    assert!(
                        bounds.expand(0.05).contains_rect(*rect),
                        "{appearance:?}, {density}, {size:?}: {rect:?}"
                    );
                    for previous in &rects[..index] {
                        if size.y < 42.0 && previous == rect {
                            continue; // Short rows expose one value field, not a separate rail.
                        }
                        assert!(
                            !previous.shrink(0.01).intersects(rect.shrink(0.01)),
                            "Distinct controls must not overlap"
                        );
                    }
                }
                if size.y < 42.0 {
                    assert!(mix.gain_scale.is_none() && mix.pan_scale.is_none());
                    assert_eq!(mix.gain_slider, mix.gain_readout);
                    assert_eq!(mix.pan_slider, mix.pan_readout);
                }
                for geometry in [mix.gain_scale, mix.pan_scale].into_iter().flatten() {
                    assert!(geometry.ticks.is_empty());
                    assert!(geometry.minimum.x < geometry.maximum.x);
                    assert!(geometry.rail.contains(geometry.minimum));
                    assert!(geometry.rail.contains(geometry.maximum));
                }
                let labels: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text()),
                        _ => None,
                    })
                    .collect();
                assert!(labels.contains(&"-6.0 dB"));
                assert!(labels.contains(&"R12"));
                assert!(!labels.contains(&"+3.5") && !labels.contains(&"−∞"));
                assert_eq!((gain, pan), (0.5, 0.12));
            }
        }
    }
}

#[test]
fn native_rails_keep_endpoint_keyboard_and_disabled_behavior_with_exact_geometry() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(235.0, 42.0);
    let (mut gain, mut pan) = (0.5, 0.0);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    let endpoint = mix.pan_scale.unwrap().maximum;
    for pressed in [true, false] {
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            true,
            pointer(endpoint, pressed, PointerButton::Primary),
        );
    }
    assert_eq!(pan, 1.0);
    assert_eq!(gain, 0.5);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    mix.pan.request_focus();
    let (changed, output) = frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::ArrowLeft, true),
    );
    assert!(changed.pan.changed() && pan < 1.0);
    assert!(
        output
            .platform_output
            .events
            .iter()
            .any(|event| event.widget_info().label.as_deref() == Some("Channel pan"))
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::ArrowLeft, false),
    );
    let unchanged = (gain, pan);
    for pressed in [true, false] {
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            false,
            pointer(endpoint, pressed, PointerButton::Primary),
        );
    }
    assert_eq!((gain, pan), unchanged);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    let minimum = mix.gain_scale.unwrap().minimum;
    for pressed in [true, false] {
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            true,
            pointer(minimum, pressed, PointerButton::Secondary),
        );
    }
    assert_eq!(
        (gain, pan),
        unchanged,
        "Opening automation must not edit the rail"
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::Escape, true),
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::Escape, false),
    );
    for pressed in [true, false] {
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            true,
            pointer(minimum, pressed, PointerButton::Primary),
        );
    }
    assert_eq!(gain, 0.0);
}

#[test]
fn caption_editor_preserves_out_of_range_values_on_open_and_cancel_then_applies_exactly() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(235.0, 42.0);
    let (mut gain, mut pan) = (2.0, 0.123456);
    let before = (gain, pan);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    assert_eq!((gain, pan), before);
    let at = mix.gain_readout.center();
    for cancel in [true, false] {
        for pressed in [true, false] {
            frame(
                &ctx,
                size,
                &mut gain,
                &mut pan,
                true,
                pointer(at, pressed, PointerButton::Primary),
            );
        }
        frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
        assert!(egui::Popup::is_any_open(&ctx));
        assert_eq!((gain, pan), before);
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            true,
            vec![Event::Text("-22.75".into())],
        );
        assert_eq!((gain, pan), before, "Drafts must never change a channel");
        let key = if cancel { Key::Escape } else { Key::Enter };
        let (result, _) = frame(&ctx, size, &mut gain, &mut pan, true, self::key(key, true));
        if cancel {
            assert_eq!((gain, pan), before);
            assert!(!result.gain.changed());
        } else {
            assert!(result.gain.changed());
            assert!((daw_ui::channel_gain_to_db(gain) + 22.75).abs() < 0.0001);
            assert_eq!(pan, before.1);
        }
        frame(&ctx, size, &mut gain, &mut pan, true, self::key(key, false));
    }
}

#[test]
fn mute_and_solo_expose_native_checkbox_commands_without_owning_channel_state() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(235.0, 42.0);
    let (mut gain, mut pan) = (0.5, 0.12);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    mix.mute.request_focus();
    let (mix, output) = frame(&ctx, size, &mut gain, &mut pan, true, key(Key::Space, true));
    assert!(mix.mute.clicked() && !mix.solo.clicked());
    let event = output
        .platform_output
        .events
        .iter()
        .find(|event| {
            matches!(event, egui::output::OutputEvent::Clicked(_))
                && event.widget_info().label.as_deref() == Some("Mute track")
        })
        .expect("Keyboard activation retains a labelled native event");
    assert_eq!(event.widget_info().typ, egui::WidgetType::Checkbox);
    assert_eq!(event.widget_info().selected, Some(false));
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::Space, false),
    );
    let at = mix.solo.rect.center();
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        pointer(at, true, PointerButton::Primary),
    );
    let (mix, _) = frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        pointer(at, false, PointerButton::Primary),
    );
    assert!(mix.solo.clicked() && !mix.mute.clicked());
    assert_eq!((gain, pan), (0.5, 0.12));
    let (mix, _) = frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        false,
        key(Key::Space, true),
    );
    assert!(!mix.solo.clicked() && !mix.mute.clicked());
}

#[test]
fn compact_value_click_opens_explicit_entry_without_clamping_stored_gain() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(235.0, 21.0);
    let (mut gain, mut pan) = (0.000_000_000_123, 0.123456);
    let before = (gain, pan);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    let at = mix.gain_readout.center();
    for pressed in [true, false] {
        frame(
            &ctx,
            size,
            &mut gain,
            &mut pan,
            true,
            pointer(at, pressed, PointerButton::Primary),
        );
    }
    frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    assert!(egui::Popup::is_any_open(&ctx));
    assert_eq!((gain, pan), before);
}

#[test]
fn compact_fields_drag_relatively_and_keep_native_keys_and_disabled_state() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(235.0, 21.0);
    let (mut gain, mut pan) = (0.5, 0.123456);
    let before = (gain, pan);
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    let at = mix.gain_readout.center();
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        pointer(at, true, PointerButton::Primary),
    );
    assert_eq!((gain, pan), before, "Pressing a value never moves it");
    let moved = at + egui::vec2(13.0, 0.0);
    let (dragging, _) = frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        vec![Event::PointerMoved(moved)],
    );
    assert!(dragging.gain.changed());
    assert!(
        (daw_ui::channel_gain_to_db(gain) - daw_ui::channel_gain_to_db(before.0) - 1.3).abs()
            < 0.001
    );
    assert_eq!(pan, before.1);
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        pointer(moved, false, PointerButton::Primary),
    );
    assert!(
        !egui::Popup::is_any_open(&ctx),
        "Dragging must not open the exact editor"
    );
    let (mix, _) = frame(&ctx, size, &mut gain, &mut pan, true, vec![]);
    mix.pan.request_focus();
    let (changed, _) = frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::ArrowRight, true),
    );
    assert!(changed.pan.changed() && pan > before.1);
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        true,
        key(Key::ArrowRight, false),
    );
    let unchanged = (gain, pan);
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        false,
        key(Key::ArrowRight, true),
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        false,
        pointer(at, true, PointerButton::Primary),
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        false,
        vec![Event::PointerMoved(moved)],
    );
    frame(
        &ctx,
        size,
        &mut gain,
        &mut pan,
        false,
        pointer(moved, false, PointerButton::Primary),
    );
    assert_eq!((gain, pan), unchanged);
}
