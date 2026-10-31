//! Compact elapsed-time labels. Musical bar/beat positions remain separate.

/// Format elapsed seconds as m:ss or h:mm:ss, adding hours only when needed.
/// Retain up to `precision` fractional digits (maximum three), without trailing
/// zeroes. Round the complete duration before splitting fields so carrying into
/// a minute or hour never produces a seconds field of 60.
pub fn format_clock_time(seconds: f64, precision: usize) -> String {
    if !seconds.is_finite() {
        return "—".into();
    }
    let precision = precision.min(3);
    let scale = 10_u64.pow(precision as u32);
    let ticks = (seconds.max(0.0) * scale as f64).round() as u64;
    let whole = ticks / scale;
    let hours = whole / 3600;
    let mut label = if hours > 0 {
        format!("{hours}:{:02}:{:02}", whole / 60 % 60, whole % 60)
    } else {
        format!("{}:{:02}", whole / 60, whole % 60)
    };
    let fraction = ticks % scale;
    if fraction > 0 {
        label.push('.');
        label.push_str(format!("{fraction:0precision$}").trim_end_matches('0'));
    }
    label
}
