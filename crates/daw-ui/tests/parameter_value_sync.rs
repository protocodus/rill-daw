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
    range: std::ops::RangeInclusive<f64>,
    round_trip: Option<fn(f64) -> f64>,
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
            range: 0.0..=16.0,
            round_trip: None,
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
                            let mut slider = daw_ui::slider(&mut self.value, self.range.clone())
                                .min_decimals(2)
                                .max_decimals(6)
                                .smart_aim(false)
                                .suffix("×");
                            if let Some(convert) = self.round_trip {
                                slider = slider.display_round_trip(convert);
                            }
                            let response = ui.add(slider);
                            let geometry = daw_ui::slider_scale_geometry(ctx, response.id).unwrap();
                            self.origin = geometry.handle;
                            self.destination = geometry.minimum.lerp(geometry.maximum, 0.25);
                            self.changed = response.changed();
                        }
                        Control::Knob => {
                            let mut knob = daw_ui::knob(&mut self.value, self.range.clone())
                                .min_decimals(2)
                                .max_decimals(6)
                                .suffix("×");
                            if let Some(convert) = self.round_trip {
                                knob = knob.display_round_trip(convert);
                            }
                            let response = ui.add(knob);
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

#[derive(Clone, Copy, Debug)]
enum PercentStorage {
    NormalizedF64,
    PanF32,
}

impl PercentStorage {
    fn store(self, percent: f64) -> f64 {
        match self {
            Self::NormalizedF64 => percent / 100.0,
            Self::PanF32 => f64::from(percent as f32 / 100.0),
        }
    }

    fn display(self, normalized: f64) -> f64 {
        match self {
            Self::NormalizedF64 => normalized * 100.0,
            Self::PanF32 => f64::from(normalized as f32 * 100.0),
        }
    }

    fn conversion(self) -> fn(f64) -> f64 {
        match self {
            Self::NormalizedF64 => |value| value / 100.0 * 100.0,
            Self::PanF32 => |value| f64::from(value as f32 / 100.0 * 100.0),
        }
    }
}

struct PercentHarness {
    widget: Harness,
    stored: f64,
    storage: PercentStorage,
}

impl PercentHarness {
    fn new(kind: Control, storage: PercentStorage) -> Self {
        let mut widget = Harness::new(kind);
        widget.range = 0.0..=100.0;
        widget.round_trip = Some(storage.conversion());
        Self {
            widget,
            stored: storage.store(0.25),
            storage,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        self.widget.value = self.storage.display(self.stored);
        let output = self.widget.frame(events);
        if self.widget.changed {
            self.stored = self.storage.store(self.widget.value);
        }
        output
    }

    fn focus(&mut self) -> Id {
        self.frame(vec![]);
        let center = self.widget.number.center();
        self.frame(pointer(center, true));
        self.frame(pointer(center, false));
        self.frame(vec![]);
        self.widget.ctx.memory(|memory| memory.focused()).unwrap()
    }
}

#[test]
fn normalized_model_round_trips_retain_typed_digits_caret_and_real_external_updates() {
    for kind in [Control::Slider, Control::Knob] {
        for (storage, draft) in [
            (PercentStorage::NormalizedF64, "61.234500"),
            (PercentStorage::PanF32, "14.1400"),
        ] {
            let mut app = PercentHarness::new(kind, storage);
            let id = app.focus();
            app.frame(vec![Event::Text(draft.into())]);
            let stored = app.stored;
            app.frame(key(Key::ArrowLeft));
            let insertion = caret(&app.widget.ctx, id);
            for _ in 0..4 {
                app.frame(vec![]);
                assert_eq!(
                    app.widget.text(id).as_deref(),
                    Some(draft),
                    "{kind:?} {storage:?}"
                );
                assert_eq!(caret(&app.widget.ctx, id), insertion);
                assert_eq!(app.stored.to_bits(), stored.to_bits());
                assert!(
                    !app.widget.changed,
                    "a model acknowledgement is not an edit"
                );
            }

            // Replacing the entire value with the same numeric draft still
            // requests a write; preserve its zeros after the owner converts it.
            app.frame(vec![Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }]);
            app.frame(vec![Event::Text(draft.into())]);
            assert_eq!(app.stored.to_bits(), stored.to_bits());
            app.frame(vec![]);
            assert_eq!(app.widget.text(id).as_deref(), Some(draft));
            assert!(!app.widget.changed);

            // A one-representable-step model update must not disappear inside
            // an epsilon, even if both values have the same short formatting.
            let displayed = app.storage.display(app.stored);
            loop {
                app.stored = match storage {
                    PercentStorage::NormalizedF64 => f64::from_bits(app.stored.to_bits() + 1),
                    PercentStorage::PanF32 => {
                        f64::from(f32::from_bits((app.stored as f32).to_bits() + 1))
                    }
                };
                if app.storage.display(app.stored).to_bits() != displayed.to_bits() {
                    break;
                }
            }
            let external = app.stored;
            app.frame(vec![]);
            assert_ne!(
                app.widget.text(id).as_deref(),
                Some(draft),
                "a genuine upstream update refreshes the field"
            );
            assert_eq!(app.stored.to_bits(), external.to_bits());
            assert!(!app.widget.changed);

            // The sibling rail/dial also remains authoritative while a field
            // is focused; conversion-aware ownership must not retain old text.
            let origin = app.widget.origin;
            let destination = app.widget.destination;
            app.frame(pointer(origin, true));
            app.frame(vec![Event::PointerMoved(destination)]);
            app.frame(pointer(destination, false));
            assert_ne!(app.stored.to_bits(), external.to_bits());
            let output = app.frame(vec![]);
            assert!(
                (painted_number(&output, app.widget.number) - storage.display(app.stored)).abs()
                    < 0.00001
            );
            assert!(!app.widget.changed);
        }
    }
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
