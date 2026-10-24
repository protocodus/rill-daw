use daw_ui as widgets;
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Response, Ui};

fn context() -> Context {
    let context = Context::default();
    widgets::theme::apply(&context);
    context
}

fn frame(
    context: &Context,
    events: Vec<Event>,
    mut add: impl FnMut(&mut Ui) -> Response,
) -> (Response, FullOutput) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 480.0),
        )),
        events,
        ..Default::default()
    };
    let mut response = None;
    let output = context.run(input, |ctx| {
        egui::CentralPanel::default()
            .frame(widgets::panel(ctx))
            .show(ctx, |ui| {
                response = Some(add(ui));
            });
    });
    (response.unwrap(), output)
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
fn clicking_a_switch_label_updates_value_and_accessibility_metadata() {
    let context = context();
    let mut value = false;
    let (initial, _) = frame(&context, vec![], |ui| {
        widgets::switch(ui, &mut value, "Enable instrument")
    });
    let position = initial.rect.right_center() - egui::vec2(3.0, 0.0);
    frame(&context, pointer(position, true), |ui| {
        widgets::switch(ui, &mut value, "Enable instrument")
    });
    let (response, output) = frame(&context, pointer(position, false), |ui| {
        widgets::switch(ui, &mut value, "Enable instrument")
    });
    assert!(value);
    assert!(response.changed());
    assert!(output.platform_output.events.iter().any(|event| {
        let info = event.widget_info();
        info.typ == egui::WidgetType::Checkbox
            && info.label.as_deref() == Some("Enable instrument")
            && info.selected == Some(true)
            && info.enabled
    }));
}

#[test]
fn boolean_controls_support_keyboard_activation_and_respect_disabled_state() {
    for control in 0..3 {
        let context = context();
        let mut value = false;
        let draw = |ui: &mut Ui, value: &mut bool| match control {
            0 => widgets::switch(ui, value, "Setting"),
            1 => widgets::checkbox(ui, value, "Setting"),
            _ => widgets::toggle(ui, value, "Setting"),
        };
        let (initial, _) = frame(&context, vec![], |ui| draw(ui, &mut value));
        initial.request_focus();
        let (response, _) = frame(&context, key(Key::Space, true), |ui| draw(ui, &mut value));
        assert!(value, "keyboard activation failed for control {control}");
        assert!(response.changed());
        frame(&context, key(Key::Space, false), |ui| draw(ui, &mut value));
        let (disabled, _) = frame(&context, key(Key::Enter, true), |ui| {
            ui.add_enabled_ui(false, |ui| draw(ui, &mut value)).inner
        });
        assert!(value, "disabled keyboard input changed control {control}");
        assert!(!disabled.changed());
        assert!(!disabled.enabled());
        let position = disabled.rect.center();
        frame(&context, pointer(position, true), |ui| {
            ui.add_enabled_ui(false, |ui| draw(ui, &mut value)).inner
        });
        frame(&context, pointer(position, false), |ui| {
            ui.add_enabled_ui(false, |ui| draw(ui, &mut value)).inner
        });
        assert!(value, "disabled pointer input changed control {control}");
    }
}

#[test]
fn radio_and_selectable_choices_report_only_actual_value_changes() {
    for control in 0..2 {
        let context = context();
        let mut value = 0;
        let draw = |ui: &mut Ui, value: &mut i32| match control {
            0 => widgets::radio_value(ui, value, 2, "Alternate"),
            _ => widgets::selectable_value(ui, value, 2, "Alternate"),
        };
        let (initial, _) = frame(&context, vec![], |ui| draw(ui, &mut value));
        initial.request_focus();
        let (changed, _) = frame(&context, key(Key::Enter, true), |ui| draw(ui, &mut value));
        assert_eq!(value, 2);
        assert!(changed.changed());
        frame(&context, key(Key::Enter, false), |ui| draw(ui, &mut value));
        let (unchanged, _) = frame(&context, key(Key::Enter, true), |ui| draw(ui, &mut value));
        assert!(!unchanged.changed());
    }
}

#[test]
fn text_field_accepts_unicode_keyboard_text() {
    let context = context();
    let mut value = String::new();
    let (initial, _) = frame(&context, vec![], |ui| {
        widgets::text_field(ui, &mut value, "Track name", 200.0)
    });
    initial.request_focus();
    let (edited, _) = frame(&context, vec![Event::Text("Piano ♯".into())], |ui| {
        widgets::text_field(ui, &mut value, "Track name", 200.0)
    });
    assert_eq!(value, "Piano ♯");
    assert!(edited.changed());
}

#[test]
fn canvas_control_preserves_parent_layout_and_fits_compact_bounds() {
    let context = context();
    let rect = egui::Rect::from_min_size(egui::pos2(80.0, 70.0), egui::vec2(24.0, 21.0));
    let (response, _) = frame(&context, vec![], |ui| {
        let cursor_before = ui.cursor();
        let minimum_before = ui.min_rect();
        let response = widgets::selectable_at(ui, rect, (7_u64, "mute"), false, "M");
        assert_eq!(
            ui.cursor(),
            cursor_before,
            "canvas controls must not advance row layout"
        );
        assert_eq!(
            ui.min_rect(),
            minimum_before,
            "canvas controls must not enlarge row layout"
        );
        response
    });
    assert!(rect.contains_rect(response.rect));
    response.request_focus();
    let (pressed, _) = frame(&context, key(Key::Space, true), |ui| {
        widgets::selectable_at(ui, rect, (7_u64, "mute"), false, "M")
    });
    assert!(
        pressed.clicked(),
        "compact controls retain keyboard activation"
    );
    assert_eq!(pressed.id, response.id);
}

#[test]
fn canvas_button_truncates_without_expanding_or_clicking_its_row() {
    let context = context();
    let draw = |ui: &mut Ui, label: &str| {
        let (row, row_response) =
            ui.allocate_exact_size(egui::vec2(248.0, 44.0), egui::Sense::click());
        let rect =
            egui::Rect::from_min_size(row.min + egui::vec2(128.0, 25.0), egui::vec2(112.0, 18.0));
        let cursor = ui.cursor();
        let minimum = ui.min_rect();
        let response = widgets::button_at(
            ui,
            rect,
            (7_u64, "instrument"),
            egui::RichText::new(label).size(10.0),
        );
        assert_eq!(ui.cursor(), cursor);
        assert_eq!(ui.min_rect(), minimum);
        assert!(rect.contains_rect(response.rect));
        assert!(row.contains_rect(response.rect));
        assert!(
            !row_response.clicked(),
            "button input must not select its row"
        );
        response
    };
    let long_label = "An instrument name much longer than the available channel header";
    let (initial, _) = frame(&context, vec![], |ui| draw(ui, "Choose instrument…"));
    let position = initial.rect.center();
    frame(&context, pointer(position, true), |ui| draw(ui, long_label));
    let (clicked, output) = frame(&context, pointer(position, false), |ui| {
        draw(ui, long_label)
    });
    assert!(clicked.clicked());
    assert_eq!(
        clicked.id, initial.id,
        "changing the label must preserve input identity"
    );
    assert_eq!(clicked.rect, initial.rect);
    assert!(
        output.platform_output.events.iter().any(|event| {
            let info = event.widget_info();
            info.typ == egui::WidgetType::Button && info.label.as_deref() == Some(long_label)
        }),
        "accessibility must retain the complete truncated label"
    );
}

struct PickerSnapshot {
    button: Response,
    choice: Option<Response>,
    open: bool,
}

fn canvas_picker_frame(
    context: &Context,
    events: Vec<Event>,
    value: &mut i32,
    enabled: bool,
) -> PickerSnapshot {
    let mut choice = None;
    let mut open = false;
    let (button, _) = frame(context, events, |ui| {
        if !enabled {
            ui.disable();
        }
        let (row, row_response) =
            ui.allocate_exact_size(egui::vec2(248.0, 44.0), egui::Sense::click());
        let rect =
            egui::Rect::from_min_size(row.min + egui::vec2(14.0, 25.0), egui::vec2(110.0, 18.0));
        let cursor = ui.cursor();
        let minimum = ui.min_rect();
        let (button, popup) = widgets::picker_at(
            ui,
            rect,
            (7_u64, "output"),
            egui::RichText::new(if *value == 0 {
                "Ch 1 > Master"
            } else {
                "Ch 1 > A long bus name"
            })
            .size(10.0),
            |ui| {
                ui.set_min_width(160.0);
                if widgets::selectable_value(ui, value, 0, "Master").clicked() {
                    ui.close();
                }
                let response = widgets::selectable_value(ui, value, 1, "A long bus name");
                if response.clicked() {
                    ui.close();
                }
                choice = Some(response);
            },
        );
        open = popup.is_some();
        assert_eq!(
            ui.cursor(),
            cursor,
            "opening the picker must not advance the canvas"
        );
        assert_eq!(
            ui.min_rect(),
            minimum,
            "a popup must not expand its canvas row"
        );
        assert!(rect.contains_rect(button.rect));
        assert!(row.contains_rect(button.rect));
        assert!(
            !row_response.clicked(),
            "picker input must not reach the underlying row"
        );
        let (next_row, _) = ui.allocate_exact_size(egui::vec2(248.0, 44.0), egui::Sense::hover());
        assert_eq!(next_row.top(), row.bottom() + ui.spacing().item_spacing.y);
        button
    });
    PickerSnapshot {
        button,
        choice,
        open,
    }
}

#[test]
fn canvas_picker_supports_keyboard_selection_and_escape_without_expanding_rows() {
    let context = context();
    let mut value = 0;
    let initial = canvas_picker_frame(&context, vec![], &mut value, true);
    assert!(!initial.open);
    assert!(
        initial.choice.is_none(),
        "closed pickers must not render their contents"
    );
    initial.button.request_focus();
    let opened = canvas_picker_frame(&context, key(Key::Enter, true), &mut value, true);
    assert!(opened.open, "a focused picker must open from the keyboard");
    let settled = canvas_picker_frame(&context, key(Key::Enter, false), &mut value, true);
    settled.choice.unwrap().request_focus();
    canvas_picker_frame(&context, key(Key::Enter, true), &mut value, true);
    assert_eq!(value, 1);
    let closed = canvas_picker_frame(&context, key(Key::Enter, false), &mut value, true);
    assert!(!closed.open, "choosing an output must close the picker");
    assert_eq!(closed.button.id, initial.button.id);
    assert_eq!(closed.button.rect, initial.button.rect);

    closed.button.request_focus();
    assert!(canvas_picker_frame(&context, key(Key::Space, true), &mut value, true).open);
    canvas_picker_frame(&context, key(Key::Space, false), &mut value, true);
    canvas_picker_frame(&context, key(Key::Escape, true), &mut value, true);
    assert!(!canvas_picker_frame(&context, key(Key::Escape, false), &mut value, true).open);
    assert_eq!(value, 1, "Escape must leave the current output unchanged");
}

#[test]
fn disabled_canvas_picker_ignores_pointer_and_keyboard_input() {
    let context = context();
    let mut value = 0;
    let initial = canvas_picker_frame(&context, vec![], &mut value, false);
    assert!(!initial.button.enabled());
    let position = initial.button.rect.center();
    canvas_picker_frame(&context, pointer(position, true), &mut value, false);
    assert!(!canvas_picker_frame(&context, pointer(position, false), &mut value, false).open);
    initial.button.request_focus();
    assert!(!canvas_picker_frame(&context, key(Key::Enter, true), &mut value, false).open);
    assert_eq!(value, 0);
}

#[test]
fn application_palette_follows_system_appearance_without_losing_shared_style() {
    let context = Context::default();
    // eframe can supply the system appearance before the app is created.
    let _ = context.run(
        RawInput {
            system_theme: Some(egui::Theme::Light),
            ..Default::default()
        },
        |_| {},
    );
    widgets::theme::apply(&context);
    for system_theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
        let _ = context.run(
            RawInput {
                system_theme: Some(system_theme),
                ..Default::default()
            },
            |ctx| {
                assert_eq!(
                    ctx.style().visuals.dark_mode,
                    system_theme == egui::Theme::Dark
                );
                let colors = widgets::theme::palette(ctx);
                assert_eq!(ctx.style().visuals.override_text_color, Some(colors.text));
                assert_eq!(ctx.style().visuals.panel_fill, colors.panel);
                assert_eq!(
                    ctx.style().spacing.interact_size.y,
                    widgets::tokens::CONTROL_HEIGHT
                );
            },
        );
    }
}

#[test]
fn select_builder_opens_choices_and_selects_a_value() {
    let context = context();
    let mut value = 0;
    let mut choice_rect = None;
    let mut draw = |ui: &mut Ui| {
        widgets::select("instrument", if value == 0 { "Sine" } else { "Triangle" })
            .show_ui(ui, |ui| {
                widgets::selectable_value(ui, &mut value, 0, "Sine");
                let choice = widgets::selectable_value(ui, &mut value, 1, "Triangle");
                choice_rect = Some(choice.rect);
                if choice.clicked() {
                    ui.close();
                }
            })
            .response
    };
    let (initial, _) = frame(&context, vec![], &mut draw);
    frame(&context, pointer(initial.rect.center(), true), &mut draw);
    frame(&context, pointer(initial.rect.center(), false), &mut draw);
    // Let the popup finish its first layout before targeting a choice.
    frame(&context, vec![], &mut draw);
    let position = choice_rect
        .expect("select popup must contain choices")
        .center();
    let mut draw = |ui: &mut Ui| {
        widgets::select("instrument", if value == 0 { "Sine" } else { "Triangle" })
            .show_ui(ui, |ui| {
                widgets::selectable_value(ui, &mut value, 0, "Sine");
                if widgets::selectable_value(ui, &mut value, 1, "Triangle").clicked() {
                    ui.close();
                }
            })
            .response
    };
    frame(&context, pointer(position, true), &mut draw);
    frame(&context, pointer(position, false), &mut draw);
    assert_eq!(value, 1);
}

#[test]
fn closed_popup_does_not_render_contents() {
    let context = context();
    let mut open = false;
    let mut rendered = false;
    let _ = context.run(RawInput::default(), |ctx| {
        widgets::popup("Inspector", ctx)
            .open(&mut open)
            .show(ctx, |ui| {
                rendered = true;
                widgets::button(ui, "Remove");
            });
    });
    assert!(!rendered);
}
