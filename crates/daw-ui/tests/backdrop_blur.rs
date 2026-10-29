use daw_ui::{ViewBackdrop, backdrop_blur_info, backdrop_blur_target_size};
use egui::{Context, Pos2, RawInput, Rect, Vec2};

fn input() -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
        ..Default::default()
    }
}

#[test]
fn blur_targets_remain_bounded_at_high_density_and_wide_viewports() {
    for dimensions in [[800.0, 600.0], [1480.0, 960.0], [7680.0, 2160.0]] {
        for density in [1.0, 1.5, 2.0, 3.0] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::from(dimensions));
            let size = backdrop_blur_target_size(rect, density);
            assert!(size.into_iter().all(|side| (1..=512).contains(&side)));
            for axis in 0..2 {
                assert!(size[axis] <= (dimensions[axis] * density / 4.0).ceil() as u32);
            }
            // Two RGBA8 surfaces use at most two MiB, independent of viewport.
            assert!(size[0] * size[1] * 8 <= 2 * 1024 * 1024);
        }
    }
}

#[test]
fn utility_blur_uses_frozen_workspace_and_closing_clears_the_request() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut backdrop = ViewBackdrop::default();
    let mut bounds = Rect::NOTHING;
    let _ = ctx.run(input(), |ctx| {
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.label("Transport");
        });
        let panel = egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Retained workspace");
        });
        bounds = panel.response.rect;
        backdrop = ViewBackdrop::capture(ctx, bounds);
    });
    for frame in 0..3 {
        let output = ctx.run(input(), |ctx| {
            egui::TopBottomPanel::top("transport").show(ctx, |ui| {
                ui.label("Transport");
            });
            egui::CentralPanel::default().show(ctx, |ui| backdrop.paint(ui));
            daw_ui::popup("Effects", ctx).show(ctx, |ui| {
                daw_ui::button(ui, "Foreground");
            });
            daw_ui::paint_popup_backdrop(ctx, true);
        });
        let requests = backdrop_blur_info(&ctx);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].bounds, bounds);
        assert!(requests[0].cached_source);
        assert!(
            !requests[0].gpu_available,
            "headless contexts create no GL resources"
        );
        if frame > 0 {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Foreground")));
        }
    }
    let _ = ctx.run(input(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Workspace");
        });
        daw_ui::paint_popup_backdrop(ctx, false);
    });
    assert!(backdrop_blur_info(&ctx).is_empty());
}

#[test]
fn modal_blurs_once_before_its_foreground_instead_of_stacking_utility_blur() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    for _ in 0..3 {
        let _ = ctx.run(input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Caller");
            });
            daw_ui::modal(ctx, egui::Id::new("confirm"), "Confirm", |ui| {
                daw_ui::button(ui, "Accept");
            });
            daw_ui::paint_popup_backdrop(ctx, true);
        });
        let requests = backdrop_blur_info(&ctx);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].bounds, ctx.content_rect());
        assert!(!requests[0].cached_source);
    }
}
