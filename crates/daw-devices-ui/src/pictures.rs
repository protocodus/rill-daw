//! The pictures of devices that are not a frequency response: a shaper's
//! curve and harmonics, and a dynamics processor's curve, level history and
//! reduction meter. Drawn from the device's own functions and telemetry.
use crate::{DeviceHost, PanelState, WHEEL_NOTCHES, panels};
use daw_devices::{
    HISTORY_COLUMNS, HISTORY_RATE, ParamSpec, Telemetry,
    devices::{compressor, saturator},
};
use egui::Ui;

/// Frames per second of a picture that follows the signal.
const REFRESH: f32 = 30.0;

/// The device's telemetry while it plays; `None` once nothing has reached
/// it for a while, so that a stopped song leaves no level standing.
fn live(
    ui: &Ui,
    telemetry: Option<&std::sync::Arc<Telemetry>>,
    state: &mut PanelState,
) -> Option<std::sync::Arc<Telemetry>> {
    let telemetry = telemetry?;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f32(1.0 / REFRESH));
    let seconds = ui.input(|input| input.stable_dt).min(0.1);
    state
        .activity
        .live(telemetry.output_position(), seconds)
        .then(|| telemetry.clone())
}

/// R1 Saturator: the static curve with the span the signal sweeps on it,
/// and the harmonics the curve adds at that level. Both only show: Drive,
/// Model and Character stand directly below and reshape them.
pub(crate) fn saturation(ui: &mut Ui, height: f32, host: &dyn DeviceHost, state: &mut PanelState) {
    use saturator::*;
    const POINTS: usize = 129;
    let settings = panels::saturator_settings(&|id| host.value(id));
    let drive = 10f32.powf(settings.drive_db / 20.0);
    // The shaper's input peak, in units of the curve's knee.
    let level = live(ui, host.telemetry().as_ref(), state)
        .map(|telemetry| telemetry.value(TELEMETRY_LEVEL))
        .filter(|level| level.is_finite() && *level > 1e-4);
    let input: [f32; POINTS] =
        std::array::from_fn(|index| index as f32 / (POINTS - 1) as f32 * 2.0 - 1.0);
    let mut curve = [0.0; POINTS];
    transfer(&settings, &input, &mut curve);
    // The curve's shape, not its level: in true units it leaves the frame
    // without Auto gain and lies flat with it. The level is the readout's.
    let largest = curve.iter().fold(1e-9_f32, |max, y| max.max(y.abs()));
    for y in &mut curve {
        *y /= largest;
    }
    let span = level.map(|level| (level / drive).min(1.0));
    let mut harmonics = [0.0; 10];
    harmonics_db(
        &settings,
        // Without a signal, a sine at −18 dBFS RMS.
        level.unwrap_or(drive * 10f32.powf(-15.0 / 20.0)),
        &mut harmonics,
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_MD;
        let curve = daw_ui::transfer_plot(
            ui,
            &daw_ui::TransferPlot {
                curve: &curve,
                input: span,
                lit: span.map(|span| (-span, span)),
                size: egui::Vec2::splat(height),
                ..Default::default()
            },
        );
        let bars = daw_ui::harmonics_view(
            ui,
            &daw_ui::HarmonicsView {
                levels_db: &harmonics,
                floor_db: -80.0,
                size: egui::vec2(ui.available_width(), height),
            },
        );
        #[cfg(test)]
        {
            state.picture_rects = vec![curve.response.rect, bars.rect];
        }
        curve
            .response
            .on_hover_text("The model's curve at the set Drive; lit where the signal sweeps it");
        bars.on_hover_text("Harmonics the curve adds at the signal's level, against the first");
    });
}

/// Width of the reduction meter beside the history.
const METER_WIDTH: f32 = 48.0;
/// The level axis of the curve and the history: floor to 0 dB.
const FLOOR_DB: f32 = -60.0;
/// The reduction axis of the history and the meter.
const MAX_REDUCTION_DB: f32 = 24.0;

/// R1 Compressor: the static curve with its threshold as a handle, the
/// levels and the reduction over the last seconds, and the reduction meter.
/// The handle and the history's line set Threshold (Ceiling in Limiter
/// mode); the wheel and the up and down keys turn Ratio. The knobs below
/// are the same values.
pub(crate) fn dynamics(
    ui: &mut Ui,
    height: f32,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    use compressor::*;
    const POINTS: usize = 121;
    let settings = panels::compressor_settings(&|id| host.value(id));
    let effective = effective(&settings);
    let limiter = settings.mode == Mode::Limiter;
    let spec = |id: u32| DESCRIPTOR.parameter(id).expect("compressor parameter");
    // What the handle and the line stand for.
    let level = spec(if limiter { CEILING } else { THRESHOLD });
    let at = effective.curve.threshold_db;
    let input: [f32; POINTS] =
        std::array::from_fn(|index| FLOOR_DB - FLOOR_DB * index as f32 / (POINTS - 1) as f32);
    let mut curve = [0.0; POINTS];
    transfer_db(&settings, &input, &mut curve);
    let telemetry = host.telemetry();
    let playing = live(ui, telemetry.as_ref(), state);
    state.history.resize(HISTORY_COLUMNS, [0.0; 3]);
    match &telemetry {
        Some(telemetry) => telemetry.read_history(&mut state.history),
        None => state.history.fill([0.0; 3]),
    }
    let reduction = playing
        .as_ref()
        .map_or(0.0, |telemetry| telemetry.value(TELEMETRY_REDUCTION));
    state.reduction_hold = state.reduction_hold.max(reduction);
    let readout = format!(
        "{} {}",
        level.name,
        level.format(host.value(level.id) as f32)
    );
    let knee = effective.curve.knee_db;
    let mut edits: Vec<(u32, f64)> = Vec::new();
    let set = |spec: &ParamSpec, plain: f32| (spec.id, f64::from(spec.to_normalized(plain)));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_MD;
        let plot = daw_ui::transfer_plot(
            ui,
            &daw_ui::TransferPlot {
                curve: &curve,
                x_range: (FLOOR_DB, 0.0),
                y_range: (FLOOR_DB, 0.0),
                axis: daw_ui::TransferAxis::Decibels,
                input: playing
                    .as_ref()
                    .map(|telemetry| telemetry.value(TELEMETRY_LEVEL)),
                lit: None,
                shade: (knee > 0.0).then_some((at - knee * 0.5, at + knee * 0.5)),
                handle: Some(daw_ui::TransferHandle {
                    x: at,
                    y: effective.curve.output_db(at),
                    label: if limiter { "C" } else { "T" },
                    readout: &readout,
                }),
                size: egui::Vec2::splat(height),
            },
        );
        let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
        // Ratio does nothing in front of a limiter.
        let ratio = |steps: f32| {
            (!limiter).then(|| {
                let value = host.value(RATIO) + f64::from(steps) / WHEEL_NOTCHES;
                (RATIO, value.clamp(0.0, 1.0))
            })
        };
        if let Some((x, _)) = plot.dragged {
            edits.push(set(level, x));
        }
        if let Some((x, y)) = plot.nudged {
            if x != 0.0 {
                edits.push(set(level, at + x));
            }
            edits.extend(ratio(y).filter(|_| y != 0.0));
        }
        if let Some(notches) = plot.scrolled {
            edits.extend(ratio(notches * fine));
        }
        let default = (level.id, f64::from(level.default_normalized()));
        if plot.double_clicked {
            edits.push(default);
        }
        if let Some(handle) = &plot.handle {
            handle.context_menu(|ui| {
                if daw_ui::menu_item(ui, format!("Reset {}", level.name), None).clicked() {
                    edits.push(default);
                    ui.close();
                }
            });
        }
        let history = daw_ui::gain_reduction_history(
            ui,
            &daw_ui::LevelHistory {
                columns: &state.history,
                floor_db: FLOOR_DB,
                max_reduction_db: MAX_REDUCTION_DB,
                threshold_db: Some(at),
                threshold_label: level.name,
                draggable: true,
                span_seconds: HISTORY_COLUMNS as f32 / HISTORY_RATE,
                size: egui::vec2(
                    ui.available_width() - METER_WIDTH - daw_ui::tokens::SPACE_MD,
                    height,
                ),
            },
        );
        if let Some(db) = history.threshold {
            edits.push(set(level, db));
        }
        let meter = daw_ui::reduction_meter(
            ui,
            &daw_ui::ReductionMeter {
                reduction_db: reduction,
                hold_db: state.reduction_hold,
                max_reduction_db: MAX_REDUCTION_DB,
                size: egui::vec2(METER_WIDTH, height),
            },
        )
        .on_hover_text("Gain reduction and its largest value; click to reset");
        if meter.clicked() {
            state.reduction_hold = 0.0;
        }
        #[cfg(test)]
        {
            state.picture_rects = vec![plot.response.rect, history.response.rect, meter.rect];
        }
    });
    for (id, value) in edits {
        host.set_value(id, value);
    }
}
