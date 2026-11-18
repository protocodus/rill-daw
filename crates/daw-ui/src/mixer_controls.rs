//! Console controls composed from the shared native slider and button widgets.
//! Audio levels and parameter values always belong to the caller.
use egui::{Pos2, Rect, Response, Sense, SliderClamping, Stroke, Ui, Vec2};

/// Rail thickness of the vertical mixer fader (its pointer target).
pub const MIXER_FADER_RAIL: f32 = crate::tokens::COMPACT_HEIGHT;
/// The fader's width: its rail, which carries the unity mark too.
pub const MIXER_FADER_WIDTH: f32 = MIXER_FADER_RAIL;
/// Space between the fader rail's target and the channel meter.
pub const MIXER_METER_GAP: f32 = crate::tokens::SPACE_XS;
/// `[rail][gap][meter]`: the unit centered on every strip axis.
pub const MIXER_FADER_UNIT_WIDTH: f32 =
    MIXER_FADER_WIDTH + MIXER_METER_GAP + crate::tokens::MIXER_METER_WIDTH;
/// Distance from the fader rail's ends to its handle travel (silence and top):
/// half the visible cap, so the cap never leaves the rail.
pub const MIXER_FADER_INSET: f32 = crate::MIXER_CAP_SIZE.y * 0.5;
/// Knob values this close to their detent snap to it while dragging.
const PAN_DETENT: f32 = 0.03;

/// Gain at the top of the fader travel (+3.5 dB); the model allows 0..=1.5.
pub const GAIN_TAPER_MAX: f32 = 1.5;
const TAPER_UNITY: f32 = 0.75;
const TAPER_PER_DB: f32 = 0.02;
const TAPER_KNEE_DB: f32 = -30.0;

fn taper_top_db() -> f32 {
    20.0 * GAIN_TAPER_MAX.log10()
}

/// Shared console gain law: linear gain to travel fraction. 0 dB sits at 75 %,
/// −10/−20/−30 dB are evenly spaced (2 % per dB), and the last stretch below
/// −30 dB is linear in amplitude so silence is reachable. Gains above the
/// fader's +3.5 dB top extrapolate the upper segment beyond 1.0.
pub fn gain_taper(gain: f32) -> f32 {
    if gain.is_nan() || gain <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * gain.log10();
    if db >= 0.0 {
        TAPER_UNITY + (1.0 - TAPER_UNITY) * db / taper_top_db()
    } else if db >= TAPER_KNEE_DB {
        TAPER_UNITY + db * TAPER_PER_DB
    } else {
        let knee = TAPER_UNITY + TAPER_KNEE_DB * TAPER_PER_DB;
        knee * gain / 10.0_f32.powf(TAPER_KNEE_DB / 20.0)
    }
}

/// Inverse of [`gain_taper`].
pub fn gain_untaper(fraction: f32) -> f32 {
    let knee = TAPER_UNITY + TAPER_KNEE_DB * TAPER_PER_DB;
    if fraction.is_nan() || fraction <= 0.0 {
        0.0
    } else if fraction >= TAPER_UNITY {
        10.0_f32.powf((fraction - TAPER_UNITY) / (1.0 - TAPER_UNITY) * taper_top_db() / 20.0)
    } else if fraction >= knee {
        10.0_f32.powf((fraction - TAPER_UNITY) / TAPER_PER_DB / 20.0)
    } else {
        fraction / knee * 10.0_f32.powf(TAPER_KNEE_DB / 20.0)
    }
}

fn db_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Labelled (major) and minor dB stops shared by the fader and channel meter,
/// as `(caption, gain, major)`. Minor stops carry no caption.
pub fn mixer_scale_stops() -> [(&'static str, f32, bool); 12] {
    [
        ("+3.5", GAIN_TAPER_MAX, true),
        ("0", 1.0, true),
        ("", db_gain(-3.0), false),
        ("−6", db_gain(-6.0), true),
        ("", db_gain(-9.0), false),
        ("−12", db_gain(-12.0), true),
        ("", db_gain(-18.0), false),
        ("−20", db_gain(-20.0), true),
        ("", db_gain(-24.0), false),
        ("−30", db_gain(-30.0), true),
        ("", db_gain(-40.0), false),
        ("−∞", 0.0, true),
    ]
}

/// Physical-pixel level row shared by the fader index, its scale and meters.
/// The unrounded rail endpoints remain the native slider's input geometry.
pub fn mixer_level_y(rect: Rect, inset: f32, gain: f32, pixels_per_point: f32) -> f32 {
    let bottom = rect.bottom() - inset;
    let top = rect.top() + inset;
    crate::slider_paint_center(
        Pos2::new(
            rect.center().x,
            bottom + (top - bottom) * gain_taper(gain).clamp(0.0, 1.0),
        ),
        pixels_per_point,
    )
    .y
}

/// Paint the fader's only landmark, unity gain: a short tick on each side
/// of the trough centered on `axis`. The exact level is in the gain readout
/// above the fader; numbers beside the rail would repeat it.
/// `bottom`/`top` are the y of silence and of the travel top.
pub fn paint_mixer_scale(ui: &Ui, axis: f32, bottom: f32, top: f32) {
    let colors = crate::theme::palette(ui.ctx());
    let painter = ui.painter();
    let y = egui::emath::GuiRounding::round_to_pixel_center(
        bottom + (top - bottom) * gain_taper(1.0),
        painter.pixels_per_point(),
    );
    // Clear of the 5 pt trough, inside the rail's target.
    for side in [-1.0, 1.0] {
        painter.line_segment(
            [
                Pos2::new(axis + side * 5.0, y),
                Pos2::new(axis + side * 9.0, y),
            ],
            Stroke::new(1.5_f32, colors.text),
        );
    }
}

/// Channel gain on the shared console taper, with a unity mark across
/// the rail. The native slider edits travel; the stored gain changes only
/// on an edit, so displaying never rounds or clamps the caller's value.
/// The returned response retains the rail identity for exact entry,
/// automation and scale geometry (whose value is the travel fraction).
pub fn mixer_fader(ui: &mut Ui, gain: &mut f32, height: f32) -> Response {
    let mut travel = gain_taper(*gain);
    let ticks: Vec<_> = mixer_scale_stops()
        .into_iter()
        .map(|(_, gain, major)| (gain_taper(gain), major))
        .collect();
    let response = ui
        .allocate_ui_with_layout(
            Vec2::new(MIXER_FADER_WIDTH, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().slider_width = height;
                ui.add(
                    crate::slider(&mut travel, 0.0..=1.0)
                        .mixer_style()
                        .scale_ticks(&ticks)
                        .zero_reference(f64::from(TAPER_UNITY))
                        .clamping(SliderClamping::Edits)
                        .vertical()
                        .show_value(false)
                        .scale_labels(false),
                )
            },
        )
        .inner;
    if response.changed() {
        *gain = gain_untaper(travel);
    }
    if let Some(scale) = crate::slider_scale_geometry(ui.ctx(), response.id)
        && ui.is_rect_visible(scale.rail)
    {
        paint_mixer_scale(ui, scale.rail.center().x, scale.minimum.y, scale.maximum.y);
    }
    crate::hover_text(
        response,
        format!(
            "Channel volume · {}\nRight-click for exact value entry and automation",
            crate::channel_gain_label(*gain)
        ),
    )
}

/// The strip's short pan value: "C", or "L"/"R" with a whole percentage.
/// The tooltip and exact entry keep the precise value.
pub fn mixer_pan_caption(pan: f32) -> String {
    let percent = (pan * 100.0).round();
    if percent == 0.0 {
        "C".to_owned()
    } else {
        format!("{}{}", if percent < 0.0 { "L" } else { "R" }, percent.abs())
    }
}

/// Pan as a labelled bipolar knob, in the same row shape as the width knob
/// above it: "Pan" on the left, the dial and its value ("L15", "C") on the
/// right. Snaps to center while dragging; double-click resets to center.
/// The whole row is one context target.
pub fn mixer_pan(ui: &mut Ui, pan: &mut f32, width: f32) -> Response {
    let response = mixer_knob_row(
        ui,
        width,
        "Pan",
        pan,
        -1.0..=1.0,
        0.0,
        &mixer_pan_caption(*pan),
    );
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*pan), "Pan"));
    crate::hover_text(
        response,
        format!(
            "Stereo balance · {} · {}%\nDrag or use the arrow keys; Shift for fine adjustment. Snaps to center; double-click to reset.\nRight-click for exact value entry and automation",
            crate::channel_pan_label(*pan),
            *pan * 100.0
        ),
    )
}

/// One strip row of a named bipolar knob: the caption on the left, the dial
/// and its value right-aligned in a cell wide enough for "200%", so the dials
/// of the rows stack on one axis. Pointer edits snap to `detent` near it, and
/// a double-click returns there. The returned response covers the whole row.
fn mixer_knob_row(
    ui: &mut Ui,
    width: f32,
    caption: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    detent: f32,
    text: &str,
) -> Response {
    let height = crate::tokens::COMPACT_HEIGHT;
    let row = ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, height));
            ui.set_max_height(height);
            ui.spacing_mut().interact_size.y = height;
            ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
            // Named in place: a bare value does not say what it controls.
            let label = ui.add(
                egui::Label::new(
                    egui::RichText::new(caption)
                        .size(crate::tokens::FONT_CAPTION)
                        .color(crate::theme::palette(ui.ctx()).muted),
                )
                .selectable(false),
            );
            let knob_width = crate::inline_knob_width(ui, "200%");
            ui.add_space((ui.available_width() - knob_width).max(0.0));
            let mut edited = f64::from(*value);
            let range = f64::from(*range.start())..=f64::from(*range.end());
            let mut response = ui.add(
                crate::inline_knob(caption, &mut edited, range, text)
                    .width(knob_width)
                    .bipolar(),
            );
            if response.changed() {
                *value = edited as f32;
                // Pointer edits only: arrow keys keep their fine steps.
                let pointer =
                    ui.input(|input| input.pointer.any_down() || input.pointer.any_released());
                if pointer && *value != detent && (*value - detent).abs() < PAN_DETENT {
                    *value = detent;
                }
            }
            if response.double_clicked() && *value != detent {
                *value = detent;
                response.mark_changed();
            }
            response.labelled_by(label.id)
        },
    );
    let id = row.inner.id;
    row.inner
        .union(ui.interact(row.response.rect, id.with("row"), Sense::click()))
}

/// The strip's short width value, a whole percentage of the original side.
pub fn mixer_width_caption(width: f32) -> String {
    format!("{}%", (width * 100.0).round())
}

/// Square polarity switch for the input row. It flips both channels together.
pub fn mixer_polarity_switch(ui: &mut Ui, invert: &mut bool) -> Response {
    let height = crate::mixer_control_height(ui);
    let colors = crate::theme::palette(ui.ctx());
    let mut response = ui.add_sized(
        Vec2::new(height, height),
        crate::IconButton::new("Ø")
            .min_size(Vec2::new(height, height))
            .label_color(if *invert {
                crate::theme::contrast_text(colors.accent)
            } else {
                colors.text
            })
            .selected(*invert)
            .fill(if *invert {
                colors.accent
            } else {
                colors.control_fill
            }),
    );
    if response.clicked() && ui.is_enabled() {
        *invert = !*invert;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *invert,
            "Invert polarity",
        )
    });
    crate::hover_text(
        response,
        "Invert polarity · flips both channels before the inserts",
    )
}

/// Stereo width as a labelled knob snapping to 100%, in the pan row's shape.
/// Right-click enters an exact value; `name` identifies the channel in that
/// editor.
pub fn mixer_stereo_width(ui: &mut Ui, stereo_width: &mut f32, width: f32, name: &str) -> Response {
    let mut response = mixer_knob_row(
        ui,
        width,
        "Width",
        stereo_width,
        0.0..=2.0,
        1.0,
        &mixer_width_caption(*stereo_width),
    );
    let mut exact = None;
    crate::context_menu(&response, |ui| {
        if let Some(value) = crate::value_entry_named(
            ui,
            response.id.with("exact_width"),
            f64::from(*stereo_width) * 100.0,
            0.0..=200.0,
            "%",
            name,
        ) {
            exact = Some((value / 100.0) as f32);
            ui.close();
        }
    });
    if let Some(value) = exact {
        *stereo_width = value.clamp(0.0, 2.0);
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*stereo_width), "Stereo width")
    });
    crate::hover_text(
        response,
        format!(
            "Stereo width · {} · 0% is mono, 100% unchanged, 200% doubles the side\nSnaps to 100%. Double-click to reset. Right-click for exact value entry",
            mixer_width_caption(*stereo_width)
        ),
    )
}

pub struct MixerMuteSoloResponse {
    pub mute: Response,
    pub solo: Response,
}

pub struct MixerPanRowResponse {
    pub pan: Response,
    pub mute: Response,
    pub solo: Response,
}

/// The pan rail with its value, then compact M and S switches underneath.
pub fn mixer_pan_row(
    ui: &mut Ui,
    pan: &mut f32,
    mute: &mut bool,
    solo: &mut bool,
    width: f32,
) -> MixerPanRowResponse {
    pan_row(ui, pan, mute, Some(solo), width)
}

/// Height of the M and S switches under the pan rail.
pub const MIXER_SWITCH_HEIGHT: f32 = crate::tokens::COMPACT_HEIGHT;

/// Height of [`mixer_pan_row`] and [`mixer_master_pan_row`].
pub const MIXER_PAN_ROW_HEIGHT: f32 =
    crate::tokens::COMPACT_HEIGHT + crate::MIXER_ROW_GAP + MIXER_SWITCH_HEIGHT;

fn pan_row(
    ui: &mut Ui,
    pan: &mut f32,
    mute: &mut bool,
    solo: Option<&mut bool>,
    width: f32,
) -> MixerPanRowResponse {
    ui.allocate_ui_with_layout(
        Vec2::new(width, MIXER_PAN_ROW_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_size(Vec2::new(width, MIXER_PAN_ROW_HEIGHT));
            ui.spacing_mut().item_spacing.y = crate::MIXER_ROW_GAP;
            let pan = mixer_pan(ui, pan, width);
            let (mute, solo) = ui
                .allocate_ui_with_layout(
                    Vec2::new(width, MIXER_SWITCH_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let gap = crate::tokens::SPACE_XS;
                        ui.set_max_height(MIXER_SWITCH_HEIGHT);
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.spacing_mut().interact_size.y = MIXER_SWITCH_HEIGHT;
                        ui.spacing_mut().button_padding = Vec2::ZERO;
                        let size = Vec2::new((width - gap) * 0.5, MIXER_SWITCH_HEIGHT);
                        let mute = mixer_switch(ui, mute, "M", size.x, size.y, true);
                        // Master keeps the solo column empty so its switch lines up with the channels'.
                        let solo = match solo {
                            Some(solo) => mixer_switch(ui, solo, "S", size.x, size.y, false),
                            None => ui.allocate_exact_size(size, egui::Sense::hover()).1,
                        };
                        (mute, solo)
                    },
                )
                .inner;
            MixerPanRowResponse { pan, mute, solo }
        },
    )
    .inner
}

/// Independent, equally sized channel switches. Native buttons provide focus,
/// keyboard activation and disabled behavior; active Mute and Solo use the
/// palette's mute and solo colors.
pub fn mixer_mute_solo(
    ui: &mut Ui,
    mute: &mut bool,
    solo: &mut bool,
    width: f32,
) -> MixerMuteSoloResponse {
    let gap = crate::tokens::SPACE_XS;
    let button_width = (width - gap) * 0.5;
    ui.allocate_ui_with_layout(
        Vec2::new(width, crate::mixer_control_height(ui)),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().button_padding.x = crate::tokens::SPACE_XS;
            MixerMuteSoloResponse {
                mute: mixer_switch(
                    ui,
                    mute,
                    "Mute",
                    button_width,
                    crate::mixer_control_height(ui),
                    true,
                ),
                solo: mixer_switch(
                    ui,
                    solo,
                    "Solo",
                    button_width,
                    crate::mixer_control_height(ui),
                    false,
                ),
            }
        },
    )
    .inner
}

/// Narrow output strips use familiar M/S captions with full accessible names
/// and the same color/state treatment as the full console switches.
pub fn mixer_folded_mute_solo(
    ui: &mut Ui,
    mute: &mut bool,
    solo: &mut bool,
    width: f32,
) -> MixerMuteSoloResponse {
    let height = crate::mixer_control_height(ui);
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let gap = crate::tokens::SPACE_XS;
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().button_padding.x = 2.0;
            MixerMuteSoloResponse {
                mute: mixer_switch(ui, mute, "M", (width - gap) * 0.5, height, true),
                solo: mixer_switch(ui, solo, "S", (width - gap) * 0.5, height, false),
            }
        },
    )
    .inner
}

/// Master has no meaningful solo destination; retain its output mute while
/// sharing the same pan and switch baselines as ordinary channel strips.
pub fn mixer_master_pan_row(
    ui: &mut Ui,
    pan: &mut f32,
    mute: &mut bool,
    width: f32,
) -> MixerPanRowResponse {
    pan_row(ui, pan, mute, None, width)
}

fn mixer_switch(
    ui: &mut Ui,
    value: &mut bool,
    label: &str,
    width: f32,
    height: f32,
    mute: bool,
) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let accent = if mute { colors.mute } else { colors.solo };
    // Color identifies the operation even while idle. A solid fill and
    // contrasting label distinguish the active state without relying on hue.
    let mut response = ui.add_sized(
        Vec2::new(width, height),
        crate::IconButton::new(label)
            .min_size(Vec2::new(width, height))
            .truncate()
            .label_color(if *value {
                crate::theme::contrast_text(accent)
            } else {
                accent
            })
            .selected(*value)
            .fill(if *value {
                accent
            } else {
                colors.control_fill.lerp_to_gamma(accent, 0.1)
            })
            .stroke(if *value {
                Stroke::new(crate::tokens::FOCUS_WIDTH, accent)
            } else {
                Stroke::NONE
            }),
    );
    if response.clicked() && ui.is_enabled() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *value,
            if mute { "Mute" } else { "Solo" },
        )
    });
    response.on_hover_text(if mute {
        "Mute this channel"
    } else {
        "Solo this channel"
    })
}

#[derive(Clone, Copy, Default)]
struct MeterHold {
    hold: [f32; 2],
    since: [f64; 2],
    clipped: bool,
}

const METER_HOLD_SECONDS: f64 = 1.5;

fn sanitize_peak(peak: f32) -> f32 {
    if peak.is_finite() { peak.max(0.0) } else { 0.0 }
}

fn peak_db_text(peak: f32) -> String {
    if peak > 0.000_01 {
        format!("{:.1}", 20.0 * peak.log10()).replace('-', "−")
    } else {
        "−∞".to_owned()
    }
}

/// Where a peak meter turns from the signal color to amber: the last 6 dB
/// before full scale, where a hot channel is worth a look.
pub const METER_HOT_DB: f32 = -6.0;

/// Peak meter zones as `(from, to, color)` in linear gain: signal, the hot
/// last 6 dB of headroom, then the warning color past full scale.
pub(crate) fn meter_zones(colors: &crate::theme::Palette) -> [(f32, f32, egui::Color32); 3] {
    let hot = 10f32.powf(METER_HOT_DB / 20.0);
    [
        (0.0, hot, colors.guide),
        (hot, 1.0, colors.orange),
        (1.0, f32::INFINITY, colors.warning),
    ]
}

/// Color stops of a vertical peak meter from its silent bottom up, blending
/// briefly across the hot and full-scale lines so the fill reads as one bar.
pub(crate) fn meter_stops(
    colors: &crate::theme::Palette,
    bottom: f32,
    hot: f32,
    unity: f32,
) -> [(f32, egui::Color32); 5] {
    const BLEND: f32 = 2.0;
    let zones = meter_zones(colors);
    [
        (bottom, zones[0].2),
        (hot + BLEND, zones[0].2),
        (hot - BLEND, zones[1].2),
        (unity + BLEND * 0.5, zones[1].2),
        (unity - BLEND * 0.5, zones[2].2),
    ]
}

/// One flat meter bar: a fully rounded trough and the level from its bottom up
/// to `level` as a single mesh whose color follows `stops` (bottom first).
pub(crate) fn paint_meter_bar(
    painter: &egui::Painter,
    bar: Rect,
    level: f32,
    stops: &[(f32, egui::Color32)],
    trough: egui::Color32,
) {
    let radius = (bar.width() * 0.5).min(2.0);
    painter.rect_filled(bar, radius, trough);
    // The fill starts at the first stop, silence, which may sit above the bar's end.
    let base = stops[0].0.min(bar.bottom());
    let top = level.clamp(bar.top(), base);
    if top >= base - 0.5 {
        return;
    }
    let color_at = |y: f32| {
        let mut previous = stops[0];
        for &stop in stops {
            if y >= stop.0 {
                let span = previous.0 - stop.0;
                let t = if span > 0.0 {
                    (previous.0 - y) / span
                } else {
                    1.0
                };
                return previous.1.lerp_to_gamma(stop.1, t.clamp(0.0, 1.0));
            }
            previous = stop;
        }
        previous.1
    };
    let mut rows: Vec<f32> = stops
        .iter()
        .map(|stop| stop.0)
        .filter(|y| *y > top && *y < base)
        .chain([base, top])
        .collect();
    rows.sort_by(|a, b| b.total_cmp(a));
    let mut mesh = egui::Mesh::default();
    for (index, y) in rows.iter().enumerate() {
        let color = color_at(*y);
        mesh.colored_vertex(Pos2::new(bar.left(), *y), color);
        mesh.colored_vertex(Pos2::new(bar.right(), *y), color);
        if index > 0 {
            let base = 2 * index as u32;
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base + 1, base);
        }
    }
    painter.add(mesh);
}

/// Vertical stereo peak meter on the fader's scale: `inset` is the distance
/// from the rect's ends to silence and to +3.5 dB, the fader's handle travel,
/// so one scale serves both. Green, amber over the last 6 dB of headroom and
/// red past full scale. Holds each peak for 1.5 s and latches clipping
/// (warning color) until clicked. Readings are sanitized for painting only.
pub fn channel_meter(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    height: f32,
    inset: f32,
) -> Response {
    channel_meter_inner(
        ui,
        id,
        peaks,
        None,
        crate::tokens::MIXER_METER_WIDTH,
        height,
        inset,
    )
}

/// Stereo meter with an engine-owned peak hold. A single hold drives its
/// peak lines, clip lamp and the adjacent [`channel_peak_indicator`] badge.
/// `width` is at least [`crate::tokens::MIXER_METER_WIDTH`]; a mixer strip
/// gives the meter the space beside its fader.
pub fn channel_meter_with_peak_hold(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    held: [f32; 2],
    width: f32,
    height: f32,
    inset: f32,
) -> Response {
    channel_meter_inner(ui, id, peaks, Some(held), width, height, inset)
}

fn channel_meter_inner(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    held: Option<[f32; 2]>,
    width: f32,
    height: f32,
    inset: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width.max(crate::tokens::MIXER_METER_WIDTH), height.max(1.0)),
        Sense::click(),
    );
    let peaks = peaks.map(sanitize_peak);
    let now = ui.input(|input| input.time);
    let mut state = ui.data(|data| data.get_temp::<MeterHold>(id).unwrap_or_default());
    if let Some(held) = held {
        state.hold = held.map(sanitize_peak);
        state.clipped = state.hold.iter().any(|peak| *peak >= 1.0);
    } else {
        for (index, peak) in peaks.into_iter().enumerate() {
            if peak >= state.hold[index] || now - state.since[index] > METER_HOLD_SECONDS {
                state.hold[index] = peak;
                state.since[index] = now;
            }
        }
        state.clipped |= peaks.iter().any(|peak| *peak >= 1.0);
    }
    if response.clicked() {
        state.clipped = false;
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    if held.is_none() && state.hold.iter().any(|hold| *hold > 0.0) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(METER_HOLD_SECONDS));
    }
    if ui.is_rect_visible(rect) {
        let colors = crate::theme::palette(ui.ctx());
        let gap = 3.0;
        let bar_width = (rect.width() - gap) * 0.5;
        let bottom = mixer_level_y(rect, inset, 0.0, ui.pixels_per_point());
        let y = |gain: f32| mixer_level_y(rect, inset, gain, ui.pixels_per_point());
        for (index, peak) in peaks.into_iter().enumerate() {
            let bar = Rect::from_min_size(
                rect.min + Vec2::new(index as f32 * (bar_width + gap), 0.0),
                Vec2::new(bar_width, rect.height()),
            );
            // The same flat, rounded trough as the fader rail beside it.
            paint_meter_bar(
                ui.painter(),
                egui::emath::GuiRounding::round_to_pixels(bar, ui.pixels_per_point()),
                y(peak),
                &meter_stops(&colors, bottom, y(10f32.powf(METER_HOT_DB / 20.0)), y(1.0)),
                colors.trough,
            );
            if state.hold[index] > 0.0 {
                let hold = y(state.hold[index]);
                ui.painter().line_segment(
                    [Pos2::new(bar.left(), hold), Pos2::new(bar.right(), hold)],
                    Stroke::new(
                        1.0 / ui.pixels_per_point(),
                        if state.hold[index] >= 1.0 {
                            colors.warning
                        } else {
                            colors.text
                        },
                    ),
                );
            }
        }
        if state.clipped {
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(rect.width(), crate::tokens::SPACE_XS)),
                1.0,
                colors.warning,
            );
        }
    }
    let db = peaks.map(|peak| 20.0 * peak.log10());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            ui.is_enabled(),
            format!("Stereo peak: L {:.1} dBFS, R {:.1} dBFS", db[0], db[1]),
        )
    });
    response.on_hover_text(format!(
        "Level · L {} · R {} dBFS{}",
        peak_db_text(peaks[0]),
        peak_db_text(peaks[1]),
        if state.clipped {
            "\nClipped · click to reset"
        } else {
            ""
        }
    ))
}

/// The held peak of [`channel_meter`] `id` in dB, centered at `center_x` in
/// `row`; warning color while clipping is latched. Click resets the latch.
pub fn channel_meter_readout(ui: &mut Ui, id: egui::Id, center_x: f32, row: Rect) -> Response {
    let state = ui.data(|data| data.get_temp::<MeterHold>(id).unwrap_or_default());
    let response = channel_peak_indicator_inner(ui, id, state.hold, state.clipped, center_x, row);
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(id, MeterHold::default()));
    }
    response
}

/// Resettable peak badge. The caller supplies engine-held block maxima and
/// handles clicks by resetting that telemetry; painting never consumes peaks.
/// A full-scale value retains a warning fill, including after signal stops.
pub fn channel_peak_indicator(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    center_x: f32,
    row: Rect,
) -> Response {
    let clipped = peaks.iter().any(|peak| peak.is_finite() && *peak >= 1.0);
    channel_peak_indicator_inner(ui, id, peaks, clipped, center_x, row)
}

fn channel_peak_indicator_inner(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    clipped: bool,
    center_x: f32,
    row: Rect,
) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let peak = peaks.map(sanitize_peak).into_iter().fold(0.0, f32::max);
    let text = peak_db_text(peak);
    let color = if clipped {
        colors.warning
    } else {
        colors.muted
    };
    let galley = ui.painter().layout_no_wrap(
        text.clone(),
        egui::FontId::proportional(crate::tokens::FONT_CAPTION),
        color,
    );
    let size = Vec2::new(
        (galley.size().x + 2.0 * crate::tokens::SPACE_XS).max(crate::tokens::CONTROL_HEIGHT),
        row.height(),
    );
    let rect = Rect::from_center_size(Pos2::new(center_x, row.center().y), size).translate(
        Vec2::new((row.right() - (center_x + size.x * 0.5)).min(0.0), 0.0),
    );
    let response = ui.interact(rect, id.with("readout"), Sense::click());
    let badge = Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width(), crate::tokens::COMPACT_HEIGHT),
    );
    ui.painter().rect_filled(
        badge,
        crate::tokens::RADIUS,
        if clipped {
            colors.panel.lerp_to_gamma(colors.warning, 0.22)
        } else {
            colors.control_fill
        },
    );
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("Peak {text} dBFS · Reset peak"),
        )
    });
    crate::hover_text(
        response,
        format!(
            "Peak {text} dBFS{}\nHighest level since reset · Click to reset",
            if clipped { " · Clipped" } else { "" }
        ),
    )
}
