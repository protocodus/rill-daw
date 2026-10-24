//! Quiet, composable workspace structure. Interaction remains native egui;
//! project state, navigation and history remain with the caller.
use crate::{theme, tokens};
use egui::{InnerResponse, Response, Ui, WidgetText};
use std::hash::Hash;

/// A compact action row which wraps as complete controls when space is limited.
pub fn toolbar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_SM as i8,
        ))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_SM);
            ui.horizontal_wrapped(contents).inner
        })
}

/// Keep related controls together inside a wrapping toolbar.
pub fn toolbar_group<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        ui.horizontal(contents).inner
    })
}

/// Secondary actions appear on demand, using native menu focus and dismissal.
pub fn action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::menu_button(ui, label, contents)
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
                            ui.add(
                                egui::Label::new(title.into().text_style(egui::TextStyle::Heading))
                                    .truncate(),
                            );
                            back
                        })
                        .inner;
                    (back, inner)
                })
                .inner
            })
            .inner
        },
    );
    WorkspaceHeaderResponse {
        back: row.inner.0,
        inner: row.inner.1,
        response: row.response,
    }
}

/// A uniform mixer channel. Color identifies the track without enclosing each
/// strip in another outlined card. Width includes the internal side margins.
pub fn mixer_strip<R>(
    ui: &mut Ui,
    id: impl Hash,
    width: f32,
    color: Option<egui::Color32>,
    selected: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.push_id(id, |ui| {
        let colors = theme::palette(ui.ctx());
        let frame = egui::Frame::new()
            .fill(if selected {
                colors.track_selected
            } else {
                colors.panel
            })
            .inner_margin(tokens::SPACE_MD);
        let content = |ui: &mut Ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width((width - 2.0 * tokens::SPACE_MD).max(0.0));
                contents(ui)
            })
            .inner
        };
        let result = if selected {
            crate::soft_surface(ui, frame, content)
        } else {
            frame.show(ui, content)
        };
        let rect = result.response.rect;
        ui.painter().line_segment(
            [rect.right_top(), rect.right_bottom()],
            egui::Stroke::new(1.0_f32, colors.line),
        );
        if let Some(color) = color {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 2.0)),
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
