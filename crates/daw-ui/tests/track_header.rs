use daw_ui::{Icon, TrackHeaderIdentity, TrackHeaderLayout, theme};
use egui::{Context, Event, Modifiers, PointerButton, RawInput, Rect, Response};

fn row(width: f32, height: f32) -> Rect {
    Rect::from_min_size(egui::pos2(21.0, 34.0), egui::vec2(width, height))
}

#[test]
fn header_sections_stay_separate_across_normal_compact_and_nested_geometry() {
    for width in [112.0, 168.0, 223.0, 224.0, 233.0, 288.0, 377.0, 600.0] {
        for height in [
            1.0, 13.0, 17.0, 21.0, 34.0, 55.0, 56.0, 72.0, 89.0, 96.0, 144.0, 233.0,
        ] {
            for kind in 0..3 {
                let rect = row(width, height);
                let layout = TrackHeaderLayout::new(rect);
                let layout = match kind {
                    0 => layout.with_record_arm(),
                    1 => layout.with_bus_badge(),
                    _ => layout.with_folder_disclosure(),
                };
                let sections = [
                    Some(layout.identity),
                    layout.mix,
                    layout.routing,
                    layout.device,
                    layout.meter,
                ];
                for (index, section) in sections.iter().enumerate() {
                    let Some(section) = section else { continue };
                    assert!(
                        rect.contains_rect(*section),
                        "{width} × {height}: {section:?}"
                    );
                    for other in sections[..index].iter().flatten() {
                        assert!(
                            !section.intersects(*other),
                            "Sections overlap at {width} × {height}: {section:?}, {other:?}"
                        );
                    }
                }
                let identity_parts = [
                    Some(layout.name),
                    Some(layout.icon),
                    Some(layout.reorder),
                    layout.details,
                    layout.record_arm,
                    layout.bus_badge,
                    layout.disclosure,
                ];
                for (index, part) in identity_parts.iter().enumerate() {
                    let Some(part) = part else { continue };
                    assert!(layout.identity.contains_rect(*part));
                    for other in identity_parts[..index].iter().flatten() {
                        assert!(
                            !part.intersects(*other),
                            "Identity targets overlap at {width} × {height}: {part:?}, {other:?}"
                        );
                    }
                }
                if width >= 223.0 && height >= 144.0 {
                    assert!(
                        layout.routing.is_some()
                            && layout.device.is_some()
                            && layout.meter.is_some()
                    );
                    assert!(!layout.compact);
                }
                if width >= 223.0 && height >= 56.0 {
                    assert!(
                        layout.mix.is_some(),
                        "Mix remains available at {width} × {height}"
                    );
                }
                if layout.compact && width >= 168.0 && height >= 21.0 {
                    assert!(
                        layout.details.is_some(),
                        "Compact rows retain channel access"
                    );
                }
                if layout.identity.height() < daw_ui::tokens::COMPACT_HEIGHT {
                    assert!(
                        layout.record_arm.is_none(),
                        "Tiny rows must not shrink arm into an unusable target"
                    );
                }
            }
        }
    }
}

#[test]
fn nested_identity_indents_without_moving_channel_controls_or_hiding_the_title() {
    let bounds = row(287.0, 144.0).translate(egui::vec2(13.0, 0.0));
    let parent = TrackHeaderLayout::new(bounds)
        .with_folder_disclosure()
        .without_channel_sections()
        .with_depth(1);
    let child = TrackHeaderLayout::new(bounds)
        .with_record_arm()
        .with_depth(2);
    assert!(child.name.left() > parent.name.left());
    for width in [112.0, 168.0, 223.0, 287.0, 600.0] {
        for height in [56.0, 89.0, 144.0] {
            for depth in [0, 1, 2, 8, 32, usize::MAX] {
                let initial = TrackHeaderLayout::new(row(width, height)).with_record_arm();
                let nested = initial.with_depth(depth);
                assert_eq!(initial.mix, nested.mix);
                assert_eq!(initial.meter, nested.meter);
                assert_eq!(initial.routing, nested.routing);
                assert_eq!(initial.device, nested.device);
                assert_eq!(initial.reorder, nested.reorder);
                assert_eq!(initial.record_arm, nested.record_arm);
                assert_eq!(initial.details, nested.details);
                assert!(nested.name.width() >= daw_ui::tokens::SPACE_LG);
                assert!(nested.identity.contains_rect(nested.name));
                assert!(nested.icon.right() < nested.name.left());
                let folder = TrackHeaderLayout::new(row(width, height))
                    .with_folder_disclosure()
                    .without_channel_sections()
                    .with_depth(depth);
                let descendant = initial.with_depth(depth.saturating_add(1));
                assert!(
                    descendant.name.left() >= folder.name.left(),
                    "Clamped depth must not reverse at {width}, depth {depth}"
                );
                assert!(
                    folder.meter.is_none() && folder.routing.is_none() && folder.device.is_none()
                );
                if width >= 168.0 {
                    assert!(folder.details.is_some());
                }
            }
        }
    }
}

#[test]
fn invalid_or_empty_headers_do_not_allocate_interactive_sections() {
    for rect in [
        Rect::NOTHING,
        row(0.0, 0.0),
        row(288.0, 0.0),
        row(f32::NAN, 144.0),
        row(288.0, f32::INFINITY),
    ] {
        let layout = TrackHeaderLayout::new(rect)
            .with_record_arm()
            .with_depth(usize::MAX);
        assert!(layout.identity.is_finite());
        assert!(layout.mix.is_none() && layout.meter.is_none());
        assert!(layout.routing.is_none() && layout.device.is_none());
        assert!(layout.record_arm.is_none() && layout.details.is_none());
    }
}

fn frame(
    ctx: &Context,
    enabled: bool,
    details: bool,
    events: Vec<Event>,
) -> (Response, egui::FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(row(800.0, 600.0)),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let layout =
                        TrackHeaderLayout::new(row(288.0, if details { 89.0 } else { 144.0 }))
                            .with_record_arm();
                    daw_ui::paint_track_header_sections(ui.painter(), &layout);
                    daw_ui::paint_track_header_identity(
                        ui.painter(),
                        &layout,
                        TrackHeaderIdentity {
                            name: "A long channel name that must never cover its controls",
                            icon: Icon::Instrument,
                            tint: theme::palette(ui.ctx()).accent,
                            folder: false,
                            collapsed: false,
                        },
                    );
                    let before = ui.next_widget_position();
                    response = Some(if details {
                        daw_ui::track_header_details_at(ui, layout.details.unwrap(), "details")
                    } else {
                        daw_ui::track_header_link_at(
                            ui,
                            layout.routing.unwrap(),
                            "routing",
                            Icon::Routing,
                            "Routing",
                            "A long destination bus name",
                        )
                    });
                    assert_eq!(
                        before,
                        ui.next_widget_position(),
                        "Canvas controls do not advance the parent row"
                    );
                });
            });
        },
    );
    (response.unwrap(), output)
}

#[test]
fn connection_and_details_controls_keep_keyboard_mouse_and_disabled_semantics() {
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        for density in [1.0, 1.5, 2.0] {
            for details in [false, true] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                frame(&ctx, true, details, vec![]);
                let (initial, _) = frame(&ctx, true, details, vec![]);
                initial.request_focus();
                let (activated, output) = frame(
                    &ctx,
                    true,
                    details,
                    vec![Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                );
                assert!(activated.clicked());
                assert_eq!(activated.rect, initial.rect);
                let events = &output.platform_output.events;
                assert_eq!(events.len(), 1);
                let info = events[0].widget_info();
                assert_eq!(info.typ, egui::WidgetType::Button);
                assert_eq!(
                    info.label.as_deref(),
                    Some(if details {
                        "Channel controls"
                    } else {
                        "Routing: A long destination bus name"
                    })
                );
                frame(
                    &ctx,
                    true,
                    details,
                    vec![Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                );
                for enabled in [true, false] {
                    let target = initial.rect.center();
                    frame(
                        &ctx,
                        enabled,
                        details,
                        vec![
                            Event::PointerMoved(target),
                            Event::PointerButton {
                                pos: target,
                                button: PointerButton::Primary,
                                pressed: true,
                                modifiers: Modifiers::NONE,
                            },
                        ],
                    );
                    let (released, _) = frame(
                        &ctx,
                        enabled,
                        details,
                        vec![Event::PointerButton {
                            pos: target,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        }],
                    );
                    assert_eq!(released.clicked(), enabled);
                }
            }
        }
    }
}
