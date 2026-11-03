//! Console controls composed from the shared native slider and button widgets.
//! Audio levels and parameter values always belong to the caller.
use egui::{Align2, FontId, Pos2, Rect, Response, Sense, SliderClamping, Stroke, Ui, Vec2};

/// Rail thickness of the vertical mixer fader (its pointer target).
pub const MIXER_FADER_RAIL: f32 = crate::tokens::COMPACT_HEIGHT;
/// The dB scale gutter plus the fader rail.
pub const MIXER_FADER_WIDTH: f32 = crate::tokens::MIXER_SCALE_GUTTER + MIXER_FADER_RAIL;
/// Space between the fader rail's target and the channel meter.
pub const MIXER_METER_GAP: f32 = crate::tokens::SPACE_XS;
/// `[scale][rail][gap][meter]`: the unit centered on every strip axis.
pub const MIXER_FADER_UNIT_WIDTH: f32 =
    MIXER_FADER_WIDTH + MIXER_METER_GAP + crate::tokens::MIXER_METER_WIDTH;
/// Distance from the fader rail's ends to its handle travel (silence and top):
/// half the visible cap, so the cap never leaves the rail.
pub const MIXER_FADER_INSET: f32 = crate::MIXER_CAP_SIZE.y * 0.5;
/// Width reserved for the pan value ("L100") beside the pan rail.
pub const MIXER_PAN_CAPTION_WIDTH: f32 = crate::tokens::SPACE_XL;
/// The pan rail row above the Mute and Solo row.
pub const MIXER_PAN_ROW_HEIGHT: f32 =
    crate::tokens::COMPACT_HEIGHT + crate::MIXER_ROW_GAP + crate::tokens::CONTROL_HEIGHT;
/// Pan values this close to center snap to it while dragging: the center detent.
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

/// Paint the dB scale into the gutter left of `edge`: ticks end 1 pt before
/// it, captions are right-aligned. `bottom`/`top` are the y of silence and of
/// the travel top; positions follow [`gain_taper`]. On short faders captions
/// that would collide are dropped, landmarks (0, −∞, +3.5) first kept.
pub fn paint_mixer_scale(ui: &Ui, edge: f32, bottom: f32, top: f32) {
    let colors = crate::theme::palette(ui.ctx());
    let painter = ui.painter();
    let hairline = 1.0 / painter.pixels_per_point();
    let stops = mixer_scale_stops();
    let y = |gain: f32| {
        egui::emath::GuiRounding::round_to_pixel_center(
            bottom + (top - bottom) * gain_taper(gain),
            painter.pixels_per_point(),
        )
    };
    for (_, gain, major) in stops {
        let reach = if major { 5.0 } else { 3.0 };
        painter.line_segment(
            [
                Pos2::new(edge - reach, y(gain)),
                Pos2::new(edge - 1.0, y(gain)),
            ],
            Stroke::new(
                hairline,
                if gain == 1.0 {
                    colors.text
                } else {
                    colors.muted
                },
            ),
        );
    }
    let mut painted: Vec<f32> = Vec::new();
    for label in ["0", "−∞", "+3.5", "−12", "−30", "−6", "−20"] {
        let Some((_, gain, _)) = stops.iter().find(|stop| stop.0 == label) else {
            continue;
        };
        let at = y(*gain);
        if painted
            .iter()
            .any(|other| (other - at).abs() < crate::tokens::FONT_CAPTION + 1.0)
        {
            continue;
        }
        painted.push(at);
        painter.text(
            Pos2::new(edge - crate::tokens::SPACE_SM, at),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(crate::tokens::FONT_CAPTION),
            if *gain == 1.0 {
                colors.text
            } else {
                colors.muted
            },
        );
    }
}

/// Channel gain on the shared console taper, with its dB scale in the
/// left gutter. The native slider edits travel; the stored gain changes only
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
                ui.add_space(crate::tokens::MIXER_SCALE_GUTTER);
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
        paint_mixer_scale(ui, scale.rail.left(), scale.minimum.y, scale.maximum.y);
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

/// Horizontal pan rail in the fader's skin with a center detent, and the
/// value ("L15", "C") in a caption right of it. The whole row is one context
/// target.
pub fn mixer_pan(ui: &mut Ui, pan: &mut f32, width: f32) -> Response {
    let height = crate::tokens::COMPACT_HEIGHT;
    let row = ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, height));
            ui.set_max_height(height);
            ui.spacing_mut().interact_size.y = height;
            ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
            ui.spacing_mut().slider_width =
                (width - MIXER_PAN_CAPTION_WIDTH - crate::tokens::SPACE_XS).max(0.0);
            let mut response = ui.add(
                crate::slider(pan, -1.0..=1.0)
                    .mixer_style()
                    .scale_ticks(&[(0.5, true)])
                    .zero_reference(0.0)
                    .clamping(SliderClamping::Edits)
                    .show_value(false)
                    .scale_labels(false),
            );
            if response.dragged() && *pan != 0.0 && pan.abs() < PAN_DETENT {
                *pan = 0.0;
                response.mark_changed();
            }
            let label = ui
                .allocate_ui_with_layout(
                    Vec2::new(MIXER_PAN_CAPTION_WIDTH, height),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(mixer_pan_caption(*pan))
                                    .size(crate::tokens::FONT_CAPTION)
                                    .color(crate::theme::palette(ui.ctx()).muted),
                            )
                            // The row owns context clicks and focus.
                            .selectable(false)
                            .sense(Sense::hover()),
                        )
                    },
                )
                .inner;
            response.labelled_by(label.id)
        },
    );
    let id = row.inner.id;
    let response = row
        .inner
        .union(ui.interact(row.response.rect, id.with("row"), Sense::click()));
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*pan), "Pan"));
    crate::hover_text(
        response,
        format!(
            "Stereo balance · {} · {}%\nDrag or use the arrow keys; Shift for fine adjustment. Snaps to center.\nRight-click for exact value entry and automation",
            crate::channel_pan_label(*pan),
            *pan * 100.0
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

/// The pan rail with its value, then Mute and Solo underneath.
pub fn mixer_pan_row(
    ui: &mut Ui,
    pan: &mut f32,
    mute: &mut bool,
    solo: &mut bool,
    width: f32,
) -> MixerPanRowResponse {
    ui.allocate_ui_with_layout(
        Vec2::new(width, MIXER_PAN_ROW_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_size(Vec2::new(width, MIXER_PAN_ROW_HEIGHT));
            ui.spacing_mut().item_spacing.y = crate::MIXER_ROW_GAP;
            let pan = mixer_pan(ui, pan, width);
            let switches = mixer_mute_solo(ui, mute, solo, width);
            MixerPanRowResponse {
                pan,
                mute: switches.mute,
                solo: switches.solo,
            }
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
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().button_padding.x = crate::tokens::SPACE_XS;
            MixerMuteSoloResponse {
                mute: mixer_switch(ui, mute, "Mute", button_width, true),
                solo: mixer_switch(ui, solo, "Solo", button_width, false),
            }
        },
    )
    .inner
}

fn mixer_switch(ui: &mut Ui, value: &mut bool, label: &str, width: f32, mute: bool) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let accent = if mute { colors.mute } else { colors.solo };
    // Fill-only until active; the active state takes its palette color.
    let mut response = ui.add_sized(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        crate::IconButton::new(egui::RichText::new(label).color(if *value {
            colors.text
        } else {
            colors.action
        }))
        .selected(*value)
        .fill(if *value {
            colors.panel.lerp_to_gamma(accent, 0.3)
        } else {
            colors.control_fill
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
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
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

/// Vertical stereo peak meter on the fader's scale: `inset` is the distance
/// from the rect's ends to silence and to +3.5 dB, the fader's handle travel,
/// so one scale serves both. Holds each peak for 1.5 s and latches clipping
/// (warning color) until clicked. Readings are sanitized for painting only.
pub fn channel_meter(
    ui: &mut Ui,
    id: egui::Id,
    peaks: [f32; 2],
    height: f32,
    inset: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(crate::tokens::MIXER_METER_WIDTH, height.max(1.0)),
        Sense::click(),
    );
    let peaks = peaks.map(sanitize_peak);
    let now = ui.input(|input| input.time);
    let mut state = ui.data(|data| data.get_temp::<MeterHold>(id).unwrap_or_default());
    for (index, peak) in peaks.into_iter().enumerate() {
        if peak >= state.hold[index] || now - state.since[index] > METER_HOLD_SECONDS {
            state.hold[index] = peak;
            state.since[index] = now;
        }
    }
    state.clipped |= peaks.iter().any(|peak| *peak >= 1.0);
    if response.clicked() {
        state.clipped = false;
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    if state.hold.iter().any(|hold| *hold > 0.0) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(METER_HOLD_SECONDS));
    }
    if ui.is_rect_visible(rect) {
        let colors = crate::theme::palette(ui.ctx());
        let gap = 3.0;
        let bar_width = (rect.width() - gap) * 0.5;
        let bottom = rect.bottom() - inset;
        let top = rect.top() + inset;
        let y = |gain: f32| bottom + (top - bottom) * gain_taper(gain).min(1.0);
        for (index, peak) in peaks.into_iter().enumerate() {
            let bar = Rect::from_min_size(
                rect.min + Vec2::new(index as f32 * (bar_width + gap), 0.0),
                Vec2::new(bar_width, rect.height()),
            );
            // The same trough as the fader rail beside it.
            ui.painter().rect(
                egui::emath::GuiRounding::round_to_pixels(bar, ui.pixels_per_point()),
                1.0,
                colors.bg,
                Stroke::new(1.0 / ui.pixels_per_point(), colors.line),
                egui::StrokeKind::Inside,
            );
            if peak > 0.0 {
                let level = y(peak);
                let unity = y(1.0);
                ui.painter().rect_filled(
                    Rect::from_x_y_ranges(bar.x_range(), level.max(unity)..=bottom),
                    1.0,
                    colors.guide,
                );
                if level < unity {
                    ui.painter().rect_filled(
                        Rect::from_x_y_ranges(bar.x_range(), level..=unity),
                        1.0,
                        colors.warning,
                    );
                }
            }
            if state.hold[index] > 0.0 {
                let hold = y(state.hold[index]);
                ui.painter().line_segment(
                    [Pos2::new(bar.left(), hold), Pos2::new(bar.right(), hold)],
                    Stroke::new(
                        1.5_f32,
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
    let colors = crate::theme::palette(ui.ctx());
    let text = peak_db_text(state.hold[0].max(state.hold[1]));
    let galley = ui.painter().layout_no_wrap(
        text.clone(),
        FontId::proportional(crate::tokens::FONT_CAPTION),
        if state.clipped {
            colors.warning
        } else {
            colors.muted
        },
    );
    let rect = Rect::from_center_size(Pos2::new(center_x, row.center().y), galley.size())
        .translate(Vec2::new(
            (row.right() - (center_x + galley.size().x * 0.5)).min(0.0),
            0.0,
        ));
    let response = ui.interact(rect, id.with("readout"), Sense::click());
    if response.clicked() {
        ui.data_mut(|data| data.get_temp_mut_or_default::<MeterHold>(id).clipped = false);
    }
    ui.painter().galley(rect.min, galley, colors.muted);
    response.on_hover_text(format!("Peak {text} dBFS"))
}
