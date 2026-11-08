//! Audio-region visualization shared by clip editors and other timeline views.

use crate::{
    TimelineViewport,
    color_math::{contrast_luminance, luminance},
    theme,
};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use std::sync::Arc;

/// Presentation only: Fit uses one full-clip reference for both channels,
/// leaving the clip's gain, fades and audio data unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveformAmplitudeScale {
    #[default]
    Default,
    Fit,
}

/// A shared view setting, suitable for a View menu. The caller owns its lifetime
/// and the waveform's amplitude reference; this control never edits audio.
pub fn waveform_amplitude_control(ui: &mut egui::Ui, scale: &mut WaveformAmplitudeScale) {
    ui.label("Waveform amplitude");
    ui.horizontal(|ui| {
        crate::selectable_value(ui, scale, WaveformAmplitudeScale::Default, "Default")
            .on_hover_text("Show waveforms at their original amplitude scale");
        crate::selectable_value(ui, scale, WaveformAmplitudeScale::Fit, "Fit").on_hover_text(
            "Fit each waveform to its clip height. Audio gain and playback stay unchanged.",
        );
    });
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioClipEdges {
    pub start: Option<Rect>,
    pub end: Option<Rect>,
}

fn visible_range(rect: Rect, view: TimelineViewport, start: f64, end: f64) -> Option<(f32, f32)> {
    if !rect.is_finite()
        || !rect.is_positive()
        || !start.is_finite()
        || !end.is_finite()
        || end <= start
        || !view.start_beat.is_finite()
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
    {
        return None;
    }
    let (left, right) = (view.x_at(rect, start), view.x_at(rect, end));
    (left.is_finite() && right.is_finite() && right >= rect.left() && left <= rect.right())
        .then_some((left, right))
}

/// A time selection shared by sample editors and comping views. Clipping never
/// turns an offscreen range boundary into a visible, draggable boundary.
pub fn paint_timeline_selection(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
) -> Option<Rect> {
    let (left, right) = visible_range(rect, view, start, end)?;
    let visible = Rect::from_min_max(
        egui::pos2(left.max(rect.left()), rect.top()),
        egui::pos2(right.min(rect.right()), rect.bottom()),
    )
    .intersect(painter.clip_rect());
    if !visible.is_positive() {
        return None;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let color = theme::palette(painter.ctx()).accent;
    painter.rect_filled(visible, 0.0, color.gamma_multiply(0.14));
    for x in [left, right] {
        if x >= rect.left() && x <= rect.right() {
            painter.vline(x, rect.y_range(), Stroke::new(1.0_f32, color));
        }
    }
    Some(visible)
}

/// Visible trim handles, with comfortable eight-point targets. The application
/// owns gesture priority and editing; this component only paints and measures.
pub fn paint_audio_clip_edges(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
    color: Color32,
) -> AudioClipEdges {
    let Some((left, right)) = visible_range(rect, view, start, end) else {
        return AudioClipEdges::default();
    };
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let edge = |x: f32| {
        if x < rect.left() || x > rect.right() {
            return None;
        }
        let hit = Rect::from_min_max(
            egui::pos2((x - 4.0).max(rect.left()), rect.top()),
            egui::pos2((x + 4.0).min(rect.right()), rect.bottom()),
        );
        painter.vline(x, rect.y_range(), Stroke::new(1.5_f32, color));
        painter.rect_filled(
            Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(4.0, 28.0))
                .intersect(rect),
            2.0,
            color,
        );
        Some(hit)
    };
    AudioClipEdges {
        start: edge(left),
        end: edge(right),
    }
}

#[derive(Clone, Copy)]
pub struct WaveformColumn {
    pub beat: f64,
    pub channel: usize,
    pub minimum: f32,
    pub maximum: f32,
}

/// Keep the saturated clip hue visible against waveform surfaces. Adjust its
/// brightness only when needed; darken in Light and lift dark colors in Dark.
/// A small contrast margin accounts for integer color rounding. Preserve alpha.
pub fn waveform_peak_color(color: Color32, palette: theme::Palette) -> Color32 {
    let opaque = color.to_opaque();
    let background = luminance(palette.bg);
    let raised = luminance(palette.raised);
    let readable = |candidate| {
        let foreground = luminance(candidate);
        contrast_luminance(foreground, background) >= 3.2
            && contrast_luminance(foreground, raised) >= 3.2
    };
    if readable(opaque) {
        return color;
    }
    let target = if palette.bg.intensity() > 0.5 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..12 {
        let amount = (low + high) * 0.5;
        if readable(opaque.lerp_to_gamma(target, amount)) {
            high = amount;
        } else {
            low = amount;
        }
    }
    opaque
        .lerp_to_gamma(target, high)
        .gamma_multiply(f32::from(color.a()) / 255.0)
}

/// A readable, desaturated endpoint for quiet audio. The neutral theme text
/// color supplies contrast; a small amount of the clip hue keeps its identity.
/// Alpha is applied once by `waveform_amplitude_color`.
pub fn waveform_quiet_color(color: Color32, neutral: Color32) -> Color32 {
    neutral.to_opaque().lerp_to_gamma(color.to_opaque(), 0.12)
}

/// Colors and display normalization are independent: the center line stays
/// quiet while the low-amplitude envelope retains readable theme contrast.
#[derive(Clone, Copy, Debug)]
pub struct WaveformColorStyle {
    pub peak: Color32,
    pub quiet: Color32,
    pub center: Color32,
    pub amplitude_reference: f64,
    pub amplitude_coloring: bool,
}

/// Desaturated at silence, smoothly approaching the clip's color at full-scale
/// audio amplitude. Preserve the clip's alpha so quiet neighbouring previews stay quiet.
pub fn waveform_amplitude_color(color: Color32, neutral: Color32, amplitude: f32) -> Color32 {
    let amount = if amplitude.is_finite() {
        amplitude.clamp(0.0, 1.0)
    } else {
        0.0
    };
    neutral
        .gamma_multiply(f32::from(color.a()) / 255.0)
        .lerp_to_gamma(color, amount)
}

/// The caller supplies bounded peak columns in musical time. Each channel uses
/// two positive rows; `headroom` is the amplitude represented by one row. Keeping
/// that reference until Fit avoids channel-center jumps during gain edits.
pub fn paint_audio_waveform_raw(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
) {
    paint_audio_waveform_raw_with_coloring(
        painter, rect, view, columns, channels, headroom, color, 1.0, true,
    );
}

/// Immediate fallback with the same optional amplitude colors as the cached mesh.
#[allow(clippy::too_many_arguments)]
pub fn paint_audio_waveform_raw_with_coloring(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
    amplitude_reference: f64,
    amplitude_coloring: bool,
) {
    let palette = theme::palette(painter.ctx());
    paint_audio_waveform_styled(
        painter,
        rect,
        view,
        columns,
        channels,
        headroom,
        WaveformColorStyle {
            peak: color,
            quiet: waveform_quiet_color(color, palette.muted),
            center: palette.line,
            amplitude_reference,
            amplitude_coloring,
        },
    );
}

/// Immediate fallback with explicit colors matching the cached envelope.
#[allow(clippy::too_many_arguments)]
pub fn paint_audio_waveform_styled(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    style: WaveformColorStyle,
) {
    let WaveformColorStyle {
        peak: color,
        quiet: quiet_color,
        center: center_color,
        amplitude_reference,
        amplitude_coloring,
    } = style;
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let headroom = headroom.max(0.001);
    for channel in 0..channels {
        let center = view.y_at(rect, (channel * 2 + 1) as f32);
        painter.line_segment(
            [
                Pos2::new(rect.left(), center),
                Pos2::new(rect.right(), center),
            ],
            Stroke::new(1.0_f32, center_color),
        );
    }
    for column in columns {
        let x = view.x_at(rect, column.beat);
        let center = (column.channel * 2 + 1) as f32;
        painter.line_segment(
            [
                Pos2::new(x, view.y_at(rect, center - column.maximum / headroom)),
                Pos2::new(x, view.y_at(rect, center - column.minimum / headroom)),
            ],
            Stroke::new(
                1.0_f32,
                if amplitude_coloring {
                    waveform_amplitude_color(
                        color,
                        quiet_color,
                        (f64::from(column.minimum.abs().max(column.maximum.abs()))
                            * amplitude_reference)
                            .clamp(0.0, 1.0) as f32,
                    )
                } else {
                    color
                },
            ),
        );
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WaveformMeshStats {
    pub vertices: usize,
    pub indices: usize,
    pub fringe_vertices: usize,
}

/// A reusable, already-built envelope. Repainting clones only small shape
/// handles; the sample data and mesh vertices are never walked again.
#[derive(Clone)]
pub struct PreparedAudioWaveform {
    rect: Rect,
    shapes: Vec<Shape>,
    pub stats: WaveformMeshStats,
}

impl PreparedAudioWaveform {
    pub fn paint(&self, painter: &Painter) {
        let painter = painter.with_clip_rect(self.rect.intersect(painter.clip_rect()));
        painter.extend(self.shapes.iter().cloned());
    }

    pub fn bytes(&self) -> usize {
        self.stats.vertices * std::mem::size_of::<egui::epaint::Vertex>()
            + self.stats.indices * std::mem::size_of::<u32>()
    }
}

/// Build a continuous peak envelope with a one-physical-pixel coverage fringe.
/// The fringe follows the edge normals rather than blurring amplitudes or
/// averaging transients. A subpixel signal remains a thin, antialiased trace.
/// Callers bound the number of columns to their visible pixel budget and may
/// cache this result until the viewport, source, or clip edits change.
#[allow(clippy::too_many_arguments)]
pub fn prepare_audio_waveform(
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
    pixels_per_point: f32,
    center_color: Color32,
) -> PreparedAudioWaveform {
    prepare_audio_waveform_with_coloring(
        rect,
        view,
        columns,
        channels,
        headroom,
        color,
        pixels_per_point,
        center_color,
        1.0,
        true,
    )
}

/// Color mode only changes existing vertex colors; geometry and draw count stay
/// identical. `amplitude_reference` restores any display normalization applied to
/// columns, so colors follow actual audio level independently of Fit or headroom.
/// The caller includes the color mode among its prepared-geometry cache keys.
#[allow(clippy::too_many_arguments)]
pub fn prepare_audio_waveform_with_coloring(
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
    pixels_per_point: f32,
    center_color: Color32,
    amplitude_reference: f64,
    amplitude_coloring: bool,
) -> PreparedAudioWaveform {
    // Keep the original context-free API compatible. Its center-line tone
    // identifies the standard light/dark palette; explicit styles below also
    // support callers with custom colors without changing the center line.
    let neutral = if center_color.intensity() > 0.5 {
        theme::LIGHT.muted
    } else {
        theme::DARK.muted
    };
    prepare_audio_waveform_styled(
        rect,
        view,
        columns,
        channels,
        headroom,
        pixels_per_point,
        WaveformColorStyle {
            peak: color,
            quiet: waveform_quiet_color(color, neutral),
            center: center_color,
            amplitude_reference,
            amplitude_coloring,
        },
    )
}

/// Prepare an envelope with an explicit, theme-aware quiet endpoint. Existing
/// peak positions, physical-pixel fringes and shared mesh storage are unchanged.
#[allow(clippy::too_many_arguments)]
pub fn prepare_audio_waveform_styled(
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    pixels_per_point: f32,
    style: WaveformColorStyle,
) -> PreparedAudioWaveform {
    let WaveformColorStyle {
        peak: color,
        quiet: quiet_color,
        center: center_color,
        amplitude_reference,
        amplitude_coloring,
    } = style;
    let mut result = PreparedAudioWaveform {
        rect,
        shapes: Vec::new(),
        stats: WaveformMeshStats::default(),
    };
    if !rect.is_finite()
        || !rect.is_positive()
        || !view.row_height.is_finite()
        || view.row_height <= 0.0
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
        || !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
    {
        return result;
    }
    let fringe = pixels_per_point.recip();
    let headroom = if headroom.is_finite() {
        headroom.max(0.001)
    } else {
        1.0
    };
    let mut mesh = egui::epaint::Mesh::default();
    mesh.vertices.reserve(columns.len() * 4 + 8);
    mesh.indices.reserve(columns.len() * 18);
    for channel in 0..channels.min(2) {
        let row = (channel * 2 + 1) as f32;
        let center = view.y_at(rect, row);
        result.shapes.push(Shape::line_segment(
            [
                Pos2::new(rect.left(), center),
                Pos2::new(rect.right(), center),
            ],
            Stroke::new(fringe, center_color),
        ));
        let mut edges: Vec<(Pos2, Pos2, Color32)> =
            Vec::with_capacity(columns.len() / channels.max(1));
        for column in columns.iter().filter(|column| column.channel == channel) {
            let x = view.x_at(rect, column.beat);
            if !x.is_finite() || !column.minimum.is_finite() || !column.maximum.is_finite() {
                continue;
            }
            if edges.last().is_some_and(|(top, _, _)| x <= top.x) {
                continue;
            }
            let mut top = view
                .y_at(rect, row - column.maximum / headroom)
                .clamp(rect.top() - fringe, rect.bottom() + fringe);
            let mut bottom = view
                .y_at(rect, row - column.minimum / headroom)
                .clamp(rect.top() - fringe, rect.bottom() + fringe);
            if !top.is_finite() || !bottom.is_finite() {
                continue;
            }
            if top > bottom {
                std::mem::swap(&mut top, &mut bottom);
            }
            if bottom - top < fringe {
                let middle = (top + bottom) * 0.5;
                top = middle - fringe * 0.5;
                bottom = middle + fringe * 0.5;
            }
            let fill = if amplitude_coloring {
                waveform_amplitude_color(
                    color,
                    quiet_color,
                    (f64::from(column.minimum.abs().max(column.maximum.abs()))
                        * amplitude_reference)
                        .clamp(0.0, 1.0) as f32,
                )
            } else {
                color
            };
            edges.push((Pos2::new(x, top), Pos2::new(x, bottom), fill));
        }
        if edges.is_empty() {
            continue;
        }
        // A one-column region still has a visible, physical-pixel-wide trace.
        if edges.len() == 1 {
            let (top, bottom, fill) = edges[0];
            edges[0].0.x -= fringe * 0.5;
            edges[0].1.x -= fringe * 0.5;
            edges.push((
                top + egui::vec2(fringe * 0.5, 0.0),
                bottom + egui::vec2(fringe * 0.5, 0.0),
                fill,
            ));
        }
        let vertex_offset = mesh.vertices.len() as u32;
        for (index, &(top, bottom, fill)) in edges.iter().enumerate() {
            let previous = edges[index.saturating_sub(1)];
            let next = edges[(index + 1).min(edges.len() - 1)];
            let normal = |a: Pos2, b: Pos2, upper: bool| {
                let tangent = b - a;
                let direction = if upper {
                    egui::vec2(tangent.y, -tangent.x)
                } else {
                    egui::vec2(-tangent.y, tangent.x)
                };
                direction.normalized() * fringe
            };
            for (position, fill) in [
                (top + normal(previous.0, next.0, true), Color32::TRANSPARENT),
                (top, fill),
                (bottom, fill),
                (
                    bottom + normal(previous.1, next.1, false),
                    Color32::TRANSPARENT,
                ),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: position,
                    uv: egui::epaint::WHITE_UV,
                    color: fill,
                });
            }
            if index > 0 {
                let previous = vertex_offset + (index as u32 - 1) * 4;
                let current = vertex_offset + index as u32 * 4;
                for band in 0..3 {
                    mesh.indices.extend_from_slice(&[
                        previous + band,
                        current + band,
                        current + band + 1,
                        previous + band,
                        current + band + 1,
                        previous + band + 1,
                    ]);
                }
            }
        }
        result.stats.fringe_vertices += edges.len() * 2;
    }
    result.stats.vertices = mesh.vertices.len();
    result.stats.indices = mesh.indices.len();
    if !mesh.vertices.is_empty() {
        result.shapes.push(Shape::Mesh(Arc::new(mesh)));
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub fn paint_audio_waveform(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
) {
    let palette = theme::palette(painter.ctx());
    prepare_audio_waveform_styled(
        rect,
        view,
        columns,
        channels,
        headroom,
        painter.ctx().pixels_per_point(),
        WaveformColorStyle {
            peak: waveform_peak_color(color, palette),
            quiet: waveform_quiet_color(color, palette.muted),
            center: palette.line,
            amplitude_reference: 1.0,
            amplitude_coloring: true,
        },
    )
    .paint(painter);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_hides_offscreen_handles_without_losing_visible_selection() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            let rect = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 160.0));
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(rect);
            let view = TimelineViewport {
                start_beat: 4.0,
                pixels_per_beat: 50.0,
                ..Default::default()
            };
            let edges = paint_audio_clip_edges(&painter, rect, view, 2.0, 8.0, Color32::WHITE);
            assert!(edges.start.is_none());
            let end = edges.end.unwrap();
            assert_eq!(end.center().x, 300.0);
            assert_eq!(end.width(), 8.0);
            let selection = paint_timeline_selection(&painter, rect, view, 2.0, 8.0).unwrap();
            assert_eq!(selection.left(), rect.left());
            assert_eq!(selection.right(), 300.0);
            assert!(paint_timeline_selection(&painter, rect, view, 0.0, 1.0).is_none());
            assert!(paint_timeline_selection(&painter, rect, view, f64::NAN, 8.0).is_none());
            let edges = paint_audio_clip_edges(&painter, rect, view, 14.0, 18.0, Color32::WHITE);
            assert!(edges.start.is_none() && edges.end.is_none());
        });
    }
}
