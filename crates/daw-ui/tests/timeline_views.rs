use daw_ui::{
    TimelineViewport, remember_timeline_view, reset_timeline_view_history,
    restore_previous_timeline_view,
};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};

fn frame(ctx: &Context, time: f64, events: Vec<Event>, f: impl FnOnce()) {
    let mut callback = Some(f);
    let _ = ctx.run(
        RawInput {
            time: Some(time),
            events,
            ..Default::default()
        },
        |_| {
            callback.take().unwrap()();
        },
    );
}

fn view(start: f64) -> TimelineViewport {
    TimelineViewport {
        start_beat: start,
        ..Default::default()
    }
}

#[test]
fn previous_view_returns_before_a_complete_wheel_burst_and_toggles_back() {
    let ctx = Context::default();
    frame(&ctx, 1.0, vec![], || {
        remember_timeline_view(&ctx, "song", view(0.0), view(2.0), false)
    });
    frame(&ctx, 1.1, vec![], || {
        remember_timeline_view(&ctx, "song", view(2.0), view(4.0), false)
    });
    let mut current = view(4.0);
    frame(&ctx, 1.2, vec![], || {
        assert!(restore_previous_timeline_view(&ctx, "song", &mut current))
    });
    assert_eq!(current, view(0.0));
    frame(&ctx, 1.3, vec![], || {
        assert!(restore_previous_timeline_view(&ctx, "song", &mut current))
    });
    assert_eq!(current, view(4.0));
}

#[test]
fn a_held_pan_keeps_its_origin_even_when_the_user_pauses_mid_drag() {
    let ctx = Context::default();
    let pos = Pos2::new(50.0, 50.0);
    frame(
        &ctx,
        1.0,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        || remember_timeline_view(&ctx, "song", view(0.0), view(0.0), false),
    );
    frame(&ctx, 1.1, vec![], || {
        remember_timeline_view(&ctx, "song", view(0.0), view(2.0), false)
    });
    frame(&ctx, 3.0, vec![], || {
        remember_timeline_view(&ctx, "song", view(2.0), view(8.0), false)
    });
    frame(
        &ctx,
        3.1,
        vec![Event::PointerButton {
            pos,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        || remember_timeline_view(&ctx, "song", view(8.0), view(8.0), false),
    );
    let mut current = view(8.0);
    frame(&ctx, 3.2, vec![], || {
        assert!(restore_previous_timeline_view(&ctx, "song", &mut current))
    });
    assert_eq!(current, view(0.0));
}

#[test]
fn explicit_fit_has_its_own_return_view_and_document_reset_discards_it() {
    let ctx = Context::default();
    frame(&ctx, 1.0, vec![], || {
        remember_timeline_view(&ctx, "song", view(0.0), view(2.0), false)
    });
    frame(&ctx, 1.1, vec![], || {
        remember_timeline_view(&ctx, "song", view(2.0), view(20.0), true)
    });
    let mut current = view(20.0);
    frame(&ctx, 1.2, vec![], || {
        assert!(restore_previous_timeline_view(&ctx, "song", &mut current))
    });
    assert_eq!(current, view(2.0));
    reset_timeline_view_history(&ctx, "song");
    assert!(!restore_previous_timeline_view(&ctx, "song", &mut current));
    assert!(!restore_previous_timeline_view(
        &ctx,
        "other pane",
        &mut current
    ));
}
