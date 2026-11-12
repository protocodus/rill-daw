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

/// A restrained automation outline and halo. This is paint only: it
/// never changes a control's bounds, hit target, focus, or parameter value.
/// Ownership stays visible while the curve is overridden or bypassed; red
/// identifies manual takeover while orange identifies the saved automation. The
/// separate state marker still explains what is currently driving the value.
pub fn paint_automation_glow(ui: &Ui, rect: egui::Rect, state: AutomationState) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let density = ui.painter().pixels_per_point();
    let rect = egui::emath::GuiRounding::round_to_pixels(rect, density);
    let colors = theme::palette(ui.ctx());
    let color = if state == AutomationState::Overridden {
        colors.automation_override
    } else {
        colors.automation
    };
    for (spread, opacity) in [(2.0, 0.08), (1.0, 0.15), (0.0, 1.0)] {
        ui.painter().rect_stroke(
            rect.expand(spread),
            tokens::RADIUS.saturating_add(spread as u8),
            egui::Stroke::new(
                if spread == 0.0 { 1.0 } else { 1.0 / density },
                color.gamma_multiply(opacity),
            ),
            egui::StrokeKind::Inside,
        );
    }
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
        // The knob itself paints the shared ownership glow around its dial;
        // the corner dot keeps its state readable like every other control.
        paint_automation_indicator(ui, response.rect, Some(state));
        return;
    }
    let rect = response.rect;
    register_automation_indicator(ui.ctx(), Some(state), rect);
    let color = state.color(ui.ctx());
    paint_automation_glow(ui, rect, state);
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

/// Corner dot every automatable control carries: a faint ring while the
/// control has no lane, then the state color. Paint only; it adds no hit
/// target and never changes layout.
pub fn paint_automation_indicator(ui: &Ui, rect: egui::Rect, state: Option<AutomationState>) {
    register_automation_indicator(ui.ctx(), state, rect);
    let quiet = state.is_none()
        && ui.data(|data| {
            data.get_temp::<bool>(ui.id().with(QUIET_INDICATORS))
                .is_some()
        });
    if quiet || !ui.is_rect_visible(rect) {
        return;
    }
    let radius = 2.5;
    let center = rect.right_top() + egui::vec2(-radius - 1.0, radius + 1.0);
    match state {
        Some(state) => ui
            .painter()
            .circle_filled(center, radius, state.color(ui.ctx())),
        None => ui.painter().circle_stroke(
            center,
            radius,
            egui::Stroke::new(1.0_f32, theme::palette(ui.ctx()).muted.gamma_multiply(0.6)),
        ),
    };
}

const QUIET_INDICATORS: &str = "rill_quiet_automation_indicators";

/// Controls drawn directly in `ui` leave out the faint ring that marks a
/// control without a lane; a lane's own marker still shows. For the cells
/// of a dense table, where a ring on every value reads as punctuation.
/// **Automate** stays in each control's context menu.
pub fn quiet_automation_indicators(ui: &Ui) {
    ui.data_mut(|data| data.insert_temp(ui.id().with(QUIET_INDICATORS), true));
}

#[derive(Clone, Default)]
struct AutomationIndicators {
    pass: u64,
    indicators: Vec<(Option<AutomationState>, egui::Rect)>,
}

const AUTOMATION_INDICATORS: &str = "rill_automation_indicators";

fn register_automation_indicator(
    ctx: &egui::Context,
    state: Option<AutomationState>,
    rect: egui::Rect,
) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let entry = data
            .get_temp_mut_or_default::<AutomationIndicators>(egui::Id::new(AUTOMATION_INDICATORS));
        if entry.pass != pass {
            entry.pass = pass;
            entry.indicators.clear();
        }
        entry.indicators.push((state, rect));
    });
}

/// Automation indicators of the latest pass, with the control each marks, for
/// host diagnostics. `None` is an automatable control without a lane.
pub fn automation_indicators(ctx: &egui::Context) -> Vec<(Option<AutomationState>, egui::Rect)> {
    ctx.data(|data| {
        data.get_temp::<AutomationIndicators>(egui::Id::new(AUTOMATION_INDICATORS))
            .map(|entry| entry.indicators)
            .unwrap_or_default()
    })
}

pub fn reset_automation_indicators(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<AutomationIndicators>(egui::Id::new(AUTOMATION_INDICATORS)));
}

/// Global warning shown while at least one control holds a manual value
/// instead of its curve. Clicking it is the caller's "resume everything".
pub fn automation_override_warning(ui: &mut Ui, count: usize) -> Response {
    let text = if count == 1 {
        "1 automation overridden".to_owned()
    } else {
        format!("{count} automations overridden")
    };
    let response = ui.add(
        crate::IconButton::with_icon(crate::Icon::Warning, text)
            .small()
            .label_color(theme::palette(ui.ctx()).automation_override),
    );
    crate::hover_text(
        response,
        "Some controls ignore their automation. Click to resume automation everywhere; Undo restores the overrides.",
    )
}
