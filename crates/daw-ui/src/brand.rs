//! Rill identity shared by native windows, application chrome and About.
use crate::{text::TextRole, theme, tokens};
use egui::{ColorImage, Context, IconData, Response, TextureHandle, Ui};
use std::sync::{Arc, OnceLock};

pub const APP_ID: &str = "dev.rill.daw";
const ICON: &[u8] = include_bytes!("../assets/brand/rill-icon-256.png");
const COMPACT_ICON: &[u8] = include_bytes!("../assets/brand/rill-icon-64.png");

/// Trusted embedded artwork is decoded once, before creating a native window.
pub fn app_icon() -> Arc<IconData> {
    static ICON_DATA: OnceLock<Arc<IconData>> = OnceLock::new();
    Arc::clone(ICON_DATA.get_or_init(|| {
        let image = image::load_from_memory(ICON)
            .expect("validated embedded Rill icon")
            .into_rgba8();
        Arc::new(IconData {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    }))
}

fn texture(ctx: &Context, name: &'static str, bytes: &[u8]) -> TextureHandle {
    let id = egui::Id::new("rill_brand_texture").with(name);
    if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let image = image::load_from_memory(bytes)
        .expect("validated embedded Rill artwork")
        .into_rgba8();
    let image = ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// Compact identity uses the embedded Inter font, rather than shrinking raster
/// lettering. The icon texture is retained per context and stays crisp at 2×.
#[derive(Clone, Copy)]
pub enum BrandSize {
    Compact,
    Hero,
}

pub fn brand_lockup(ui: &mut Ui, size: BrandSize) -> Response {
    let (name, bytes, icon_size, font_size, spacing, tracking) = match size {
        BrandSize::Compact => (
            "rill_compact_icon",
            COMPACT_ICON,
            26.0,
            tokens::FONT_BODY,
            tokens::SPACE_XS,
            1.4,
        ),
        BrandSize::Hero => ("rill_large_icon", ICON, 96.0, 44.0, tokens::SPACE_MD, 4.2),
    };
    let icon = texture(ui.ctx(), name, bytes);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        ui.add(
            egui::Image::new((icon.id(), egui::Vec2::splat(icon_size))).alt_text("RILL app icon"),
        );
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "RILL",
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::new(font_size, TextRole::Label.font().family),
                color: theme::palette(ui.ctx()).text,
                extra_letter_spacing: tracking,
                ..Default::default()
            },
        );
        ui.add(egui::Label::new(job).selectable(false));
    })
    .response
    .on_hover_text("RILL · Audio workstation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_native_icon_is_visible_transparent_and_decoded_once() {
        let icon = app_icon();
        assert!(Arc::ptr_eq(&icon, &app_icon()));
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        let pixels = icon.rgba.as_chunks::<4>().0;
        assert_eq!(pixels[0][3], 0, "outside the tile is transparent");
        assert!(pixels.iter().filter(|pixel| pixel[3] > 250).count() > pixels.len() / 2);
        assert!(
            pixels
                .iter()
                .any(|pixel| pixel[1] > pixel[0].saturating_add(20) && pixel[3] > 250)
        );
    }

    #[test]
    fn artwork_and_wordmark_follow_theme_and_cache_textures_per_context() {
        let ctx = Context::default();
        theme::apply(&ctx);
        for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
            theme::set_appearance(&ctx, appearance);
            let mut texture_ids = Vec::new();
            for frame in 0..2 {
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let lockup = brand_lockup(ui, BrandSize::Compact);
                        assert!(lockup.rect.width() < 110.0);
                        assert!(lockup.rect.height() <= tokens::CONTROL_HEIGHT);
                        brand_lockup(ui, BrandSize::Hero);
                        texture_ids.push(texture(ctx, "rill_compact_icon", COMPACT_ICON).id());
                    });
                });
                let wordmark = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == "RILL" => Some(text),
                        _ => None,
                    })
                    .expect("live-font wordmark is painted");
                assert_eq!(
                    wordmark.galley.job.sections[0].format.color,
                    theme::palette(&ctx).text
                );
                if frame > 0 {
                    assert!(
                        output
                            .textures_delta
                            .set
                            .iter()
                            .all(|(id, _)| *id == egui::TextureId::Managed(0)),
                        "brand textures must not upload every frame"
                    );
                }
            }
            assert_eq!(texture_ids[0], texture_ids[1]);
        }
    }
}
