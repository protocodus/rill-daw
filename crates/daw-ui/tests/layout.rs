use daw_ui::{self as controls, theme};
use egui::{Context, Event, Key, Modifiers, RawInput, Rect, Response, Vec2};

fn input(events: Vec<Event>, size: Vec2) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    }
}

#[test]
fn workspace_header_stays_at_the_top_and_back_keeps_native_keyboard_accessibility() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut back: Option<Response> = None;
    for activate in [false, true] {
        if let Some(back) = &back {
            back.request_focus();
        }
        let events = if activate {
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        } else {
            vec![]
        };
        let output = ctx.run(input(events, egui::vec2(1100.0, 1200.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let start = ui.cursor().top();
                let header = controls::workspace_header(
                    ui,
                    "A long but readable clip workspace title",
                    |ui| controls::button(ui, "Properties"),
                );
                assert!(
                    header.response.rect.height()
                        <= controls::tokens::CONTROL_HEIGHT + 2.0 * controls::tokens::SPACE_SM,
                    "header must not consume canvas height: {:?}",
                    header.response.rect
                );
                assert!(header.back.rect.top() <= start + 13.0);
                assert!(header.inner.rect.right() <= ui.max_rect().right());
                assert!(header.back.rect.right() < header.inner.rect.left());
                assert_eq!(header.back.clicked(), activate);
                let canvas = ui.available_rect_before_wrap();
                assert!(
                    canvas.height() > 1100.0,
                    "canvas retains available height: {canvas:?}"
                );
                back = Some(header.back);
            });
        });
        if activate {
            assert!(output.platform_output.events.iter().any(|event| {
                let info = event.widget_info();
                info.typ == egui::WidgetType::Button && info.label.as_deref() == Some("Back")
            }));
        }
    }
}

#[test]
fn toolbar_wraps_complete_native_controls_without_eating_the_canvas() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let _ = ctx.run(input(vec![], egui::vec2(360.0, 760.0)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let row = controls::toolbar(ui, |ui| {
                [
                    "Select",
                    "Pencil",
                    "Cut",
                    "Duplicate",
                    "Fit",
                    "Notes",
                    "Pitch",
                ]
                .map(|label| controls::button(ui, label))
            });
            assert!(row.response.rect.height() < 140.0);
            let buttons = row.inner;
            assert!(buttons.last().unwrap().rect.top() > buttons[0].rect.top());
            for button in buttons {
                assert!(button.rect.right() <= ui.max_rect().right() + 0.1);
                assert!(button.rect.height() >= controls::tokens::CONTROL_HEIGHT);
            }
        });
    });
}

#[test]
fn sized_toolbar_choices_use_remaining_row_space_and_wrap_as_one_group() {
    for (width, should_wrap) in [(800.0, false), (560.0, true)] {
        let ctx = Context::default();
        theme::apply(&ctx);
        for _ in 0..2 {
            let _ = ctx.run(input(vec![], egui::vec2(width, 760.0)), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let available = ui.max_rect();
                    let row = controls::toolbar(ui, |ui| {
                        let leading = ui.add_sized(
                            egui::vec2(240.0, controls::tokens::CONTROL_HEIGHT),
                            controls::button_widget("Select · Pencil · Generate"),
                        );
                        let group = controls::toolbar_group_sized(
                            ui,
                            2.0 * controls::tokens::SELECT_WIDTH + controls::tokens::SPACE_SM,
                            |ui| {
                                [
                                    controls::select("stack", "Chord stack")
                                        .show_ui(ui, |_| {})
                                        .response,
                                    controls::select("chord", "Major triad")
                                        .show_ui(ui, |_| {})
                                        .response,
                                ]
                            },
                        );
                        (leading, group.inner)
                    });
                    let (leading, choices) = row.inner;
                    assert!(
                        (choices[0].rect.top() - choices[1].rect.top()).abs() < 0.5,
                        "related choices must stay together: {choices:?}"
                    );
                    if should_wrap {
                        assert!(
                            choices[0].rect.top() > leading.rect.bottom(),
                            "insufficient remaining width must wrap the whole group"
                        );
                    } else {
                        assert!(
                            (choices[0].rect.center().y - leading.rect.center().y).abs() < 0.5,
                            "a fitting group must not consume an unnecessary canvas row"
                        );
                    }
                    let targets = [leading.rect, choices[0].rect, choices[1].rect];
                    for (index, rect) in targets.iter().enumerate() {
                        assert!(
                            available.contains_rect(*rect),
                            "control exceeds canvas: {rect:?}"
                        );
                        assert!(
                            targets[..index]
                                .iter()
                                .all(|other| !other.intersects(*rect)),
                            "toolbar hit targets overlap: {targets:?}"
                        );
                    }
                    assert!(
                        ui.available_height() > 600.0,
                        "toolbar must preserve editing space"
                    );
                });
            });
        }
    }
}

#[test]
fn utility_popup_is_centered_bounded_and_scrolls_long_content() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let viewport = egui::vec2(1100.0, 760.0);
    for _ in 0..3 {
        let _ = ctx.run(input(vec![], viewport), |ctx| {
            let mut open = true;
            let response = controls::popup("Properties", ctx)
                .open(&mut open)
                .show(ctx, |ui| {
                    for index in 0..100 {
                        ui.label(format!("Parameter {index}"));
                    }
                })
                .unwrap()
                .response;
            assert!(
                response.rect.top() >= 104.0,
                "utility covers persistent transport: {:?}",
                response.rect
            );
            assert!(
                response.rect.bottom() <= viewport.y - 20.0,
                "utility exceeds viewport: {:?}",
                response.rect
            );
            assert!((response.rect.center().x - viewport.x * 0.5).abs() < 2.0);
        });
    }
}

#[test]
fn utility_popup_can_reach_actions_after_asynchronous_body_growth() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = egui::vec2(1100.0, 720.0);
    let mut action = None;
    let mut window = Rect::NOTHING;
    let mut clip = Rect::NOTHING;
    let mut clicked = false;
    let mut draw = |loaded: bool, events| {
        let _ = ctx.run(input(events, size), |ctx| {
            let response = controls::popup("Growing utility", ctx)
                .show(ctx, |ui| {
                    ui.label("Device settings");
                    if loaded {
                        for index in 0..100 {
                            ui.label(format!("Discovered property {index}"));
                        }
                    }
                    let response = controls::button(ui, "Apply");
                    clicked |= response.clicked();
                    action = Some(response);
                    clip = ui.clip_rect();
                })
                .unwrap();
            window = response.response.rect;
        });
        (action.as_ref().unwrap().rect, clip, window)
    };
    for _ in 0..3 {
        draw(false, vec![]);
    }
    let (_, _, window) = draw(true, vec![]);
    draw(
        true,
        vec![
            Event::PointerMoved(window.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -10000.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let mut state = draw(true, vec![]);
    for _ in 0..12 {
        state = draw(true, vec![]);
    }
    assert!(
        state.1.contains_rect(state.0),
        "late action must be reachable through the window scroll: {state:?}"
    );
    assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(state.0));
    for pressed in [true, false] {
        draw(
            true,
            vec![
                Event::PointerMoved(state.0.center()),
                Event::PointerButton {
                    pos: state.0.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert!(clicked);
}

#[test]
fn thumbnail_arrival_keeps_catalog_text_position_and_row_height_stable() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let texture = ctx.load_texture(
        "preview",
        egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
        Default::default(),
    );
    let output = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let first = controls::catalog_item(ui, false, "Missing preview", "Instrument", None);
            let second =
                controls::catalog_item(ui, false, "Ready preview", "Instrument", Some(&texture));
            assert_eq!(first.rect.size(), second.rect.size());
        });
    });
    let positions: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text)
                if ["Missing preview", "Ready preview"].contains(&text.galley.text()) =>
            {
                Some(text.pos.x)
            }
            _ => None,
        })
        .collect();
    assert_eq!(positions.len(), 2);
    assert_eq!(positions[0], positions[1]);
}

#[test]
fn mixer_strips_stack_their_contents_and_meter_rejects_nonfinite_readings() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let output = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                for (id, peaks) in [[f32::NAN, f32::INFINITY], [0.5, 1.2]]
                    .into_iter()
                    .enumerate()
                {
                    let strip = controls::mixer_strip(
                        ui,
                        id,
                        egui::vec2(180.0, 400.0),
                        None,
                        false,
                        |ui| {
                            let label = ui.label("Channel");
                            let meter = controls::stereo_meter(ui, peaks, egui::vec2(40.0, 100.0));
                            assert!(meter.rect.top() > label.rect.bottom());
                        },
                    );
                    assert!((strip.response.rect.width() - 180.0).abs() <= 0.1);
                    assert!((strip.response.rect.height() - 400.0).abs() <= 0.1);
                }
            });
        });
    });
    for shape in output.shapes {
        if let egui::epaint::Shape::Rect(rect) = shape.shape {
            assert!(rect.rect.is_finite());
        }
    }
}
