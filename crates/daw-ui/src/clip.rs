//! Clip identity presentation shared by timeline canvases and property views.
//! The application decides which instances are linked and owns all editing.
use crate::{Icon, text, tokens};
use egui::{Color32, FontId, Painter, Rect, Response, Sense, Ui, Vec2};

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
    response.on_hover_text(format!("{title}\nLinked clip — edits and name are shared"))
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
            "\nLinked clip — edits and name are shared"
        } else {
            ""
        }
    ))
}
