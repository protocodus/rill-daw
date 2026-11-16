//! Waveform and spectrum in a dedicated sample preview area. The caller owns
//! analysis, caching, source identity, and status; painting never reads audio.
use crate::{theme, tokens};
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// Only completed analysis may be drawn. Pending or failed analysis has no
/// invented waveform, spectrum, or progress animation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioPreviewState {
    #[default]
    Empty,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioPreview<'a> {
    /// Uniform time bins covering the analyzed source, in full-scale amplitude.
    pub waveform: &'a [[f32; 2]],
    /// Actual spectral levels normalized to 0..1 at uniformly spaced log-Hz
    /// band centers. The host owns the frequency and dB ranges and aggregation.
    pub spectrum: &'a [f32],
    pub state: AudioPreviewState,
}

/// Bounds of the analysis actually painted, useful to a host's diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioPreviewGeometry {
    pub waveform: Option<Rect>,
    pub spectrum: Option<Rect>,
}

/// Paint the selected sample into its own rectangle, clipped to the viewport.
pub fn paint_audio_preview(
    painter: &Painter,
    rect: Rect,
    data: AudioPreview<'_>,
) -> AudioPreviewGeometry {
    let mut geometry = AudioPreviewGeometry::default();
    let clip = rect.intersect(painter.clip_rect());
    if data.state != AudioPreviewState::Ready || !rect.is_finite() || !rect.is_positive() {
        return geometry;
    }
    let inner = rect.shrink(tokens::SPACE_SM);
    let height = (inner.height() - tokens::SPACE_LG) * 0.5;
    if inner.width() < tokens::CONTROL_HEIGHT || height < tokens::SPACE_LG {
        return geometry;
    }
    let painter = painter.with_clip_rect(clip);
    let palette = theme::palette(painter.ctx());
    let waveform_rect = Rect::from_min_max(inner.min, pos2(inner.right(), inner.top() + height));
    let spectrum_rect = Rect::from_min_max(pos2(inner.left(), inner.bottom() - height), inner.max);
    // One point per logical column, with a fixed upper bound, is enough for a
    // detailed preview. Aggregation preserves extrema rather than skipping hits.
    let max_columns = inner.width().ceil().clamp(2.0, 512.0) as usize;
    if !data.waveform.is_empty()
        && data
            .waveform
            .iter()
            .all(|peak| peak[0].is_finite() && peak[1].is_finite() && peak[0] <= peak[1])
    {
        let count = data.waveform.len().min(max_columns).max(2);
        let mut upper = Vec::with_capacity(count);
        let mut lower = Vec::with_capacity(count);
        for column in 0..count {
            let range = bin_range(column, count, data.waveform.len());
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for peak in &data.waveform[range] {
                low = low.min(peak[0]);
                high = high.max(peak[1]);
            }
            let x =
                waveform_rect.left() + waveform_rect.width() * column as f32 / (count - 1) as f32;
            let y = |amplitude: f32| {
                waveform_rect.center().y - amplitude.clamp(-1.0, 1.0) * waveform_rect.height() * 0.5
            };
            upper.push(pos2(x, y(high)));
            lower.push(pos2(x, y(low)));
        }
        fill_between(&painter, &upper, &lower, palette.action);
        painter.add(Shape::line(upper, Stroke::new(1.0, palette.action)));
        painter.add(Shape::line(lower, Stroke::new(1.0, palette.action)));
        geometry.waveform = Some(waveform_rect);
    }
    if !data.spectrum.is_empty() && data.spectrum.iter().all(|level| level.is_finite()) {
        let count = data.spectrum.len().min(max_columns).max(2);
        let mut trace = Vec::with_capacity(count);
        let mut baseline = Vec::with_capacity(count);
        for column in 0..count {
            let range = bin_range(column, count, data.spectrum.len());
            let level = data.spectrum[range]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max)
                .clamp(0.0, 1.0);
            let x =
                spectrum_rect.left() + spectrum_rect.width() * column as f32 / (count - 1) as f32;
            trace.push(pos2(
                x,
                spectrum_rect.bottom() - level * spectrum_rect.height(),
            ));
            baseline.push(pos2(x, spectrum_rect.bottom()));
        }
        fill_between(&painter, &trace, &baseline, palette.muted);
        painter.add(Shape::line(trace, Stroke::new(1.0, palette.muted)));
        geometry.spectrum = Some(spectrum_rect);
    }
    geometry
}

fn bin_range(column: usize, columns: usize, length: usize) -> std::ops::Range<usize> {
    let start = (column * length / columns).min(length - 1);
    let end = ((column + 1) * length / columns).max(start + 1).min(length);
    start..end
}

fn fill_between(painter: &Painter, upper: &[Pos2], lower: &[Pos2], color: Color32) {
    let mut mesh = Mesh::default();
    for (index, (top, bottom)) in upper.iter().zip(lower).enumerate() {
        mesh.colored_vertex(*top, color);
        mesh.colored_vertex(*bottom, color);
        if index > 0 {
            let at = index as u32 * 2;
            mesh.add_triangle(at - 2, at - 1, at);
            mesh.add_triangle(at - 1, at + 1, at);
        }
    }
    painter.add(Shape::mesh(mesh));
}
