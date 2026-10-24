use daw_ui::{self as controls, theme};
use egui::{Context, Event, Modifiers, RawInput, Rect, Response};
use std::collections::HashMap;

struct Menus {
    ctx: Context,
    responses: HashMap<&'static str, Response>,
    chosen: bool,
}

impl Menus {
    fn new() -> Self {
        let ctx = Context::default();
        theme::apply(&ctx);
        Self {
            ctx,
            responses: HashMap::new(),
            chosen: false,
        }
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.responses.clear();
        let _ = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(162.0);
                    ui.horizontal(|ui| {
                        ui.add_space(1015.0);
                        let pitch = controls::menu_button(ui, "Pitch", |ui| {
                            let root = controls::menu_button(ui, "C", |ui| {
                                let leaf = controls::button(ui, "C♯");
                                // Deliberately rely on the menu's native click-to-close.
                                self.chosen |= leaf.clicked();
                                self.responses.insert("leaf", leaf);
                            });
                            self.responses.insert("root", root.response);
                            controls::menu_button(ui, "Natural minor", |ui| {
                                ui.label("Natural minor");
                            });
                            controls::menu_button(ui, "Major triad", |ui| {
                                ui.label("Major triad");
                            });
                        });
                        self.responses.insert("pitch", pitch.response);
                    });
                });
            },
        );
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }

    fn click_at(&mut self, position: egui::Pos2) {
        // A fast native click may move and press in the same frame. Do not add a
        // preparatory hover frame: it hides provisional submenu hit-area bugs.
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.settle();
    }

    fn key(&mut self, key: egui::Key) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.settle();
    }
}

#[test]
fn fast_nested_menu_click_survives_provisional_area_and_leaf_still_closes() {
    let mut menus = Menus::new();
    menus.settle();
    menus.click_at(menus.responses["pitch"].rect.center());
    let root = menus.responses["root"].rect;
    // Click the short label at the left edge, as a user would, rather than the
    // center of the wider row reserved for the other submenu names.
    menus.click_at(egui::pos2(root.left() + 7.0, root.center().y));
    assert!(menus.responses.contains_key("leaf"));
    menus.click_at(menus.responses["leaf"].rect.center());
    assert!(menus.chosen);
    assert!(!menus.responses.contains_key("root"));
    assert!(!menus.responses.contains_key("leaf"));
}

#[test]
fn nested_menus_keep_keyboard_activation_escape_and_outside_click_dismissal() {
    let mut menus = Menus::new();
    menus.settle();
    menus.responses["pitch"].request_focus();
    menus.key(egui::Key::Enter);
    menus.responses["root"].request_focus();
    menus.key(egui::Key::Space);
    assert!(menus.responses.contains_key("leaf"));
    menus.key(egui::Key::Escape);
    assert!(!menus.responses.contains_key("root"));
    menus.click_at(menus.responses["pitch"].rect.center());
    menus.click_at(menus.responses["root"].rect.center());
    assert!(menus.responses.contains_key("leaf"));
    menus.click_at(egui::pos2(200.0, 100.0));
    assert!(!menus.responses.contains_key("root"));
    assert!(!menus.responses.contains_key("leaf"));
}
