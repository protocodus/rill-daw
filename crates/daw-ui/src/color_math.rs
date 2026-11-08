//! Shared sRGB contrast calculations for themed canvas and widget colors.
use egui::Color32;
use std::sync::LazyLock;

// Color32 channels have only 256 possible values. Calculate the original sRGB
// transfer function once rather than repeating powf for every painted mark.
static LINEAR_CHANNELS: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|value| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
});

/// Relative luminance of the stored RGB channels. Callers deciding contrast
/// for translucent colors supply the intended opaque foreground/background.
pub(crate) fn luminance(color: Color32) -> f32 {
    let linear = &*LINEAR_CHANNELS;
    0.2126 * linear[usize::from(color.r())]
        + 0.7152 * linear[usize::from(color.g())]
        + 0.0722 * linear[usize::from(color.b())]
}

pub(crate) fn contrast_luminance(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_preserves_every_channel_value_exactly() {
        let reference = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        for value in 0..=255 {
            assert_eq!(LINEAR_CHANNELS[usize::from(value)], reference(value));
            for color in [
                Color32::from_rgb(value, 0, 0),
                Color32::from_rgb(0, value, 0),
                Color32::from_rgb(0, 0, value),
                Color32::from_gray(value),
                Color32::from_rgba_unmultiplied(value, 255 - value, value / 2, 91),
            ] {
                assert_eq!(
                    luminance(color),
                    0.2126 * reference(color.r())
                        + 0.7152 * reference(color.g())
                        + 0.0722 * reference(color.b()),
                );
            }
        }
        assert_eq!(contrast_luminance(0.0, 1.0), (1.0_f32 + 0.05) / 0.05);
        assert_eq!(contrast_luminance(0.25, 0.25), 1.0);
    }
}
