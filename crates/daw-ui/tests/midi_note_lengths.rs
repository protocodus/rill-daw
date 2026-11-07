use daw_ui::{MidiNoteLengthMode, MidiNoteLengthResponse, MidiNoteLengthSettings};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

fn frame(
    ctx: &Context,
    settings: &mut MidiNoteLengthSettings,
    width: f32,
    events: Vec<Event>,
) -> (MidiNoteLengthResponse, FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 420.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(daw_ui::midi_note_length_preview(
                    ui,
                    settings,
                    8192,
                    1.0 / 64.0,
                    1_000_000.0,
                ));
            });
        },
    );
    (response.unwrap(), output)
}
fn label_rect(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing label {label}"))
}
fn click(
    ctx: &Context,
    settings: &mut MidiNoteLengthSettings,
    width: f32,
    label: &str,
) -> MidiNoteLengthResponse {
    let (_, output) = frame(ctx, settings, width, vec![]);
    let pos = label_rect(&output, label).center();
    let input = |pressed| {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    frame(ctx, settings, width, input(true));
    frame(ctx, settings, width, input(false)).0
}

#[test]
fn duration_preview_wraps_inside_small_editors_without_rounding_untouched_beats() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for scale in [1.0, 1.25, 2.0] {
            for width in [320.0, 640.0] {
                let ctx = Context::default();
                daw_ui::theme::apply(&ctx);
                ctx.set_theme(theme);
                ctx.set_pixels_per_point(scale);
                for mode in [
                    MidiNoteLengthMode::Fixed,
                    MidiNoteLengthMode::Grow,
                    MidiNoteLengthMode::Shrink,
                ] {
                    let mut settings = MidiNoteLengthSettings {
                        mode,
                        beats: 0.1234567890123,
                    };
                    let original = settings;
                    // A requested DPI change is applied at the next pass;
                    // establish the viewport before measuring stable layout.
                    frame(&ctx, &mut settings, width, vec![]);
                    for _ in 0..3 {
                        let (response, output) = frame(&ctx, &mut settings, width, vec![]);
                        assert_eq!(settings, original);
                        assert!(!response.apply && !response.cancel);
                        assert!(response.amount.is_finite() && response.amount.is_positive());
                        assert!(response.amount.left() >= 0.0 && response.amount.right() <= width);
                        for label in ["Fixed", "Grow", "Shrink", "Apply lengths", "Cancel lengths"]
                        {
                            let rect = label_rect(&output, label);
                            assert!(
                                rect.left() >= 0.0
                                    && rect.right() <= width
                                    && rect.bottom() <= 420.0,
                                "{label}: {rect:?}; {theme:?} scale={scale} width={width}, screen={:?}",
                                ctx.content_rect()
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn duration_modes_and_explicit_commit_cancel_actions_work_with_the_mouse() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut settings = MidiNoteLengthSettings::default();
    click(&ctx, &mut settings, 320.0, "Grow");
    assert_eq!(settings.mode, MidiNoteLengthMode::Grow);
    settings.beats = 0.0;
    click(&ctx, &mut settings, 320.0, "Shrink");
    assert_eq!(settings.beats, 0.0);
    click(&ctx, &mut settings, 320.0, "Fixed");
    assert_eq!(settings.mode, MidiNoteLengthMode::Fixed);
    assert_eq!(settings.beats, 1.0 / 64.0);
    let apply = click(&ctx, &mut settings, 320.0, "Apply lengths");
    assert!(apply.apply && !apply.cancel);
    let cancel = click(&ctx, &mut settings, 320.0, "Cancel lengths");
    assert!(cancel.cancel && !cancel.apply);
}
