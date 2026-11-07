//! Read-only indication of an audio source's admitted storage mode.
//! The caller obtains metadata off the UI thread and decides when it applies.
use egui::emath::GuiRounding as _;
use egui::{Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

pub const DISK_STREAMED_AUDIO: &str = "Disk-streamed audio";
pub const AUDIO_STREAMING_BADGE_SIZE: f32 = 16.0;

/// Paint a quiet drive badge inside a clip without allocating an interaction
/// target or altering its title, waveform, rename, edge or fade geometry.
/// The bottom corner keeps the upper fade handles clear; the side inset leaves
/// the ten-point trim target unobscured. Tiny or clipped badges are omitted.
/// Return the actual painted enclosure for canvas diagnostics and hover tests.
pub fn paint_audio_streaming_badge(painter: &Painter, clip: Rect) -> Option<Rect> {
    paint_audio_streaming_badge_avoiding(painter, clip, &[])
}

/// Keep the badge clear of existing fade/rename geometry supplied by the
/// canvas. This remains paint only, so the underlying controls retain their
/// interaction ownership even when space is too tight for a badge.
pub fn paint_audio_streaming_badge_avoiding(
    painter: &Painter,
    clip: Rect,
    excluded: &[Rect],
) -> Option<Rect> {
    let side = AUDIO_STREAMING_BADGE_SIZE;
    let right_inset = crate::tokens::SPACE_MD;
    let bottom_inset = crate::tokens::SPACE_XS;
    if !clip.is_finite()
        || clip.width() < side + right_inset * 2.0
        || clip.height() < side + bottom_inset * 2.0
    {
        return None;
    }
    let visible = clip.intersect(painter.clip_rect());
    let mut right = (clip.right() - right_inset).min(visible.right() - bottom_inset);
    let left_bound = (clip.left() + right_inset).max(visible.left() + bottom_inset);
    let mut placement = None;
    // Never scan through an arbitrarily long offscreen clip. Only a few
    // visible candidates are needed to avoid the two fade handles and name.
    for _ in 0..8 {
        let candidate = Rect::from_min_size(
            Pos2::new(right - side, clip.bottom() - bottom_inset - side),
            Vec2::splat(side),
        )
        .round_to_pixels(painter.pixels_per_point());
        if candidate.left() < left_bound {
            break;
        }
        if painter.clip_rect().contains_rect(candidate)
            && !excluded
                .iter()
                .any(|rect| rect.is_finite() && rect.intersects(candidate.expand(1.0)))
        {
            placement = Some(candidate);
            break;
        }
        right -= side + crate::tokens::SPACE_XS;
    }
    let rect = placement?;
    let colors = crate::theme::palette(painter.ctx());
    painter.rect(
        rect,
        crate::tokens::RADIUS,
        colors.panel,
        Stroke::new(
            1.0 / painter.pixels_per_point(),
            colors.muted.gamma_multiply(0.6),
        ),
        StrokeKind::Inside,
    );
    crate::Icon::Disk.paint(painter, rect.shrink(2.0), colors.text);
    Some(rect)
}

/// The same source mode in Properties, with a normal accessible text label.
pub fn audio_streaming_status(ui: &mut Ui) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
        let (rect, icon) =
            ui.allocate_exact_size(Vec2::splat(crate::tokens::SPACE_LG), Sense::hover());
        crate::Icon::Disk.paint(ui.painter(), rect, crate::theme::palette(ui.ctx()).text);
        icon.union(crate::text::label(ui, DISK_STREAMED_AUDIO))
    })
    .inner
    .on_hover_text(
        "The source is read from disk in bounded windows; it is not fully decoded into memory.",
    )
}
