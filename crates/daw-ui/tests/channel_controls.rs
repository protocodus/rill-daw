use daw_ui::{ChannelControlsResponse, channel_controls_at};
use egui::{Context, Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

fn frame(
    ctx: &Context,
    events: Vec<Event>,
    gain: &mut f32,
    pan: &mut f32,
    compact: bool,
    enabled: bool,
) -> ChannelControlsResponse {
    let mut response = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let before = ui.next_widget_position();
                    response = Some(channel_controls_at(
                        ui,
                        Rect::from_min_size(
                            pos2(80.0, 80.0),
                            vec2(162.0, if compact { 24.0 } else { 44.0 }),
                        ),
                        "channel",
                        gain,
                        pan,
                        [0.1, 0.2],
                        compact,
                    ));
                    assert_eq!(
                        before,
                        ui.next_widget_position(),
                        "canvas controls cannot move their parent layout"
                    );
                });
            });
        },
    );
    response.unwrap()
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

#[test]
fn compact_and_regular_sliders_drag_independently_and_respect_disabled_state() {
    for compact in [false, true] {
        for dark in [false, true] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let (mut gain, mut pan) = (0.5, 0.0);
            let initial = frame(&ctx, vec![], &mut gain, &mut pan, compact, true);
            let gain_target = initial.gain_slider.left_center() + vec2(12.0, 0.0);
            frame(
                &ctx,
                pointer(gain_target, true),
                &mut gain,
                &mut pan,
                compact,
                true,
            );
            frame(
                &ctx,
                pointer(gain_target, false),
                &mut gain,
                &mut pan,
                compact,
                true,
            );
            assert!(gain > 0.0 && gain < 0.1, "volume rail edits gain: {gain}");
            assert_eq!(pan, 0.0);
            let changed_gain = gain;
            let pan_target = initial.pan_slider.right_center() - vec2(8.0, 0.0);
            frame(
                &ctx,
                pointer(pan_target, true),
                &mut gain,
                &mut pan,
                compact,
                true,
            );
            frame(
                &ctx,
                pointer(pan_target, false),
                &mut gain,
                &mut pan,
                compact,
                true,
            );
            assert!(pan > 0.5 && pan <= 1.0, "pan rail edits pan: {pan}");
            assert_eq!(gain, changed_gain);
            let unchanged = (gain, pan);
            frame(
                &ctx,
                pointer(initial.gain_slider.right_center(), true),
                &mut gain,
                &mut pan,
                compact,
                false,
            );
            frame(
                &ctx,
                pointer(initial.gain_slider.right_center(), false),
                &mut gain,
                &mut pan,
                compact,
                false,
            );
            assert_eq!((gain, pan), unchanged);
            assert!(initial.gain_slider.right() < initial.meter.rect.left());
            assert!(initial.pan_slider.bottom() <= initial.meter.rect.bottom());
        }
    }
}

#[test]
fn inspecting_out_of_range_channel_values_never_changes_them() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let (mut gain, mut pan) = (2.0, 1.2);
    for compact in [false, true] {
        frame(&ctx, vec![], &mut gain, &mut pan, compact, true);
        assert_eq!((gain, pan), (2.0, 1.2));
    }
}
