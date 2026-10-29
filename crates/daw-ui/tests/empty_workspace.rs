use daw_ui::{EmptyWorkspaceAction, EmptyWorkspaceResponse};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

fn actions() -> [EmptyWorkspaceAction<'static>; 3] {
    [
        EmptyWorkspaceAction {
            id: "instrument",
            label: "Choose instrument…",
            description: "Browse installed instruments before creating a track.",
            primary: true,
        },
        EmptyWorkspaceAction {
            id: "import",
            label: "Import audio or MIDI…",
            description: "Choose a recording or MIDI file.",
            primary: false,
        },
        EmptyWorkspaceAction {
            id: "assistant",
            label: "Compose with AI",
            description: "Browse musical examples without sending a request.",
            primary: false,
        },
    ]
}

fn frame(ctx: &Context, width: f32, enabled: bool, events: Vec<Event>) -> EmptyWorkspaceResponse {
    let mut result = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 420.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.2),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    result = Some(daw_ui::empty_workspace(
                        ui,
                        "Start a song",
                        "Choose a sound, import a recording, or compose with AI.",
                        &actions(),
                    ));
                });
            });
        },
    );
    result.unwrap()
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

#[test]
fn actions_fit_narrow_and_wide_workspaces_in_both_appearances() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            daw_ui::theme::set_appearance(
                &ctx,
                if dark {
                    daw_ui::theme::Appearance::Dark
                } else {
                    daw_ui::theme::Appearance::Light
                },
            );
            ctx.set_pixels_per_point(density);
            frame(&ctx, 640.0, true, vec![]);
            for width in [320.0, 480.0, 960.0] {
                frame(&ctx, width, true, vec![]);
                let shown = frame(&ctx, width, true, vec![]);
                assert!(shown.action.is_none());
                assert!(shown.rect.width() <= 560.0);
                assert!(shown.rect.height() < 260.0);
                assert!(shown.rect.left() >= 0.0 && shown.rect.right() <= width);
                let buttons: Vec<_> = shown
                    .actions
                    .iter()
                    .map(|(_, response)| response.rect)
                    .collect();
                for (index, rect) in buttons.iter().enumerate() {
                    assert!(rect.height() >= daw_ui::tokens::CONTROL_HEIGHT);
                    assert!(
                        shown.rect.contains_rect(*rect),
                        "{rect:?} outside {:?}",
                        shown.rect
                    );
                    for other in &buttons[index + 1..] {
                        assert!(!rect.intersects(*other));
                    }
                }
                if width == 320.0 {
                    assert!(buttons[2].top() > buttons[1].bottom());
                } else {
                    assert_eq!(buttons[2].top(), buttons[1].top());
                }
            }
        }
    }
}

#[test]
fn mouse_activates_only_the_chosen_action_and_disabled_workspace_stays_inert() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    frame(&ctx, 640.0, true, vec![]);
    for wanted in ["instrument", "import", "assistant"] {
        let shown = frame(&ctx, 640.0, true, vec![]);
        let target = shown
            .actions
            .iter()
            .find(|(id, _)| id == wanted)
            .unwrap()
            .1
            .rect
            .center();
        assert!(
            frame(&ctx, 640.0, true, pointer(target, true))
                .action
                .is_none()
        );
        let clicked = frame(&ctx, 640.0, true, pointer(target, false));
        assert_eq!(clicked.action.as_deref(), Some(wanted));
        let hover = frame(&ctx, 640.0, true, vec![Event::PointerMoved(target)]);
        assert!(hover.action.is_none());
        assert!(
            frame(&ctx, 640.0, false, pointer(target, true))
                .action
                .is_none()
        );
        let disabled = frame(&ctx, 640.0, false, pointer(target, false));
        assert!(disabled.action.is_none());
        assert!(
            disabled
                .actions
                .iter()
                .all(|(_, response)| !response.enabled())
        );
    }
}

#[test]
fn an_empty_workspace_does_not_take_keyboard_focus_while_it_is_drawn() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    frame(&ctx, 640.0, true, vec![]);
    let shown = frame(&ctx, 640.0, true, vec![]);
    assert!(
        shown
            .actions
            .iter()
            .all(|(_, response)| !response.has_focus())
    );
    shown.actions[0].1.request_focus();
    let activated = frame(
        &ctx,
        640.0,
        true,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(activated.action.as_deref(), Some("instrument"));
}
