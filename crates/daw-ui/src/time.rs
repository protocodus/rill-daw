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

/// Compact one-based "bar.beat.sixteenth" position, the same form the rulers
/// use. Rounded to the nearest sixteenth so a readout never flickers between
/// two neighbouring values at a sixteenth boundary.
pub fn musical_position_label(beat: f64, meter: &crate::MeterMap) -> String {
    let sixteenths = (beat.max(0.0) * 4.0).round();
    let (bar, offset) = meter.position(sixteenths / 4.0);
    let sixteenth = (offset * 4.0).round() as u64;
    format!("{bar}.{}.{}", sixteenth / 4 + 1, sixteenth % 4 + 1)
}

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn positions_count_from_one_and_carry_into_bars() {
        let four = crate::MeterMap::uniform(4);
        assert_eq!(musical_position_label(0.0, &four), "1.1.1");
        assert_eq!(musical_position_label(0.25, &four), "1.1.2");
        assert_eq!(musical_position_label(3.75, &four), "1.4.4");
        assert_eq!(musical_position_label(4.0, &four), "2.1.1");
        assert_eq!(
            musical_position_label(6.5, &crate::MeterMap::uniform(3)),
            "3.1.3"
        );
        assert_eq!(musical_position_label(-2.0, &four), "1.1.1");
        assert_eq!(musical_position_label(3.999, &four), "2.1.1");
        let changing = crate::MeterMap::new(
            4,
            &[daw_timeline::MeterChange {
                bar: 2,
                beats_per_bar: 3,
            }],
        );
        assert_eq!(musical_position_label(6.75, &changing), "2.3.4");
        assert_eq!(musical_position_label(7.0, &changing), "3.1.1");
    }
}
