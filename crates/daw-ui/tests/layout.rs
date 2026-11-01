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
fn mixed_control_rows_keep_targets_and_text_centered_during_numeric_entry() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for (density, width) in [
            (1.0, 1600.0),
            (1.25, 1600.0),
            (2.0, 1600.0),
            (1.0, 500.0),
            (2.0, 500.0),
        ] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let size = egui::vec2(width, 400.0);
            let _ = ctx.run(input(vec![], size), |_| {});
            let mut number = -6.5;
            let mut name = "Glass".to_owned();
            let mut enabled = false;
            for editing in [false, true] {
                let mut targets = Vec::new();
                let output = ctx.run(input(vec![], size), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        controls::toolbar(ui, |ui| {
                            targets.push(("Mix", ui.label("Mix")));
                            targets.push((
                                "number",
                                ui.add(
                                    controls::number(&mut number)
                                        .fixed_decimals(2)
                                        .suffix(" dB"),
                                ),
                            ));
                            targets.push((
                                "Glass",
                                controls::text_field(ui, &mut name, "Name", 120.0),
                            ));
                            targets.push(("Apply", controls::button(ui, "Apply")));
                            targets.push((
                                "Snap 1/16",
                                controls::select("row_snap", "Snap 1/16")
                                    .show_ui(ui, |_| {})
                                    .response,
                            ));
                            targets
                                .push(("Monitor", controls::switch(ui, &mut enabled, "Monitor")));
                            targets
                                .push(("View", controls::view_menu(ui, "View", |_| {}).response));
                        });
                    });
                });
                assert_eq!(targets[1].1.has_focus(), editing);
                let mut row = Rect::NOTHING;
                let mut center = 0.0;
                let mut row_centers = Vec::new();
                for (name, response) in &targets {
                    if !row.is_positive() || response.rect.top() >= row.bottom() {
                        row = response.rect;
                        center = row.center().y;
                    } else {
                        row = row.union(response.rect);
                    }
                    row_centers.push(center);
                    assert!(
                        (response.rect.center().y - center).abs() <= 0.5 / density,
                        "{name} target drifts at {density}×, width {width}, editing={editing}: {:?}; expected center {center}",
                        response.rect,
                    );
                }
                if width < 600.0 {
                    assert!(targets.last().unwrap().1.rect.top() > targets[0].1.rect.bottom());
                }
                let mut seen = vec![false; targets.len()];
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        let text_bounds = text.galley.rect.translate(text.pos.to_vec2());
                        let label = text.galley.text();
                        if let Some((index, (name, response))) =
                            targets.iter().enumerate().find(|(_, (name, _))| {
                                *name == label
                                    || (*name == "number" && ["-6.50", "-6.50 dB"].contains(&label))
                            })
                        {
                            seen[index] = true;
                            let center = row_centers[index];
                            assert!(response.rect.contains_rect(text_bounds));
                            assert!(
                                (text_bounds.center().y - center).abs() <= 0.5 / density,
                                "{name} text drifts at {density}×, width {width}, editing={editing}: {text_bounds:?}; expected center {center}",
                            );
                        }
                    }
                }
                assert!(
                    seen.iter().all(|seen| *seen),
                    "Every control must expose its real painted caption: {seen:?}"
                );
                if !editing {
                    targets[1].1.request_focus();
                }
                assert_eq!(number, -6.5);
                assert_eq!(name, "Glass");
                assert!(!enabled);
            }
        }
    }
}

#[test]
fn parameter_rows_align_slider_rails_with_fields_labels_and_actions() {
    for density in [1.0, 1.25, 2.0] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        let size = egui::vec2(1400.0, 400.0);
        let _ = ctx.run(input(vec![], size), |_| {});
        let mut strength = 100.0;
        let mut swing = 50.0;
        let mut ends = false;
        let mut targets = Vec::new();
        let output = ctx.run(input(vec![], size), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                controls::parameter_row(ui, |ui| {
                    targets.push(ui.label("Quantize preview"));
                    targets.push(
                        controls::select("quantize_grid", "1/16 beat")
                            .width(100.0)
                            .show_ui(ui, |_| {})
                            .response,
                    );
                    targets.push(
                        ui.add(
                            controls::slider(&mut strength, 0.0..=100.0)
                                .text("Strength")
                                .suffix("%"),
                        ),
                    );
                    targets.push(
                        ui.add(
                            controls::slider(&mut swing, 50.0..=75.0)
                                .text("Swing")
                                .suffix("%"),
                        ),
                    );
                    targets.push(controls::checkbox(ui, &mut ends, "Note ends"));
                    targets.push(controls::button(ui, "Apply quantize"));
                });
            });
        });
        let center = targets[0].rect.center().y;
        for target in &targets {
            assert!(
                (target.rect.center().y - center).abs() <= 0.5 / density,
                "Mixed parameter targets drift at {density}×: {:?}, expected center {center}",
                target.rect,
            );
        }
        for target in &targets[2..4] {
            let geometry = controls::slider_scale_geometry(&ctx, target.id).unwrap();
            assert!(
                (geometry.rail.center().y - center).abs() <= 0.5 / density,
                "Slider rail is offset from neighboring fields at {density}×: {:?}, expected center {center}",
                geometry.rail,
            );
        }
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape
                && ["Strength", "Swing", "Note ends", "Apply quantize"]
                    .contains(&text.galley.text())
            {
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                assert!((bounds.center().y - center).abs() <= 0.5 / density);
            }
        }
        assert_eq!((strength, swing, ends), (100.0, 50.0, false));
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
fn contextual_workspace_header_keeps_command_and_canvas_geometry_across_clip_kinds() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for (width, density) in [(800.0, 1.0), (800.0, 1.25), (800.0, 2.0), (1440.0, 1.0)] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            // Apply egui's pending scale/viewport update before comparing
            // workspace geometry, as the native app does during startup.
            let _ = ctx.run(input(vec![], egui::vec2(width, 760.0)), |_| {});
            let mut properties_rect = None;
            let mut canvas_top = None;
            for midi in [false, true, false] {
                let _ = ctx.run(input(vec![], egui::vec2(width, 760.0)), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let header = controls::workspace_header_with_context(
                            ui,
                            "Clip Editor · A repeated phrase with a deliberately very long Unicode name 夜のシンセ",
                            Some("Song 9:1–11:1 · Evening lead"),
                            |ui| {
                                let properties = controls::quiet_button(ui, "Properties");
                                if midi {
                                    controls::button(ui, "Choose instrument…");
                                }
                                controls::quiet_button(ui, "Next");
                                properties
                            },
                        );
                        let context = header.context.as_ref().unwrap();
                        assert!(header.title.rect.right() < header.inner.rect.left());
                        assert!(context.rect.right() < header.inner.rect.left());
                        assert!(context.rect.top() - header.title.rect.bottom()
                            >= controls::tokens::SPACE_XS - 1.0 / density);
                        let identity = header.title.rect.union(context.rect);
                        assert!((identity.center().y - header.inner.rect.center().y).abs()
                            <= 1.0 / density);
                        assert!(header.response.rect.contains_rect(header.title.rect));
                        assert!(header.response.rect.contains_rect(context.rect));
                        assert!(header.response.rect.right() <= ui.max_rect().right() + 0.1);
                        assert!(
                            header.response.rect.height()
                                <= identity.height() + 2.0 * controls::tokens::SPACE_SM + 1.0,
                            "Context reserves only its measured lines and padding: {:?}",
                            header.response.rect,
                        );
                        if let Some(expected) = properties_rect {
                            assert_eq!(header.inner.rect, expected, "Properties changed slots");
                            assert_eq!(ui.cursor().top(), canvas_top.unwrap());
                        }
                        properties_rect = Some(header.inner.rect);
                        canvas_top = Some(ui.cursor().top());
                    });
                });
            }
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
fn command_families_wrap_together_without_shrinking_targets_or_leaking_styles() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 2.0] {
            for (width, wraps) in [(380.0, true), (800.0, false)] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                // egui applies a requested scale at the first frame boundary.
                let _ = ctx.run(input(vec![], egui::vec2(width, 600.0)), |_| {});
                let _ = ctx.run(input(vec![], egui::vec2(width, 600.0)), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let style = ui.style().clone();
                        let available = ui.max_rect();
                        let mut click = false;
                        let mut looping = true;
                        let groups = ui
                            .horizontal_wrapped(|ui| {
                                let playback = controls::command_group(ui, 280.0, |ui| {
                                    [
                                        controls::quiet_button(ui, "Start"),
                                        controls::primary_button(ui, "Play"),
                                        controls::quiet_button(ui, "Stop"),
                                    ]
                                });
                                let guides = controls::command_group(ui, 200.0, |ui| {
                                    [
                                        controls::quiet_toggle(ui, &mut click, "Click"),
                                        controls::quiet_toggle(ui, &mut looping, "Loop"),
                                    ]
                                });
                                (playback, guides)
                            })
                            .inner;
                        assert_eq!(*ui.style(), style);
                        assert!(!click && looping, "layout must not change playback guides");
                        let (playback, guides) = groups;
                        if wraps {
                            assert!(
                                guides.response.rect.top() >= playback.response.rect.bottom(),
                                "width {width}, density {density}, available {available:?}: playback {:?}, guides {:?}",
                                playback.response.rect, guides.response.rect,
                            );
                        } else {
                            assert!(
                                (guides.response.rect.top() - playback.response.rect.top()).abs()
                                    < 0.5
                            );
                        }
                        let mut preceding = Vec::new();
                        for (group, targets) in [
                            (&playback.response, playback.inner.as_slice()),
                            (&guides.response, guides.inner.as_slice()),
                        ] {
                            assert!(available.contains_rect(group.rect));
                            for target in targets {
                                assert!(group.rect.contains_rect(target.rect));
                                assert!(target.rect.height() >= controls::tokens::CONTROL_HEIGHT);
                                assert!(
                                    (target.rect.center().y - targets[0].rect.center().y).abs()
                                        < 0.5
                                );
                                assert!(
                                    preceding
                                        .iter()
                                        .all(|rect: &Rect| !rect.intersects(target.rect))
                                );
                                preceding.push(target.rect);
                            }
                        }
                    });
                });
            }
        }
    }
}

#[test]
fn trailing_toolbar_group_uses_current_row_and_wraps_only_when_needed() {
    for (width, wraps) in [(800.0, false), (420.0, true)] {
        let ctx = Context::default();
        theme::apply(&ctx);
        let _ = ctx.run(input(vec![], egui::vec2(width, 600.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let row = controls::toolbar(ui, |ui| {
                    let leading = ui.add_sized(
                        egui::vec2(280.0, 34.0),
                        controls::button_widget("Editing tools"),
                    );
                    let trailing = controls::toolbar_trailing(ui, 180.0, |ui| {
                        let fit = controls::view_button(ui, "Fit");
                        let choice = controls::select("snap", "Snap 1/16")
                            .width(100.0)
                            .show_ui(ui, |_| {})
                            .response;
                        (fit, choice)
                    });
                    (leading, trailing)
                });
                let (leading, trailing) = row.inner;
                let (fit, choice) = trailing.inner;
                assert!((fit.rect.center().y - choice.rect.center().y).abs() < 0.1);
                assert!(
                    (fit.rect.right() - row.response.rect.right() + controls::tokens::SPACE_MD)
                        .abs()
                        < 0.1,
                    "Trailing controls remain right aligned after wrapping"
                );
                if wraps {
                    assert!(fit.rect.top() >= leading.rect.bottom());
                } else {
                    assert!(
                        (fit.rect.center().y - leading.rect.center().y).abs() < 0.1,
                        "Available current-row space must be used before wrapping"
                    );
                }
            });
        });
    }
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

#[test]
fn mixer_card_padding_and_parent_gutters_survive_display_scaling() {
    for density in [1.0, 1.25, 2.0] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        let _ = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |_| {});
        let _ = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = controls::tokens::SPACE_SM;
                    let mut previous: Option<egui::Rect> = None;
                    for selected in [false, true] {
                        let strip = controls::mixer_strip(
                            ui,
                            selected,
                            egui::vec2(180.0, 400.0),
                            None,
                            selected,
                            |ui| controls::button(ui, "Effects").rect,
                        );
                        let tolerance = 1.0 / density;
                        assert!(
                            strip.inner.left() - strip.response.rect.left()
                                >= controls::tokens::SPACE_MD - tolerance
                        );
                        assert!(
                            strip.inner.top() - strip.response.rect.top()
                                >= controls::tokens::SPACE_MD - tolerance
                        );
                        assert!(
                            strip.response.rect.right() - strip.inner.right()
                                >= controls::tokens::SPACE_MD - tolerance
                        );
                        assert!((strip.response.rect.width() - 180.0).abs() <= tolerance);
                        if let Some(previous) = previous {
                            assert!(
                                (strip.response.rect.left()
                                    - previous.right()
                                    - controls::tokens::SPACE_SM)
                                    .abs()
                                    <= tolerance
                            );
                        }
                        previous = Some(strip.response.rect);
                    }
                });
            });
        });
    }
}

#[test]
fn take_lane_labels_keep_clearance_from_the_waveform_at_display_scales() {
    for density in [1.0, 1.25, 2.0] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        let _ = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |_| {});
        let mut card = egui::Rect::NOTHING;
        let mut plot = egui::Rect::NOTHING;
        let output = ctx.run(input(vec![], egui::vec2(640.0, 480.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                card = controls::audio_take_lane_with_waveform(
                    ui,
                    &controls::TakeLane {
                        name: "Vocal take",
                        detail: "Eight beats · Alternative",
                        selected: true,
                        color: egui::Color32::LIGHT_BLUE,
                        peaks: &[],
                        span: (0.0, 1.0),
                        selection: Some((0.25, 0.75)),
                    },
                    |painter, bounds| {
                        plot = bounds;
                        assert_eq!(painter.clip_rect(), bounds);
                    },
                )
                .rect;
            });
        });
        let mut labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && ["Vocal take", "Eight beats · Alternative"].contains(&text.galley.text())
                {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                } else {
                    None
                }
            })
            .collect();
        labels.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert_eq!(labels.len(), 2);
        let tolerance = 1.0 / density;
        assert!(labels[0].left() - card.left() >= controls::tokens::SPACE_MD - tolerance);
        assert!(labels[1].top() - labels[0].bottom() >= controls::tokens::SPACE_XS - tolerance);
        assert!(plot.top() - labels[1].bottom() >= controls::tokens::SPACE_XS - tolerance);
        assert!(plot.height() >= controls::tokens::SPACE_LG - tolerance);
        assert!(card.bottom() - plot.bottom() >= controls::tokens::SPACE_SM - tolerance);
        assert!((card.height() - 89.0).abs() <= tolerance);
    }
}
