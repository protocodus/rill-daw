//! Clip identity presentation shared by timeline canvases and property views.
//! The application decides which instances are linked and owns all editing.
use crate::{
    Icon,
    color_math::{contrast_luminance, luminance},
    text, tokens,
};
use egui::emath::GuiRounding as _;
use egui::{
    Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

/// Paint one occupied interval on a collapsed track. A zero-duration interval
/// marks a base automation point. Each clip or point owns its own mark, so gaps
/// remain visible. The returned bounds exclude any scrolled-out portion.
pub fn collapsed_track_span(
    painter: &Painter,
    lane: Rect,
    viewport: crate::TimelineViewport,
    start: f64,
    end: f64,
    color: Color32,
) -> Option<Rect> {
    let visible = lane.intersect(painter.clip_rect());
    if !visible.is_positive() || !start.is_finite() || !end.is_finite() || end < start {
        return None;
    }
    let left = viewport.x_at(lane, start);
    let right = viewport.x_at(lane, end);
    if right < visible.left() || left > visible.right() {
        return None;
    }
    let half_height = tokens::SPACE_XS.min(lane.height()) * 0.5;
    let extra = (tokens::SPACE_XS - (right - left)).max(0.0) * 0.5;
    let rect = Rect::from_min_max(
        Pos2::new(left - extra, lane.center().y - half_height),
        Pos2::new(right + extra, lane.center().y + half_height),
    )
    .round_to_pixels(painter.pixels_per_point())
    .intersect(visible);
    if !rect.is_positive() {
        return None;
    }
    painter
        .with_clip_rect(visible)
        .rect_filled(rect, tokens::RADIUS, color);
    Some(rect)
}

/// Separate clip identity from its musical preview at every track height.
#[derive(Clone, Copy, Debug)]
pub struct ClipSurfaceLayout {
    pub title: Rect,
    pub content: Rect,
    pub title_color: Color32,
    /// Contrasting track hue for notes on the body.
    pub preview_color: Color32,
    /// The painted title band, including the thin strip on unnamed clips.
    /// It is geometry only; the caller retains body, edge and fade ownership.
    pub name_hit: Rect,
}

impl ClipSurfaceLayout {
    /// Keep labels inside the actual name band when a lane is only one row high.
    pub fn title_font(self) -> FontId {
        FontId::proportional(tokens::FONT_CANVAS.min(self.title.height() * 0.78))
    }
}

/// An opaque, track-colored event with a contrasting name band and selection halo.
/// This is paint only: the caller retains the clip's complete interaction target.
pub fn clip_surface(
    painter: &Painter,
    rect: Rect,
    color: Color32,
    selected: bool,
) -> ClipSurfaceLayout {
    ClipSurfaceColors::new(painter.ctx(), color).paint(painter, rect, selected, true)
}

/// An optional clip name uses space only when present. Blank/whitespace names
/// leave a thin top strip while their musical preview recovers the title area.
pub fn clip_surface_named(
    painter: &Painter,
    rect: Rect,
    color: Color32,
    selected: bool,
    name: &str,
) -> ClipSurfaceLayout {
    ClipSurfaceColors::new(painter.ctx(), color).paint(
        painter,
        rect,
        selected,
        !name.trim().is_empty(),
    )
}

/// Shared vertical bounds for events, automation, overlaps and live recordings.
/// Thin gutters keep time-aligned content legible as track height is reduced.
pub fn clip_lane_rect(row: Rect) -> Rect {
    let gutter = (row.height() / 16.0).round().min(tokens::SPACE_XS);
    row.shrink2(Vec2::new(0.0, gutter))
}

/// The painted title is the rename target. Even a collapsed row keeps room
/// below the title for selecting and opening the clip body.
pub fn clip_title_band(rect: Rect, named: bool, pixels_per_point: f32) -> Rect {
    let rect = rect.round_to_pixels(pixels_per_point);
    let height = if named {
        tokens::FONT_CANVAS + tokens::SPACE_SM
    } else {
        3.0
    };
    // Keep at least half of a single-row event available for its musical content.
    let bottom = (rect.top() + height.min(rect.height() * 0.5)).round_to_pixels(pixels_per_point);
    Rect::from_min_max(rect.min, Pos2::new(rect.right(), bottom))
}

/// Keep the full time axis for waveform sampling, fades and automation points.
/// Callers can clip horizontal padding without shifting the musical origin.
pub fn clip_preview_rect(rect: Rect, band: Rect) -> Rect {
    let height = (rect.bottom() - band.bottom()).max(0.0);
    let inset: f32 = if rect.height() <= tokens::SPACE_XL {
        1.0
    } else {
        2.0
    };
    let top_gap = inset.min(height * 0.25);
    let bottom_gap = (inset * 1.5).min(height * 0.25);
    Rect::from_x_y_ranges(
        rect.x_range(),
        (band.bottom() + top_gap)..=(rect.bottom() - bottom_gap),
    )
}

/// Derive the preview ink and opaque body once for events sharing a track color.
#[derive(Clone, Copy)]
pub struct ClipSurfaceColors {
    track: Color32,
    body: Color32,
    ink: Color32,
}

impl ClipSurfaceColors {
    pub fn new(ctx: &egui::Context, color: Color32) -> Self {
        let colors = crate::theme::palette(ctx);
        // Limit the tint by contrast with both ends of the amplitude-colored waveform.
        let ink = crate::waveform_peak_color(color, colors);
        let peak = luminance(ink.to_opaque());
        let quiet = luminance(crate::waveform_quiet_color(color, colors.muted));
        let body_fill = [0.55, 0.40, 0.28, 0.16, 0.08]
            .into_iter()
            .map(|tint| colors.bg.blend(color.gamma_multiply(tint)))
            .find(|fill| {
                let background = luminance(*fill);
                contrast_luminance(background, peak) >= 3.0
                    && contrast_luminance(background, quiet) >= 3.0
            })
            .unwrap_or(colors.bg);
        Self {
            track: color,
            body: body_fill,
            ink,
        }
    }

    pub fn paint(
        self,
        painter: &Painter,
        rect: Rect,
        selected: bool,
        named: bool,
    ) -> ClipSurfaceLayout {
        let colors = crate::theme::palette(painter.ctx());
        let Self {
            track: color,
            body: body_fill,
            ink,
        } = self;
        let header_fill =
            colors
                .panel
                .blend(color.gamma_multiply(if selected { 0.96 } else { 0.80 }));
        // Whole physical pixels keep the rounded outline continuous where the
        // title band meets it, at any display density.
        let pixel = 1.0 / painter.pixels_per_point();
        let rect = rect.round_to_pixels(painter.pixels_per_point());
        let band = clip_title_band(rect, named, painter.pixels_per_point());
        let header_bottom = band.bottom();
        let inset_x = tokens::SPACE_XS.min(rect.width().max(0.0) * 0.08);
        let preview = clip_preview_rect(rect, band);
        let layout = ClipSurfaceLayout {
            // The name starts where the waveform or notes start.
            title: Rect::from_min_max(
                Pos2::new(rect.left() + inset_x, rect.top()),
                Pos2::new(rect.right() - inset_x, header_bottom),
            ),
            content: preview.shrink2(Vec2::new(inset_x, 0.0)),
            title_color: crate::theme::contrast_text(header_fill),
            preview_color: body_fill.blend(ink),
            name_hit: band,
        };
        if !rect.is_positive() || !rect.is_finite() {
            return layout;
        }
        if selected {
            crate::paint_selection_glow(painter, rect, tokens::CLIP_RADIUS, color);
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        painter.rect_filled(rect, tokens::CLIP_RADIUS, body_fill);
        painter.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), header_bottom)),
            CornerRadius {
                nw: tokens::CLIP_RADIUS,
                ne: tokens::CLIP_RADIUS,
                sw: 0,
                se: 0,
            },
            header_fill,
        );
        painter.rect_stroke(
            rect,
            tokens::CLIP_RADIUS,
            Stroke::new(
                if selected { 2.0 * pixel } else { pixel },
                if selected {
                    ink.to_opaque().lerp_to_gamma(colors.text, 0.65)
                } else {
                    body_fill.blend(ink.gamma_multiply(0.65))
                },
            ),
            StrokeKind::Inside,
        );
        layout
    }
}

/// Paint one MIDI thumbnail note inside a clip's dedicated preview region.
/// The caller maps source timing into canvas coordinates and supplies a common
/// pitch range for the clip. Notes can never cover its title or adjacent clips.
pub fn clip_note_preview(
    painter: &Painter,
    content: Rect,
    time: std::ops::Range<f32>,
    pitch: u8,
    pitches: std::ops::RangeInclusive<u8>,
    color: Color32,
) -> Option<Rect> {
    if !content.is_positive() || !content.is_finite() || time.end <= time.start {
        return None;
    }
    // One physical pixel keeps close intervals distinct in short previews.
    // Taller lanes use bolder strokes, with at least eight pitch rows.
    let low = f32::from(*pitches.start());
    let high = f32::from(*pitches.end());
    let span = (high - low + 2.0).max(8.0);
    let compact = content.height() < tokens::SPACE_LG;
    let pixels_per_point = painter.pixels_per_point();
    let row_height = content.height() / (span + 1.0);
    let height = if compact {
        row_height
            .round_to_pixels(pixels_per_point)
            .max(1.0 / pixels_per_point)
    } else {
        row_height.clamp(2.0, tokens::SPACE_XS + 1.0)
    }
    .min(content.height());
    let pitch_y = (0.5 + (f32::from(pitch) - (low + high) * 0.5) / span).clamp(0.0, 1.0);
    let y = content.bottom() - height - pitch_y * (content.height() - height);
    let y = if compact {
        y.round_to_pixels(pixels_per_point)
            .max(content.top())
            .min(content.bottom() - height)
    } else {
        y
    };
    let rect = Rect::from_min_size(
        Pos2::new(time.start, y),
        Vec2::new((time.end - time.start - 1.0).max(2.0), height),
    )
    .intersect(content);
    if !rect.is_positive() || !rect.intersects(painter.clip_rect()) {
        return None;
    }
    painter
        .with_clip_rect(content.intersect(painter.clip_rect()))
        .rect_filled(rect, 0.0, color);
    Some(rect)
}

#[derive(Clone, Copy, Debug)]
pub struct ClipTitleLayout {
    pub icon: Option<Rect>,
    pub text: Rect,
}

/// Paint a single-line clip name, optionally preceded by a vector chain icon.
/// Both parts stay inside the caller's rectangle and current painter clip.
/// The caller owns its hover target and full-name tooltip. Returned rectangles
/// expose the reserved geometry without introducing another interaction target.
pub fn clip_title(
    painter: &Painter,
    rect: Rect,
    font: FontId,
    color: Color32,
    title: &str,
    linked: bool,
) -> ClipTitleLayout {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let mut layout = ClipTitleLayout {
        icon: None,
        text: rect,
    };
    if title.trim().is_empty() {
        return layout;
    }
    if linked && rect.is_positive() {
        let size = rect.width().min(rect.height()).min(tokens::SPACE_LG);
        let icon = Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - size * 0.5),
            Vec2::splat(size),
        );
        Icon::Link.paint(&painter, icon, color);
        layout.icon = Some(icon);
        layout.text.min.x = (icon.right() + tokens::SPACE_XS).min(rect.right());
    }
    if layout.text.width() >= font.size && layout.text.is_positive() {
        text::paint_single_line(&painter, layout.text, font, color, title);
    }
    layout
}

/// Reveal a clip's complete name and occurrence timing without adding another
/// canvas target or changing its title geometry. Shared names remain distinct
/// by track and position; ghost source-sharing remains explicit.
pub fn clip_occurrence_hover(
    response: Response,
    title: &str,
    track: &str,
    range: std::ops::Range<f64>,
    tempo: f64,
    meter: &crate::MeterMap,
    linked: bool,
) -> Response {
    let seconds_per_beat = 60.0 / tempo;
    crate::hover_text(
        response,
        format!(
            "{title}\n{track} · {}\n{}–{}{}\nDouble-click the title to rename or the body to edit",
            crate::duration::musical_position(range.start, meter),
            crate::duration::format_seconds(range.start * seconds_per_beat),
            crate::duration::format_seconds(range.end * seconds_per_beat),
            if linked {
                "\nGhost clip · Edits and name are shared. Position and trim are independent."
            } else {
                ""
            },
        ),
    )
}

/// A read-only clip name for Properties or other bounded rows. The chain is
/// vector geometry; the native text label retains the complete accessible name.
/// Linked names explain their shared identity on hover instead of permanent text.
pub fn clip_label(ui: &mut Ui, title: &str, linked: bool) -> Response {
    if !linked {
        return text::truncated(ui, text::TextRole::Label, title);
    }
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
            let size = tokens::SPACE_LG.min(ui.available_width().max(0.0));
            let (rect, icon_response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
            Icon::Link.paint(ui.painter(), rect, crate::theme::palette(ui.ctx()).text);
            let label =
                ui.add(egui::Label::new(text::TextRole::Label.rich(ui.ctx(), title)).truncate());
            icon_response.union(label)
        })
        .inner;
    response.on_hover_text(format!("{title}\nGhost clip · edits and name are shared"))
}

/// A clip choice with visible instance metadata, so linked copies with the same
/// name remain distinguishable. Native checkbox focus and interaction are kept.
pub fn clip_choice(
    ui: &mut Ui,
    selected: &mut bool,
    title: &str,
    linked: bool,
    detail: &str,
) -> Response {
    // The detail hugs its name, indented to the checkbox label's text.
    let response = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = tokens::SPACE_XS;
            let response = crate::checkbox(ui, selected, title);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
                ui.spacing_mut().interact_size.y = tokens::COMPACT_HEIGHT;
                ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
                if linked {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::splat(tokens::SPACE_MD), Sense::hover());
                    Icon::Link.paint(ui.painter(), rect, crate::theme::palette(ui.ctx()).muted);
                }
                text::note(ui, detail);
            });
            response
        })
        .inner;
    response.on_hover_text(format!(
        "{title}\n{detail}{}",
        if linked {
            "\nGhost clip · edits and name are shared"
        } else {
            ""
        }
    ))
}
