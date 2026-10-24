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
            .frame(widgets::panel())
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
        widgets::popup("Inspector").open(&mut open).show(ctx, |ui| {
            rendered = true;
            widgets::button(ui, "Remove");
        });
    });
    assert!(!rendered);
}
