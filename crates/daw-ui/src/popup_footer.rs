//! A dialog's actions own the bottom edge; its body owns the remaining viewport.
use crate::{theme, tokens};
use egui::emath::GuiRounding as _;
use egui::{Id, InnerResponse, Rect, Response, Ui, UiBuilder, WidgetText, pos2};
use std::hash::Hash;

pub struct PopupFooter {
    id: Id,
    rect: Rect,
    clip: Rect,
    body_top: f32,
    body: Ui,
}

/// Reserve first, draw through `PopupFooter::body`, then call `show` on the
/// original window Ui. The body never includes the footer's input region.
/// The footer follows the body's measured content, so a window hugs short
/// content; when content needs more room than the window allows, the footer
/// stays at the bottom edge and the body scrolls. Both measurements are
/// remembered across frames and never change the window's requested size.
pub fn popup_footer(ui: &mut Ui, id_salt: impl Hash) -> PopupFooter {
    let id = ui.make_persistent_id(("popup_footer", id_salt));
    let (height, content) = ui
        .data(|data| data.get_temp::<(f32, f32)>(id))
        .unwrap_or((tokens::CONTROL_HEIGHT, f32::INFINITY));
    let available = ui.available_rect_before_wrap().round_ui();
    // The divider sits SPACE_MD below the body and SPACE_MD above the actions.
    let gap = 2.0 * tokens::SPACE_MD;
    let lowest = (available.bottom() - height).max(available.top());
    let top = (available.top() + content + gap).min(lowest);
    let rect = Rect::from_min_size(
        pos2(available.left(), top),
        egui::vec2(available.width(), height),
    )
    .round_ui();
    let clip = ui.clip_rect();
    // The body may use every point above the lowest footer position, so its
    // content can grow; only its measured height positions the footer.
    let body_bottom = (lowest - gap).max(available.top());
    let body_rect =
        Rect::from_min_max(available.min, pos2(available.right(), body_bottom)).round_ui();
    let mut body = ui.new_child(
        UiBuilder::new()
            .id_salt(id.with("body"))
            .max_rect(body_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    body.set_clip_rect(clip.intersect(body_rect));
    ui.expand_to_include_rect(rect);
    PopupFooter {
        id,
        rect,
        clip,
        body_top: available.top(),
        body,
    }
}

impl PopupFooter {
    pub fn body(&mut self) -> &mut Ui {
        &mut self.body
    }

    pub fn show<R>(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let mut footer = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id)
                .max_rect(self.rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        footer.set_clip_rect(self.clip.intersect(self.rect));
        let inner = contents(&mut footer);
        let height = footer.min_size().y.ceil().max(1.0);
        let content = (self.body.min_rect().bottom() - self.body_top)
            .ceil()
            .max(0.0);
        let previous = ui.data_mut(|data| {
            let previous = data.get_temp::<(f32, f32)>(self.id);
            data.insert_temp(self.id, (height, content));
            previous
        });
        if previous.is_none_or(|(previous, previous_content)| {
            (previous - height).abs() > 0.5 || (previous_content - content).abs() > 0.5
        }) {
            // A new, wrapped or resized layout gets the right allocation next
            // frame. Its temporary overflow is clipped, never fed into window size.
            ui.ctx().request_repaint();
        }
        // A crisp one-physical-pixel rule inside the window margin, so it never
        // meets the rounded window outline.
        let painter = ui.painter().with_clip_rect(self.clip);
        let y = painter.round_to_pixel_center(self.rect.top() - tokens::SPACE_MD);
        painter.hline(
            self.rect.x_range(),
            y,
            egui::Stroke::new(
                1.0 / ui.ctx().pixels_per_point(),
                theme::palette(ui.ctx()).line,
            ),
        );
        // Only the reserved bounds affect window size, including on the first
        // frame of a wrapped footer. Its contents never stretch the window.
        ui.expand_to_include_rect(self.rect);
        ui.set_min_width(self.body.min_size().x.max(footer.min_size().x));
        InnerResponse::new(inner, footer.response())
    }
}

/// The one footer convention for dialogs. Actions are supplied right to left:
/// the primary action first (it lands at the right edge), then Cancel/Done,
/// then any alternative. `status` fills the left edge with read-only text or
/// view-only toggles; settings that change the result belong in the body.
pub fn dialog_actions<A, S>(
    ui: &mut Ui,
    actions: impl FnOnce(&mut Ui) -> A,
    status: impl FnOnce(&mut Ui) -> S,
) -> InnerResponse<(A, S)> {
    crate::action_row(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
            actions(ui)
        },
        status,
    )
}

/// A text-only dialog action. Footer labels never gain label-derived icons;
/// use `IconButton::with_icon` where a footer action really wants one.
pub fn dialog_button_widget(label: impl Into<WidgetText>) -> crate::IconButton {
    // The tab constructor is the only icon-free one; the action role and an
    // always-visible frame turn it back into an ordinary command.
    crate::IconButton::tab(false, label)
        .action()
        .frame_when_inactive(true)
}

pub fn dialog_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(dialog_button_widget(label))
}

/// The single primary action of a dialog footer: solid action fill, text only.
pub fn dialog_primary_widget(ui: &Ui, label: impl Into<WidgetText>) -> crate::IconButton {
    let action = theme::palette(ui.ctx()).action;
    dialog_button_widget(label)
        .fill(action)
        .label_color(theme::contrast_text(action))
}

pub fn dialog_primary(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.add(dialog_primary_widget(ui, label))
}
