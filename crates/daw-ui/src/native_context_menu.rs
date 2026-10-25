//! Shared menu model for isolated native editor helpers.
//!
//! The native renderers are synchronous and must run in a disposable menu
//! process, never on the audio or plug-in event thread.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeContextMenu {
    /// Platform screen coordinates (AppKit uses its bottom-left screen origin).
    pub position: [f64; 2],
    pub items: Vec<NativeMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NativeMenuItem {
    Action {
        id: u32,
        label: String,
        enabled: bool,
        checked: bool,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<NativeMenuItem>,
    },
}

impl NativeContextMenu {
    /// Validate the bounded, untrusted menu description before opening a window.
    pub fn validate(&self) -> Result<(), String> {
        if !self.position.iter().all(|value| value.is_finite()) {
            return Err("Invalid menu position".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut count = 0;
        fn visit(
            items: &[NativeMenuItem],
            depth: usize,
            count: &mut usize,
            ids: &mut std::collections::HashSet<u32>,
        ) -> Result<(), String> {
            if depth > 16 {
                return Err("Context menu is too deeply nested".into());
            }
            for item in items {
                *count += 1;
                if *count > 2048 {
                    return Err("Context menu has too many items".into());
                }
                let label = match item {
                    NativeMenuItem::Action { id, label, .. } => {
                        if !ids.insert(*id) {
                            return Err("Context menu has duplicate action IDs".into());
                        }
                        label
                    }
                    NativeMenuItem::Submenu { label, items } => {
                        visit(items, depth + 1, count, ids)?;
                        label
                    }
                    NativeMenuItem::Separator => continue,
                };
                if label.len() > 4096 || label.contains('\0') {
                    return Err("Invalid context menu label".into());
                }
            }
            Ok(())
        }
        visit(&self.items, 0, &mut count, &mut ids)
    }

    pub fn contains_enabled_action(&self, id: u32) -> bool {
        fn contains(items: &[NativeMenuItem], id: u32) -> bool {
            items.iter().any(|item| match item {
                NativeMenuItem::Action {
                    id: item_id,
                    enabled,
                    ..
                } => *item_id == id && *enabled,
                NativeMenuItem::Submenu { items, .. } => contains(items, id),
                NativeMenuItem::Separator => false,
            })
        }
        contains(&self.items, id)
    }
}

/// Shared fallback menu body for platforms without a built-in native toolkit.
/// Rendering and actions use the same theme and buttons as the main app.
pub fn native_context_menu_ui(ui: &mut egui::Ui, items: &[NativeMenuItem]) -> Option<u32> {
    let mut selected = None;
    for item in items {
        match item {
            NativeMenuItem::Action {
                id,
                label,
                enabled,
                checked,
            } => {
                let label = if *checked {
                    format!("✓  {label}")
                } else {
                    label.clone()
                };
                if ui
                    .add_enabled(*enabled, crate::button_widget(label))
                    .clicked()
                {
                    selected = Some(*id);
                    ui.close();
                }
            }
            NativeMenuItem::Separator => {
                ui.separator();
            }
            NativeMenuItem::Submenu { label, items } => {
                ui.menu_button(label, |ui| {
                    if let Some(id) = native_context_menu_ui(ui, items) {
                        selected = Some(id);
                    }
                });
            }
        }
    }
    selected
}

#[cfg(target_os = "macos")]
#[path = "native_context_menu_macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "native_context_menu_windows.rs"]
mod platform;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn show_native_context_menu(menu: &NativeContextMenu) -> Result<Option<u32>, String> {
    menu.validate()?;
    platform::show(menu)
}

/// Register the isolated worker with AppKit before its parent yields activation.
#[cfg(target_os = "macos")]
pub fn initialize_native_context_menu_worker() -> Result<(), String> {
    platform::initialize_worker()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: u32, enabled: bool) -> NativeMenuItem {
        NativeMenuItem::Action {
            id,
            label: "Fréquence 🎛".into(),
            enabled,
            checked: true,
        }
    }

    #[test]
    fn zero_id_unicode_and_nested_enabled_actions_are_preserved() {
        let menu = NativeContextMenu {
            position: [-120.0, 50.0],
            items: vec![NativeMenuItem::Submenu {
                label: "Réglages".into(),
                items: vec![action(0, true), NativeMenuItem::Separator, action(1, false)],
            }],
        };
        assert_eq!(menu.validate(), Ok(()));
        assert!(menu.contains_enabled_action(0));
        assert!(!menu.contains_enabled_action(1));
        assert!(!menu.contains_enabled_action(2));
    }

    #[test]
    fn malformed_and_excessive_menus_are_rejected_before_rendering() {
        let mut menu = NativeContextMenu {
            position: [0.0; 2],
            items: vec![action(0, true), action(0, true)],
        };
        assert!(menu.validate().is_err());
        menu.items = vec![action(1, true)];
        menu.position[0] = f64::NAN;
        assert!(menu.validate().is_err());
        menu.position[0] = 0.0;
        for _ in 0..17 {
            menu.items = vec![NativeMenuItem::Submenu {
                label: "Nested".into(),
                items: menu.items,
            }];
        }
        assert!(menu.validate().is_err());
    }

    #[test]
    fn menu_body_activates_zero_id_and_respects_disabled_actions_without_a_window() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for enabled in [false, true] {
            let items = vec![action(0, enabled)];
            let mut click = egui::Pos2::ZERO;
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(340.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    click = ui.next_widget_position() + egui::vec2(40.0, 12.0);
                    assert_eq!(native_context_menu_ui(ui, &items), None);
                });
            });
            let mut result = None;
            for pressed in [true, false] {
                let _ = ctx.run(
                    input(vec![
                        egui::Event::PointerMoved(click),
                        egui::Event::PointerButton {
                            pos: click,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]),
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            result = result.or(native_context_menu_ui(ui, &items));
                        });
                    },
                );
            }
            assert_eq!(result, enabled.then_some(0));
        }
    }
}
