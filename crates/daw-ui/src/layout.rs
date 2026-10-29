//! Quiet, composable workspace structure. Interaction remains native egui;
//! project state, navigation and history remain with the caller.
use crate::{theme, tokens};
use egui::{InnerResponse, Response, Ui, WidgetText};
use std::hash::Hash;

/// A compact action row which wraps as complete controls when space is limited.
pub fn toolbar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let colors = theme::palette(ui.ctx());
    let row = egui::Frame::new()
        .fill(colors.panel)
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_SM as i8,
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_SM);
            ui.horizontal_wrapped(contents).inner
        });
    ui.painter().hline(
        row.response.rect.x_range(),
        row.response.rect.bottom(),
        egui::Stroke::new(1.0, colors.line),
    );
    row
}

/// A compact visual family of tools. The measured width lets a wrapping toolbar
/// move the entire family together; the background never owns input or focus.
pub fn tool_group<R>(
    ui: &mut Ui,
    width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let colors = theme::palette(ui.ctx());
    let background = ui.painter().add(egui::Shape::Noop);
    let group = toolbar_group_sized_with_spacing(ui, width, tokens::SPACE_XS, contents);
    ui.painter().set(
        background,
        egui::Shape::rect_filled(group.response.rect, tokens::RADIUS, colors.bg),
    );
    group
}

/// Keep related controls together inside a wrapping toolbar.
pub fn toolbar_group<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        ui.horizontal(contents).inner
    })
}

/// Reserve a known-width group before drawing its left-to-right contents. The
/// wrapping parent can move the complete group to the next row without adding
/// unused trailing space or changing the behavior of right-aligned controls.
pub fn toolbar_group_sized<R>(
    ui: &mut Ui,
    minimum_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    toolbar_group_sized_with_spacing(ui, minimum_width, tokens::SPACE_SM, contents)
}

/// Reserve a complete group before painting, with compact parent spacing.
pub fn toolbar_group_sized_with_spacing<R>(
    ui: &mut Ui,
    minimum_width: f32,
    spacing: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let width = minimum_width.max(0.0).min(ui.max_rect().width().max(0.0));
    ui.allocate_ui_with_layout(
        egui::vec2(width, tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = spacing.max(0.0);
            contents(ui)
        },
    )
}

/// Reserve trailing controls before right alignment, wrapping the whole group
/// when the remaining row cannot fit it. This keeps its hit area off prior tools.
pub fn toolbar_trailing<R>(
    ui: &mut Ui,
    minimum_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let width = ui
        .available_width()
        .max(minimum_width)
        .min(ui.max_rect().width());
    ui.allocate_ui_with_layout(
        egui::vec2(width, tokens::CONTROL_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        contents,
    )
}

/// Secondary actions appear on demand, using native menu focus and dismissal.
pub fn action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::Action, contents)
}

/// A text-first action menu for narrow columns. The opener retains its full
/// control height and native menu behavior, with compact horizontal padding.
pub fn compact_action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_compact_action_menu(ui, label, contents)
}

/// Presentation settings use a quiet outlined eye button. Unlike action menus,
/// toggling a setting keeps the menu open; Escape or an outside click dismisses it.
/// View changes remain the caller's responsibility and must not edit project data.
pub fn view_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::View, contents)
}

/// A field-like dropdown for related choices, labelled with the current value.
/// Use `ui.close()` to accept a choice, or leave it open for several settings.
/// Nested choice menus use native submenus rather than competing popup state.
pub fn choice_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::Choice, contents)
}

/// A small section heading and its controls, grouped by spacing rather than a box.
pub fn context_section<R>(
    ui: &mut Ui,
    title: impl Into<String>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.vertical(|ui| {
        crate::text::label(ui, title.into());
        contents(ui)
    })
}

/// A utility body that hugs short content and scrolls when it reaches the
/// window's available height. Use inside a window with its own scrolling off.
pub fn scroll_body<R>(
    ui: &mut Ui,
    id: impl Hash,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .min_scrolled_height(0.0)
        .max_height(ui.available_height().max(0.0))
        .auto_shrink([false, true])
        .show(ui, contents)
}

/// Two equal, bounded content panes. Each pane owns its scrolling; headers can
/// stay fixed while only the long list or preview beneath them scrolls.
pub fn bounded_columns<R>(
    ui: &mut Ui,
    id: impl Hash,
    height: f32,
    mut contents: impl FnMut(usize, &mut Ui) -> R,
) -> InnerResponse<[R; 2]> {
    ui.push_id(id, |ui| {
        let height = if height.is_finite() {
            height.max(0.0).min(ui.available_height().max(0.0))
        } else {
            0.0
        };
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(0.0), height),
            egui::Sense::hover(),
        );
        let gap = tokens::SPACE_LG.min(rect.width());
        let width = (rect.width() - gap) * 0.5;
        [0, 1].map(|index| {
            let pane = egui::Rect::from_min_size(
                rect.min + egui::vec2(index as f32 * (width + gap), 0.0),
                egui::vec2(width, height),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(index)
                    .max_rect(pane)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(ui.clip_rect().intersect(pane));
            child.set_width(width);
            contents(index, &mut child)
        })
    })
}

pub struct WorkspaceHeaderResponse<R> {
    pub back: Response,
    pub title: Response,
    pub inner: R,
    pub response: Response,
}

/// One workspace title, Back, and trailing actions. Actions are laid out from
/// right to left. The caller decides what Back does; this never owns navigation.
pub fn workspace_header<R>(
    ui: &mut Ui,
    title: impl Into<WidgetText>,
    actions: impl FnOnce(&mut Ui) -> R,
) -> WorkspaceHeaderResponse<R> {
    let row = crate::soft_surface(
        ui,
        egui::Frame::new().inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_SM as i8,
        )),
        |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let inner = actions(ui);
                    ui.add_space(tokens::SPACE_MD);
                    let back = ui
                        .with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let back = crate::button(ui, "Back")
                                .on_hover_text("Return to the previous workspace");
                            ui.add_space(tokens::SPACE_XS);
                            let title = ui.add(
                                egui::Label::new(title.into().text_style(egui::TextStyle::Heading))
                                    .truncate(),
                            );
                            (back, title)
                        })
                        .inner;
                    (back.0, back.1, inner)
                })
                .inner
            })
            .inner
        },
    );
    WorkspaceHeaderResponse {
        back: row.inner.0,
        title: row.inner.1,
        inner: row.inner.2,
        response: row.response,
    }
}

/// A channel name action that keeps the arranger's exact track color. Text
/// contrast follows the user-chosen color rather than the application theme.
pub fn mixer_channel_button(ui: &mut Ui, name: &str, width: f32, color: egui::Color32) -> Response {
    let foreground = theme::contrast_text(color);
    let response = ui.add_sized(
        egui::vec2(width, tokens::CONTROL_HEIGHT),
        egui::Button::new(
            crate::text::TextRole::Label
                .rich(ui.ctx(), name)
                .color(foreground),
        )
        .fill(color)
        .corner_radius(tokens::RADIUS)
        .truncate(),
    );
    // A fixed identity fill needs its own hover/focus cue without changing hue.
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            tokens::RADIUS,
            egui::Stroke::new(
                if response.has_focus() {
                    2.0_f32
                } else {
                    1.0_f32
                },
                foreground.gamma_multiply(0.65),
            ),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// Bus headers retain native button input and a distinct, reserved BUS badge.
/// The name truncates before the badge instead of painting beneath it.
pub fn mixer_bus_button(ui: &mut Ui, name: &str, width: f32, tint: egui::Color32) -> Response {
    let fill = crate::bus_header_fill(ui.ctx(), tint);
    let foreground = theme::contrast_text(fill);
    let badge_id = egui::Id::new("mixer_bus_badge");
    let mut atoms = egui::Atoms::new(
        crate::text::TextRole::Label
            .rich(ui.ctx(), name)
            .color(foreground),
    );
    atoms.push_right(egui::Atom::custom(badge_id, egui::vec2(32.0, 18.0)));
    let button = egui::Button::new(atoms)
        .min_size(egui::vec2(width, tokens::CONTROL_HEIGHT))
        .fill(fill)
        .corner_radius(tokens::RADIUS)
        .truncate()
        .atom_ui(ui);
    if let Some(rect) = button.rect(badge_id) {
        crate::paint_bus_badge(ui.painter(), rect, tint);
    }
    button.response
}

/// A uniform mixer channel with a quiet wash of its arranger track color.
/// Size includes the internal margins; all channels can share the viewport
/// height while their controls remain stacked. Uncolored channels stay neutral.
pub fn mixer_strip<R>(
    ui: &mut Ui,
    id: impl Hash,
    size: egui::Vec2,
    color: Option<egui::Color32>,
    selected: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.push_id(id, |ui| {
        let colors = theme::palette(ui.ctx());
        let base = if selected {
            colors.track_selected
        } else {
            colors.panel
        };
        let fill = color.map_or(base, |color| {
            let amount = match (ui.ctx().theme(), selected) {
                (egui::Theme::Dark, false) => 0.08,
                (egui::Theme::Dark, true) => 0.14,
                (egui::Theme::Light, false) => 0.06,
                (egui::Theme::Light, true) => 0.12,
            };
            base.lerp_to_gamma(color, amount)
        });
        // A real canvas gutter keeps adjacent strips separate even when they
        // share a track color. Preserve the total width and control alignment.
        let gutter = tokens::SPACE_XS * 0.5;
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(tokens::RADIUS)
            .outer_margin(egui::Margin::symmetric(gutter as i8, 0))
            .inner_margin(egui::Margin::symmetric(
                (tokens::SPACE_MD - gutter) as i8,
                tokens::SPACE_MD as i8,
            ));
        let content = |ui: &mut Ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width((size.x - 2.0 * tokens::SPACE_MD).max(0.0));
                ui.set_height((size.y - 2.0 * tokens::SPACE_MD).max(0.0));
                contents(ui)
            })
            .inner
        };
        let result = if selected {
            crate::soft_surface(ui, frame, content)
        } else {
            frame.show(ui, content)
        };
        let rect = result.response.rect.shrink2(egui::vec2(gutter, 0.0));
        if selected {
            crate::paint_selection_glow(
                ui.painter(),
                rect,
                tokens::RADIUS,
                color.unwrap_or(colors.accent),
            );
        }
        ui.painter().rect_stroke(
            rect,
            tokens::RADIUS,
            egui::Stroke::new(1.0_f32, if selected { colors.accent } else { colors.line }),
            egui::StrokeKind::Inside,
        );
        if let Some(color) = color {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    rect.min + egui::vec2(tokens::RADIUS as f32, 0.0),
                    egui::vec2(rect.width() - 2.0 * tokens::RADIUS as f32, 3.0),
                ),
                0.0,
                color,
            );
        }
        result.inner
    })
}

/// Stereo peak display, −60 dBFS to 0 dBFS. Non-finite readings show silence;
/// normalized peaks at/above unity use the warning color. No smoothing state.
pub fn stereo_meter(ui: &mut Ui, peaks: [f32; 2], size: egui::Vec2) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(size.max(egui::Vec2::splat(1.0)), egui::Sense::hover());
    let colors = theme::palette(ui.ctx());
    let gap = (rect.width() * 0.15).min(tokens::SPACE_SM);
    let width = (rect.width() - gap) * 0.5;
    let mut db = [f32::NEG_INFINITY; 2];
    for (index, peak) in peaks.into_iter().enumerate() {
        let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
        db[index] = 20.0 * peak.log10();
        let level = ((db[index] + 60.0) / 60.0).clamp(0.0, 1.0);
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(index as f32 * (width + gap), 0.0),
            egui::vec2(width, rect.height()),
        );
        ui.painter().rect_filled(bar, 1.0, colors.bg);
        if level > 0.0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(bar.left(), bar.bottom() - bar.height() * level),
                    bar.max,
                ),
                1.0,
                if peak >= 1.0 {
                    colors.orange
                } else {
                    colors.accent
                },
            );
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            ui.is_enabled(),
            format!("Stereo peak: L {:.1} dBFS, R {:.1} dBFS", db[0], db[1]),
        )
    });
    response
}
