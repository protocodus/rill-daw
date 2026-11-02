//! A dialog's actions own the bottom edge; its body owns the remaining viewport.
use crate::{theme, tokens};
use egui::emath::GuiRounding as _;
use egui::{Id, InnerResponse, Rect, Ui, UiBuilder, pos2};
use std::hash::Hash;

pub struct PopupFooter {
    id: Id,
    rect: Rect,
    clip: Rect,
    body: Ui,
}

/// Reserve first, draw through `PopupFooter::body`, then call `show` on the
/// original window Ui. The body never includes the footer's input region.
/// The measured action height includes wrapping and optional status rows.
/// Remembering it never changes the window's requested size.
pub fn popup_footer(ui: &mut Ui, id_salt: impl Hash) -> PopupFooter {
    let id = ui.make_persistent_id(("popup_footer", id_salt));
    let height = ui
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(tokens::CONTROL_HEIGHT);
    let available = ui.available_rect_before_wrap().round_ui();
    let top = (available.bottom() - height).max(available.top());
    let rect = Rect::from_min_size(
        pos2(available.left(), top),
        egui::vec2(available.width(), height),
    )
    .round_ui();
    let clip = ui.clip_rect();
    let body_bottom = (top - tokens::SPACE_LG).max(available.top());
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
        let previous = ui.data_mut(|data| {
            let previous = data.get_temp::<f32>(self.id);
            data.insert_temp(self.id, height);
            previous
        });
        if previous.is_none_or(|previous| (previous - height).abs() > 0.5) {
            // A new or wrapped footer gets the right allocation next frame.
            // Its temporary overflow is clipped, never fed into window size.
            ui.ctx().request_repaint();
        }
        ui.painter().with_clip_rect(self.clip).hline(
            self.rect.x_range(),
            self.rect.top() - tokens::SPACE_MD,
            egui::Stroke::new(tokens::BORDER_WIDTH, theme::palette(ui.ctx()).line),
        );
        // Only the reserved bounds affect window size, including on the first
        // frame of a wrapped footer. Its contents never stretch the window.
        ui.expand_to_include_rect(self.rect);
        ui.set_min_width(self.body.min_size().x.max(footer.min_size().x));
        InnerResponse::new(inner, footer.response())
    }
}
