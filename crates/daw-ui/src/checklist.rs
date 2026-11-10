//! A bounded list of independent choices with whole-list shortcuts.
use crate::{small_button, text, tokens};
use egui::Ui;
use std::{collections::HashSet, hash::Hash};

/// A scrolling checklist. `selected` holds the checked keys; items keep the
/// order given. **Select all** and **Select none** act on every item, and the
/// count stays visible while the list scrolls. Returns whether the selection
/// changed this frame.
pub fn checklist<T: Copy + Eq + Hash>(
    ui: &mut Ui,
    id_salt: impl Hash + Copy,
    items: &[(T, String)],
    selected: &mut HashSet<T>,
    max_height: f32,
) -> bool {
    let mut changed = false;
    crate::toolbar(ui, |ui| {
        if small_button(ui, "Select all").clicked() {
            selected.extend(items.iter().map(|(key, _)| *key));
            changed = true;
        }
        if small_button(ui, "Select none").clicked() {
            selected.clear();
            changed = true;
        }
        let count = items
            .iter()
            .filter(|(key, _)| selected.contains(key))
            .count();
        text::note(ui, format!("{count} of {}", items.len()));
    });
    ui.add_space(tokens::SPACE_XS);
    crate::scroll_area(ui, (id_salt, "checklist"), [false, true])
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (key, label) in items {
                let mut checked = selected.contains(key);
                if crate::checkbox(ui, &mut checked, label.as_str()).changed() {
                    if checked {
                        selected.insert(*key);
                    } else {
                        selected.remove(key);
                    }
                    changed = true;
                }
            }
        });
    changed
}
