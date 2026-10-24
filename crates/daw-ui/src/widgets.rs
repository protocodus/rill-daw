//! Components use native egui interaction and report changes through `Response`.
//! Give repeated rows stable `ui.push_id(model_id, ...)` scopes.

use crate::{theme, tokens};
use egui::{Button, ComboBox, DragValue, Frame, InnerResponse, Response, Slider, Ui, WidgetText};
use std::{hash::Hash, ops::RangeInclusive};

pub fn button_widget(label: impl Into<WidgetText>) -> crate::IconButton {
    crate::IconButton::new(label)
}

pub fn button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(button_widget(label))
}

pub fn primary_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(
        button_widget(label)
            .fill(theme::palette(ui.ctx()).accent.gamma_multiply(0.24))
            .stroke(egui::Stroke::new(1.0_f32, theme::palette(ui.ctx()).accent)),
    )
}

pub fn small_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(
        button_widget(label)
            .small()
            .min_size(egui::vec2(0.0, tokens::COMPACT_HEIGHT)),
    )
}

pub fn menu_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let label = label.into();
    if let Some(icon) = crate::Icon::for_label(label.text()) {
        let response = ui.menu_button(
            (
                egui::Atom::custom(egui::Id::new("rill-menu-icon"), egui::Vec2::splat(18.0)),
                label,
            ),
            add_contents,
        );
        let rect = egui::Rect::from_center_size(
            egui::pos2(
                response.response.rect.left() + ui.spacing().button_padding.x + 9.0,
                response.response.rect.center().y,
            ),
            egui::Vec2::splat(18.0),
        );
        icon.paint(
            ui.painter(),
            rect,
            ui.style().interact(&response.response).text_color(),
        );
        response
    } else {
        ui.menu_button(label, add_contents)
    }
}

/// A searchable or editable popover stays open while its internal controls are used.
/// Call `ui.close()` when an item is accepted.
pub fn picker_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<InnerResponse<R>>) {
    egui::containers::menu::MenuButton::from_button(
        Button::new(label.into())
            .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT))
            .corner_radius(tokens::RADIUS),
    )
    .config(
        egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
    )
    .ui(ui, contents)
}

/// Application command bar; menus use the same native focus and popup behavior
/// as contextual menus throughout the app.
pub fn menu_bar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    egui::MenuBar::new().ui(ui, contents)
}

pub fn toggle(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    let mut response = selectable(ui, *value, label);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
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
        Some(egui::TextWrapMode::Wrap),
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
            theme::palette(ui.ctx())
                .accent
                .gamma_multiply(if ui.is_enabled() { 0.65 } else { 0.25 })
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
                egui::Stroke::new(1.5_f32, theme::palette(ui.ctx()).accent),
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
    ui.add(
        button_widget(label)
            .selected(selected)
            .frame_when_inactive(selected)
            .frame(true),
    )
}

pub fn selectable_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<WidgetText>,
) -> Response {
    let mut response = selectable(ui, *current == value, label);
    if response.clicked() && *current != value {
        *current = value;
        response.mark_changed();
    }
    response
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

/// A compact, truncated action in a canvas-owned rectangle. The full label stays
/// available to accessibility; callers can add a tooltip for clipped text.
pub fn button_at(
    ui: &mut Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    label: impl Into<WidgetText>,
) -> Response {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().button_padding = egui::vec2(3.0, 0.0);
    child.spacing_mut().interact_size = egui::Vec2::ZERO;
    child.add(
        button_widget(label)
            .small()
            .frame(true)
            .frame_when_inactive(false)
            .truncate()
            .min_size(rect.size()),
    )
}

/// A canvas action with an anchored, on-demand choice popup. Neither the action
/// nor its popup advances or enlarges the parent canvas layout.
pub fn picker_at<R>(
    ui: &mut Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<InnerResponse<R>>) {
    let response = button_at(ui, rect, id_salt, label);
    let inner = egui::Popup::menu(&response).show(contents);
    (response, inner)
}

pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width.max(0.0))
            .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
    )
}

/// Editable numeric canvas control that preserves the parent's row geometry.
/// Drag to adjust, or double-click to type an exact value.
pub fn number_at(
    ui: &mut Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    value: DragValue<'_>,
) -> Response {
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
    child.add_sized(rect.size(), value)
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

/// Search-result row with an optional real preview image. A missing image leaves
/// the row compact; callers do not need to invent an instrument thumbnail.
pub fn catalog_item(
    ui: &mut Ui,
    selected: bool,
    title: &str,
    detail: &str,
    thumbnail: Option<&egui::TextureHandle>,
) -> Response {
    let height = if detail.is_empty() { 38.0 } else { 58.0 };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            title,
        )
    });
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        tokens::RADIUS,
        if selected {
            theme::palette(ui.ctx()).accent.gamma_multiply(0.15)
        } else if response.hovered() {
            theme::palette(ui.ctx()).raised
        } else {
            theme::palette(ui.ctx()).panel
        },
    );
    let mut x = rect.left() + tokens::SPACE_MD;
    if let Some(texture) = thumbnail {
        let size = egui::vec2(58.0, height - 8.0);
        let original = texture.size_vec2();
        let scale = (size.x / original.x).min(size.y / original.y);
        let image_rect = egui::Rect::from_center_size(
            egui::pos2(x + size.x * 0.5, rect.center().y),
            original * scale,
        );
        painter.image(
            texture.id(),
            image_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        x += size.x + tokens::SPACE_MD;
    }
    crate::text::paint_row(
        &painter,
        egui::Rect::from_min_max(
            egui::pos2(x, rect.top() + tokens::SPACE_XS),
            egui::pos2(
                rect.right() - tokens::SPACE_MD,
                rect.top()
                    + if detail.is_empty() {
                        height - tokens::SPACE_XS
                    } else {
                        30.0
                    },
            ),
        ),
        crate::text::TextRole::Label,
        title,
    );
    if !detail.is_empty() {
        crate::text::paint_row(
            &painter,
            egui::Rect::from_min_max(
                egui::pos2(x, rect.top() + 30.0),
                egui::pos2(
                    rect.right() - tokens::SPACE_MD,
                    rect.bottom() - tokens::SPACE_XS,
                ),
            ),
            crate::text::TextRole::Note,
            detail,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            tokens::RADIUS,
            egui::Stroke::new(1.0_f32, theme::palette(ui.ctx()).accent),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(if detail.is_empty() {
        title.to_owned()
    } else {
        format!("{title}\n{detail}")
    })
}

pub fn image_preview(ui: &mut Ui, texture: &egui::TextureHandle, max_size: egui::Vec2) -> Response {
    let original = texture.size_vec2();
    let scale = (max_size.x / original.x)
        .min(max_size.y / original.y)
        .min(1.0);
    ui.image((texture.id(), original * scale))
}

pub fn panel(ctx: &egui::Context) -> Frame {
    Frame::new()
        .fill(theme::palette(ctx).panel)
        .inner_margin(tokens::SPACE_LG)
        .corner_radius(tokens::RADIUS)
}

pub fn card(ctx: &egui::Context) -> Frame {
    Frame::new()
        .fill(theme::palette(ctx).raised)
        .inner_margin(tokens::SPACE_MD)
        .corner_radius(tokens::RADIUS)
        .stroke(egui::Stroke::new(1.0_f32, theme::palette(ctx).line))
}

pub fn inset(ctx: &egui::Context) -> Frame {
    Frame::new()
        .fill(theme::palette(ctx).bg)
        .inner_margin(tokens::SPACE_SM)
        .corner_radius(tokens::RADIUS)
}

/// On-demand property/utility window with the same surfaces and spacing as the
/// main workspace. The caller supplies `.open(&mut open)` and persistent `.id`.
pub fn popup<'a>(title: impl Into<WidgetText>, ctx: &egui::Context) -> egui::Window<'a> {
    egui::Window::new(title)
        .frame(panel(ctx).stroke(egui::Stroke::new(1.0_f32, theme::palette(ctx).line)))
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
    popup(title, ctx)
        .fixed_pos(viewport.min + egui::vec2(24.0, 112.0))
        .fixed_size(size)
        .movable(false)
}

/// A dedicated drag affordance keeps row switches and editor buttons clickable.
pub fn reorder_handle<T: Send + Sync + 'static>(ui: &mut Ui, payload: T) -> Response {
    let response = ui
        .add(button_widget("⋮⋮").sense(egui::Sense::drag()))
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to reorder");
    response.dnd_set_drag_payload(payload);
    response
}

/// Highlight the destination row and return its payload only on release.
pub fn reorder_drop_target<T: Send + Sync + 'static>(
    ui: &Ui,
    response: &Response,
) -> Option<std::sync::Arc<T>> {
    if response.dnd_hover_payload::<T>().is_some() {
        ui.painter().rect_stroke(
            response.rect,
            tokens::RADIUS,
            egui::Stroke::new(1.5_f32, theme::palette(ui.ctx()).accent),
            egui::StrokeKind::Inside,
        );
    }
    response.dnd_release_payload::<T>()
}
