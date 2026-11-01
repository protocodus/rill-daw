use daw_ui::{self as controls, theme, tokens};
use egui::{Context, Pos2, RawInput, Rect, Vec2};

#[test]
fn conservative_footer_reservations_do_not_shrink_the_scroll_body_on_idle_frames() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
            let mut settled = None;
            for frame in 0..40 {
                let mut state = None;
                let _ = ctx.run(
                    RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |ctx| {
                        controls::popup("Recording options", ctx)
                            .default_height(480.0)
                            .vscroll(false)
                            .show(ctx, |ui| {
                                controls::text::label(ui, "Input and timing");
                                // Reserve a little more than the current footer so
                                // a longer status or wrapped action can fit later.
                                let reserve = tokens::CONTROL_HEIGHT + 2.0 * tokens::SPACE_MD;
                                let (first, clip) =
                                    controls::popup_scroll_body(ui, "inputs", reserve, |ui| {
                                        let first = controls::button(ui, "Audio setup…").rect;
                                        for _ in 0..20 {
                                            controls::button(ui, "Input option");
                                        }
                                        (first, ui.clip_rect())
                                    });
                                let record = controls::button(ui, "Record").rect;
                                state = Some((first, clip, record));
                            });
                    },
                );
                if frame >= 4 {
                    let state = state.unwrap();
                    assert!(
                        state.1.contains_rect(state.0),
                        "input target clipped at {density}×: {state:?}"
                    );
                    assert!(
                        state.1.height() >= 89.0,
                        "scroll body collapsed at frame {frame}: {state:?}"
                    );
                    assert!(
                        viewport.contains_rect(state.2),
                        "Record left the viewport: {state:?}"
                    );
                    if let Some(previous) = settled {
                        assert_eq!(state, previous, "idle frames must not erode the body");
                    }
                    settled = Some(state);
                }
            }
        }
    }
}
