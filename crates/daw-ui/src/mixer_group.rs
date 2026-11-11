//! Shared presentation for the independent outputs of one instrument.
use egui::{Color32, FontId, Rect, Response, Sense, Ui, Vec2};

/// A folded output keeps its identity, switches and calibrated stereo meter.
/// Folding is view state; the caller retains every channel in the signal graph.
pub const MIXER_COLLAPSED_CHANNEL_WIDTH: f32 = 64.0;

/// The title across a contiguous instrument output group. The action folds or
/// opens auxiliary strips together; its main output remains expanded.
pub fn mixer_output_group_header(
    ui: &mut Ui,
    rect: Rect,
    name: &str,
    outputs: usize,
    expanded: bool,
) -> Response {
    let label = format!("{name} · {outputs} outputs");
    let mut response = ui.interact(
        rect,
        ui.make_persistent_id(("output_group", name)),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let colors = crate::theme::palette(ui.ctx());
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect, crate::tokens::RADIUS, colors.control_hover);
        }
        let icon_rect = Rect::from_min_size(rect.min, Vec2::splat(rect.height()));
        if expanded {
            crate::Icon::ChevronDown
        } else {
            crate::Icon::ChevronRight
        }
        .paint(ui.painter(), icon_rect.shrink(3.0), colors.text);
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.clone(),
            FontId::proportional(crate::tokens::FONT_CAPTION),
            colors.text,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(
            (rect.width() - icon_rect.width()).max(0.0),
        );
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(
            egui::pos2(icon_rect.right(), rect.center().y - galley.size().y * 0.5),
            galley,
            colors.text,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!(
                "{} outputs · {name}",
                if expanded { "Collapse" } else { "Expand" }
            ),
        )
    });
    response = crate::hover_text(
        response,
        format!(
            "{label}\n{} auxiliary channels · Main output stays open",
            if expanded { "Collapse" } else { "Expand" }
        ),
    );
    response
}

/// The same explicit disclosure is used on expanded and folded output strips.
pub fn mixer_output_disclosure(ui: &mut Ui, name: &str, expanded: bool, width: f32) -> Response {
    let label = format!(
        "{} output · {name}",
        if expanded { "Collapse" } else { "Expand" }
    );
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::ZERO;
            ui.add_sized(
                Vec2::new(width, crate::mixer_control_height(ui)),
                crate::IconButton::icon_only(
                    if expanded {
                        crate::Icon::ChevronDown
                    } else {
                        crate::Icon::ChevronRight
                    },
                    &label,
                )
                .min_size(Vec2::new(width, crate::mixer_control_height(ui)))
                .quiet(),
            )
        })
        .inner;
    crate::hover_text(response, label)
}

/// A compact output identity uses the channel color and retains its full name
/// in the tooltip and accessibility tree when the bounded caption truncates.
pub fn mixer_folded_output_name(ui: &mut Ui, name: &str, width: f32, color: Color32) -> Response {
    crate::hover_text(
        crate::mixer_name_plate(ui, crate::MixerNamePlateKind::Track, name, width, color),
        format!("Open channel · {name}"),
    )
}

/// A folded route remains a clear mouse action without a clipped destination
/// fragment; its complete destination is in the accessible name and tooltip.
pub fn mixer_folded_output_route(ui: &mut Ui, width: f32, destination: &str) -> Response {
    let label = format!("Output → {destination}");
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::ZERO;
            ui.add_sized(
                Vec2::new(width, crate::mixer_control_height(ui)),
                crate::IconButton::icon_only(crate::Icon::Routing, &label)
                    .min_size(Vec2::new(width, crate::mixer_control_height(ui)))
                    .quiet(),
            )
        })
        .inner;
    crate::hover_text(response, format!("{label}\nChoose output destination"))
}

/// Polarity switch, then the pre-insert channel gain. The exact entry popup
/// shares the ordinary dB controls and never rounds a stored value merely by
/// displaying it.
pub fn mixer_input_gain(
    ui: &mut Ui,
    gain: &mut f32,
    invert: &mut bool,
    width: f32,
    name: &str,
) -> MixerInputResponse {
    let height = crate::mixer_control_height(ui);
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, height));
            ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
            let polarity = crate::mixer_polarity_switch(ui, invert);
            let available = ui.available_width();
            let gain = ui
                .scope(|ui| {
                    ui.set_min_width(available);
                    ui.set_max_width(available);
                    ui.spacing_mut().interact_size.y = height;
                    let response =
                        crate::gain_value_button_named(ui, "input_gain", gain, 16.0, "", name);
                    crate::hover_text(
                        response,
                        "Input gain · before inserts · Right-click for automation",
                    )
                })
                .inner;
            MixerInputResponse { gain, polarity }
        },
    )
    .inner
}

pub struct MixerInputResponse {
    pub gain: Response,
    pub polarity: Response,
}
