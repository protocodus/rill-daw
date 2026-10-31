//! Visible mouse steps and optional exact entry beside a precise numeric field.
use egui::{Response, Ui};
use std::ops::RangeInclusive;

pub struct NumberStepperResponse {
    /// Retains the numeric field's focus/drag response; changed includes a step.
    pub response: Response,
    /// A successful step or keypad Apply starts a separate editing gesture.
    pub stepped: bool,
}

/// Keep the caller's numeric editor intact and append bounded steps and a keypad.
/// Steps preserve fractional values. Showing an existing out-of-range value, or
/// clicking a disabled boundary button, never changes it. The caller continues
/// to own validation, edit transactions and numeric formatting.
pub fn number_stepper(
    ui: &mut Ui,
    value: &mut f64,
    step: f64,
    range: RangeInclusive<f64>,
    name: &str,
    field: impl FnOnce(&mut Ui, &mut f64) -> Response,
) -> NumberStepperResponse {
    number_stepper_impl(ui, value, step, range, name, false, field)
}

/// The same control for counts such as bars. Keypad drafts must be whole
/// numbers; Apply never silently rounds a fraction into a different value.
pub fn number_stepper_integer(
    ui: &mut Ui,
    value: &mut f64,
    step: f64,
    range: RangeInclusive<f64>,
    name: &str,
    field: impl FnOnce(&mut Ui, &mut f64) -> Response,
) -> NumberStepperResponse {
    number_stepper_impl(ui, value, step, range, name, true, field)
}

fn number_stepper_impl(
    ui: &mut Ui,
    value: &mut f64,
    step: f64,
    range: RangeInclusive<f64>,
    name: &str,
    whole_numbers: bool,
    field: impl FnOnce(&mut Ui, &mut f64) -> Response,
) -> NumberStepperResponse {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
        // DragValue removes its prefix/suffix in text mode. Retain the measured
        // field width while it has focus so a second click still reaches the
        // editor instead of a step button that moved under the pointer.
        // Sibling horizontal rows share their persistent Ui id. Use the next
        // widget id so neighbouring fields cannot overwrite each other's size.
        let width_id = ui.next_auto_id().with("number_stepper_field_width");
        let previous = ui.data(|data| data.get_temp::<(egui::Id, f32)>(width_id));
        let original_width = ui.spacing().interact_size.x;
        if let Some((id, width)) = previous
            && ui.memory(|memory| memory.has_focus(id))
        {
            ui.spacing_mut().interact_size.x = original_width.max(width);
        }
        let mut response = field(ui, value)
            .on_hover_text("Drag for fine adjustment; use −/+ to step or 123 to enter a value");
        ui.spacing_mut().interact_size.x = original_width;
        if !response.has_focus() || previous.is_none() {
            ui.data_mut(|data| data.insert_temp(width_id, (response.id, response.rect.width())));
        }
        let mut stepped = false;
        for (symbol, direction, verb) in [("−", -1.0, "Decrease"), ("+", 1.0, "Increase")] {
            let next = *value + direction * step;
            let allowed = step.is_finite()
                && step > 0.0
                && next.is_finite()
                && next != *value
                && range.contains(&next);
            let button = ui
                .add_enabled(
                    allowed,
                    crate::button_widget(symbol).small().min_size(egui::vec2(
                        crate::tokens::COMPACT_HEIGHT,
                        crate::tokens::CONTROL_HEIGHT,
                    )),
                )
                .on_hover_text(format!("{verb} {name} by {step}"));
            button.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled() && allowed,
                    format!("{verb} {name} by {step}"),
                )
            });
            if button.clicked() {
                *value = next;
                stepped = true;
                response.mark_changed();
            }
        }
        let keypad = ui
            .add(
                crate::button_widget("123")
                    .small()
                    .min_size(egui::vec2(34.0, crate::tokens::CONTROL_HEIGHT)),
            )
            .on_hover_text(format!("Enter {name} using mouse"));
        keypad.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Enter {name} using mouse"),
            )
        });
        let contents = |ui: &mut Ui| {
            crate::value_entry::numeric_value_entry(
                ui,
                response.id,
                *value,
                range,
                name,
                whole_numbers,
            )
        };
        let popup = if egui::containers::menu::is_in_menu(ui) {
            // Native popup memory holds one root popup. A second root would
            // replace Range and stop painting this field on the next frame.
            // A submenu retains its caller and inherits the menu close chain.
            let mut config = egui::containers::menu::MenuConfig::new();
            config.close_behavior = if keypad.clicked() {
                // Its provisional sizing area can cover the opening button.
                egui::PopupCloseBehavior::IgnoreClicks
            } else {
                egui::PopupCloseBehavior::CloseOnClickOutside
            };
            egui::containers::menu::SubMenu::default()
                .config(config)
                .show(ui, &keypad, contents)
        } else {
            egui::Popup::menu(&keypad)
                .id(response.id.with("numeric_keypad"))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(contents)
        };
        if let Some(next) = popup.and_then(|popup| popup.inner)
            && ui.is_enabled()
            && next != *value
        {
            *value = next;
            stepped = true;
            response.mark_changed();
        }
        NumberStepperResponse { response, stepped }
    })
    .inner
}
