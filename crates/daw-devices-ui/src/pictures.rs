//! The pictures of devices that are not a plain frequency response: a
//! shaper's curve and harmonics, a dynamics processor's curve, level history
//! and reduction meter, a modulation effect's LFO and swept response, and
//! the echoes and decay of a delay and reverb. Drawn from the device's own
//! functions and telemetry.
use crate::{DeviceHost, PanelState, SampleRequest, WHEEL_NOTCHES, panels};
use daw_devices::{
    DeviceDescriptor, HISTORY_COLUMNS, HISTORY_RATE, ParamSpec, Telemetry,
    devices::{compressor, drums, modulator, sampler, saturator, spacer},
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

/// The parameter's value in words, as its knob prints it.
fn text(spec: &ParamSpec, host: &dyn DeviceHost) -> String {
    spec.format(host.value(spec.id) as f32)
}

/// A continuous value moved by `steps` wheel notches, or a choice by one
/// option in their direction.
fn stepped(spec: &ParamSpec, host: &dyn DeviceHost, steps: f32) -> (u32, f64) {
    let value = host.value(spec.id);
    let next = match spec.steps() {
        Some(count) => f64::from(
            spec.to_normalized(
                (spec.to_plain(value as f32).round() + steps.signum())
                    .clamp(0.0, count.saturating_sub(1) as f32),
            ),
        ),
        None => (value + f64::from(steps) / WHEEL_NOTCHES).clamp(0.0, 1.0),
    };
    (spec.id, next)
}

fn reset(spec: &ParamSpec) -> (u32, f64) {
    (spec.id, f64::from(spec.default_normalized()))
}

/// Points of an LFO cycle.
const CYCLE: usize = 129;
/// Notches of drag across the LFO for one step of a synced rate.
const DIVISION_DRAG: f32 = 4.0;

/// R1 Modulator: the LFO, one trace per voice with the two channels'
/// positions on it, beside the response it sweeps. Dragging across the LFO
/// turns Rate (or steps Division); the handle on the response is the
/// mode's Center and the wheel over it Depth. The knobs below are the same
/// values.
pub(crate) fn modulation(
    ui: &mut Ui,
    height: f32,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    use modulator::*;
    let settings = panels::modulator_settings(&|id| host.value(id));
    let kernel = settings.kernel();
    let spec = |id: u32| DESCRIPTOR.parameter(id).expect("modulator parameter");
    let playing = live(ui, host.telemetry().as_ref(), state);
    let rate = playing
        .as_ref()
        .map(|telemetry| telemetry.value(TELEMETRY_SAMPLE_RATE))
        .filter(|rate| *rate >= 8000.0)
        .unwrap_or(panels::RESPONSE_RATE);
    let voices = if settings.mode == Mode::Chorus {
        settings.voices.clamp(1, 4)
    } else {
        1
    };
    let mut cycle = [0.0; CYCLE];
    lfo_cycle(settings.shape, &mut cycle);
    // The shape at a phase in cycles, as the picture draws it.
    let shape_at = |phase: f64| {
        let position = phase.rem_euclid(1.0) * (CYCLE - 1) as f64;
        let low = position.floor() as usize;
        let high = (low + 1).min(CYCLE - 1);
        cycle[low] + (cycle[high] - cycle[low]) * position.fract() as f32
    };
    let traces: Vec<[f32; CYCLE]> = (0..voices)
        .map(|voice| {
            std::array::from_fn(|index| {
                let phase = index as f64 / (CYCLE - 1) as f64 + voice as f64 / voices as f64;
                settings.depth * shape_at(phase)
            })
        })
        .collect();
    let offset = daw_devices::dsp::modulation::channel_offset(&kernel);
    let phase = playing
        .as_ref()
        .map(|telemetry| f64::from(telemetry.value(TELEMETRY_PHASE)));
    // The LFO value of every voice of both channels, as the device reports
    // them: a random shape is nowhere else.
    let positions = playing.as_ref().map(|telemetry| {
        [TELEMETRY_SWEEP_LEFT, TELEMETRY_SWEEP_RIGHT].map(|slot| {
            let mut lfo = [0.0; 4];
            for (voice, value) in lfo.iter_mut().enumerate().take(voices) {
                *value = telemetry.value(slot + voice).clamp(-1.0, 1.0);
            }
            lfo
        })
    });
    // Where an LFO value puts what the handle and the markers stand for:
    // the stages' corner, or the first tooth of the comb.
    let place = |lfo: f32| {
        let swept = daw_devices::dsp::modulation::swept(&kernel, lfo, rate);
        match settings.mode {
            Mode::Phaser => swept,
            Mode::Chorus | Mode::Flanger => 2.0 * rate / swept.max(1.0),
        }
    };
    let center = spec(match settings.mode {
        Mode::Chorus => CENTER_CHORUS,
        Mode::Flanger => CENTER_FLANGER,
        Mode::Phaser => CENTER_PHASER,
    });
    let speed = spec(if settings.sync { DIVISION } else { RATE });
    let mut edits: Vec<(u32, f64)> = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_MD;
        let lfo_traces: Vec<daw_ui::LfoTrace> = traces
            .iter()
            .enumerate()
            .map(|(voice, samples)| daw_ui::LfoTrace {
                samples,
                emphasis: voice == 0,
            })
            .collect();
        let markers = phase.map(|phase| {
            [
                daw_ui::LfoMarker {
                    phase: phase as f32,
                    label: "L",
                },
                daw_ui::LfoMarker {
                    phase: (phase + offset) as f32,
                    label: "R",
                },
            ]
        });
        let lfo = daw_ui::lfo_view(
            ui,
            &daw_ui::LfoView {
                traces: &lfo_traces,
                markers: markers.as_ref().map_or(&[], |markers| &markers[..]),
                draggable: true,
                size: egui::Vec2::splat(height),
            },
        );
        if let Some(notches) = lfo.dragged {
            if settings.sync {
                // Shorter divisions to the right, as a faster rate is.
                state.rate_drag += notches;
                if state.rate_drag.abs() >= DIVISION_DRAG {
                    edits.push(stepped(speed, host, state.rate_drag));
                    state.rate_drag = 0.0;
                }
            } else {
                edits.push(stepped(speed, host, notches));
            }
        } else if !lfo.response.dragged() {
            state.rate_drag = 0.0;
        }
        if lfo.double_clicked {
            edits.push(reset(speed));
        }
        // A point every two points of width: the band between the least
        // and the largest gain there holds what a finer grid would show.
        let size = egui::vec2(ui.available_width(), height);
        let points = ((size.x / 2.0) as usize).max(2);
        let frequencies: Vec<f32> = (0..points)
            .map(|index| daw_ui::log_grid_frequency(index, points))
            .collect();
        let band = |lfo: [f32; 4]| {
            let (mut low, mut high) = (vec![0.0; points], vec![0.0; points]);
            response_band_db(&settings, lfo, rate, &frequencies, &mut low, &mut high);
            (low, high)
        };
        let (left, right) = match positions {
            Some([left, right]) => (band(left), Some(band(right))),
            None => (band([0.0; 4]), None),
        };
        let mut curves = Vec::new();
        if let Some((_, high)) = &right {
            curves.push(daw_ui::FrequencyCurve {
                db: high,
                emphasis: false,
                low: None,
            });
        }
        curves.push(daw_ui::FrequencyCurve {
            db: &left.1,
            emphasis: true,
            low: Some(&left.0),
        });
        let (from, to) = (place(-1.0), place(1.0));
        let readout = format!(
            "Center {} · Depth {}",
            text(center, host),
            text(spec(DEPTH), host)
        );
        let node = daw_ui::FrequencyNode {
            gain_editable: false,
            label: Some("C"),
            readout: Some(&readout),
            ..daw_ui::FrequencyNode::new(place(0.0), 0.0)
        };
        let live_markers = positions.map(|[left, right]| {
            [
                daw_ui::FrequencyMarker {
                    frequency: place(left[0]),
                    label: "L",
                },
                daw_ui::FrequencyMarker {
                    frequency: place(right[0]),
                    label: "R",
                },
            ]
        });
        let plot = daw_ui::frequency_plot(
            ui,
            &daw_ui::FrequencyPlot {
                curves: &curves,
                spectrum: None,
                nodes: &[node],
                range_db: 24.0,
                limit_db: 24.0,
                spans: &[(from.min(to), from.max(to))],
                markers: live_markers.as_ref().map_or(&[], |markers| &markers[..]),
                size,
            },
        );
        if let Some(drag) = plot.drags.first() {
            // A comb's first tooth is one over its delay.
            let plain = match settings.mode {
                Mode::Phaser => drag.frequency,
                Mode::Chorus | Mode::Flanger => 1000.0 / drag.frequency,
            };
            edits.push((center.id, f64::from(center.to_normalized(plain))));
        }
        if let Some((_, notches)) = plot.scrolled {
            let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
            edits.push(stepped(spec(DEPTH), host, notches * fine));
        }
        if plot.double_clicked.is_some() {
            edits.push(reset(center));
        }
        if let Some(handle) = plot.nodes.first() {
            handle.context_menu(|ui| {
                if daw_ui::menu_item(ui, "Reset Center", None).clicked() {
                    edits.push(reset(center));
                    ui.close();
                }
            });
        }
        #[cfg(test)]
        {
            state.picture_rects = vec![lfo.response.rect, plot.response.rect];
        }
        lfo.response.on_hover_text(if settings.sync {
            "One cycle of the LFO at the set Depth; drag sideways to step Division"
        } else {
            "One cycle of the LFO at the set Depth; drag sideways to turn Rate"
        });
    });
    for (id, value) in edits {
        host.set_value(id, value);
    }
}

/// The level of the picture's centre line.
const SPACE_FLOOR_DB: f32 = -60.0;
/// The time spans the picture steps through, in seconds.
const SPANS: [f32; 7] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// The tempo the picture assumes while the device does not report one.
const DEFAULT_TEMPO: f64 = 120.0;

/// The Feedback that gives a loop gain: the inverse of `echo::loop_gain`,
/// which rises with Feedback.
fn feedback_for(model: spacer::EchoModel, gain: f32) -> f32 {
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..16 {
        let middle = 0.5 * (low + high);
        if daw_devices::dsp::echo::loop_gain(model, middle) < gain {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

/// R1 Spacer: every head of the echoes, blurred as Diffusion smears them,
/// the reverb's first reflections and its decay at 1 kHz with the faster
/// one at 8 kHz inside it, over time on a decibel axis, left above the line
/// and right below. Handles: T on the first echo
/// (Time or Division, and Echo level), F on the second (Feedback), P where
/// the reverb starts (Pre-delay and Reverb level), D where it has decayed
/// (Decay). The knobs below are the same values. Fills the space it is given.
pub(crate) fn space(ui: &mut Ui, host: &mut dyn DeviceHost, state: &mut PanelState) {
    use spacer::*;
    let settings = panels::spacer_settings(&|id| host.value(id));
    let spec = |id: u32| DESCRIPTOR.parameter(id).expect("spacer parameter");
    let plain = |id: u32| spec(id).to_plain(host.value(id) as f32);
    let telemetry = host.telemetry();
    let playing = live(ui, telemetry.as_ref(), state);
    let value = |slot: usize| playing.as_ref().map(|telemetry| telemetry.value(slot));
    // Tempo and sample rate are settings, not levels: they hold while
    // nothing plays, and the song's tempo before the device has run.
    let last = |slot: usize| telemetry.as_ref().map(|telemetry| telemetry.value(slot));
    let tempo = host
        .tempo()
        .or(last(TELEMETRY_TEMPO).map(f64::from))
        .filter(|tempo| *tempo >= 1.0)
        .unwrap_or(DEFAULT_TEMPO);
    let rate = last(TELEMETRY_SAMPLE_RATE)
        .filter(|rate| *rate >= 8000.0)
        .unwrap_or(panels::RESPONSE_RATE);
    let time = echo_seconds(&settings, tempo);
    let (echo_db, reverb_db) = (plain(ECHO_LEVEL), plain(REVERB_LEVEL));
    let pre_delay = settings.reverb.pre_delay_seconds;
    let decay = decay_seconds_at(&settings, 1000.0, rate);
    // What Damping and Color leave of the top.
    let decay_high = decay_seconds_at(&settings, 8000.0, rate);
    // Where the tail has fallen to the picture's floor.
    let tail_end = pre_delay + decay * (reverb_db - SPACE_FLOOR_DB).max(0.0) / 60.0;

    let mut echoes = [(0.0, 0.0, 0.0); 32];
    let count = echo_taps(&settings, tempo, &mut echoes);
    let echoes = &echoes[..count];
    let smear = daw_devices::dsp::echo::smear_seconds(&settings.echo(tempo));
    let mut taps: Vec<daw_ui::DelayTap> = echoes
        .iter()
        .map(|(seconds, left, right)| daw_ui::DelayTap {
            seconds: *seconds,
            left: *left,
            right: *right,
            thin: false,
            faint: !settings.echo_on,
            // Every pass through the loop smears a repeat further.
            blur_seconds: smear * (seconds / time.max(1e-3) - 1e-3).ceil().max(1.0),
        })
        .collect();
    let mut early = [[(0.0, 0.0); daw_devices::dsp::reverb::EARLY_TAPS]; 2];
    early_taps(&settings, &mut early);
    for (channel, reflections) in early.iter().enumerate() {
        taps.extend(reflections.iter().map(|(seconds, gain)| {
            let gain = gain * settings.reverb_level;
            daw_ui::DelayTap {
                seconds: pre_delay + seconds,
                left: if channel == 0 { gain } else { 0.0 },
                right: if channel == 1 { gain } else { 0.0 },
                thin: true,
                faint: !settings.reverb_on,
                blur_seconds: 0.0,
            }
        }));
    }
    let mut tails = vec![daw_ui::DelayTail {
        start_seconds: pre_delay,
        start_db: reverb_db,
        decay_seconds: decay,
        inner_decay_seconds: Some(decay_high),
        faint: !settings.reverb_on,
    }];
    if settings.series && settings.echo_on && settings.reverb_on {
        // What the reverb makes of the first echoes.
        tails.extend(
            echoes
                .iter()
                .take(6)
                .map(|(seconds, left, right)| daw_ui::DelayTail {
                    start_seconds: seconds + pre_delay,
                    start_db: reverb_db + 20.0 * left.max(*right).max(1e-6).log10(),
                    decay_seconds: decay,
                    inner_decay_seconds: None,
                    faint: true,
                }),
        );
    }
    let span = state.held_span.unwrap_or_else(|| {
        let needed = (6.0 * time).max(pre_delay + decay) * 1.1;
        SPANS
            .into_iter()
            .find(|span| *span >= needed)
            .unwrap_or(SPANS[SPANS.len() - 1])
    });
    // Quarter notes with the bars numbered while the echo follows the
    // tempo; seconds otherwise.
    let mut grid: Vec<(f32, String)> = Vec::new();
    if settings.echo_sync {
        let beat = (60.0 / tempo) as f32;
        // Bars only, once beats would stand closer than a few points.
        let step = if span / beat > 48.0 { 4 } else { 1 };
        let mut index = step;
        while index as f32 * beat < span {
            let bar = index % 4 == 0;
            grid.push((
                index as f32 * beat,
                if bar {
                    format!("{}", index / 4 + 1)
                } else {
                    String::new()
                },
            ));
            index += step;
        }
    } else {
        for line in 1..4 {
            let seconds = span * line as f32 / 4.0;
            grid.push((
                seconds,
                if seconds < 1.0 {
                    format!("{:.0} ms", seconds * 1000.0)
                } else {
                    format!("{seconds:.0} s")
                },
            ));
        }
    }
    let grid: Vec<(f32, &str)> = grid
        .iter()
        .map(|(seconds, name)| (*seconds, name.as_str()))
        .collect();

    // The handles and the parameters each moves along time and level.
    let speed = spec(if settings.echo_sync {
        ECHO_DIVISION
    } else {
        ECHO_TIME
    });
    let gain = daw_devices::dsp::echo::loop_gain(settings.echo.model, settings.echo.feedback);
    let readouts = [
        format!(
            "{} {} · Level {}",
            if settings.echo_sync {
                "Division"
            } else {
                "Time"
            },
            text(speed, host),
            text(spec(ECHO_LEVEL), host)
        ),
        format!("Feedback {}", text(spec(ECHO_FEEDBACK), host)),
        format!(
            "Pre-delay {} · Level {}",
            text(spec(PRE_DELAY), host),
            text(spec(REVERB_LEVEL), host)
        ),
        format!("Decay {}", text(spec(DECAY), host)),
    ];
    let handle = |seconds: f32, db: f32, below: bool, label: &'static str, moves: (bool, bool)| {
        daw_ui::DelayHandle {
            seconds,
            db,
            below,
            label,
            readout: "",
            horizontal: moves.0,
            vertical: moves.1,
        }
    };
    let mut handles: Vec<(daw_ui::DelayHandle, [Option<&ParamSpec>; 2])> = Vec::new();
    if settings.echo_on {
        handles.push((
            daw_ui::DelayHandle {
                readout: &readouts[0],
                ..handle(time, echo_db, false, "T", (true, true))
            },
            [Some(speed), Some(spec(ECHO_LEVEL))],
        ));
        handles.push((
            daw_ui::DelayHandle {
                readout: &readouts[1],
                ..handle(
                    2.0 * time,
                    echo_db + 20.0 * gain.max(1e-6).log10(),
                    settings.echo.stereo == EchoStereo::PingPong,
                    "F",
                    (false, true),
                )
            },
            [None, Some(spec(ECHO_FEEDBACK))],
        ));
    }
    if settings.reverb_on {
        handles.push((
            daw_ui::DelayHandle {
                readout: &readouts[2],
                ..handle(pre_delay, reverb_db, false, "P", (true, true))
            },
            [Some(spec(PRE_DELAY)), Some(spec(REVERB_LEVEL))],
        ));
        // A silent reverb has no slope to hold.
        if reverb_db > SPACE_FLOOR_DB + 1.0 {
            handles.push((
                daw_ui::DelayHandle {
                    readout: &readouts[3],
                    ..handle(tail_end, SPACE_FLOOR_DB, false, "D", (true, false))
                },
                [Some(spec(DECAY)), None],
            ));
        }
    }
    let drawn: Vec<daw_ui::DelayHandle> = handles.iter().map(|(handle, _)| *handle).collect();
    // What each section is turned down by now, on the scale of a full duck.
    let envelope = value(TELEMETRY_DUCK).unwrap_or(0.0);
    let reductions: Vec<(f32, f32, &str)> = [
        (settings.echo_on, settings.echo_duck, "E"),
        (settings.reverb_on, settings.reverb_duck, "R"),
    ]
    .into_iter()
    .filter(|(on, amount, _)| *on && *amount > 0.0)
    .map(|(_, amount, name)| (envelope * amount, DUCK_RANGE_DB, name))
    .collect();
    let output = daw_ui::delay_view(
        ui,
        &daw_ui::DelayView {
            taps: &taps,
            tails: &tails,
            grid: &grid,
            span_seconds: span,
            floor_db: SPACE_FLOOR_DB,
            handles: &drawn,
            reductions: &reductions,
            size: ui.available_size(),
        },
    );
    state.held_span = output.dragging.then_some(span);

    // The parameter value that puts a handle at a time or a level.
    let normalized = |spec: &ParamSpec, plain: f32| f64::from(spec.to_normalized(plain));
    let at_time = |spec: &ParamSpec, seconds: f32| match spec.id {
        ECHO_TIME => normalized(spec, seconds * 1000.0),
        // The division nearest in pitch, as the tempo stands.
        ECHO_DIVISION => {
            let beats = f64::from(seconds.max(1e-3)) * tempo / 60.0;
            let nearest = (0..DIVISIONS.len())
                .min_by(|a, b| {
                    let distance = |index: &usize| (division_beats(*index) / beats).ln().abs();
                    distance(a).total_cmp(&distance(b))
                })
                .unwrap_or(0);
            normalized(spec, nearest as f32)
        }
        PRE_DELAY => normalized(spec, seconds * 1000.0),
        // The tail falls 60 dB in Decay; the handle stands where it has
        // fallen from the reverb's level to the floor.
        _ => normalized(
            spec,
            (seconds - pre_delay).max(0.0) * 60.0 / (reverb_db - SPACE_FLOOR_DB).max(1.0),
        ),
    };
    let at_level = |spec: &ParamSpec, db: f32| match spec.id {
        ECHO_FEEDBACK => {
            let gain = 10f32.powf((db - echo_db) / 20.0).min(2.0);
            normalized(spec, 100.0 * feedback_for(settings.echo.model, gain))
        }
        _ => normalized(spec, db),
    };
    let mut edits: Vec<Vec<(u32, f64)>> = Vec::new();
    for drag in &output.drags {
        let [time, level] = handles[drag.index].1;
        edits.push(
            time.map(|spec| (spec.id, at_time(spec, drag.seconds)))
                .into_iter()
                .chain(level.map(|spec| (spec.id, at_level(spec, drag.db))))
                .collect(),
        );
    }
    for (index, x, y) in &output.nudges {
        let [time, level] = handles[*index].1;
        for (spec, steps) in [(time, *x), (level, *y)] {
            if let Some(spec) = spec.filter(|_| steps != 0.0) {
                edits.push(vec![stepped(spec, host, steps)]);
            }
        }
    }
    let defaults = |index: usize| -> Vec<(u32, f64)> {
        handles[index].1.into_iter().flatten().map(reset).collect()
    };
    if let Some(index) = output.double_clicked {
        edits.push(defaults(index));
    }
    for (index, hit) in output.handles.iter().enumerate() {
        hit.context_menu(|ui| {
            if daw_ui::menu_item(ui, "Reset", None).clicked() {
                edits.push(defaults(index));
                ui.close();
            }
        });
    }
    #[cfg(test)]
    {
        state.picture_rects = vec![output.response.rect];
    }
    for edit in edits {
        match edit[..] {
            [(id, value)] => host.set_value(id, value),
            _ => host.set_values(&edit),
        }
    }
}

/// R1 Sampler: the recording with its start, end and loop markers, which
/// drag like their knobs, and a playhead for every voice. Above it the
/// recording's name and the buttons that load or clear it; empty, it offers
/// to load one. A file dropped on it loads.
pub(crate) fn sample(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    height: f32,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    use sampler::*;
    let share = |id: u32, host: &dyn DeviceHost| {
        descriptor
            .parameter(id)
            .map_or(0.0, |spec| spec.to_plain(host.value(id) as f32) / 100.0)
    };
    let summary = host.sample(0).cloned();
    let mut request = None;
    daw_ui::parameter_row(ui, |ui| {
        match &summary {
            Some(summary) => {
                daw_ui::text::label(ui, &summary.name);
                daw_ui::text::note(ui, format!("{:.2} s", summary.seconds));
            }
            None => {
                daw_ui::text::note(ui, "No sample");
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if summary.is_some() && daw_ui::button(ui, "Clear").clicked() {
                request = Some(SampleRequest::Clear);
            }
            if daw_ui::button(ui, "Load sample…").clicked() {
                request = Some(SampleRequest::Pick);
            }
        });
    });
    ui.add_space(daw_ui::tokens::SPACE_XS);
    let settings = panels::sampler_settings(&|id| host.value(id));
    let playheads: Vec<f32> = live(ui, host.telemetry().as_ref(), state)
        .map(|telemetry| {
            (0..MAX_VOICES)
                .map(|voice| telemetry.value(TELEMETRY_PLAYHEADS + voice))
                .filter(|at| (0.0..=1.0).contains(at))
                .collect()
        })
        .unwrap_or_default();
    let empty = Vec::new();
    let data = daw_ui::SampleViewData {
        peaks: summary.as_ref().map_or(&empty, |summary| &summary.peaks),
        start: share(START, host),
        end: share(END, host),
        loop_range: settings
            .loop_mode
            .loops()
            .then(|| (share(LOOP_START, host), share(LOOP_END, host))),
        playheads: &playheads,
        envelope: &[],
    };
    let size = egui::vec2(ui.available_width(), height);
    // The response also covers the markers' handles, which reach past it.
    let view = egui::Rect::from_min_size(ui.cursor().min, size);
    let output = daw_ui::sample_view(ui, &data, size);
    #[cfg(test)]
    state.picture_rects.push(view);
    if summary.is_none() {
        // Empty: the view itself offers the recording it waits for.
        let center = view.center();
        let button = egui::Rect::from_center_size(
            center - egui::vec2(0.0, daw_ui::tokens::SPACE_SM),
            egui::vec2(160.0, daw_ui::tokens::CONTROL_HEIGHT),
        );
        if ui
            .put(button, daw_ui::button_widget("Load sample…"))
            .clicked()
        {
            request = Some(SampleRequest::Pick);
        }
        ui.put(
            egui::Rect::from_center_size(
                center + egui::vec2(0.0, daw_ui::tokens::CONTROL_HEIGHT),
                egui::vec2(size.x, daw_ui::tokens::CONTROL_HEIGHT),
            ),
            egui::Label::new(
                egui::RichText::new("or drop an audio file here")
                    .color(daw_ui::theme::palette(ui.ctx()).muted),
            ),
        );
    }
    for (marker, at) in output.drags {
        let id = match marker {
            daw_ui::SampleMarker::Start => START,
            daw_ui::SampleMarker::End => END,
            daw_ui::SampleMarker::LoopStart => LOOP_START,
            daw_ui::SampleMarker::LoopEnd => LOOP_END,
        };
        if let Some(spec) = descriptor.parameter(id) {
            host.set_value(id, f64::from(spec.to_normalized(at * 100.0)));
        }
    }
    if let Some(path) = output.files.into_iter().next() {
        request = Some(SampleRequest::File(path));
    }
    if let Some(request) = request {
        host.request_sample(0, request);
    }
}

/// Width of the kit's Volume fader cell beside the pads.
const KIT_FADER_CELL: f32 = 60.0;
/// Points of the envelope drawn over a pad's recording.
const ENVELOPE_POINTS: usize = 96;

/// R1 Drums: the kit card holds the sixteen pads, bottom row first as on a
/// drum controller, with the kit's Volume beside them; a click selects a
/// pad and sounds it, and an audio file dropped on a pad loads into it. The
/// selected pad's card shows its recording with its start and end markers,
/// which drag like their knobs, the amplifier envelope over it and the
/// latest hit's playhead.
pub(crate) fn drums(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    height: f32,
    layout: &panels::Layout,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    use drums::*;
    let pad = state.pad.min(PADS - 1);
    let telemetry = live(ui, host.telemetry().as_ref(), state);
    let settings = panels::drums_settings(&|id| host.value(id));
    let summaries: Vec<Option<crate::SampleSummary>> = (0..PADS)
        .map(|slot| host.sample(slot as u32).cloned())
        .collect();
    let cells = [
        daw_ui::GridCell {
            span: 5,
            framed: true,
        },
        daw_ui::GridCell {
            span: 7,
            framed: true,
        },
    ];
    let mut request: Option<(usize, SampleRequest)> = None;
    let mut edits: Vec<(u32, f64)> = Vec::new();
    daw_ui::grid_row_cells(ui, &cells, |ui, index| {
        if index == 0 {
            daw_ui::device_section_title(ui, "Kit");
            ui.add_space(daw_ui::tokens::SPACE_XS);
            // The card's width, which a row's own would not tell yet.
            let width = ui.available_width();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let views: [daw_ui::PadView; PADS] = std::array::from_fn(|index| daw_ui::PadView {
                    name: summaries[index]
                        .as_ref()
                        .map_or("", |summary| summary.name.as_str()),
                    loaded: summaries[index].is_some(),
                    activity: telemetry
                        .as_ref()
                        .map_or(0.0, |telemetry| telemetry.value(TELEMETRY_ACTIVITY + index)),
                    muted: settings.pads[index].mute,
                });
                let size = egui::vec2(width - KIT_FADER_CELL - daw_ui::tokens::SPACE_SM, height);
                #[cfg(test)]
                state
                    .picture_rects
                    .push(egui::Rect::from_min_size(ui.cursor().min, size));
                let output = daw_ui::pad_grid(ui, &views, pad, size);
                if let Some((index, velocity)) = output.pressed {
                    host.audition(Some((FIRST_NOTE + index as u8, velocity)));
                } else if output.released.is_some() {
                    host.audition(None);
                }
                if let Some(index) = output.selected {
                    state.pad = index;
                }
                if let Some((index, path)) = output.files {
                    request = Some((index, SampleRequest::File(path)));
                }
                ui.add_space(daw_ui::tokens::SPACE_SM);
                if let Some(spec) = descriptor.parameter(VOLUME) {
                    // The caption and value take their lines; the fader the rest.
                    let text =
                        ui.fonts_mut(|fonts| fonts.row_height(&daw_ui::control_caption_font()));
                    let cell = crate::Cell {
                        width: KIT_FADER_CELL,
                        natural: KIT_FADER_CELL,
                        height: height - 2.0 * text - 2.0 * ui.spacing().item_spacing.y,
                        lead: 0.0,
                        fader: true,
                    };
                    crate::control(ui, descriptor, spec, host, state, cell, layout);
                }
            });
            return;
        }
        let summary = summaries[pad].as_ref();
        let ids = |offset: u32| id(pad, offset);
        let title = format!("Pad {}", pad + 1);
        daw_ui::device_section_title(ui, &title);
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let top = ui.cursor().min.y;
        // One control line, level with the pads' top row.
        let line = egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT);
        ui.allocate_ui_with_layout(
            line,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                match summary {
                    Some(summary) => {
                        daw_ui::text::label(ui, &summary.name);
                        daw_ui::text::note(ui, format!("{:.2} s", summary.seconds));
                    }
                    None => {
                        daw_ui::text::note(ui, "No sample");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if summary.is_some() && daw_ui::button(ui, "Clear").clicked() {
                        request = Some((pad, SampleRequest::Clear));
                    }
                    if daw_ui::button(ui, "Load sample…").clicked() {
                        request = Some((pad, SampleRequest::Pick));
                    }
                });
            },
        );
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let pad_settings = &settings.pads[pad];
        let playheads: Vec<f32> = telemetry
            .as_ref()
            .map(|telemetry| telemetry.value(TELEMETRY_PLAYHEADS + pad))
            .filter(|at| (0.0..=1.0).contains(at))
            .into_iter()
            .collect();
        // The envelope over the stretch of the recording a hit plays, at
        // the pad's pitch.
        let envelope: Vec<f32> = summary
            .map(|summary| {
                let seconds = summary.seconds * (pad_settings.end - pad_settings.start).abs()
                    / 2f32.powf(pad_settings.pitch / 12.0);
                let mut levels: Vec<f32> = (0..ENVELOPE_POINTS)
                    .map(|point| {
                        let at = seconds * point as f32 / (ENVELOPE_POINTS - 1) as f32;
                        pad_settings.envelope(at)
                    })
                    .collect();
                if pad_settings.reverse {
                    levels.reverse();
                }
                levels
            })
            .unwrap_or_default();
        let empty = Vec::new();
        let data = daw_ui::SampleViewData {
            peaks: summary.map_or(&empty, |summary| &summary.peaks),
            start: pad_settings.start,
            end: pad_settings.end,
            loop_range: None,
            playheads: &playheads,
            envelope: &envelope,
        };
        let size = egui::vec2(
            ui.available_width(),
            (height - (ui.cursor().min.y - top)).max(daw_ui::tokens::CONTROL_HEIGHT),
        );
        let view = egui::Rect::from_min_size(ui.cursor().min, size);
        let output = daw_ui::sample_view(ui, &data, size);
        #[cfg(test)]
        state.picture_rects.push(view);
        if summary.is_none() {
            // Empty: the view itself offers the recording it waits for.
            let center = view.center();
            let button = egui::Rect::from_center_size(
                center - egui::vec2(0.0, daw_ui::tokens::SPACE_SM),
                egui::vec2(160.0, daw_ui::tokens::CONTROL_HEIGHT),
            );
            if ui
                .put(button, daw_ui::button_widget("Load sample…"))
                .clicked()
            {
                request = Some((pad, SampleRequest::Pick));
            }
            ui.put(
                egui::Rect::from_center_size(
                    center + egui::vec2(0.0, daw_ui::tokens::CONTROL_HEIGHT),
                    egui::vec2(size.x, daw_ui::tokens::CONTROL_HEIGHT),
                ),
                egui::Label::new(
                    egui::RichText::new("or drop an audio file here or on a pad")
                        .color(daw_ui::theme::palette(ui.ctx()).muted),
                ),
            );
        }
        for (marker, at) in output.drags {
            let id = match marker {
                daw_ui::SampleMarker::Start => ids(START),
                daw_ui::SampleMarker::End => ids(END),
                _ => continue,
            };
            if let Some(spec) = descriptor.parameter(id) {
                edits.push((id, f64::from(spec.to_normalized(at * 100.0))));
            }
        }
        if let Some(path) = output.files.into_iter().next() {
            request = Some((pad, SampleRequest::File(path)));
        }
    });
    for (id, value) in edits {
        host.set_value(id, value);
    }
    if let Some((slot, request)) = request {
        host.request_sample(slot as u32, request);
    }
}
