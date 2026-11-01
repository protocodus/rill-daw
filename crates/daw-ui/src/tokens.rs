//! Shared dimensions and type scale in logical screen points.
//!
//! Fibonacci steps establish the rhythm; phi interpolates the type and emphasis
//! between those steps. Canvas/data geometry and GPU budgets remain independent.

pub const GOLDEN_RATIO: f32 = 1.618_034;
pub const GOLDEN_RATIO_INVERSE: f32 = 1.0 / GOLDEN_RATIO;
/// Tight caption/detail separation; never the padding of an ordinary surface.
pub const SPACE_XS: f32 = 5.0;
/// Related controls and compact toolbar groups.
pub const SPACE_SM: f32 = 8.0;
/// Ordinary rows, card/inset padding, and horizontal menu-item padding.
pub const SPACE_MD: f32 = 13.0;
/// Section boundaries, pane gutters, and horizontal window padding.
pub const SPACE_LG: f32 = 21.0;
/// Major workspace separation.
pub const SPACE_XL: f32 = 34.0;
pub const CONTROL_HEIGHT: f32 = 34.0;
/// A centered control line with room for readable slider scale captions.
pub const PARAMETER_ROW_HEIGHT: f32 = 55.0;
pub const COMPACT_HEIGHT: f32 = 21.0;
pub const SELECT_WIDTH: f32 = 144.0;
pub const RADIUS: u8 = 5;
pub const SURFACE_RADIUS: u8 = 8;
pub const BORDER_WIDTH: f32 = 1.0;
pub const FOCUS_WIDTH: f32 = GOLDEN_RATIO;
/// Soft selection halo, bounded independently of display scale or object size.
pub const SELECTION_GLOW_EXTENT: f32 = 6.0;
/// Inactive scenes use at most two 512 × 512 RGBA8 GPU targets.
pub const BACKDROP_BLUR_MAX_SIDE: u32 = 512;
pub const BACKDROP_BLUR_DOWNSAMPLE: f32 = 4.0;
pub const MIXER_ROUTE_WIDTH: f32 = 3.0;
pub const MIXER_ROUTE_HEAD: f32 = 9.0;
pub const MIXER_ROUTE_LANE_HEIGHT: f32 = 40.0;
pub const BUTTON_PADDING_X: f32 = SPACE_MD;
pub const BUTTON_PADDING_Y: f32 = SPACE_SM;
pub const POPUP_MARGIN: f32 = SPACE_LG;
// egui's native titlebar reserves 32 points at each side for its controls.
// Keep its close target outside the overlapping titlebar click region.
pub const WINDOW_MARGIN: egui::Margin = egui::Margin {
    left: SPACE_LG as i8,
    right: SPACE_LG as i8,
    top: SPACE_MD as i8,
    bottom: SPACE_MD as i8,
};
pub const WINDOW_BORDER_WIDTH: f32 = FOCUS_WIDTH;
/// Two persistent control rows, their panel padding, and a comfortable gap.
pub const POPUP_TOP: f32 = CONTROL_HEIGHT * 2.0 + SPACE_MD * 2.0 + POPUP_MARGIN;
pub const FONT_CAPTION: f32 = 13.0;
pub const FONT_SMALL: f32 = FONT_CAPTION;
pub const FONT_BUTTON: f32 = FONT_CAPTION + 3.0 * GOLDEN_RATIO_INVERSE;
pub const FONT_BODY: f32 = FONT_BUTTON;
pub const FONT_SUBTITLE: f32 = FONT_CAPTION + SPACE_XS * GOLDEN_RATIO_INVERSE;
pub const FONT_HEADING: f32 = 21.0;
pub const FONT_CANVAS: f32 = FONT_CAPTION;
/// Bounded parameter-scale captions in dense canvas controls.
pub const FONT_SCALE: f32 = SPACE_SM + 3.0 * GOLDEN_RATIO_INVERSE;
/// Two proportional label rows with clear space between their glyph bounds.
pub const TIMELINE_RULER_HEIGHT: f32 = SPACE_XL + SPACE_XS;
pub const TIMELINE_LOOP_HEIGHT: f32 = SPACE_LG;
pub const TIMELINE_LOOP_STRIP_HEIGHT: f32 = 5.0;
/// Keep loop feedback below both ruler label rows, with a clear gap.
pub const TIMELINE_LOOP_RULER_HEIGHT: f32 =
    TIMELINE_RULER_HEIGHT + SPACE_SM + TIMELINE_LOOP_STRIP_HEIGHT;
pub const FONT_TRACK: f32 = FONT_BUTTON;
pub const SWITCH_WIDTH: f32 = 34.0;
pub const SWITCH_HEIGHT: f32 = 21.0;
