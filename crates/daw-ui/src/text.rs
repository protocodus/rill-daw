//! Shared typography. Text wraps by default; bounded rows opt into ellipsis and
//! retain their complete value in a tooltip. Sizes are logical points, so egui
//! rasterizes the fonts at the display's native pixel density.
use crate::{theme, tokens};
use egui::{FontData, FontDefinitions, FontFamily, FontId, Response, RichText, Ui};

const SEMIBOLD: &str = "Rill Semibold";

/// All fonts are embedded: the same project remains readable offline and in
/// containers without depending on the operating system's installed fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "Inter Regular",
            include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice(),
        ),
        (
            "Inter Semibold",
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        ),
        (
            "Noto Sans CJK",
            include_bytes!("../assets/fonts/NotoSansCJKjp-Regular.otf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
    }
    let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    proportional.insert(0, "Inter Regular".into());
    proportional.push("Noto Sans CJK".into());
    let mut semibold = proportional.clone();
    semibold[0] = "Inter Semibold".into();
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    // Keep Hack first for aligned numeric readouts; retain egui's outline emoji
    // fallbacks in both families. CJK comes last and is loaded only when needed.
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .push("Noto Sans CJK".into());
    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Title,
    Subtitle,
    Body,
    Label,
    Note,
}

impl TextRole {
    pub fn font(self) -> FontId {
        let size = match self {
            Self::Title => tokens::FONT_HEADING,
            Self::Subtitle => tokens::FONT_SUBTITLE,
            Self::Body | Self::Label => tokens::FONT_BODY,
            Self::Note => tokens::FONT_SMALL,
        };
        let family = match self {
            Self::Title | Self::Subtitle | Self::Label => FontFamily::Name(SEMIBOLD.into()),
            Self::Body | Self::Note => FontFamily::Proportional,
        };
        FontId::new(size, family)
    }

    pub fn rich(self, ctx: &egui::Context, text: impl Into<String>) -> RichText {
        RichText::new(text)
            .font(self.font())
            .color(if self == Self::Note {
                theme::palette(ctx).muted
            } else {
                theme::palette(ctx).text
            })
    }
}

/// A block expands vertically to fit its content, including long words and
/// Unicode. Its parent owns spacing, so adjacent controls keep shared padding.
pub fn block(ui: &mut Ui, role: TextRole, value: impl Into<String>) -> Response {
    ui.add(egui::Label::new(role.rich(ui.ctx(), value)).wrap())
}

/// Use only where a fixed single row is intentional (names in a catalog, etc.).
pub fn truncated(ui: &mut Ui, role: TextRole, value: impl Into<String>) -> Response {
    let value = value.into();
    ui.add(egui::Label::new(role.rich(ui.ctx(), &value)).truncate())
        .on_hover_text(value)
}

/// Single-line canvas text with an explicit width budget. Ellipsis replaces
/// clipping through a glyph; the full source text remains in the layout job.
pub fn paint_row(painter: &egui::Painter, rect: egui::Rect, role: TextRole, value: &str) {
    paint_single_line(
        painter,
        rect,
        role.font(),
        if role == TextRole::Note {
            theme::palette(painter.ctx()).muted
        } else {
            theme::palette(painter.ctx()).text
        },
        value,
    );
}

pub fn paint_single_line(
    painter: &egui::Painter,
    rect: egui::Rect,
    font: FontId,
    color: egui::Color32,
    value: &str,
) {
    let mut job =
        egui::text::LayoutJob::simple(value.to_owned(), font, color, rect.width().max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    if galley.size().y <= rect.height() {
        painter.with_clip_rect(rect).galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
    }
}

pub fn title(ui: &mut Ui, value: impl Into<String>) -> Response {
    block(ui, TextRole::Title, value)
}
pub fn subtitle(ui: &mut Ui, value: impl Into<String>) -> Response {
    block(ui, TextRole::Subtitle, value)
}
pub fn body(ui: &mut Ui, value: impl Into<String>) -> Response {
    block(ui, TextRole::Body, value)
}
pub fn label(ui: &mut Ui, value: impl Into<String>) -> Response {
    block(ui, TextRole::Label, value)
}
pub fn note(ui: &mut Ui, value: impl Into<String>) -> Response {
    block(ui, TextRole::Note, value)
}
