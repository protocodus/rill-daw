//! Compact captions reuse the deliberate, horizontally scrolling value editor.
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

struct Harness {
    ctx: Context,
    gain: f32,
    pan: f32,
}

impl Harness {
    fn new() -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            gain: 0.5,
            pan: 0.12345679,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    daw_ui::channel_controls_at(
                        ui,
                        Rect::from_min_size(pos2(80.0, 60.0), vec2(162.0, 44.0)),
                        "channel",
                        &mut self.gain,
                        &mut self.pan,
                        [0.0; 2],
                        false,
                    );
                });
            },
        )
    }

    fn click(&mut self, label: &str) {
        let output = self.frame(vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        self.frame(pointer(pos, true));
        self.frame(pointer(pos, false));
        self.frame(vec![]);
    }
}

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
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

#[test]
fn dismissing_an_untouched_or_changed_draft_never_overwrites_external_channel_updates() {
    for type_draft in [false, true] {
        let mut app = Harness::new();
        app.click("-6.0 dB");
        assert!(daw_ui::value_entry_rect(&app.ctx).is_some());
        if type_draft {
            app.frame(vec![Event::Text("-12.3456789".into())]);
        }
        assert_eq!(app.gain, 0.5);
        // Model/automation changes while a precise value is being inspected.
        app.gain = 0.73123455;
        app.frame(vec![]);
        app.click("Cancel");
        assert_eq!(app.gain, 0.73123455);
        assert_eq!(app.pan, 0.12345679);
    }
}

#[test]
fn precise_popup_scrolls_to_keep_home_and_end_carets_inside_the_input() {
    let mut app = Harness::new();
    app.click("-6.0 dB");
    let original = (app.gain, app.pan);
    let input = daw_ui::value_entry_rect(&app.ctx).unwrap();
    app.frame(pointer(input.center(), true));
    app.frame(pointer(input.center(), false));
    app.frame(vec![]);
    assert!(
        daw_ui::value_entry_rect(&app.ctx).is_some(),
        "clicking inside the editor must keep it open"
    );
    app.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    let long = "-12.123456789012345678901234567890123456789012345678901234567890123456789";
    app.frame(vec![Event::Text(long.into())]);
    for key_press in [Key::Home, Key::End] {
        let output = app.frame(key(key_press));
        let field = daw_ui::value_entry_rect(&app.ctx).unwrap();
        let id = app
            .ctx
            .memory(|memory| memory.focused())
            .expect("value input owns focus");
        let state = egui::TextEdit::load_state(&app.ctx, id).unwrap();
        let cursor = state.cursor.char_range().unwrap().primary;
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == long => Some(text),
                _ => None,
            })
            .expect("the exact draft remains painted");
        let caret = text
            .galley
            .pos_from_cursor(cursor)
            .translate(text.pos.to_vec2());
        assert!(
            caret.left() >= field.left() && caret.right() <= field.right(),
            "{key_press:?} caret must remain visible: {caret:?} in {field:?}"
        );
        assert_eq!(
            (app.gain, app.pan),
            original,
            "typing is still only a draft"
        );
        app.frame(vec![Event::Key {
            key: key_press,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
    }
    app.frame(key(Key::Escape));
    assert_eq!((app.gain, app.pan), original);
}
