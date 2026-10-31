use daw_ui::{self as controls, theme};
use egui::{Context, Event, FullOutput, Key, Modifiers, RawInput, Rect, Response, Shape, Ui};

fn input(events: Vec<Event>, width: f32) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 600.0),
        )),
        events,
        ..Default::default()
    }
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

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[derive(Clone, Copy)]
enum SettingsMenu {
    Choice,
    View,
    QuietChoice,
    QuietView,
}

impl SettingsMenu {
    const ALL: [Self; 4] = [Self::Choice, Self::View, Self::QuietChoice, Self::QuietView];

    fn label(self) -> &'static str {
        match self {
            Self::Choice | Self::QuietChoice => "C · Major",
            Self::View | Self::QuietView => "View 🎹",
        }
    }
}

struct MenuFrame {
    opener: Response,
    setting: Option<Response>,
    accept: Option<Response>,
    open: bool,
    output: FullOutput,
}

fn menu_frame(
    ctx: &Context,
    events: Vec<Event>,
    menu: SettingsMenu,
    enabled: bool,
    setting: &mut bool,
) -> MenuFrame {
    let mut opener = None;
    let mut setting_response = None;
    let mut accept = None;
    let mut open = false;
    let output = ctx.run(input(events, 640.0), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let style = ui.style().clone();
                let contents = |ui: &mut Ui| {
                    setting_response = Some(controls::checkbox(ui, setting, "Highlight scale"));
                    let response = controls::button(ui, "Done");
                    if response.clicked() {
                        ui.close();
                    }
                    accept = Some(response);
                };
                let result = match menu {
                    SettingsMenu::View => controls::view_menu(ui, menu.label(), contents),
                    SettingsMenu::Choice => controls::choice_menu(ui, menu.label(), contents),
                    SettingsMenu::QuietView => {
                        controls::quiet_view_menu(ui, menu.label(), contents)
                    }
                    SettingsMenu::QuietChoice => {
                        let (response, inner) =
                            controls::quiet_picker_button(ui, menu.label(), contents);
                        egui::InnerResponse::new(inner.map(|inner| inner.inner), response)
                    }
                };
                assert_eq!(*ui.style(), style, "menu opener styles must stay local");
                open = result.inner.is_some();
                opener = Some(result.response);
            });
        });
    });
    MenuFrame {
        opener: opener.unwrap(),
        setting: setting_response,
        accept,
        open,
        output,
    }
}

#[test]
fn quiet_action_menu_opens_by_mouse_runs_once_and_dismisses_without_losing_its_target() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut invoked = 0;
        let mut draw = |events: Vec<Event>, enabled: bool| {
            let mut opener = None;
            let mut action = None;
            let mut open = false;
            let _ = ctx.run(input(events, 640.0), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        let style = ui.style().clone();
                        let result = controls::quiet_action_menu(ui, "Notes", |ui| {
                            let response = controls::button(ui, "Duplicate");
                            if response.clicked() {
                                invoked += 1;
                            }
                            action = Some(response);
                        });
                        assert_eq!(*ui.style(), style);
                        open = result.inner.is_some();
                        opener = Some(result.response);
                    });
                });
            });
            (opener.unwrap(), action, open, invoked)
        };
        let idle = draw(vec![], true);
        assert!(!idle.2);
        assert!(idle.0.rect.height() >= controls::tokens::CONTROL_HEIGHT);
        let target = idle.0.rect.center();
        draw(pointer(target, true), true);
        draw(pointer(target, false), true);
        let opened = draw(vec![], true);
        assert!(opened.2);
        assert_eq!(opened.3, 0);
        let command = opened.1.unwrap().rect.center();
        draw(pointer(command, true), true);
        draw(pointer(command, false), true);
        let closed = draw(vec![], true);
        assert!(!closed.2);
        assert_eq!(closed.3, 1);
        assert_eq!(closed.0.rect, idle.0.rect);
        assert_eq!(closed.0.id, idle.0.id);

        let disabled = draw(vec![], false);
        disabled.0.request_focus();
        assert!(!draw(key(Key::Enter, true), false).2);
        draw(pointer(target, true), false);
        let disabled = draw(pointer(target, false), false);
        assert!(!disabled.2 && !disabled.0.enabled());
        assert_eq!(disabled.3, 1);
    }
}

#[test]
fn settings_menus_keep_toggles_open_support_keyboard_and_dismiss_on_accept_or_escape() {
    for menu in SettingsMenu::ALL {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut value = false;
        let initial = menu_frame(&ctx, vec![], menu, true, &mut value);
        assert!(!initial.open);
        assert!(initial.setting.is_none());
        initial.opener.request_focus();
        let opened = menu_frame(&ctx, key(Key::Enter, true), menu, true, &mut value);
        assert!(opened.open, "focused menus open using Enter");
        assert!(opened.output.platform_output.events.iter().any(|event| {
            let info = event.widget_info();
            info.typ == egui::WidgetType::Button && info.label.as_deref() == Some(menu.label())
        }));
        let settled = menu_frame(&ctx, key(Key::Enter, false), menu, true, &mut value);
        let target = settled.setting.unwrap().rect.center();
        menu_frame(&ctx, pointer(target, true), menu, true, &mut value);
        menu_frame(&ctx, pointer(target, false), menu, true, &mut value);
        assert!(value, "clicking the setting changes its value");
        let toggled = menu_frame(&ctx, vec![], menu, true, &mut value);
        assert!(toggled.open, "changing a view setting keeps the menu open");
        toggled.accept.unwrap().request_focus();
        menu_frame(&ctx, key(Key::Space, true), menu, true, &mut value);
        let closed = menu_frame(&ctx, key(Key::Space, false), menu, true, &mut value);
        assert!(!closed.open, "explicit acceptance dismisses the menu");
        assert_eq!(closed.opener.id, initial.opener.id);
        assert_eq!(closed.opener.rect, initial.opener.rect);

        closed.opener.request_focus();
        menu_frame(&ctx, key(Key::Enter, true), menu, true, &mut value);
        menu_frame(&ctx, key(Key::Enter, false), menu, true, &mut value);
        menu_frame(&ctx, key(Key::Escape, true), menu, true, &mut value);
        assert!(
            !menu_frame(&ctx, key(Key::Escape, false), menu, true, &mut value).open,
            "Escape dismisses view and choice menus"
        );
        assert!(value, "dismissing a menu does not undo its settings");
    }
}

#[test]
fn disabled_settings_menus_ignore_pointer_and_keyboard() {
    for menu in SettingsMenu::ALL {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut value = false;
        let initial = menu_frame(&ctx, vec![], menu, false, &mut value);
        let target = initial.opener.rect.center();
        assert!(!initial.opener.enabled());
        menu_frame(&ctx, pointer(target, true), menu, false, &mut value);
        assert!(!menu_frame(&ctx, pointer(target, false), menu, false, &mut value).open);
        initial.opener.request_focus();
        assert!(!menu_frame(&ctx, key(Key::Enter, true), menu, false, &mut value).open);
        assert!(!value);
    }
}

fn rectangle_style(output: &FullOutput, rect: Rect) -> (egui::Color32, egui::Stroke) {
    fn visit(shape: &Shape, rect: Rect, style: &mut Option<(egui::Color32, egui::Stroke)>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, rect, style);
                }
            }
            Shape::Rect(shape)
                if shape.rect == rect && shape.blur_width == 0.0 && shape.brush.is_none() =>
            {
                // Materials keep the flat base and keyline in separate shapes.
                // The translucent gradient and blurred halo are decoration;
                // the native transparent frame must not erase the material.
                let (fill, stroke) =
                    style.get_or_insert((egui::Color32::TRANSPARENT, egui::Stroke::NONE));
                if shape.fill != egui::Color32::TRANSPARENT {
                    *fill = shape.fill;
                }
                if !shape.stroke.is_empty() {
                    *stroke = shape.stroke;
                }
            }
            _ => {}
        }
    }
    let mut style = None;
    for shape in &output.shapes {
        visit(&shape.shape, rect, &mut style);
    }
    style.expect("each semantic control paints its material frame")
}

#[test]
fn actions_choices_and_views_stay_visually_distinct_and_wrap_in_both_themes() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for width in [340.0, 1000.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            let mut responses = None;
            let output = ctx.run(input(vec![], width), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let original = ui.style().clone();
                    let row = controls::toolbar(ui, |ui| {
                        [
                            controls::button(ui, "Apply"),
                            controls::action_menu(ui, "Notes", |_| {}).response,
                            controls::choice_menu(ui, "C · Natural minor", |_| {}).response,
                            controls::view_menu(ui, "View", |_| {}).response,
                        ]
                    });
                    assert_eq!(*ui.style(), original, "menu styles must not leak");
                    for (index, control) in row.inner.iter().enumerate() {
                        assert!(ui.max_rect().contains_rect(control.rect));
                        assert!(control.rect.height() >= controls::tokens::CONTROL_HEIGHT);
                        for previous in &row.inner[..index] {
                            assert!(
                                !control.rect.intersects(previous.rect),
                                "toolbar hit areas cannot overlap"
                            );
                        }
                    }
                    responses = Some(row.inner);
                });
            });
            let [action, menu, choice, view] = responses.unwrap();
            let action = rectangle_style(&output, action.rect);
            let menu = rectangle_style(&output, menu.rect);
            let choice = rectangle_style(&output, choice.rect);
            let view = rectangle_style(&output, view.rect);
            assert_ne!(action.0, egui::Color32::TRANSPARENT);
            assert_eq!(action.1.width, controls::tokens::BORDER_WIDTH);
            assert_eq!(
                action.1.color,
                theme::palette(&ctx)
                    .line
                    .gamma_multiply(controls::tokens::GOLDEN_RATIO_INVERSE)
            );
            assert_eq!(menu, action, "action menus share action button treatment");
            assert_eq!(choice.0, theme::palette(&ctx).bg);
            assert!(choice.1.width >= 1.0);
            assert_eq!(view.0, egui::Color32::TRANSPARENT);
            assert!(view.1.width >= 1.0);
        }
    }
}

#[test]
fn nested_choice_menus_preserve_parent_and_accept_selection() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut value = 0;
    let mut draw = |events: Vec<Event>| {
        let mut root = None;
        let mut nested = None;
        let mut choice = None;
        let _ = ctx.run(input(events, 640.0), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let result = controls::choice_menu(ui, "Pitch", |ui| {
                    nested = Some(
                        controls::choice_menu(ui, "Scale", |ui| {
                            let response = controls::selectable_value(ui, &mut value, 1, "Minor");
                            if response.clicked() {
                                ui.close();
                            }
                            choice = Some(response);
                        })
                        .response,
                    );
                });
                root = Some((result.response, result.inner.is_some()));
            });
        });
        (root.unwrap(), nested, choice)
    };
    draw(vec![]).0.0.request_focus();
    draw(key(Key::Enter, true));
    let (_, nested, _) = draw(key(Key::Enter, false));
    nested.unwrap().request_focus();
    draw(key(Key::Space, true));
    let (root, _, choice) = draw(key(Key::Space, false));
    assert!(root.1, "a nested field must retain its parent popup");
    choice.unwrap().request_focus();
    draw(key(Key::Enter, true));
    assert!(!draw(key(Key::Enter, false)).0.1);
    assert_eq!(value, 1);
}
