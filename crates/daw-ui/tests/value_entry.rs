use daw_ui::{AutomationControl, AutomationState, automatable_parameter_value_context};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

struct Harness {
    ctx: Context,
    value: f64,
    control: Rect,
    automation: Option<(u64, AutomationState)>,
    action: Option<daw_ui::AutomationNavigation>,
}

impl Harness {
    fn new() -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            value: 1.25,
            control: Rect::NOTHING,
            automation: None,
            action: None,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        let mut selected = None;
        let output = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = daw_ui::button(ui, "Parameter");
                    self.control = response.rect;
                    let (action, value) = automatable_parameter_value_context(
                        ui,
                        &response,
                        AutomationControl::Pan { track: 42 },
                        self.automation,
                        self.value,
                        -60.0..=6.0,
                        "dB",
                    );
                    self.action = action;
                    selected = value;
                });
            },
        );
        if let Some(value) = selected {
            self.value = value;
        }
        output
    }

    fn open(&mut self) -> FullOutput {
        self.frame(vec![]);
        self.frame(pointer(
            self.control.center(),
            PointerButton::Secondary,
            true,
        ));
        self.frame(pointer(
            self.control.center(),
            PointerButton::Secondary,
            false,
        ));
        self.frame(vec![])
    }

    fn click(&mut self, output: &FullOutput, label: &str) -> FullOutput {
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label}"));
        self.frame(pointer(pos, PointerButton::Primary, true));
        self.frame(pointer(pos, PointerButton::Primary, false))
    }
}

fn pointer(pos: egui::Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn key(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn contains(output: &FullOutput, needle: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text().contains(needle))
    })
}

#[test]
fn exact_entry_selects_current_value_keeps_draft_and_only_commits_on_apply() {
    let mut app = Harness::new();
    let opened = app.open();
    assert!(contains(&opened, "Range: -60 to 6 dB"));
    assert!(contains(&opened, "1.25"));
    assert!(daw_ui::value_entry_rect(&app.ctx).is_some());
    let editing = app.frame(vec![Event::Text("-12.3456789".into())]);
    assert!(
        contains(&editing, "-12.3456789"),
        "{:?} focus {:?}",
        editing
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        app.ctx.memory(|memory| memory.focused())
    );
    assert_eq!(app.value, 1.25, "typing is not a parameter gesture");
    // The sounding value may keep moving while the user edits a draft.
    app.value = -8.0;
    let retained = app.frame(vec![]);
    assert!(contains(&retained, "-12.3456789"));
    app.click(&retained, "Apply value");
    assert_eq!(app.value, -12.3456789);
    assert!(daw_ui::value_entry_rect(&app.ctx).is_none());
}

#[test]
fn invalid_values_never_clamp_or_commit_and_cancel_reopens_fresh() {
    let mut app = Harness::new();
    for text in ["NaN", "inf", "7", "-61", "not a number"] {
        app.open();
        let invalid = app.frame(vec![Event::Text(text.into())]);
        assert!(
            contains(
                &invalid,
                if text == "7" || text == "-61" {
                    "The value must be inside the range."
                } else {
                    "Enter a finite number."
                }
            ),
            "input {text}: {:?}",
            invalid
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
        let after = app.click(&invalid, "Apply value");
        assert_eq!(app.value, 1.25);
        assert!(contains(&after, "Cancel"));
        let retained = app.frame(vec![]);
        assert!(
            contains(&retained, "Cancel"),
            "invalid Apply must keep the editor open"
        );
        let field = daw_ui::value_entry_rect(&app.ctx).unwrap();
        app.frame(pointer(field.center(), PointerButton::Primary, true));
        app.frame(pointer(field.center(), PointerButton::Primary, false));
        let clicked_field = app.frame(vec![]);
        assert!(
            contains(&clicked_field, "Cancel"),
            "mouse caret changes must not dismiss the editor"
        );
        app.click(&clicked_field, "Cancel");
        assert_eq!(app.value, 1.25);
    }
    app.value = 0.123456789;
    let fresh = app.open();
    assert!(contains(&fresh, "0.123456789"));
    let editing = app.frame(vec![Event::Text("2e-3".into())]);
    app.click(&editing, "Apply value");
    assert_eq!(app.value, 0.002);
}

#[test]
fn focused_enter_applies_escape_discards_and_automation_actions_share_the_menu() {
    let mut app = Harness::new();
    app.open();
    app.frame(vec![Event::Text("-3.75".into())]);
    app.frame(key(Key::Escape));
    assert_eq!(app.value, 1.25);
    app.open();
    app.frame(vec![Event::Text("-3.75".into())]);
    app.frame(key(Key::Enter));
    assert_eq!(app.value, -3.75);
    app.automation = Some((9, AutomationState::Read));
    let opened = app.open();
    assert!(contains(&opened, "Edit automation"));
    let value = app.value;
    app.click(&opened, "Override automation");
    assert!(matches!(
        app.action,
        Some(daw_ui::AutomationNavigation::Override(9))
    ));
    assert_eq!(
        app.value, value,
        "navigation must not silently apply a value"
    );
    assert!(!egui::Popup::is_any_open(&app.ctx));
}

#[test]
fn exact_value_formatting_round_trips_extreme_and_fractional_values() {
    for value in [
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -0.0,
        1.0 / 3.0,
        1e-50,
        -123456.789,
    ] {
        let text = daw_ui::exact_value_text(value);
        assert!(text.len() < 128);
        assert_eq!(text.parse::<f64>().unwrap().to_bits(), value.to_bits());
    }
}
