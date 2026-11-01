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
    // Compact rings already encode automation on their value arc. A generic
    // rectangular badge would put text back inside the text-free control.
    // Wider caption rows and separate value buttons retain their own markers.
    if crate::knob_geometry(ui.ctx(), response.id).is_some_and(|knob| {
        knob.divisions == 0
            && knob.number == egui::Rect::NOTHING
            && knob
                .dial
                .expand(1.0 / ui.ctx().pixels_per_point())
                .contains_rect(response.rect)
    }) {
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
    let marker = match state {
        AutomationState::Read => "A",
        AutomationState::Overridden => "!",
        AutomationState::Bypassed => "–",
    };
    let label = if let Some(scale) = crate::slider_scale_geometry(ui.ctx(), response.id) {
        // Put the marker beside the handle, never on its precise center line
        // or the endpoint captions below the rail. The opposite side is used
        // near the maximum so it remains visible throughout the travel.
        let vertical = scale.orientation == egui::SliderOrientation::Vertical;
        let size = if vertical {
            egui::vec2(10.0_f32.min(scale.rail.width()), 11.0)
        } else {
            egui::vec2(10.0, 11.0_f32.min(scale.rail.height()))
        };
        let mut center = scale.handle;
        if vertical {
            let reach = scale.handle_half_extent.y + size.y * 0.5 + 1.0;
            center.y += if center.y + reach + size.y * 0.5 <= scale.rail.bottom() {
                reach
            } else {
                -reach
            };
        } else {
            let reach = scale.handle_half_extent.x + size.x * 0.5 + 1.0;
            center.x += if center.x + reach + size.x * 0.5 <= scale.rail.right() {
                reach
            } else {
                -reach
            };
        }
        egui::Rect::from_center_size(center, size).intersect(scale.rail)
    } else {
        // Leave the center of a numeric field, including its sign, readable.
        egui::Rect::from_min_size(
            rect.right_top() + egui::vec2(-10.0, 0.0),
            egui::vec2(9.0, 9.0_f32.min(rect.height() * 0.4)),
        )
    };
    ui.painter()
        .rect_filled(label, 2.0, theme::palette(ui.ctx()).panel);
    ui.painter().text(
        label.center(),
        egui::Align2::CENTER_CENTER,
        marker,
        egui::FontId::proportional(label.height().min(10.0)),
        color,
    );
}
