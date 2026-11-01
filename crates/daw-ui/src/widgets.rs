//! Components use native egui interaction and report changes through `Response`.
//! Give repeated rows stable `ui.push_id(model_id, ...)` scopes.

use crate::{theme, tokens};
use egui::{Button, ComboBox, DragValue, Frame, InnerResponse, Response, Ui, Widget, WidgetText};
use std::{hash::Hash, ops::RangeInclusive};

pub fn button_widget(label: impl Into<WidgetText>) -> crate::IconButton {
    // A quiet filled action retains its hit target and native focus outline.
    // Input widgets continue to use the theme's visible boundary stroke.
    crate::IconButton::new(label)
}

pub fn button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(button_widget(label))
}

/// A secondary toolbar command with a full target and native hover/focus feedback.
pub fn quiet_button_widget(label: impl Into<WidgetText>) -> crate::IconButton {
    button_widget(label).quiet()
}

pub fn quiet_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(quiet_button_widget(label))
}

/// Playback guides keep their selected fill; idle choices share the bar surface.
pub fn quiet_toggle(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    let mut response = ui.add(button_widget(label).selected(*value).quiet());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// A command that changes presentation without editing the document or tool.
pub fn view_button_widget(label: impl Into<WidgetText>) -> crate::IconButton {
    let label = label.into();
    crate::IconButton::with_icon(
        crate::Icon::for_label(label.text()).unwrap_or(crate::Icon::View),
        label,
    )
    .presentation()
}

pub fn view_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(view_button_widget(label))
}

/// A persistent workspace or presentation choice, using the quiet view outline.
/// The owner handles navigation; this component never changes a tool or document.
pub fn view_choice(ui: &mut Ui, selected: bool, label: impl Into<WidgetText>) -> Response {
    ui.add(button_widget(label).presentation().selected(selected))
}

/// A quiet page choice with an active underline, native keyboard activation,
/// and the same full-size target as other controls. The host owns navigation.
pub fn tab_choice(ui: &mut Ui, selected: bool, label: impl Into<WidgetText>) -> Response {
    ui.add(crate::IconButton::tab(selected, label))
}

/// A presentation or playback-guide toggle with the same view affordance.
/// State and any project/history semantics remain with the caller.
pub fn view_toggle(ui: &mut Ui, value: &mut bool, label: impl Into<WidgetText>) -> Response {
    let mut response = view_choice(ui, *value, label);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// A persistent, exclusive interaction mode. Application state stays with the caller.
pub fn tool_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<WidgetText>,
) -> Response {
    let mut response = ui.add(crate::IconButton::tool(*current == value, label));
    if response.clicked() && *current != value {
        *current = value;
        response.mark_changed();
    }
    response
}

pub fn primary_button_widget(ui: &Ui, label: impl Into<WidgetText>) -> crate::IconButton {
    button_widget(label).fill(theme::palette(ui.ctx()).track_selected)
}

pub fn primary_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(primary_button_widget(ui, label))
}

pub fn small_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    compact_button(ui, button_widget(label))
}

// Compact actions save horizontal space while retaining normal label padding
// and target height. Keep the widget in the parent Ui so wrapping stays native.
fn compact_button(ui: &mut Ui, button: crate::IconButton) -> Response {
    let padding = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = egui::vec2(tokens::SPACE_SM, tokens::BUTTON_PADDING_Y);
    let response = ui.add(button.min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)));
    ui.spacing_mut().button_padding = padding;
    response
}

/// Reserve trailing actions before laying out a flexible label or control.
/// Actions are supplied right to left; long names wrap inside the remaining
/// width instead of pushing controls out of their window.
pub fn action_row<A, C>(
    ui: &mut Ui,
    actions: impl FnOnce(&mut Ui) -> A,
    content: impl FnOnce(&mut Ui) -> C,
) -> InnerResponse<(A, C)> {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let actions = actions(ui);
            ui.add_space(tokens::SPACE_SM);
            let content = ui
                .with_layout(egui::Layout::left_to_right(egui::Align::Center), content)
                .inner;
            (actions, content)
        })
        .inner
    })
}

pub fn menu_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let label = label.into();
    if let Some(icon) = crate::Icon::for_label(label.text()) {
        let response = native_menu_button(
            ui,
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
        native_menu_button(ui, label, add_contents)
    }
}

/// Keep full-size commands reachable with the mouse in a short window.
pub fn command_menu_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    menu_button(ui, label, contents)
}

/// Bound a command list to the viewport, including its frame and outer spacing.
pub fn command_menu_contents<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    // Legacy callers may explicitly bound contents inside a shared menu.
    // Each popup owns one scroll pane; nested submenus have their own layer.
    let scope_id = ui.layer_id().id.with("command_menu_scroll_scope");
    if ui.ctx().data(|data| data.get_temp::<bool>(scope_id)) == Some(true) {
        return contents(ui);
    }
    // Popup chooses its position from the previous frame's size. A capacity
    // derived from that moving position feeds back into placement, making a
    // submenu shrink after a wheel gesture has already reached its end. Keep
    // the capacity stable and let the native popup constrain the whole frame.
    let max_height = (ui.ctx().content_rect().height()
        - ui.spacing().menu_margin.sum().y
        - 2.0 * tokens::SPACE_SM)
        .max(tokens::CONTROL_HEIGHT);
    let frame = ui.ctx().cumulative_frame_nr();
    let pass = ui.ctx().cumulative_pass_nr();
    let seen_id = ui.id().with("command_menu_seen");
    let newly_open = ui
        .ctx()
        .data(|data| data.get_temp::<u64>(seen_id))
        .is_none_or(|last| frame > last.saturating_add(1));
    let mut area = egui::ScrollArea::vertical()
        .id_salt(ui.id().with("command_menu_scroll"))
        .max_height(max_height)
        // Popup areas remember the previous frame's height. Reserve capacity
        // before auto-shrinking to content, or reopening a longer menu can
        // inherit a short viewport even when the screen has ample space.
        .min_scrolled_height(max_height)
        .auto_shrink([true, true]);
    if newly_open {
        area = area.vertical_scroll_offset(0.0);
    }
    ui.ctx().data_mut(|data| data.insert_temp(scope_id, true));
    let scroll = area.show(ui, contents);
    ui.ctx().data_mut(|data| {
        data.remove::<bool>(scope_id);
        data.insert_temp(seen_id, frame);
        // Submenus finish before their parent. Retain the innermost scroll
        // viewport for mouse automation, refreshing it on every layout pass.
        let diagnostic_id = egui::Id::new("command_menu_scroll_rect");
        if !data
            .get_temp::<(u64, u64, egui::Rect)>(diagnostic_id)
            .is_some_and(|(_, drawn_pass, _)| drawn_pass == pass)
        {
            data.insert_temp(diagnostic_id, (frame, pass, scroll.inner_rect));
        }
    });
    scroll.inner
}

pub fn command_menu_scroll_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| {
        data.get_temp::<(u64, u64, egui::Rect)>(egui::Id::new("command_menu_scroll_rect"))
    })
    .filter(|(drawn, _, _)| frame <= drawn.saturating_add(1))
    .map(|(_, _, rect)| rect)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuRole {
    Action,
    Choice,
    View,
    QuietAction,
    QuietChoice,
    QuietView,
}

/// A secondary command menu, retaining its icon and disclosure without a box.
pub fn quiet_action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    semantic_menu(ui, label, MenuRole::QuietAction, contents)
}

pub fn quiet_view_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    semantic_menu(ui, label, MenuRole::QuietView, contents)
}

/// A quiet chooser preserves the open popup while its values are edited.
pub fn quiet_picker_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<InnerResponse<R>>) {
    semantic_picker(ui, label, MenuRole::QuietChoice, false, None, contents)
}

/// The three toolbar roles share native layout, focus, and popup handling.
/// Only their affordance changes: filled commands, inset choices, outlined views.
pub(crate) fn semantic_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    role: MenuRole,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    if egui::containers::menu::is_in_menu(ui)
        && matches!(role, MenuRole::Action | MenuRole::QuietAction)
    {
        return menu_button(ui, label, contents);
    }
    let (response, inner) = semantic_picker(ui, label, role, false, None, contents);
    InnerResponse::new(inner.map(|inner| inner.inner), response)
}

pub(crate) fn semantic_compact_action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    if egui::containers::menu::is_in_menu(ui) {
        return menu_button(ui, label, contents);
    }
    let (response, inner) = semantic_picker(ui, label, MenuRole::Action, true, None, contents);
    InnerResponse::new(inner.map(|inner| inner.inner), response)
}

/// Text-first automation state in a fixed-width channel source row. Full
/// control height and menu semantics remain native; only horizontal space is compact.
pub(crate) fn semantic_narrow_action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let (response, inner) = semantic_picker(
        ui,
        label,
        MenuRole::Action,
        true,
        Some(tokens::SPACE_XS),
        contents,
    );
    InnerResponse::new(inner.map(|inner| inner.inner), response)
}

fn semantic_picker<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    role: MenuRole,
    compact: bool,
    horizontal_padding: Option<f32>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<InnerResponse<R>>) {
    use egui::containers::menu::{MenuConfig, MenuState, SubMenu};

    let (role, quiet) = match role {
        MenuRole::QuietAction => (MenuRole::Action, true),
        MenuRole::QuietChoice => (MenuRole::Choice, true),
        MenuRole::QuietView => (MenuRole::View, true),
        role => (role, false),
    };
    let nested = egui::containers::menu::is_in_menu(ui);
    let label = label.into();
    let icon = match role {
        MenuRole::View => Some(crate::Icon::View),
        MenuRole::Action if !compact => crate::Icon::for_label(label.text()),
        MenuRole::Action => None,
        _ => None,
    };
    let icon_id = egui::Id::new("rill-menu-role-icon");
    let disclosure_id = egui::Id::new("rill-menu-disclosure");
    let mut atoms = egui::Atoms::new(label);
    if icon.is_some() {
        atoms.push_left(egui::Atom::custom(icon_id, egui::Vec2::splat(18.0)));
    }
    atoms.push_right(egui::Atom::custom(disclosure_id, egui::Vec2::splat(12.0)));

    // Style only the opener, never its popup contents or following controls.
    // Avoid a nested Ui scope so horizontal_wrapped can wrap each whole control.
    let previous_style = ui.style().clone();
    let colors = theme::palette(ui.ctx());
    ui.spacing_mut().button_padding = egui::vec2(
        horizontal_padding.unwrap_or(if compact {
            tokens::SPACE_SM
        } else {
            tokens::BUTTON_PADDING_X
        }),
        tokens::BUTTON_PADDING_Y,
    );
    if horizontal_padding.is_some() {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
    }
    let visuals = &mut ui.visuals_mut().widgets;
    match role {
        MenuRole::Action => {
            visuals.inactive.bg_stroke = egui::Stroke::new(
                tokens::BORDER_WIDTH,
                colors.line.gamma_multiply(tokens::GOLDEN_RATIO_INVERSE),
            );
        }
        MenuRole::Choice => {
            visuals.inactive.weak_bg_fill = colors.bg;
            visuals.inactive.bg_stroke = egui::Stroke::new(tokens::BORDER_WIDTH, colors.line);
        }
        MenuRole::View => {
            visuals.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            visuals.inactive.bg_stroke = egui::Stroke::new(tokens::BORDER_WIDTH, colors.line);
            visuals.hovered.bg_stroke = egui::Stroke::new(tokens::BORDER_WIDTH, colors.accent);
        }
        _ => unreachable!("quiet menu roles normalize before styling"),
    }
    if nested {
        let next_id = ui.next_auto_id();
        let open = MenuState::from_ui(ui, |state, _| {
            state.open_item == Some(SubMenu::id_from_widget_id(next_id))
        });
        if open {
            ui.visuals_mut().widgets.inactive = ui.visuals().widgets.open;
        }
    }
    // Keep the native frame's padding and hit target. Only its paint is replaced;
    // `frame(false)` would also remove the button's horizontal breathing room.
    let background = ui.painter().add(egui::Shape::Noop);
    let has_frame = ui.visuals().button_frame;
    let button = Button::new(atoms)
        .frame_when_inactive(!quiet)
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .frame(has_frame)
        .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT))
        .corner_radius(tokens::RADIUS)
        .wrap_mode(egui::TextWrapMode::Truncate)
        .atom_ui(ui);
    let visuals = ui.style().interact(&button.response);
    let color = visuals.text_color();
    let engaged = button.response.hovered()
        || button.response.is_pointer_button_down_on()
        || button.response.has_focus();
    if has_frame && (!quiet || engaged) {
        ui.painter().set(
            background,
            crate::component_shape(
                ui.painter(),
                button.response.rect,
                tokens::RADIUS,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                crate::ComponentMaterial {
                    elevated: false,
                    active: button.response.enabled() && button.response.has_focus(),
                },
            ),
        );
    }
    if let (Some(icon), Some(rect)) = (icon, button.rect(icon_id)) {
        icon.paint(ui.painter(), rect, color);
    }
    if let Some(rect) = button.rect(disclosure_id) {
        if nested {
            crate::Icon::ChevronRight
        } else {
            crate::Icon::ChevronDown
        }
        .paint(ui.painter(), rect, color);
    }
    ui.set_style(previous_style);

    let response = button.response;
    let mut config = MenuConfig::new();
    if role != MenuRole::Action {
        config.close_behavior = egui::PopupCloseBehavior::CloseOnClickOutside;
    }
    let inner = if nested {
        // A submenu's provisional first-frame area can cover its opener. Retain
        // the opening click while egui settles that area's final position.
        if response.clicked() {
            config.close_behavior = egui::PopupCloseBehavior::IgnoreClicks;
        }
        SubMenu::default()
            .config(config)
            .show(ui, &response, |ui| command_menu_contents(ui, contents))
    } else {
        egui::Popup::menu(&response)
            .close_behavior(config.close_behavior)
            .style(config.style.clone())
            .info(
                egui::UiStackInfo::new(egui::UiKind::Menu)
                    .with_tag_value(MenuConfig::MENU_CONFIG_TAG, config),
            )
            .show(|ui| command_menu_contents(ui, contents))
    };
    (response, inner)
}

fn native_menu_button<'a, R>(
    ui: &mut Ui,
    atoms: impl egui::IntoAtoms<'a>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    use egui::containers::menu::{MenuConfig, MenuState, SubMenu, SubMenuButton};
    if !egui::containers::menu::is_in_menu(ui) {
        return ui.menu_button(atoms, |ui| command_menu_contents(ui, add_contents));
    }
    let submenu = SubMenuButton::new(atoms);
    let id = ui.next_auto_id();
    let open = MenuState::from_ui(ui, |state, _| {
        state.open_item == Some(SubMenu::id_from_widget_id(id))
    });
    let inactive = ui.visuals().widgets.inactive;
    if open {
        ui.visuals_mut().widgets.inactive = ui.visuals().widgets.open;
    }
    let response = ui.add(submenu.button);
    ui.visuals_mut().widgets.inactive = inactive;
    let mut config = MenuConfig::find(ui);
    if response.clicked() {
        // A new submenu's provisional first-frame area can cover this button.
        // Its subsequent move must not turn the opening click into an outside
        // click. All later leaf/outside clicks retain the inherited behavior.
        config.close_behavior = egui::PopupCloseBehavior::IgnoreClicks;
    }
    let inner = submenu
        .sub_menu
        .config(config)
        .show(ui, &response, |ui| command_menu_contents(ui, add_contents));
    InnerResponse::new(inner.map(|inner| inner.inner), response)
}

/// A searchable or editable choice field with a disclosure chevron. Its popover
/// stays open while internal controls are used. Call `ui.close()` to accept.
pub fn picker_button<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<InnerResponse<R>>) {
    semantic_picker(ui, label, MenuRole::Choice, false, None, contents)
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
        let colors = theme::palette(ui.ctx());
        let visuals = ui.style().interact(&response);
        let track = egui::Rect::from_center_size(
            egui::pos2(rect.left() + tokens::SWITCH_WIDTH * 0.5, rect.center().y),
            egui::vec2(tokens::SWITCH_WIDTH, tokens::SWITCH_HEIGHT),
        );
        let fill = if *value {
            colors.accent
        } else {
            visuals.bg_fill
        };
        ui.painter().add(crate::component_shape(
            ui.painter(),
            track,
            (tokens::SWITCH_HEIGHT * 0.5).round() as u8,
            fill,
            visuals.bg_stroke,
            crate::ComponentMaterial {
                elevated: false,
                active: response.enabled() && response.has_focus(),
            },
        ));
        let animation = ui.ctx().animate_bool(response.id, *value);
        let radius = (tokens::SWITCH_HEIGHT - tokens::SPACE_XS) * 0.5;
        let center_x = egui::lerp(
            (track.left() + tokens::SWITCH_HEIGHT * 0.5)
                ..=(track.right() - tokens::SWITCH_HEIGHT * 0.5),
            animation,
        );
        let thumb = egui::Rect::from_center_size(
            egui::pos2(center_x, track.center().y),
            egui::Vec2::splat(radius * 2.0),
        );
        ui.painter().add(crate::component_shape(
            ui.painter(),
            thumb,
            radius.round() as u8,
            if *value { colors.bg } else { colors.muted },
            egui::Stroke::NONE,
            crate::ComponentMaterial::default(),
        ));
        ui.painter().galley(
            egui::pos2(track.right() + gap, rect.center().y - galley.size().y * 0.5),
            galley,
            visuals.text_color(),
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                tokens::RADIUS,
                egui::Stroke::new(tokens::FOCUS_WIDTH, theme::palette(ui.ctx()).accent),
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

/// A full-width navigation row with a trailing item count, such as a library
/// collection or producer. Selection and filtering stay with the caller.
pub fn count_item(ui: &mut Ui, selected: bool, label: &str, count: usize) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), tokens::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            format!("{label}, {count}"),
        )
    });
    if ui.is_rect_visible(rect) {
        let colors = theme::palette(ui.ctx());
        let painter = ui.painter_at(rect);
        if selected || response.hovered() {
            painter.rect_filled(
                rect,
                tokens::RADIUS,
                if selected {
                    colors.track_selected
                } else {
                    colors.control_hover
                },
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                tokens::RADIUS,
                egui::Stroke::new(1.5_f32, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        let inner = rect.shrink2(egui::vec2(tokens::SPACE_SM, 0.0));
        let count = painter.layout_no_wrap(
            count.to_string(),
            crate::text::TextRole::Note.font(),
            colors.muted,
        );
        painter.galley(
            egui::pos2(
                inner.right() - count.size().x,
                inner.center().y - count.size().y * 0.5,
            ),
            count.clone(),
            colors.muted,
        );
        crate::text::paint_row(
            &painter,
            inner.with_max_x(inner.right() - count.size().x - tokens::SPACE_SM),
            crate::text::TextRole::Body,
            label,
        );
    }
    response.on_hover_text(label)
}

/// An active filter shown as a compact chip; clicking it removes the filter.
pub fn filter_chip(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    compact_button(
        ui,
        crate::IconButton::with_icon(crate::Icon::Close, label).selected(true),
    )
    .on_hover_text("Remove filter")
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
    let inner = egui::Popup::menu(&response).show(|ui| command_menu_contents(ui, contents));
    (response, inner)
}

pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .frame(false)
            .hint_text(hint)
            .desired_width(width.max(0.0))
            .margin(egui::vec2(
                tokens::BUTTON_PADDING_X,
                tokens::BUTTON_PADDING_Y,
            ))
            .vertical_align(egui::Align::Center)
            .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
    );
    paint_input_material(ui, background, &response, ui.visuals().text_edit_bg_color());
    response
}

/// Paint an editor's frame below its text and selection, without allocating a
/// widget or changing native focus, geometry, or text-edit state.
fn paint_input_material(
    ui: &Ui,
    background: egui::layers::ShapeIdx,
    response: &Response,
    fill: egui::Color32,
) {
    let visuals = ui.style().interact(response);
    let stroke = if response.has_focus() {
        ui.visuals().selection.stroke
    } else {
        visuals.bg_stroke
    };
    ui.painter().set(
        background,
        crate::component_shape(
            ui.painter(),
            response.rect,
            tokens::RADIUS,
            fill,
            stroke,
            crate::ComponentMaterial {
                elevated: false,
                active: response.enabled() && response.has_focus(),
            },
        ),
    );
}

/// Editable numeric canvas control that preserves the parent's row geometry.
/// Drag to adjust, or click to replace the complete value.
pub fn number_at(
    ui: &mut Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    value: impl Widget,
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

pub fn slider<T: egui::emath::Numeric>(
    value: &mut T,
    range: RangeInclusive<T>,
) -> crate::Slider<'_> {
    crate::Slider::new(value, range)
}

/// Shared numeric field retaining native dragging, parsing and focus behavior.
/// Clicking enters text mode; double/triple-clicking selects the whole value,
/// including the sign and decimal part, instead of one punctuation-delimited
/// word. Prefixes and units remain outside the editable numeric text.
#[must_use = "Add the numeric field to a Ui"]
pub struct Number<'a>(DragValue<'a>);

impl<'a> Number<'a> {
    pub fn speed(mut self, speed: impl Into<f64>) -> Self {
        self.0 = self.0.speed(speed);
        self
    }

    pub fn range<T: egui::emath::Numeric>(mut self, range: RangeInclusive<T>) -> Self {
        self.0 = self.0.range(range);
        self
    }

    pub fn clamp_existing_to_range(mut self, clamp: bool) -> Self {
        self.0 = self.0.clamp_existing_to_range(clamp);
        self
    }

    pub fn prefix(mut self, prefix: impl ToString) -> Self {
        self.0 = self.0.prefix(prefix);
        self
    }

    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.0 = self.0.suffix(suffix);
        self
    }

    pub fn min_decimals(mut self, decimals: usize) -> Self {
        self.0 = self.0.min_decimals(decimals);
        self
    }

    pub fn max_decimals(mut self, decimals: usize) -> Self {
        self.0 = self.0.max_decimals(decimals);
        self
    }

    pub fn max_decimals_opt(mut self, decimals: Option<usize>) -> Self {
        self.0 = self.0.max_decimals_opt(decimals);
        self
    }

    pub fn fixed_decimals(mut self, decimals: usize) -> Self {
        self.0 = self.0.fixed_decimals(decimals);
        self
    }

    pub fn custom_formatter(
        mut self,
        formatter: impl 'a + Fn(f64, RangeInclusive<usize>) -> String,
    ) -> Self {
        self.0 = self.0.custom_formatter(formatter);
        self
    }

    pub fn custom_parser(mut self, parser: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.0 = self.0.custom_parser(parser);
        self
    }

    pub fn update_while_editing(mut self, update: bool) -> Self {
        self.0 = self.0.update_while_editing(update);
        self
    }
}

impl Widget for Number<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let id = ui.next_auto_id();
        let rectangle_id = id.with("number_edit_rectangle");
        let edit_rect = ui.data(|data| data.get_temp::<egui::Rect>(rectangle_id));
        let numeric_multiclick = ui.is_enabled()
            && ui.memory(|memory| memory.has_focus(id))
            && ui.input(|input| {
                !input.modifiers.any()
                    && (input
                        .pointer
                        .button_double_clicked(egui::PointerButton::Primary)
                        || input
                            .pointer
                            .button_triple_clicked(egui::PointerButton::Primary))
                    && input.pointer.interact_pos().is_some_and(|position| {
                        edit_rect.is_some_and(|rect| rect.contains(position))
                    })
            });
        let pointer = if numeric_multiclick {
            // TextEdit selects a word before processing keyboard events. Set
            // the numeric selection first and suppress only this widget's
            // multi-click pointer handling, including coalesced text input.
            // Restore pointer state immediately below so ordinary caret clicks,
            // dragging and other widgets retain their native behavior.
            if let Some(text) = ui.data(|data| data.get_temp::<String>(id)) {
                select_number_text(ui, id, &text);
            }
            Some(ui.input_mut(|input| std::mem::take(&mut input.pointer)))
        } else {
            None
        };
        // DragValue alternates between native Button and TextEdit. Retain both
        // layouts and all text selection behavior while giving them one frame.
        let background = ui.painter().add(egui::Shape::Noop);
        let previous_style = ui.style().clone();
        let visuals = ui.visuals_mut();
        visuals.text_edit_bg_color = Some(egui::Color32::TRANSPARENT);
        // Keep the selection color: TextEdit also uses it for selected glyphs.
        visuals.selection.stroke.width = 0.0;
        for visual in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            visual.bg_fill = egui::Color32::TRANSPARENT;
            visual.weak_bg_fill = egui::Color32::TRANSPARENT;
            visual.bg_stroke.width = 0.0;
        }
        let response = self.0.ui(ui);
        ui.set_style(previous_style);
        let fill = if response.has_focus() {
            ui.visuals().text_edit_bg_color()
        } else {
            ui.style().interact(&response).weak_bg_fill
        };
        paint_input_material(ui, background, &response, fill);
        if let Some(pointer) = pointer {
            ui.input_mut(|input| input.pointer = pointer);
        }
        ui.data_mut(|data| data.insert_temp(rectangle_id, response.rect));
        // DragValue selects everything when focus is first acquired. A rapid
        // second click then reaches TextEdit, whose word selection can leave
        // part of a decimal (or its sign) in place. Preserve native caret/drag
        // editing, but make a numeric multi-click mean the complete value.
        if !numeric_multiclick
            && response.has_focus()
            && ui.input(|input| !input.modifiers.any())
            && (response.double_clicked() || response.triple_clicked())
            && let Some(text) = ui.data(|data| data.get_temp::<String>(response.id))
        {
            select_number_text(ui, response.id, &text);
            ui.ctx().request_repaint();
        }
        response
    }
}

fn select_number_text(ui: &Ui, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::default(),
            egui::text::CCursor::new(text.chars().count()),
        )));
    state.store(ui.ctx(), id);
}

pub fn number<T: egui::emath::Numeric>(value: &mut T) -> Number<'_> {
    Number(DragValue::new(value))
}

pub fn color_edit(ui: &mut Ui, value: &mut [u8; 3]) -> Response {
    ui.color_edit_button_srgb(value)
}

/// Search-result row with an optional real preview image. Missing images reserve
/// the same space so titles stay aligned as asynchronous previews arrive.
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
            theme::palette(ui.ctx()).track_selected
        } else if response.hovered() {
            theme::palette(ui.ctx()).raised
        } else {
            theme::palette(ui.ctx()).panel
        },
    );
    let image_x = rect.left() + tokens::SPACE_MD;
    let image_size = egui::vec2(58.0, height - 8.0);
    if let Some(texture) = thumbnail {
        let original = texture.size_vec2();
        let scale = (image_size.x / original.x).min(image_size.y / original.y);
        let image_rect = egui::Rect::from_center_size(
            egui::pos2(image_x + image_size.x * 0.5, rect.center().y),
            original * scale,
        );
        painter.image(
            texture.id(),
            image_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let x = image_x + image_size.x + tokens::SPACE_MD;
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
        .corner_radius(tokens::SURFACE_RADIUS)
}

pub fn card(ctx: &egui::Context) -> Frame {
    let colors = theme::palette(ctx);
    Frame::new()
        .fill(colors.raised)
        .stroke(egui::Stroke::new(
            tokens::BORDER_WIDTH,
            colors.line.gamma_multiply(tokens::GOLDEN_RATIO_INVERSE),
        ))
        .inner_margin(tokens::SPACE_MD)
        .corner_radius(tokens::SURFACE_RADIUS)
}

pub fn inset(ctx: &egui::Context) -> Frame {
    let colors = theme::palette(ctx);
    Frame::new()
        .fill(colors.bg)
        .stroke(egui::Stroke::new(
            tokens::BORDER_WIDTH,
            colors.line.gamma_multiply(tokens::GOLDEN_RATIO_INVERSE),
        ))
        .inner_margin(tokens::SPACE_MD)
        .corner_radius(tokens::SURFACE_RADIUS)
}

fn popup_bounds(ctx: &egui::Context) -> egui::Rect {
    let viewport = ctx.content_rect();
    // The transport wraps on narrow displays. Follow its reserved panel area
    // so every utility leaves the actual playback controls available.
    let top =
        (ctx.available_rect().top() + tokens::SPACE_SM).max(viewport.top() + tokens::POPUP_TOP);
    egui::Rect::from_min_max(
        egui::pos2(viewport.left() + tokens::POPUP_MARGIN, top),
        viewport.max - egui::Vec2::splat(tokens::POPUP_MARGIN),
    )
}

/// Maximum utility content size after reserving transport, title, and frame.
/// Clamp explicit minimum heights to this size so compact displays stay usable.
pub fn popup_body_size(ctx: &egui::Context) -> egui::Vec2 {
    // egui adds the frame, a padded native title row, and its separator outside
    // the body. Reserve all of them so content cannot hide the close button or
    // grow beyond the viewport when an asynchronous view adds more rows.
    let margin = tokens::WINDOW_MARGIN.sum();
    let frame = margin + egui::Vec2::splat(2.0 * tokens::WINDOW_BORDER_WIDTH);
    let title = tokens::CONTROL_HEIGHT + margin.y + tokens::WINDOW_BORDER_WIDTH;
    (popup_bounds(ctx).size() - frame - egui::vec2(0.0, title)).max(egui::vec2(160.0, 120.0))
}

/// On-demand property/utility window with the same surfaces and spacing as the
/// main workspace. The caller supplies `.open(&mut open)` and persistent `.id`.
pub fn popup<'a>(title: impl Into<WidgetText>, ctx: &egui::Context) -> egui::Window<'a> {
    use egui::emath::GuiRounding as _;

    let available = popup_bounds(ctx);
    let body_size = popup_body_size(ctx);
    let style = ctx.style();
    let title = title.into().fallback_text_style(egui::TextStyle::Heading);
    let mut job =
        (*title.into_layout_job(&style, egui::TextStyle::Heading.into(), egui::Align::Center))
            .clone();
    let title_height = ctx
        .fonts_mut(|fonts| job.font_height(fonts))
        .max(style.spacing.interact_size.y);
    // Native Window reserves a symmetric icon lane around its centered title.
    // It also relays ordinary text with infinite width, making long names an
    // unbounded minimum window width. A prelaid galley retains its full source
    // text for window identity/accessibility while bounding its painted row.
    let icon_width = style.spacing.icon_width.min(title_height);
    let icon_padding = ((title_height - icon_width) * 0.5).round_ui();
    let controls_width = 2.0 * (icon_padding + icon_width + style.spacing.item_spacing.x);
    job.wrap = egui::text::TextWrapping::truncate_at_width((body_size.x - controls_width).max(0.0));
    let title = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    egui::Window::new(title)
        .frame(
            panel(ctx)
                .inner_margin(tokens::WINDOW_MARGIN)
                .stroke(ctx.style().visuals.window_stroke)
                .shadow(ctx.style().visuals.window_shadow),
        )
        .default_width(480.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(available.center())
        .max_size(body_size)
        .constrain_to(available)
        .movable(false)
        .scroll([false, true])
        .collapsible(false)
        .resizable(false)
}

/// Scroll a popup's content while keeping the actions below it visible. Use
/// with `.vscroll(false)` and reserve the footer's height, including spacing.
pub fn popup_scroll_body<R>(
    ui: &mut Ui,
    id: impl Hash,
    footer_height: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    // An auto-sized window can be shorter than its initial request. Subtracting
    // the footer from that last frame's height repeatedly collapses the body
    // when the footer reservation is even slightly larger than its contents.
    // Use the viewport-derived capacity, retaining the actual header offset.
    let header_height = (ui.cursor().top() - ui.max_rect().top()).max(0.0);
    let body_height = (popup_body_size(ui.ctx()).y - header_height - footer_height).max(0.0);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(body_height)
        .min_scrolled_height(body_height.min(tokens::COMPACT_HEIGHT))
        .auto_shrink([false, true])
        .show(ui, contents)
        .inner
}

/// A focused canvas tool, opened on demand over the arrangement. Its viewport
/// bounds leave the transport accessible and keep close controls on screen.
pub fn workspace_popup<'a>(title: impl Into<WidgetText>, ctx: &egui::Context) -> egui::Window<'a> {
    popup(title, ctx)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(popup_bounds(ctx).min)
        .fixed_size(popup_body_size(ctx))
        .movable(false)
}

/// A dedicated drag affordance keeps row switches and editor buttons clickable.
pub fn reorder_handle<T: Send + Sync + 'static>(ui: &mut Ui, payload: T) -> Response {
    let response = ui
        .add(
            crate::IconButton::icon_only(crate::Icon::Grip, "Reorder insert")
                .sense(egui::Sense::drag()),
        )
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
