//! Consistent automation state on curves and the controls they drive.
use crate::{text, theme, tokens};
use egui::{Color32, Response, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationState {
    Read,
    Overridden,
    Bypassed,
}

impl AutomationState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "Automated",
            Self::Overridden => "Overridden",
            Self::Bypassed => "Bypassed",
        }
    }

    pub fn color(self, ctx: &egui::Context) -> Color32 {
        let colors = theme::palette(ctx);
        match self {
            Self::Read => colors.automation,
            Self::Overridden => colors.automation_override,
            Self::Bypassed => colors.muted,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Read => "Following automation · adjusting this control overrides its curve",
            Self::Overridden => "Manual override · Resume automation to follow the saved curve",
            Self::Bypassed => "Automation bypassed · the manual control is active",
        }
    }

    pub fn rich_label(self, ctx: &egui::Context) -> egui::RichText {
        egui::RichText::new(self.label()).color(self.color(ctx))
    }
}

/// A text state marker, using both wording and color so override never depends
/// on hue alone. The underlying curve and project state belong to the caller.
pub fn automation_badge(ui: &mut Ui, state: AutomationState) -> Response {
    ui.label(
        text::TextRole::Note
            .rich(ui.ctx(), state.label())
            .color(state.color(ui.ctx())),
    )
    .on_hover_text(state.hint())
}

/// A small state marker on an existing control. It never adds an overlapping
/// hit target or changes the native control's layout, focus or accessible name.
pub fn paint_automation_control(ui: &Ui, response: &Response, state: AutomationState) {
    if !ui.is_rect_visible(response.rect) {
        return;
    }
    let rect = response.rect;
    let color = state.color(ui.ctx());
    ui.painter().rect_stroke(
        rect,
        tokens::RADIUS,
        egui::Stroke::new(1.5_f32, color),
        egui::StrokeKind::Inside,
    );
    // Slider endpoints and their scale must stay unobstructed, including the
    // short rails in track headers. Their outline carries the state; the
    // caller's status menu and tooltip provide its explicit wording.
    if crate::slider_scale_geometry(ui.ctx(), response.id).is_some() {
        return;
    }
    let marker = if state == AutomationState::Overridden {
        "!"
    } else {
        "A"
    };
    let label = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(1.0, 1.0),
        egui::vec2(12.0, 14.0),
    );
    ui.painter()
        .rect_filled(label, 2.0, theme::palette(ui.ctx()).panel);
    ui.painter().text(
        label.center(),
        egui::Align2::CENTER_CENTER,
        marker,
        egui::FontId::proportional(tokens::FONT_CANVAS),
        color,
    );
}
