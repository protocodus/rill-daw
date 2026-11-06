use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

struct Harness {
    ctx: Context,
    gain: f32,
    maximum: f32,
    control: Rect,
}

impl Harness {
    fn new(gain: f32, maximum: f32) -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            gain,
            maximum,
            control: Rect::NOTHING,
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
                    self.control =
                        daw_ui::gain_value_button(ui, "gain", &mut self.gain, self.maximum, "Gain")
                            .rect;
                });
            },
        )
    }

    fn open(&mut self) -> FullOutput {
        self.frame(vec![]);
        let position = self.control.center();
        self.frame(pointer(position, true));
        self.frame(pointer(position, false));
        self.frame(vec![]);
        self.frame(vec![])
    }

    fn click(&mut self, output: &FullOutput, label: &str) {
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        self.frame(pointer(position, true));
        self.frame(pointer(position, false));
        self.frame(vec![]);
    }

    fn enter(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.into())]);
        self.frame(key(Key::Enter));
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
fn gain_axes_use_actual_linear_values_including_send_range_and_quiet_values() {
    assert_eq!(daw_ui::gain_axis_label(0.0), "−∞");
    assert_eq!(daw_ui::gain_axis_label(1.0), "0 dB");
    assert_eq!(daw_ui::gain_axis_label(1.5), "+3.5 dB");
    assert_eq!(daw_ui::gain_axis_label(16.0), "+24.1 dB");
    assert_eq!(daw_ui::gain_axis_label(0.000_000_01), "−160.0 dB");
}

#[test]
fn viewing_unchanged_or_cancelled_gain_preserves_exact_linear_bits() {
    for gain in [0.0, 0.000_000_01, 0.374_981_3, 1.5, 16.0] {
        let mut app = Harness::new(gain, 16.0);
        let before = gain.to_bits();
        app.open();
        app.frame(key(Key::Enter));
        app.frame(key(Key::Escape));
        assert_eq!(app.gain.to_bits(), before, "untouched popup: {gain}");
        app.open();
        app.frame(vec![Event::Text("-8.125".into())]);
        assert_eq!(app.gain.to_bits(), before, "draft must not edit: {gain}");
        app.frame(key(Key::Escape));
        assert_eq!(app.gain.to_bits(), before, "cancelled popup: {gain}");
    }
}

#[test]
fn exact_db_entry_silence_and_bounds_follow_the_target_gain_range() {
    for maximum in [1.5, 16.0] {
        let mut app = Harness::new(0.25, maximum);
        app.open();
        app.enter("-6");
        assert!((app.gain - 10.0_f32.powf(-6.0 / 20.0)).abs() < 0.000_001);

        let output = app.open();
        app.click(&output, "Silence");
        assert_eq!(app.gain, 0.0);

        app.open();
        app.enter("24");
        if maximum == 1.5 {
            assert_eq!(app.gain, 0.0, "track gain cannot use the send range");
            app.frame(key(Key::Escape));
        } else {
            assert!((app.gain - 10.0_f32.powf(24.0 / 20.0)).abs() < 0.000_01);
        }

        app.open();
        app.enter(&f64::from(daw_ui::channel_gain_to_db(maximum)).to_string());
        assert!((app.gain - maximum).abs() < 0.000_01);
        assert!(app.gain <= maximum);

        app.open();
        app.enter("-60");
        assert_eq!(app.gain, 0.0, "the existing dB editing floor means silence");
    }
}
