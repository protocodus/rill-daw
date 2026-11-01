//! A parameter's rail/dial and focused native number always share its live value.
use egui::{
    Context, Event, FullOutput, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2,
};

#[derive(Clone, Copy, Debug)]
enum Control {
    Slider,
    Knob,
}

struct Harness {
    ctx: Context,
    value: f64,
    kind: Control,
    number: Rect,
    origin: Pos2,
    destination: Pos2,
    changed: bool,
}

impl Harness {
    fn new(kind: Control) -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            value: 0.25,
            kind,
            number: Rect::NOTHING,
            origin: Pos2::ZERO,
            destination: Pos2::ZERO,
            changed: false,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        let output = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 420.0))),
                time: Some(self.ctx.cumulative_frame_nr() as f64 * 0.1),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().slider_width = 200.0;
                    match self.kind {
                        Control::Slider => {
                            let response = ui.add(
                                daw_ui::slider(&mut self.value, 0.0..=16.0)
                                    .min_decimals(2)
                                    .max_decimals(6)
                                    .smart_aim(false)
                                    .suffix("×"),
                            );
                            let geometry = daw_ui::slider_scale_geometry(ctx, response.id).unwrap();
                            self.origin = geometry.handle;
                            self.destination = geometry.minimum.lerp(geometry.maximum, 0.25);
                            self.changed = response.changed();
                        }
                        Control::Knob => {
                            let response = ui.add(
                                daw_ui::knob(&mut self.value, 0.0..=16.0)
                                    .min_decimals(2)
                                    .max_decimals(6)
                                    .suffix("×"),
                            );
                            let geometry = daw_ui::knob_geometry(ctx, response.id).unwrap();
                            self.number = geometry.number;
                            self.origin = geometry.center;
                            self.destination = geometry.center + vec2(0.0, -12.0);
                            self.changed = response.changed();
                        }
                    }
                    let _ = daw_ui::button(ui, "Done");
                });
            },
        );
        if matches!(self.kind, Control::Slider) && self.number == Rect::NOTHING {
            self.number = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "0.25×" => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .expect("slider paints its current numeric field");
        }
        output
    }

    fn focus_number(&mut self) -> Id {
        self.frame(vec![]);
        let center = self.number.center();
        self.frame(pointer(center, true));
        self.frame(pointer(center, false));
        self.frame(vec![]);
        self.ctx
            .memory(|memory| memory.focused())
            .expect("number has native text focus")
    }

    fn text(&self, id: Id) -> Option<String> {
        self.ctx.data(|data| data.get_temp::<String>(id))
    }

    fn blur(&mut self) {
        self.frame(pointer(pos2(560.0, 380.0), true));
        self.frame(pointer(pos2(560.0, 380.0), false));
        self.frame(vec![]);
    }
}

fn pointer(position: Pos2, pressed: bool) -> Vec<Event> {
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

fn key(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn caret(ctx: &Context, id: Id) -> std::ops::Range<usize> {
    egui::TextEdit::load_state(ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap()
        .as_sorted_char_range()
}

fn painted_number(output: &FullOutput, number: Rect) -> f64 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if number
                    .expand(4.0)
                    .intersects(text.galley.rect.translate(text.pos.to_vec2())) =>
            {
                text.galley.text().trim_end_matches('×').trim().parse().ok()
            }
            _ => None,
        })
        .expect("a numeric value is visibly painted in the number field")
}

#[test]
fn focused_stable_draft_and_caret_survive_idle_frames_for_slider_and_knob() {
    for kind in [Control::Slider, Control::Knob] {
        let mut app = Harness::new(kind);
        let id = app.focus_number();
        // Numerically unchanged text is still a real draft: trailing decimal
        // digits and the user's insertion position must survive repainting.
        app.frame(vec![Event::Text("0.2500".into())]);
        assert_eq!(app.value, 0.25, "{kind:?}");
        assert_eq!(app.text(id).as_deref(), Some("0.2500"));
        app.frame(key(Key::ArrowLeft));
        let selected = caret(&app.ctx, id);
        assert_eq!(selected, 5..5);
        for _ in 0..4 {
            let shown = app.frame(vec![]);
            assert_eq!(app.value, 0.25, "{kind:?}");
            assert_eq!(app.text(id).as_deref(), Some("0.2500"), "{kind:?}");
            assert_eq!(caret(&app.ctx, id), selected, "{kind:?}");
            assert!(
                !app.changed,
                "a stable focused field is not a parameter edit"
            );
            assert_eq!(painted_number(&shown, app.number), 0.25);
        }
    }
}

#[test]
fn genuine_external_change_refreshes_focused_text_and_cannot_be_overwritten_on_blur() {
    for kind in [Control::Slider, Control::Knob] {
        let mut app = Harness::new(kind);
        let id = app.focus_number();
        app.frame(vec![Event::Text("0.2500".into())]);
        app.frame(key(Key::ArrowLeft));
        assert_eq!(app.text(id).as_deref(), Some("0.2500"));
        // A second view or automation can change the live parameter while the
        // inline number remains focused. That is a genuine model change.
        app.value = 1.0;
        let shown = app.frame(vec![]);
        assert_eq!(
            app.value, 1.0,
            "stale focused text overwrote the live {kind:?} value"
        );
        assert!(
            !app.changed,
            "refreshing feedback must not emit a second edit"
        );
        assert_eq!(painted_number(&shown, app.number), 1.0, "{kind:?}");
        assert_eq!(
            app.text(id).unwrap().parse::<f64>().unwrap(),
            1.0,
            "{kind:?}"
        );
        let refreshed_caret = caret(&app.ctx, id);
        app.frame(vec![]);
        assert_eq!(caret(&app.ctx, id), refreshed_caret);
        app.blur();
        assert_eq!(app.value, 1.0, "blurring restored a stale {kind:?} value");
        assert!(!app.changed);
    }
}

#[test]
fn default_slider_clamps_range_boundaries_without_rounding_in_range_storage() {
    let mut app = Harness::new(Control::Slider);
    app.frame(vec![]);
    for current in [0.123456789, 1.23456789e-7, 15.999999789] {
        app.value = current;
        app.frame(vec![]);
        assert_eq!(app.value.to_bits(), current.to_bits());
        assert!(
            !app.changed,
            "paint must not apply configured edit precision"
        );
    }
    for (outside, clamped) in [(-0.123456789, 0.0), (16.123456789, 16.0)] {
        app.value = outside;
        app.frame(vec![]);
        assert_eq!(
            app.value, clamped,
            "Always retains its range-clamping contract"
        );
        assert!(
            app.changed,
            "clamping an existing out-of-range value is reported"
        );
        app.frame(vec![]);
        assert!(!app.changed);
    }
}

#[test]
fn externally_refreshed_fractional_values_keep_exact_bits_through_idle_and_each_blur_frame() {
    for kind in [Control::Slider, Control::Knob] {
        for current in [0.123_456_789_f64, 0.000_000_123_456_789] {
            let mut app = Harness::new(kind);
            let id = app.focus_number();
            app.frame(vec![Event::Text("0.2500".into())]);
            app.frame(key(Key::ArrowLeft));
            app.value = current;
            app.frame(vec![]);
            assert_eq!(
                app.value.to_bits(),
                current.to_bits(),
                "refreshing {kind:?} feedback must retain the exact incoming value"
            );
            assert!(
                !app.changed,
                "refreshing {kind:?} feedback is not a user edit"
            );
            let refreshed_text = app.text(id).expect("refreshed focused draft");
            assert_ne!(refreshed_text, "0.2500");
            let refreshed_caret = caret(&app.ctx, id);
            for _ in 0..3 {
                app.frame(vec![]);
                assert_eq!(app.value.to_bits(), current.to_bits(), "{kind:?}");
                assert_eq!(app.text(id).as_deref(), Some(refreshed_text.as_str()));
                assert_eq!(caret(&app.ctx, id), refreshed_caret);
                assert!(
                    !app.changed,
                    "an idle refreshed {kind:?} must not emit an edit"
                );
            }
            // DragValue reparses its displayed text on the focus-loss frame.
            // Check that exact frame too: a later idle frame would hide a
            // transient changed() and an unsolicited application history edit.
            for events in [
                pointer(pos2(560.0, 380.0), true),
                pointer(pos2(560.0, 380.0), false),
                vec![],
            ] {
                app.frame(events);
                assert_eq!(
                    app.value.to_bits(),
                    current.to_bits(),
                    "blurring {kind:?} must not commit its rounded display for {current}"
                );
                assert!(
                    !app.changed,
                    "blurring an unchanged refreshed {kind:?} must not emit an edit"
                );
            }
        }
    }
}

#[test]
fn a_real_text_edit_after_external_fractional_refresh_still_commits() {
    for kind in [Control::Slider, Control::Knob] {
        for current in [0.123_456_789_f64, 0.000_000_123_456_789] {
            let mut app = Harness::new(kind);
            app.focus_number();
            app.value = current;
            app.frame(vec![]);
            assert_eq!(app.value.to_bits(), current.to_bits());
            // Select through native keyboard input after the refresh so the
            // new text is an intentional edit, independent of prior caret state.
            app.frame(vec![Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }]);
            app.frame(vec![Event::Text("0.625123".into())]);
            assert_eq!(
                app.value.to_bits(),
                0.625_123_f64.to_bits(),
                "an intentional {kind:?} edit after refresh must reach the parameter"
            );
            assert!(
                app.changed,
                "the intentional {kind:?} edit must be reported"
            );
            app.blur();
            assert_eq!(app.value.to_bits(), 0.625_123_f64.to_bits());
        }
    }
}

#[test]
fn dragging_a_sibling_rail_or_dial_updates_the_number_without_a_stale_commit() {
    for kind in [Control::Slider, Control::Knob] {
        let mut app = Harness::new(kind);
        let id = app.focus_number();
        assert_eq!(app.text(id).unwrap().parse::<f64>().unwrap(), 0.25);
        let origin = app.origin;
        let destination = app.destination;
        app.frame(pointer(origin, true));
        app.frame(vec![Event::PointerMoved(destination)]);
        let changed = app.value;
        assert!(
            changed > 0.25,
            "the {kind:?} gesture must actually edit its parameter"
        );
        let shown = app.frame(pointer(destination, false));
        assert_eq!(
            app.value, changed,
            "release restored the previous numeric draft"
        );
        assert!(
            (painted_number(&shown, app.number) - changed).abs() < 0.000_01,
            "the {kind:?} number disagrees with the live rail/dial after dragging"
        );
        app.blur();
        assert_eq!(
            app.value, changed,
            "blur restored a stale {kind:?} numeric value"
        );
        app.focus_number();
        let focused = app.ctx.memory(|memory| memory.focused()).unwrap();
        assert!(
            (app.text(focused).unwrap().parse::<f64>().unwrap() - changed).abs() < 0.000_01,
            "reopening the {kind:?} number must show its current value"
        );
    }
}
