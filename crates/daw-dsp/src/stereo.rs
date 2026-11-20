/// Mid/side width: 0 folds to mono, 1 is unchanged, 2 doubles the side.
/// The mixer's channel and R1 Strip both use it, so they sound the same.
#[inline]
pub fn stereo_width([left, right]: [f32; 2], width: f32) -> [f32; 2] {
    let mid = (left + right) * 0.5;
    let side = (left - right) * 0.5 * width;
    [mid + side, mid - side]
}

/// Left and right gains of the mixer's balance pan, −1 (left) to 1 (right):
/// unity in the center, the far side fades out linearly.
#[inline]
pub fn balance(pan: f32) -> [f32; 2] {
    let pan = pan.clamp(-1.0, 1.0);
    [1.0 - pan.max(0.0), 1.0 + pan.min(0.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_folds_keeps_and_widens() {
        assert_eq!(stereo_width([1.0, 0.0], 0.0), [0.5, 0.5]);
        assert_eq!(stereo_width([0.5, -0.25], 1.0), [0.5, -0.25]);
        assert_eq!(stereo_width([1.0, 0.0], 2.0), [1.5, -0.5]);
    }

    #[test]
    fn balance_is_unity_in_the_center() {
        assert_eq!(balance(0.0), [1.0, 1.0]);
        assert_eq!(balance(0.5), [0.5, 1.0]);
        assert_eq!(balance(-1.0), [1.0, 0.0]);
        assert_eq!(balance(3.0), [0.0, 1.0]);
    }
}
