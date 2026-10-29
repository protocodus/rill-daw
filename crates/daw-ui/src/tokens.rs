//! Shared dimensions and type scale in logical screen points.

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;
pub const CONTROL_HEIGHT: f32 = 34.0;
pub const COMPACT_HEIGHT: f32 = 24.0;
pub const SELECT_WIDTH: f32 = 160.0;
pub const RADIUS: u8 = 5;
pub const SURFACE_RADIUS: u8 = 8;
/// Soft selection halo, bounded independently of display scale or object size.
pub const SELECTION_GLOW_EXTENT: f32 = 6.0;
/// Inactive scenes use at most two 512 × 512 RGBA8 GPU targets.
pub const BACKDROP_BLUR_MAX_SIDE: u32 = 512;
pub const BACKDROP_BLUR_DOWNSAMPLE: f32 = 4.0;
pub const MIXER_ROUTE_WIDTH: f32 = 3.0;
pub const MIXER_ROUTE_HEAD: f32 = 9.0;
pub const MIXER_ROUTE_LANE_HEIGHT: f32 = 40.0;
pub const BUTTON_PADDING_X: f32 = 12.0;
pub const BUTTON_PADDING_Y: f32 = 6.0;
pub const POPUP_MARGIN: f32 = SPACE_XL;
// egui's native titlebar reserves 32 points at each side for its controls.
// Keep its close target outside the overlapping titlebar click region.
pub const WINDOW_MARGIN: egui::Margin = egui::Margin {
    left: SPACE_XL as i8,
    right: SPACE_XL as i8,
    top: SPACE_SM as i8,
    bottom: SPACE_SM as i8,
};
pub const WINDOW_BORDER_WIDTH: f32 = 1.5;
/// Two persistent control rows, their panel padding, and a comfortable gap.
pub const POPUP_TOP: f32 = CONTROL_HEIGHT * 2.0 + 28.0 + POPUP_MARGIN;
pub const FONT_CAPTION: f32 = 13.0;
pub const FONT_SMALL: f32 = 14.0;
pub const FONT_BUTTON: f32 = 15.0;
pub const FONT_BODY: f32 = 15.0;
pub const FONT_SUBTITLE: f32 = 16.0;
pub const FONT_HEADING: f32 = 20.0;
pub const FONT_CANVAS: f32 = 12.0;
/// Bounded parameter-scale captions in dense canvas controls.
pub const FONT_SCALE: f32 = 10.0;
pub const TIMELINE_RULER_HEIGHT: f32 = 36.0;
pub const TIMELINE_LOOP_STRIP_HEIGHT: f32 = 5.0;
/// Keep loop feedback below both ruler label rows, with a clear gap.
pub const TIMELINE_LOOP_RULER_HEIGHT: f32 =
    TIMELINE_RULER_HEIGHT + SPACE_SM + TIMELINE_LOOP_STRIP_HEIGHT;
pub const FONT_TRACK: f32 = 14.0;
pub const SWITCH_WIDTH: f32 = 36.0;
pub const SWITCH_HEIGHT: f32 = 20.0;
