use daw_ui::{
    MidiExpressionControls, MidiExpressionKind, MidiExpressionPoint, MidiExpressionResponse,
};
use egui::{Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

fn frame(
    ctx: &Context,
    controls: &mut MidiExpressionControls,
    points: &[MidiExpressionPoint],
    events: Vec<Event>,
    enabled: bool,
) -> MidiExpressionResponse {
    let mut response = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 800.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_max_width(610.0);
                response = Some(
                    ui.add_enabled_ui(enabled, |ui| {
                        daw_ui::midi_expression_editor(
                            ui,
                            Id::new("expression"),
                            controls,
                            points,
                            2.0,
                        )
                    })
                    .inner,
                );
            });
        },
    );
    response.unwrap()
}

fn controls() -> MidiExpressionControls {
    MidiExpressionControls {
        velocity: 32769,
        release_velocity: 65535,
        pitch_semitones: 60.501953125,
        selected: None,
        kind: MidiExpressionKind::Pitch,
        controller: 73,
        beat: 0.123456789,
        value: 0x8000_0001,
    }
}

fn pointer(at: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn viewing_high_resolution_values_never_rounds_or_changes_them_in_either_theme() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for scale in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_theme(theme);
            ctx.set_pixels_per_point(scale);
            let mut state = controls();
            for _ in 0..3 {
                let result = frame(&ctx, &mut state, &[], vec![], true);
                assert!(
                    !result.velocity_changed
                        && !result.release_changed
                        && !result.pitch_changed
                        && result.action.is_none()
                );
                assert_eq!(state.velocity, 32769);
                assert_eq!(state.release_velocity, 65535);
                assert_eq!(state.value, 0x8000_0001);
                assert_eq!(state.pitch_semitones, 60.501953125);
                assert_eq!(state.beat, 0.123456789);
                for rect in [
                    result.velocity_rect,
                    result.release_rect,
                    result.pitch_rect,
                    result.time_rect,
                    result.value_rect,
                    result.normalized_rect,
                    result.controller_rect,
                ] {
                    assert!(rect.is_finite() && rect.is_positive());
                    assert!(
                        rect.right() <= 620.0 && rect.bottom() < 800.0,
                        "Control remains inside small editor: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn disabled_expression_controls_do_not_edit_and_large_point_lists_are_virtualized() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let points: Vec<_> = (0..32768)
        .map(|index| MidiExpressionPoint {
            id: index + 1,
            beat: index as f64 / 32768.0,
            kind: Some(MidiExpressionKind::Pressure),
            controller: 0,
            value: index as u32,
        })
        .collect();
    let mut state = controls();
    let idle = frame(&ctx, &mut state, &points, vec![], false);
    assert!(
        idle.rows.len() < 12,
        "Only visible event rows become widgets"
    );
    let at = idle.normalized_rect.left_center() + vec2(2.0, 0.0);
    frame(&ctx, &mut state, &points, pointer(at, true), false);
    let result = frame(&ctx, &mut state, &points, pointer(at, false), false);
    assert_eq!(state.value, 0x8000_0001);
    assert!(result.action.is_none());
    assert_eq!(state.velocity, 32769);
}

#[test]
fn normalized_slider_can_edit_both_u32_endpoints_without_touching_note_velocity() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut state = controls();
    let idle = frame(&ctx, &mut state, &[], vec![], true);
    let at = idle.normalized_rect.left_center() + vec2(3.0, 0.0);
    frame(&ctx, &mut state, &[], pointer(at, true), true);
    frame(&ctx, &mut state, &[], pointer(at, false), true);
    assert_eq!(state.value, 0);
    let idle = frame(&ctx, &mut state, &[], vec![], true);
    let at = idle.normalized_rect.right_center() - vec2(3.0, 0.0);
    frame(&ctx, &mut state, &[], pointer(at, true), true);
    frame(&ctx, &mut state, &[], pointer(at, false), true);
    assert_eq!(state.value, u32::MAX);
    assert_eq!(state.velocity, 32769);
    assert_eq!(state.release_velocity, 65535);
}
