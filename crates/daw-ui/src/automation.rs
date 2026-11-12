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

/// What drives a control drawn inside [`automated`]: its lane's state and,
/// while a hand holds the control elsewhere, where the lane is. `lane` is in
/// the control's own terms: a knob's or fader's value, the position of a
/// choice's option in the list it was given, 0 or 1 for a lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationMark {
    pub state: AutomationState,
    pub lane: Option<f64>,
}

impl AutomationMark {
    /// The lane's position to mark, only while a hand overrides it.
    pub fn ghost(self) -> Option<f64> {
        (self.state == AutomationState::Overridden)
            .then_some(self.lane)
            .flatten()
    }
}

const MARK: &str = "rill_automation_mark";

/// Draws `add` with `mark` on every control in it that shows its value in
/// color: a knob's arc, a fader's fill, a chosen option, a lit lamp. Orange
/// while the lane drives the control; red while a hand overrides it, with an
/// orange ghost where the lane is; neutral while the lane is bypassed.
/// Widgets the library does not paint itself (a drop-down) take the color
/// on their frame. Give each automatable control a scope of its own.
pub fn automated<R>(
    ui: &mut Ui,
    mark: Option<AutomationMark>,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let Some(mark) = mark else {
        return add(ui);
    };
    ui.scope(|ui| {
        let color = mark.state.color(ui.ctx());
        if mark.state != AutomationState::Bypassed {
            let visuals = &mut ui.visuals_mut().widgets;
            for widget in [
                &mut visuals.inactive,
                &mut visuals.hovered,
                &mut visuals.active,
            ] {
                widget.bg_stroke = egui::Stroke::new(tokens::BORDER_WIDTH, color);
                widget.weak_bg_fill = blend(widget.weak_bg_fill, color, 0.18);
                widget.bg_fill = blend(widget.bg_fill, color, 0.18);
            }
        }
        let id = ui.unique_id().with(MARK);
        ui.data_mut(|data| data.insert_temp(id, mark));
        let inner = add(ui);
        // Only what is drawn inside carries the mark.
        ui.data_mut(|data| data.remove::<AutomationMark>(id));
        inner
    })
    .inner
}

/// The mark of the innermost [`automated`] scope `ui` draws in, if any.
pub fn automation_mark(ui: &Ui) -> Option<AutomationMark> {
    ui.data(|data| {
        ui.stack()
            .iter()
            .find_map(|frame| data.get_temp::<AutomationMark>(frame.id.with(MARK)))
    })
}

/// `base` tinted toward `tint` by `amount` (0–1), opaque.
pub(crate) fn blend(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Color32::from_rgb(
        mix(base.r(), tint.r()),
        mix(base.g(), tint.g()),
        mix(base.b(), tint.b()),
    )
}

/// A dashed outline in the lane's color: where an overridden lane is.
pub(crate) fn paint_ghost_frame(ui: &Ui, rect: egui::Rect) {
    let color = theme::palette(ui.ctx()).automation;
    let rect = rect.shrink(0.5);
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        egui::Stroke::new(1.0_f32, color),
        3.0,
        2.0,
    ));
}

/// Tells [`paint_automation_control`] that the control drawn under `id`
/// already shows its automation itself.
pub(crate) fn register_self_marked(ctx: &egui::Context, id: egui::Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(id.with(SELF_MARKED), pass));
}

const SELF_MARKED: &str = "rill_automation_self_marked";

fn self_marked(ctx: &egui::Context, id: egui::Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(id.with(SELF_MARKED))) == Some(pass)
}

/// The state marker of an existing control. A control that colors its own
/// value (see [`automated`]) needs nothing more; any other gets an outline
/// in the state color and a glyph beside its handle. Paint only: it never
/// adds a hit target or changes layout, focus or the accessible name.
pub fn paint_automation_control(ui: &Ui, response: &Response, state: AutomationState) {
    let rect = response.rect;
    register_automation_indicator(ui.ctx(), Some(state), rect);
    if self_marked(ui.ctx(), response.id)
        || automation_mark(ui).is_some()
        || !ui.is_rect_visible(rect)
    {
        return;
    }
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

/// Records an automatable control for host diagnostics. Nothing is painted:
/// a control without a lane looks like any other, and one with a lane shows
/// it in its own value's color.
pub fn paint_automation_indicator(ui: &Ui, rect: egui::Rect, state: Option<AutomationState>) {
    register_automation_indicator(ui.ctx(), state, rect);
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
