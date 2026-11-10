use daw_ui::{self as controls, theme};
use egui::{Context, Event, Pos2, RawInput, Rect};
use std::collections::HashSet;

fn run(ctx: &Context, events: Vec<Event>, selected: &mut HashSet<u32>) -> bool {
    let items: Vec<(u32, String)> = (0..40).map(|id| (id, format!("Channel {id}"))).collect();
    let mut changed = false;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(360.0, 400.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed |= controls::checklist(ui, "channels", &items, selected, 160.0);
            });
        },
    );
    changed
}

fn click(at: Pos2) -> Vec<Event> {
    [true, false]
        .into_iter()
        .flat_map(|pressed| {
            [
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        })
        .collect()
}

/// Locate a painted label by running a frame and reading its text shape.
fn find(ctx: &Context, label: &str, selected: &mut HashSet<u32>) -> Pos2 {
    let items: Vec<(u32, String)> = (0..40).map(|id| (id, format!("Channel {id}"))).collect();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(360.0, 400.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                controls::checklist(ui, "channels", &items, selected, 160.0);
            });
        },
    );
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"))
}

#[test]
fn select_all_none_and_single_choices_update_the_selection() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut selected = HashSet::from([3]);
    for _ in 0..3 {
        run(&ctx, vec![], &mut selected);
    }
    let all = find(&ctx, "Select all", &mut selected);
    assert!(run(&ctx, click(all), &mut selected));
    assert_eq!(
        selected.len(),
        40,
        "every item, including those scrolled out of view"
    );
    let none = find(&ctx, "Select none", &mut selected);
    assert!(run(&ctx, click(none), &mut selected));
    assert!(selected.is_empty());
    let first = find(&ctx, "Channel 0", &mut selected);
    assert!(run(&ctx, click(first), &mut selected));
    assert_eq!(selected, HashSet::from([0]));
    find(&ctx, "1 of 40", &mut selected);
}
