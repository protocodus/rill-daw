use daw_ui::{self as ui, DeviceBrowserIdentity, Icon};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

fn frame(ctx: &Context, width: f32, events: Vec<Event>, mut render: impl FnMut(&mut egui::Ui)) {
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.1),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| render(ui));
        },
    );
}

#[test]
fn results_heading_keeps_compact_text_height_without_shrinking_controls() {
    let ctx = Context::default();
    ui::theme::apply(&ctx);
    frame(&ctx, 800.0, vec![], |ui| {
        let original_target_height = ui.spacing().interact_size.y;
        let heading = ui::device_browser_results_heading(
            ui,
            12,
            "instrument",
            "instruments",
            Some("Scanning plug-ins…"),
        );
        assert_eq!(heading.rect.height(), ui::tokens::SPACE_LG);
        assert_eq!(ui.spacing().interact_size.y, original_target_height);
        assert!(ui::button(ui, "Details").rect.height() >= ui::tokens::CONTROL_HEIGHT);
    });
}

#[test]
fn clearing_search_by_mouse_returns_focus_and_keeps_control_geometry() {
    let ctx = Context::default();
    ui::theme::apply(&ctx);
    let mut query = "warm piano".to_owned();
    let mut field = Rect::NOTHING;
    let mut clear = Rect::NOTHING;
    let mut search_id = None;
    frame(&ctx, 440.0, vec![], |ui| {
        let response = ui::device_browser_search(ui, &mut query, "Search instruments");
        field = response.search.rect;
        clear = response.clear.rect;
        search_id = Some(response.search.id);
    });
    assert!(field.right() < clear.left());
    assert!((field.center().y - clear.center().y).abs() < 1.0);
    let at = clear.center();
    let mut cleared = false;
    for pressed in [true, false] {
        frame(
            &ctx,
            440.0,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
            |ui| {
                let response = ui::device_browser_search(ui, &mut query, "Search instruments");
                cleared |= response.cleared;
                assert_eq!(response.search.rect, field);
                assert_eq!(response.clear.rect, clear);
            },
        );
    }
    assert!(cleared);
    assert!(query.is_empty());
    assert!(ctx.memory(|memory| memory.has_focus(search_id.unwrap())));
}

#[test]
fn long_device_identity_and_destination_remain_bounded_in_narrow_panes() {
    for dark in [false, true] {
        let ctx = Context::default();
        ui::theme::apply(&ctx);
        ui::theme::set_appearance(
            &ctx,
            if dark {
                ui::theme::Appearance::Dark
            } else {
                ui::theme::Appearance::Light
            },
        );
        for width in [233.0, 377.0, 610.0] {
            frame(&ctx, width, vec![], |ui| {
                let bounds = ui.max_rect();
                let identity = ui::device_browser_identity(
                    ui,
                    &DeviceBrowserIdentity {
                        name: "An exceptionally long instrument model name with a complete factory collection",
                        developer: "A long developer name with several collaborators",
                        kind_format: "Instrument · VST3",
                        categories: "Piano · Acoustic · Cinematic · Vintage · Expressive",
                        preview: None,
                        icon: Icon::Instrument,
                    },
                );
                let summary = ui::device_browser_selection_summary(
                    ui,
                    "An exceptionally long selected instrument model name",
                    "Replace on a channel with an exceptionally long name · Current instrument: Piano",
                );
                for rect in [identity.rect, summary.rect] {
                    assert!(
                        bounds.contains_rect(rect),
                        "{width}: {bounds:?} does not contain {rect:?}"
                    );
                }
                assert!(
                    identity.rect.height() < 180.0,
                    "No image leaves compact read-only identity"
                );
                assert!(summary.rect.height() <= 2.0 * ui::tokens::SPACE_LG);
                assert!(
                    summary
                        .rect
                        .min
                        .distance(pos2(bounds.left(), summary.rect.top()))
                        < 1.0
                );
            });
        }
    }
}
