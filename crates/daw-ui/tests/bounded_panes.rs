use daw_ui::{bounded_columns, text, theme};
use egui::{Context, Event, Modifiers, RawInput, Rect, Shape};

#[test]
fn bounded_panes_keep_headers_fixed_and_scrolling_independent() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut header = Rect::NOTHING;
    let mut action = Rect::NOTHING;
    let mut panes = [Rect::NOTHING; 2];
    let mut offsets = [0.0; 2];
    let mut frame = |events| {
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    header = ui.button("Search and filters").rect;
                    let columns = bounded_columns(ui, "panes", 250.0, |index, ui| {
                        panes[index] = ui.max_rect();
                        if index == 1 {
                            action = ui.button("Create instance").rect;
                        }
                        let scroll = egui::ScrollArea::vertical()
                            .id_salt(index)
                            .max_height(ui.available_height())
                            .min_scrolled_height(0.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for row in 0..50 {
                                    ui.label(format!("Row {row}"));
                                }
                            });
                        offsets[index] = scroll.state.offset.y;
                    });
                    assert_eq!(columns.response.rect.height(), 250.0);
                    assert!(ui.button("Below the panes").rect.top() >= panes[0].bottom());
                });
            },
        );
        (header, action, panes, offsets)
    };
    let initial = frame(vec![]);
    assert!(initial.2[0].right() < initial.2[1].left());
    assert_eq!(initial.2[0].size(), initial.2[1].size());
    let point = initial.2[0].center();
    frame(vec![
        Event::PointerMoved(point),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -1000.0),
            modifiers: Modifiers::NONE,
        },
    ]);
    let mut current = frame(vec![]);
    for _ in 0..15 {
        current = frame(vec![]);
    }
    assert_eq!(current.0, initial.0);
    assert_eq!(current.1, initial.1);
    assert_eq!(current.2, initial.2);
    assert!(current.3[0] > 100.0);
    assert_eq!(current.3[1], 0.0);
}

#[test]
fn bounded_context_titles_wrap_and_keep_full_source_within_row_limit() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let title = "Žluťoučký 🎹 東京 — A very long instrument name that must never displace its primary action";
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(226.0, 300.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = text::bounded_block(ui, text::TextRole::Title, title, 3);
                assert!(
                    response.rect.height() > 30.0,
                    "{:?} available {}",
                    response.rect,
                    ui.available_width()
                );
                assert!(response.rect.height() <= 80.0);
                assert!(response.rect.width() <= 210.0);
            });
        },
    );
    let text = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(text.galley.text(), title);
    assert_eq!(text.galley.rows.len(), 3);
    assert!(text.galley.elided);
}

#[test]
fn utility_scroll_body_hugs_short_content_and_bounds_growth_without_moving_menu_controls() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let frame = |count: usize, events| {
        let mut body = Rect::NOTHING;
        let mut window = Rect::NOTHING;
        let mut menu = Rect::NOTHING;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                window = daw_ui::popup("Effects", ctx)
                    .vscroll(false)
                    .show(ctx, |ui| {
                        let scroll = daw_ui::scroll_body(ui, "effects_body", |ui| {
                            ui.label("Channel");
                            for index in 0..count {
                                ui.button(format!("Insert {index}")).clicked();
                            }
                            menu = daw_ui::menu_button(ui, "More", |ui| {
                                ui.button("Add insert").clicked();
                            })
                            .response
                            .rect;
                        });
                        body = Rect::from_min_size(
                            scroll.inner_rect.min,
                            egui::vec2(
                                scroll.inner_rect.width(),
                                scroll.content_size.y.min(scroll.inner_rect.height()),
                            ),
                        );
                    })
                    .unwrap()
                    .response
                    .rect;
            },
        );
        (window, body, menu)
    };
    for _ in 0..3 {
        frame(0, vec![]);
    }
    let short = frame(0, vec![]);
    assert!(short.0.height() < 260.0, "short utility should fit content");
    assert!(short.1.height() < 100.0);
    let mut pressed = short;
    for down in [true, false] {
        pressed = frame(
            0,
            vec![
                Event::PointerMoved(short.2.center()),
                Event::PointerButton {
                    pos: short.2.center(),
                    button: egui::PointerButton::Primary,
                    pressed: down,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(pressed.0, short.0);
        assert_eq!(pressed.2, short.2, "opening click must not shift the menu");
    }
    assert_eq!(pressed.0, short.0);
    frame(80, vec![]);
    frame(80, vec![]);
    let long = frame(80, vec![]);
    assert!(long.0.height() > short.0.height() + 150.0);
    assert!(long.0.top() >= daw_ui::tokens::POPUP_TOP);
    assert!(long.0.bottom() <= 720.0 - daw_ui::tokens::POPUP_MARGIN);
    assert!(long.1.height() <= 420.0);
    for _ in 0..3 {
        frame(0, vec![]);
    }
    let shrunk = frame(0, vec![]);
    assert_eq!(shrunk.0.size(), short.0.size());
}
