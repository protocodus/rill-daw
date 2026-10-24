//! Components use native egui interaction and report changes through `Response`.
//! Give repeated rows stable `ui.push_id(model_id, ...)` scopes.

use crate::{theme, tokens};
use egui::{Button, ComboBox, DragValue, Frame, InnerResponse, Response, Slider, Ui, WidgetText};
use std::{hash::Hash, ops::RangeInclusive};

pub fn button_widget(label: impl Into<WidgetText>) -> Button<'static> {
    Button::new(label.into())
        .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT))
        .corner_radius(tokens::RADIUS)
}

pub fn button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(button_widget(label))
}

pub fn primary_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(
        button_widget(label)
            .fill(theme::ACCENT.gamma_multiply(0.24))
            .stroke(egui::Stroke::new(1.0, theme::ACCENT)),
    )
}

pub fn small_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(
        Button::new(label.into())
            .small()
            .min_size(egui::vec2(0.0, tokens::COMPACT_HEIGHT))
            .corner_radius(tokens::RADIUS),
    )
}

pub fn menu_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    ui.menu_button(label.into(), add_contents)
}

/// Application command bar; menus use the same native focus and popup behavior
/// as contextual menus throughout the app.
pub fn menu_bar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    egui::MenuBar::new().ui(ui, contents)
}

pub fn toggle(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    ui.toggle_value(value, label.into())
}

pub fn checkbox(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    ui.checkbox(value, label.into())
}

/// A labeled boolean switch. Its entire row is clickable and focusable. Native
/// `Response::clicked` supplies pointer, Space/Enter, and accessibility activation.
pub fn switch(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    let label = label.into();
    let accessible_label = label.text().to_owned();
    let available = (ui.available_width() - tokens::SWITCH_WIDTH - tokens::SPACE_SM).max(0.0);
    let galley = label.into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        available,
        egui::TextStyle::Button,
    );
    let gap = if accessible_label.is_empty() {
        0.0
    } else {
        tokens::SPACE_SM
    };
    let desired = egui::vec2(
        tokens::SWITCH_WIDTH + gap + galley.size().x,
        tokens::CONTROL_HEIGHT.max(galley.size().y),
    );
    let (rect, mut response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *value,
            accessible_label.clone(),
        )
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let track = egui::Rect::from_center_size(
            egui::pos2(rect.left() + tokens::SWITCH_WIDTH * 0.5, rect.center().y),
            egui::vec2(tokens::SWITCH_WIDTH, tokens::SWITCH_HEIGHT),
        );
        let fill = if *value {
            theme::ACCENT.gamma_multiply(if ui.is_enabled() { 0.65 } else { 0.25 })
        } else {
            visuals.bg_fill
        };
        ui.painter().rect(
            track,
            tokens::SWITCH_HEIGHT * 0.5,
            fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let animation = ui.ctx().animate_bool(response.id, *value);
        let radius = (tokens::SWITCH_HEIGHT - tokens::SPACE_XS) * 0.5;
        let center_x = egui::lerp(
            (track.left() + tokens::SWITCH_HEIGHT * 0.5)
                ..=(track.right() - tokens::SWITCH_HEIGHT * 0.5),
            animation,
        );
        ui.painter().circle_filled(
            egui::pos2(center_x, track.center().y),
            radius,
            visuals.text_color(),
        );
        ui.painter().galley(
            egui::pos2(track.right() + gap, rect.center().y - galley.size().y * 0.5),
            galley,
            visuals.text_color(),
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                tokens::RADIUS,
                egui::Stroke::new(1.5, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
    }
    response
}

pub fn radio_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<WidgetText>,
) -> Response {
    ui.radio_value(current, value, label.into())
}

pub fn selectable(ui: &mut Ui, selected: bool, label: impl Into<WidgetText>) -> Response {
    ui.selectable_label(selected, label.into())
}

pub fn selectable_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<WidgetText>,
) -> Response {
    ui.selectable_value(current, value, label.into())
}

/// Compact canvas control with a caller-owned stable ID; used for track mute,
/// solo, and expansion controls where the arrangement owns the row geometry.
pub fn selectable_at(
    ui: &mut Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    selected: bool,
    label: impl Into<WidgetText>,
) -> Response {
    // Canvas overlays must not consume space in their parent's layout. Unlike
    // `push_id`/`scope`, a direct child does not advance or enlarge that layout.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::LeftToRight,
            )),
    );
    child.spacing_mut().button_padding = egui::vec2(3.0, 0.0);
    child.spacing_mut().interact_size = egui::Vec2::ZERO;
    child.add_sized(
        rect.size(),
        Button::selectable(selected, label.into())
            .small()
            .min_size(egui::Vec2::ZERO)
            .corner_radius(tokens::RADIUS),
    )
}

pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width.max(0.0))
            .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
    )
}

pub fn select(id_salt: impl Hash, selected_text: impl Into<WidgetText>) -> ComboBox {
    ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(tokens::SELECT_WIDTH)
}

pub fn select_labeled(
    label: impl Into<WidgetText>,
    selected_text: impl Into<WidgetText>,
) -> ComboBox {
    ComboBox::from_label(label)
        .selected_text(selected_text)
        .width(tokens::SELECT_WIDTH)
}

pub fn slider<T: egui::emath::Numeric>(value: &mut T, range: RangeInclusive<T>) -> Slider<'_> {
    Slider::new(value, range)
}

pub fn number<T: egui::emath::Numeric>(value: &mut T) -> DragValue<'_> {
    DragValue::new(value)
}

pub fn color_edit(ui: &mut Ui, value: &mut [u8; 3]) -> Response {
    ui.color_edit_button_srgb(value)
}

pub fn panel() -> Frame {
    Frame::new()
        .fill(theme::PANEL)
        .inner_margin(tokens::SPACE_LG)
        .corner_radius(tokens::RADIUS)
}

pub fn card() -> Frame {
    Frame::new()
        .fill(theme::RAISED)
        .inner_margin(tokens::SPACE_MD)
        .corner_radius(tokens::RADIUS)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
}

pub fn inset() -> Frame {
    Frame::new()
        .fill(theme::BG)
        .inner_margin(tokens::SPACE_SM)
        .corner_radius(tokens::RADIUS)
}

/// On-demand property/utility window with the same surfaces and spacing as the
/// main workspace. The caller supplies `.open(&mut open)` and persistent `.id`.
pub fn popup<'a>(title: impl Into<WidgetText>) -> egui::Window<'a> {
    egui::Window::new(title)
        .frame(panel().stroke(egui::Stroke::new(1.0, theme::LINE)))
        .default_width(480.0)
        .collapsible(false)
        .resizable(false)
}

/// A focused canvas tool, opened on demand over the arrangement. Its viewport
/// bounds leave the transport accessible and keep close controls on screen.
pub fn workspace_popup<'a>(title: impl Into<WidgetText>, ctx: &egui::Context) -> egui::Window<'a> {
    let viewport = ctx.content_rect();
    let size = egui::vec2(
        (viewport.width() - 96.0).max(320.0),
        (viewport.height() - 196.0).max(240.0),
    );
    popup(title)
        .fixed_pos(viewport.min + egui::vec2(24.0, 112.0))
        .fixed_size(size)
        .movable(false)
}
