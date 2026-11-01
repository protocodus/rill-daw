use daw_ui::{AutomationControl, AutomationState, automatable_parameter_value_context};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

struct Harness {
    ctx: Context,
    value: f64,
    control: Rect,
    automation: Option<(u64, AutomationState)>,
    action: Option<daw_ui::AutomationNavigation>,
    height: f32,
    name: Option<String>,
    range: std::ops::RangeInclusive<f64>,
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
            height: 480.0,
            name: None,
            range: -60.0..=6.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        let mut selected = None;
        let output = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(640.0, self.height),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = daw_ui::button(ui, "Parameter");
                    self.control = response.rect;
                    let (action, value) = if let Some(name) = &self.name {
                        daw_ui::automatable_parameter_value_context_named(
                            ui,
                            &response,
                            AutomationControl::Pan { track: 42 },
                            self.automation,
                            self.value,
                            self.range.clone(),
                            "dB",
                            name,
                        )
                    } else {
                        automatable_parameter_value_context(
                            ui,
                            &response,
                            AutomationControl::Pan { track: 42 },
                            self.automation,
                            self.value,
                            self.range.clone(),
                            "dB",
                        )
                    };
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
    assert!(contains(&fresh, "0.123457"));
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

#[test]
fn concise_parameter_display_removes_float_noise_without_rounding_unchanged_apply() {
    for (stored, displayed) in [
        (14.00000059604645, "14"),
        (15.00000596046448, "15"),
        (0.17345678806304932, "0.173457"),
    ] {
        assert_eq!(daw_ui::parameter_value_text(stored), displayed);
        let mut app = Harness::new();
        // This harness's editable range ends at 6, so keep residue cases in it.
        app.value = stored / 100.0;
        let exact = app.value.to_bits();
        let opened = app.open();
        app.click(&opened, "Apply value");
        assert_eq!(
            app.value.to_bits(),
            exact,
            "display must not quantize storage"
        );
        assert!(daw_ui::value_entry_rect(&app.ctx).is_none());
    }
}

#[test]
fn rounded_boundary_displays_validate_the_untouched_exact_value_and_allow_noop_apply() {
    let maximum_db = f64::from(20.0 * 1.5_f32.log10());
    let minimum = 0.12345621;
    for (stored, range) in [(maximum_db, -60.0..=maximum_db), (minimum, minimum..=1.0)] {
        let display = daw_ui::parameter_value_text(stored);
        assert!(
            !range.contains(&display.parse::<f64>().unwrap()),
            "fixture must round beyond its boundary"
        );
        let mut app = Harness::new();
        app.value = stored;
        app.range = range;
        let opened = app.open();
        assert!(contains(&opened, &display));
        assert!(!contains(&opened, "The value must be inside the range."));
        app.click(&opened, "Apply value");
        assert_eq!(app.value.to_bits(), stored.to_bits());
        assert!(
            daw_ui::value_entry_rect(&app.ctx).is_none(),
            "an untouched valid boundary must remain applicable"
        );

        // Entering the rounded number deliberately is different: it really
        // lies outside the range and must still be rejected without clamping.
        app.open();
        let invalid = app.frame(vec![Event::Text(display)]);
        assert!(contains(&invalid, "The value must be inside the range."));
        app.click(&invalid, "Apply value");
        assert_eq!(app.value.to_bits(), stored.to_bits());
        assert!(daw_ui::value_entry_rect(&app.ctx).is_some());
    }
}

#[test]
fn deliberately_reentering_the_rounded_display_applies_the_requested_exact_value() {
    let mut app = Harness::new();
    app.value = 1.250000059604645;
    let original = app.value;
    let opened = app.open();
    assert!(contains(&opened, "1.25"));
    app.click(&opened, "Apply value");
    assert_eq!(
        app.value, original,
        "untouched Apply preserves the exact value"
    );
    let opened = app.open();
    app.click(&opened, "123");
    let mut keypad = app.frame(vec![]);
    let keys = visible_text(&keypad, "7")
        .expect("first keypad row")
        .center();
    for _ in 0..8 {
        if visible_text(&keypad, "Clear").is_some() {
            break;
        }
        keypad = app.frame(vec![
            Event::PointerMoved(keys),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -40.0),
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    assert!(
        visible_text(&keypad, "Clear").is_some(),
        "mouse scrolling reaches Clear"
    );
    let mut output = app.click(&keypad, "Clear");
    for key in ["1", ".", "2", "5"] {
        output = app.click(&output, key);
    }
    assert_eq!(app.value, original, "keypad edits remain a draft");
    app.click(&output, "Apply value");
    assert_eq!(
        app.value, 1.25,
        "explicit entry replaces conversion residue"
    );
}

#[test]
fn pristine_editor_follows_changed_parameter_while_stable_drafts_keep_the_caret() {
    let mut app = Harness::new();
    app.open();
    app.value = 0.17345678806304932;
    let updated = app.frame(vec![]);
    assert!(contains(&updated, "0.173457"));
    let exact = app.value.to_bits();
    app.click(&updated, "Apply value");
    assert_eq!(app.value.to_bits(), exact);

    app.open();
    app.frame(vec![Event::Text("2.3456789".into())]);
    let focus = app.ctx.memory(|memory| memory.focused()).unwrap();
    let mut edit = egui::TextEdit::load_state(&app.ctx, focus).unwrap();
    edit.cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(4),
        )));
    edit.store(&app.ctx, focus);
    for _ in 0..3 {
        let output = app.frame(vec![]);
        assert!(contains(&output, "2.3456789"));
        let edit = egui::TextEdit::load_state(&app.ctx, focus).unwrap();
        assert_eq!(edit.cursor.char_range().unwrap().primary.index, 4);
    }
    let edited = app.frame(vec![Event::Text("9".into())]);
    app.click(&edited, "Apply value");
    assert_eq!(app.value, 2.34956789, "edits preserve all supplied digits");
}

fn visible_text(output: &FullOutput, label: &str) -> Option<Rect> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            shape.clip_rect.contains_rect(rect).then_some(rect)
        }
        _ => None,
    })
}

#[test]
fn short_parameter_menu_scrolls_only_keys_and_keeps_apply_and_automation_reachable() {
    let mut app = Harness::new();
    app.height = 420.0;
    app.automation = Some((9, AutomationState::Read));
    let compact = app.open();
    assert!(contains(&compact, "123"));
    assert!(!contains(&compact, "Clear"));
    app.click(&compact, "123");
    let mut expanded = app.frame(vec![]);
    let actions = [
        "Apply value",
        "Cancel",
        "Edit automation",
        "Override automation",
    ];
    let positions = actions.map(|name| {
        let rect =
            visible_text(&expanded, name).unwrap_or_else(|| panic!("Missing visible {name}"));
        assert!(rect.center().y - daw_ui::tokens::CONTROL_HEIGHT * 0.5 >= 0.0);
        assert!(rect.center().y + daw_ui::tokens::CONTROL_HEIGHT * 0.5 <= app.height);
        rect.center()
    });
    let keys = visible_text(&expanded, "7")
        .expect("first row of keypad")
        .center();
    for _ in 0..8 {
        if visible_text(&expanded, "Clear").is_some() {
            break;
        }
        expanded = app.frame(vec![
            Event::PointerMoved(keys),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -40.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        for (name, position) in actions.into_iter().zip(positions) {
            assert!(
                visible_text(&expanded, name)
                    .unwrap()
                    .center()
                    .distance(position)
                    < 0.1
            );
        }
    }
    assert!(
        visible_text(&expanded, "Clear").is_some(),
        "mouse scrolling must reveal lower keypad rows"
    );
    let cleared = app.click(&expanded, "Clear");
    assert_eq!(
        app.value, 1.25,
        "keypad drafting must not edit the parameter"
    );
    for (name, position) in actions.into_iter().zip(positions) {
        assert!(
            visible_text(&cleared, name)
                .unwrap()
                .center()
                .distance(position)
                < 0.1
        );
    }
    app.click(&cleared, "Cancel");
    let reopened = app.open();
    assert!(contains(&reopened, "1.25"));
    assert!(!contains(&reopened, "Clear"), "a new editor starts compact");
    assert_eq!(app.value, 1.25);
}

#[test]
fn long_context_titles_keep_the_field_and_actions_inside_a_short_viewport() {
    let mut app = Harness::new();
    app.height = 420.0;
    app.name = Some(format!(
        "{} → Room · Send level",
        "Long channel name ".repeat(30)
    ));
    app.automation = Some((9, AutomationState::Read));
    let opened = app.open();
    app.click(&opened, "123");
    let expanded = app.frame(vec![]);
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, app.height));
    assert!(viewport.contains_rect(daw_ui::value_entry_rect(&app.ctx).unwrap()));
    for name in [
        "Apply value",
        "Cancel",
        "Edit automation",
        "Override automation",
    ] {
        assert!(
            viewport.contains_rect(visible_text(&expanded, name).unwrap()),
            "{name}"
        );
    }
}
