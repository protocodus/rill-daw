//! Configurable source-spectrum backdrops. Build/upload each texture once; painting
//! clips a single quad and never walks the spectrum or changes waveform data.
use egui::{Color32, ColorImage, Painter, Rect, TextureId, pos2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpectrumPalette {
    #[default]
    Monochrome,
    BlueYellow,
}

impl SpectrumPalette {
    pub fn label(self) -> &'static str {
        match self {
            Self::Monochrome => "Monochrome",
            Self::BlueYellow => "Blue–yellow",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Monochrome => 0,
            Self::BlueYellow => 1,
        }
    }
}

/// Presentation settings shared by every source-spectrum surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectrumViewOptions {
    pub opacity: f32,
    pub palette: SpectrumPalette,
}

impl Default for SpectrumViewOptions {
    fn default() -> Self {
        Self {
            opacity: 72.0 / 255.0,
            palette: SpectrumPalette::Monochrome,
        }
    }
}

pub struct SpectrumViewResponse {
    pub intensity: egui::Response,
    pub palette: egui::Response,
}

/// A slider for broad changes and a percent field for exact visual intensity.
/// Palette and opacity change presentation, never source audio or clip gain.
pub fn spectrum_view_controls(
    ui: &mut egui::Ui,
    options: &mut SpectrumViewOptions,
) -> SpectrumViewResponse {
    ui.label("Spectrum");
    let mut intensity = options.opacity * 100.0;
    let mut slider_changed = false;
    let number = ui.horizontal(|ui| {
        ui.label("Intensity");
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = 100.0;
            slider_changed = ui
                .add(crate::slider(&mut intensity, 0.0..=100.0).show_value(false))
                .changed();
        });
        ui.add(
            crate::number(&mut intensity)
                .range(0.0..=100.0)
                .speed(1.0)
                .fixed_decimals(0)
                .suffix(" %"),
        )
        .on_hover_text("Spectrum strength in all waveform views. Zero hides it; audio and waveforms stay unchanged.")
    });
    if ui.is_enabled() && (number.inner.changed() || slider_changed) && intensity.is_finite() {
        options.opacity = intensity / 100.0;
    }
    ui.label("Palette");
    let palette = ui.horizontal(|ui| {
        let mono = crate::selectable_value(
            ui,
            &mut options.palette,
            SpectrumPalette::Monochrome,
            "Monochrome",
        );
        let color = crate::selectable_value(
            ui,
            &mut options.palette,
            SpectrumPalette::BlueYellow,
            "Blue–yellow",
        );
        mono.union(color)
    });
    SpectrumViewResponse {
        intensity: number.inner,
        palette: palette.inner,
    }
}

/// Build a normalized-alpha texture for a cached palette. Opacity is supplied
/// by paint tint, so changing intensity never rebuilds or uploads the texture.
pub fn spectrogram_palette_image(
    columns: usize,
    bands: usize,
    levels: &[u8],
    palette: SpectrumPalette,
) -> Option<ColorImage> {
    if columns == 0
        || bands == 0
        || columns > 2048
        || bands > 128
        || columns.checked_mul(bands)? != levels.len()
    {
        return None;
    }
    let mut pixels = vec![Color32::TRANSPARENT; levels.len()];
    for column in 0..columns {
        for band in 0..bands {
            let level = f32::from(levels[column * bands + band]) / 255.0;
            let alpha = (level.powf(1.6) * 255.0).round() as u8;
            let rgb = match palette {
                SpectrumPalette::Monochrome => [255, 255, 255],
                SpectrumPalette::BlueYellow => {
                    let (a, b, t) = if level < 0.45 {
                        ([25.0, 65.0, 190.0], [35.0, 175.0, 215.0], level / 0.45)
                    } else {
                        (
                            [35.0, 175.0, 215.0],
                            [255.0, 225.0, 75.0],
                            (level - 0.45) / 0.55,
                        )
                    };
                    std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * t).round() as u8)
                }
            };
            pixels[(bands - 1 - band) * columns + column] =
                Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha);
        }
    }
    Some(ColorImage::new([columns, bands], pixels))
}

/// Convert time-major low-to-high log bands into a top-high texture. Call from
/// background work, then upload once on the UI thread. Fixed faint alpha leaves
/// waveforms, selection and edit handles visually dominant in either theme.
pub fn spectrogram_image(columns: usize, bands: usize, levels: &[u8]) -> Option<ColorImage> {
    if columns == 0
        || bands == 0
        || columns > 2048
        || bands > 128
        || columns.checked_mul(bands)? != levels.len()
    {
        return None;
    }
    let mut pixels = vec![Color32::TRANSPARENT; levels.len()];
    for column in 0..columns {
        for band in 0..bands {
            let intensity = f32::from(levels[column * bands + band]) / 255.0;
            let alpha = (intensity.powf(1.6) * 72.0).round() as u8;
            pixels[(bands - 1 - band) * columns + column] = Color32::from_white_alpha(alpha);
        }
    }
    Some(ColorImage::new([columns, bands], pixels))
}

#[derive(Clone, Copy, Debug)]
pub struct SpectrogramSource {
    pub texture: TextureId,
    pub frames: usize,
    pub min_frequency: f32,
    pub max_frequency: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpectrogramPaint {
    pub rect: Rect,
    pub uv: Rect,
}

/// `source_range` maps the rectangle's left/right edges to source frames. It
/// may descend for reverse playback or exceed source bounds after trimming.
/// A pitch change shifts log frequencies vertically; unavailable frequencies
/// remain clear. Time stretching is represented solely by source_range.
pub fn paint_audio_spectrogram(
    painter: &Painter,
    rect: Rect,
    source: SpectrogramSource,
    source_range: (f64, f64),
    transpose_semitones: f32,
    tint: Color32,
) -> Option<SpectrogramPaint> {
    if !rect.is_finite()
        || !rect.is_positive()
        || source.frames == 0
        || !source_range.0.is_finite()
        || !source_range.1.is_finite()
        || source_range.0 == source_range.1
        || !transpose_semitones.is_finite()
        || !source.min_frequency.is_finite()
        || !source.max_frequency.is_finite()
        || source.min_frequency <= 0.0
        || source.max_frequency <= source.min_frequency
    {
        return None;
    }
    let clip = rect.intersect(painter.clip_rect());
    if !clip.is_positive() {
        return None;
    }
    let delta = source_range.1 - source_range.0;
    if !delta.is_finite() {
        return None;
    }
    let zero = -source_range.0 / delta;
    let end = (source.frames as f64 - source_range.0) / delta;
    let left = zero
        .min(end)
        .max(0.0)
        .max(f64::from((clip.left() - rect.left()) / rect.width()));
    let right = zero
        .max(end)
        .min(1.0)
        .min(f64::from((clip.right() - rect.left()) / rect.width()));
    let shift = transpose_semitones / 12.0 * std::f32::consts::LN_2
        / (source.max_frequency / source.min_frequency).ln();
    let top = (-shift)
        .max(0.0)
        .max((clip.top() - rect.top()) / rect.height());
    let bottom = (1.0 - shift)
        .min(1.0)
        .min((clip.bottom() - rect.top()) / rect.height());
    if right <= left || bottom <= top {
        return None;
    }
    let painted = Rect::from_min_max(
        pos2(
            rect.left() + rect.width() * left as f32,
            rect.top() + rect.height() * top,
        ),
        pos2(
            rect.left() + rect.width() * right as f32,
            rect.top() + rect.height() * bottom,
        ),
    );
    let uv = Rect::from_min_max(
        pos2(
            ((source_range.0 + delta * left) / source.frames as f64).clamp(0.0, 1.0) as f32,
            (top + shift).clamp(0.0, 1.0),
        ),
        pos2(
            ((source_range.0 + delta * right) / source.frames as f64).clamp(0.0, 1.0) as f32,
            (bottom + shift).clamp(0.0, 1.0),
        ),
    );
    painter
        .with_clip_rect(clip)
        .image(source.texture, painted, uv, tint);
    Some(SpectrogramPaint { rect: painted, uv })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_has_quiet_transparent_low_bands_at_bottom_and_bounded_alpha() {
        let image = spectrogram_image(2, 2, &[255, 0, 128, 64]).unwrap();
        assert_eq!(image.size, [2, 2]);
        assert_eq!(image.pixels[0], Color32::TRANSPARENT);
        assert_eq!(image.pixels[2].a(), 72);
        assert!(image.pixels[3].a() > image.pixels[1].a());
        assert!(image.pixels.iter().all(|pixel| pixel.a() <= 72));
        assert!(spectrogram_image(0, 48, &[]).is_none());
        assert!(spectrogram_image(4096, 48, &[]).is_none());
        assert!(spectrogram_image(1, 2, &[0]).is_none());
    }
    #[test]
    fn palette_images_share_level_alpha_but_distinguish_blue_and_yellow() {
        let mono = spectrogram_palette_image(1, 4, &[0, 64, 128, 255], SpectrumPalette::Monochrome)
            .unwrap();
        let color =
            spectrogram_palette_image(1, 4, &[0, 64, 128, 255], SpectrumPalette::BlueYellow)
                .unwrap();
        assert_eq!(mono.size, color.size);
        assert_eq!(mono.pixels[3], Color32::TRANSPARENT);
        assert_eq!(color.pixels[3], Color32::TRANSPARENT);
        assert_eq!(mono.pixels[0], Color32::WHITE);
        assert!(color.pixels[0].r() > color.pixels[0].b());
        assert!(color.pixels[2].b() > color.pixels[2].r());
        for (a, b) in mono.pixels.iter().zip(&color.pixels) {
            assert_eq!(a.a(), b.a());
            assert_eq!(a.r(), a.g());
            assert_eq!(a.g(), a.b());
        }
        assert!(spectrogram_palette_image(0, 48, &[], SpectrumPalette::BlueYellow).is_none());
    }
    #[test]
    fn crop_reverse_and_pitch_shift_use_one_clipped_texture_quad() {
        let ctx = egui::Context::default();
        let source = SpectrogramSource {
            texture: TextureId::Managed(7),
            frames: 1000,
            min_frequency: 40.0,
            max_frequency: 2560.0,
        };
        let mut counts = 0;
        let output = ctx.run(Default::default(), |ctx| {
            let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 120.0));
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(Rect::from_min_max(pos2(20.0, 10.0), pos2(180.0, 110.0)));
            let forward = paint_audio_spectrogram(
                &painter,
                rect,
                source,
                (-500.0, 1500.0),
                0.0,
                Color32::WHITE,
            )
            .unwrap();
            assert_eq!(forward.rect.left(), 50.0);
            assert_eq!(forward.rect.right(), 150.0);
            assert_eq!(forward.uv.left(), 0.0);
            assert_eq!(forward.uv.right(), 1.0);
            assert_eq!(forward.rect.top(), 10.0);
            assert_eq!(forward.rect.bottom(), 110.0);
            let reversed = paint_audio_spectrogram(
                &painter,
                rect,
                source,
                (1500.0, -500.0),
                12.0,
                Color32::WHITE,
            )
            .unwrap();
            assert_eq!(reversed.uv.left(), 1.0);
            assert_eq!(reversed.uv.right(), 0.0);
            assert!((reversed.rect.bottom() - 100.0).abs() < 1e-4);
            assert!((reversed.uv.bottom() - 1.0).abs() < 1e-6);
            assert!(
                paint_audio_spectrogram(
                    &painter,
                    rect,
                    source,
                    (2000.0, 3000.0),
                    0.0,
                    Color32::WHITE
                )
                .is_none()
            );
            assert!(
                paint_audio_spectrogram(
                    &painter,
                    rect,
                    source,
                    (0.0, 1000.0),
                    f32::NAN,
                    Color32::WHITE
                )
                .is_none()
            );
        });
        for shape in output.shapes {
            if let egui::Shape::Mesh(mesh) = shape.shape {
                assert_eq!(mesh.texture_id, source.texture);
                assert_eq!(mesh.vertices.len(), 4);
                assert_eq!(mesh.indices.len(), 6);
                counts += 1;
            }
        }
        assert_eq!(counts, 2);
    }
}
