use egui::{Context, Event, Modifiers, PointerButton, RawInput, Response, Ui, pos2, vec2};

fn frame(ctx: &Context, events: Vec<Event>, mut draw: impl FnMut(&mut Ui) -> Response) -> Response {
    let mut response = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(640.0, 480.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| response = Some(draw(ui)));
        },
    );
    response.unwrap()
}

fn pointer(position: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn numeric_multiclick_replaces_sign_and_fraction_without_select_all_shortcut() {
    for (initial, replacement, expected) in [(120.72, "72", 72.0), (-2.5, "-24", -24.0)] {
        for clicks in [2, 3] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            let mut value = initial;
            let draw = |ui: &mut Ui, value: &mut f64| {
                ui.add(
                    daw_ui::number(value)
                        .speed(0.1)
                        .fixed_decimals(2)
                        .suffix(" units"),
                )
            };
            let original = frame(&ctx, vec![], |ui| draw(ui, &mut value));
            // The fractional part is a different TextEdit word from the integer
            // part. A numeric multi-click must select both and the sign.
            let position = original.rect.left_center() + vec2(28.0, 0.0);
            for _ in 0..clicks {
                frame(&ctx, pointer(position, true), |ui| draw(ui, &mut value));
                frame(&ctx, pointer(position, false), |ui| draw(ui, &mut value));
            }
            let changed = frame(&ctx, vec![Event::Text(replacement.into())], |ui| {
                draw(ui, &mut value)
            });
            assert!(changed.changed());
            assert_eq!(value, expected, "{clicks} clicks on {initial}");
        }
    }
}

#[test]
fn numeric_double_click_and_typing_in_one_frame_preserve_following_characters() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 120.72;
    let draw = |ui: &mut Ui, value: &mut f64| {
        ui.add(
            daw_ui::number(value)
                .speed(0.1)
                .fixed_decimals(2)
                .suffix(" BPM"),
        )
    };
    let field = frame(&ctx, vec![], |ui| draw(ui, &mut value));
    let position = field.rect.left_center() + vec2(28.0, 0.0);
    frame(&ctx, pointer(position, true), |ui| draw(ui, &mut value));
    frame(&ctx, pointer(position, false), |ui| draw(ui, &mut value));
    frame(&ctx, vec![], |ui| draw(ui, &mut value));
    frame(&ctx, pointer(position, true), |ui| draw(ui, &mut value));
    let mut release_and_type = pointer(position, false);
    release_and_type.push(Event::Text("7".into()));
    frame(&ctx, release_and_type, |ui| draw(ui, &mut value));
    assert_eq!(
        value, 7.0,
        "coalesced input must replace the complete decimal"
    );
    assert!(
        ctx.input(|input| input.pointer.button_double_clicked(PointerButton::Primary)),
        "pointer state must remain available to other widgets"
    );
    frame(&ctx, vec![Event::Text("2".into())], |ui| {
        draw(ui, &mut value)
    });
    assert_eq!(value, 72.0, "later typing must append to the new value");
}

#[test]
fn disabled_number_cannot_take_focus_or_change_value_from_mouse_and_text() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 120.72;
    let draw = |ui: &mut Ui, value: &mut f64| {
        ui.add_enabled(false, daw_ui::number(value).fixed_decimals(2))
    };
    let field = frame(&ctx, vec![], |ui| draw(ui, &mut value));
    for _ in 0..2 {
        frame(&ctx, pointer(field.rect.center(), true), |ui| {
            draw(ui, &mut value)
        });
        frame(&ctx, pointer(field.rect.center(), false), |ui| {
            draw(ui, &mut value)
        });
    }
    let response = frame(&ctx, vec![Event::Text("7".into())], |ui| {
        draw(ui, &mut value)
    });
    assert!(!response.has_focus());
    assert!(!response.changed());
    assert_eq!(value, 120.72);
}

#[test]
fn tab_focus_still_selects_the_complete_next_number() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let (mut first, mut second) = (12.5, -2.5);
    let mut second_id = None;
    let mut draw = |ui: &mut Ui| {
        ui.horizontal(|ui| {
            let first = ui.add(daw_ui::number(&mut first));
            second_id = Some(ui.add(daw_ui::number(&mut second).suffix(" dB")).id);
            first
        })
        .inner
    };
    let first_response = frame(&ctx, vec![], &mut draw);
    first_response.request_focus();
    frame(&ctx, vec![], &mut draw);
    frame(
        &ctx,
        vec![Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        &mut draw,
    );
    frame(&ctx, vec![], &mut draw);
    let focused = ctx.memory(|memory| memory.focused());
    frame(&ctx, vec![Event::Text("-24".into())], &mut draw);
    assert_eq!(focused, second_id);
    assert_eq!(first, 12.5);
    assert_eq!(second, -24.0);
}

#[test]
fn shared_number_keeps_native_dragging_and_delayed_commit() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut value = 12.0;
    let draw = |ui: &mut Ui, value: &mut f64| {
        ui.add(daw_ui::number(value).speed(0.1).range(-100.0..=100.0))
    };
    let original = frame(&ctx, vec![], |ui| draw(ui, &mut value));
    let start = original.rect.center();
    frame(&ctx, pointer(start, true), |ui| draw(ui, &mut value));
    frame(
        &ctx,
        vec![Event::PointerMoved(start + vec2(50.0, 0.0))],
        |ui| draw(ui, &mut value),
    );
    frame(&ctx, pointer(start + vec2(50.0, 0.0), false), |ui| {
        draw(ui, &mut value)
    });
    assert!(value > 12.0 && value <= 17.1, "native drag: {value}");

    // Acquire text-entry focus through the same mouse gesture on both native
    // and shared controls. Calling request_focus between frames does not count
    // as newly gained focus during egui's next pass.
    let committed = value;
    for native in [true, false] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut value = committed;
        let draw = |ui: &mut Ui, value: &mut f64| {
            if native {
                ui.add(egui::DragValue::new(value).update_while_editing(false))
            } else {
                ui.add(daw_ui::number(value).update_while_editing(false))
            }
        };
        let field = frame(&ctx, vec![], |ui| draw(ui, &mut value));
        frame(&ctx, pointer(field.rect.center(), true), |ui| {
            draw(ui, &mut value)
        });
        frame(&ctx, pointer(field.rect.center(), false), |ui| {
            draw(ui, &mut value)
        });
        frame(&ctx, vec![], |ui| draw(ui, &mut value));
        frame(&ctx, vec![Event::Text("7".into())], |ui| {
            draw(ui, &mut value)
        });
        assert_eq!(
            value, committed,
            "deferred edits must wait for commit; native={native}"
        );
        frame(&ctx, pointer(pos2(400.0, 300.0), true), |ui| {
            draw(ui, &mut value)
        });
        frame(&ctx, pointer(pos2(400.0, 300.0), false), |ui| {
            draw(ui, &mut value)
        });
        frame(&ctx, vec![], |ui| draw(ui, &mut value));
        assert_eq!(value, 7.0, "deferred commit; native={native}");
    }
}
