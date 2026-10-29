//! Clip identity presentation shared by timeline canvases and property views.
//! The application decides which instances are linked and owns all editing.
use crate::{Icon, text, tokens};
use egui::{
    Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

/// Separate clip identity from its musical preview at every track height.
#[derive(Clone, Copy, Debug)]
pub struct ClipSurfaceLayout {
    pub title: Rect,
    pub content: Rect,
    pub title_color: Color32,
}

/// An opaque clip surface with a quiet track-colored title band and a soft
/// selection halo. The opaque
/// body keeps the beat grid from competing with waveforms and MIDI thumbnails.
/// This is paint only: the caller retains the clip's complete interaction target.
pub fn clip_surface(
    painter: &Painter,
    rect: Rect,
    color: Color32,
    selected: bool,
) -> ClipSurfaceLayout {
    let colors = crate::theme::palette(painter.ctx());
    let header_bottom = (rect.top() + tokens::FONT_CANVAS + tokens::SPACE_SM).min(rect.bottom());
    let inset_x = tokens::SPACE_XS.min(rect.width().max(0.0) * 0.5);
    let content_top = (header_bottom + tokens::SPACE_XS * 0.5).min(rect.bottom());
    let content_bottom = (rect.bottom() - tokens::SPACE_XS).max(content_top);
    let header_fill = colors
        .panel
        .lerp_to_gamma(color, if selected { 0.42 } else { 0.28 });
    let layout = ClipSurfaceLayout {
        title: Rect::from_min_max(
            Pos2::new(
                (rect.left() + tokens::SPACE_SM).min(rect.right()),
                rect.top(),
            ),
            Pos2::new(
                (rect.right() - inset_x).max((rect.left() + tokens::SPACE_SM).min(rect.right())),
                header_bottom,
            ),
        ),
        content: Rect::from_min_max(
            Pos2::new(rect.left() + inset_x, content_top),
            Pos2::new(rect.right() - inset_x, content_bottom),
        ),
        title_color: crate::theme::contrast_text(header_fill),
    };
    if !rect.is_positive() || !rect.is_finite() {
        return layout;
    }
    if selected {
        crate::paint_selection_glow(painter, rect, tokens::RADIUS, color);
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    painter.rect_filled(
        rect,
        tokens::RADIUS,
        colors
            .bg
            .lerp_to_gamma(color, if selected { 0.22 } else { 0.12 }),
    );
    painter.rect_filled(
        Rect::from_min_max(rect.min, Pos2::new(rect.right(), header_bottom)),
        CornerRadius {
            nw: tokens::RADIUS,
            ne: tokens::RADIUS,
            sw: 0,
            se: 0,
        },
        header_fill,
    );
    painter.rect_stroke(
        rect,
        tokens::RADIUS,
        Stroke::new(
            if selected { 1.8 } else { 0.8 },
            color.gamma_multiply(if selected { 1.0 } else { 0.65 }),
        ),
        StrokeKind::Inside,
    );
    layout
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
    let height = 2.5_f32.min(content.height());
    let low = f32::from(*pitches.start());
    let high = f32::from(*pitches.end());
    let span = (high - low).max(12.0);
    let pitch_y = (0.5 + (f32::from(pitch) - (low + high) * 0.5) / span).clamp(0.0, 1.0);
    let y = content.bottom() - height - pitch_y * (content.height() - height);
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
        .rect_filled(rect, 1.0, color.gamma_multiply(0.85));
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
    if layout.text.is_positive() {
        text::paint_single_line(&painter, layout.text, font, color, title);
    }
    layout
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
    let response = crate::checkbox(ui, selected, title);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
        ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(tokens::SPACE_LG), Sense::hover());
        if linked {
            Icon::Link.paint(ui.painter(), rect, crate::theme::palette(ui.ctx()).muted);
        }
        text::note(ui, detail);
    });
    response.on_hover_text(format!(
        "{title}\n{detail}{}",
        if linked {
            "\nGhost clip · edits and name are shared"
        } else {
            ""
        }
    ))
}
