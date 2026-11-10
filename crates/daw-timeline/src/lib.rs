//! Bar and beat arithmetic for a song whose meter can change at bar lines.
//! Positions are quarter-note beats from the song start; bars count from one.
//! This crate has no model, UI or audio dependencies so every layer can share
//! the same conversions.
use serde::{Deserialize, Serialize};

pub const MAX_BEATS_PER_BAR: u8 = 32;
pub const MAX_METER_CHANGES: usize = 256;
/// Positions this close to a bar line belong to that bar line.
const EPSILON: f64 = 1e-6;

/// A new meter starting at the downbeat of `bar` (one-based, at least 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterChange {
    pub bar: u32,
    pub beats_per_bar: u8,
}

/// One constant-meter run. `bar` is the number of the bar starting at `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterSegment {
    pub start: f64,
    pub bar: u32,
    pub beats_per_bar: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeterMap {
    /// Never empty; the first segment starts at beat 0 with bar 1.
    segments: Vec<MeterSegment>,
}

/// Validate persisted changes: sorted unique bars after bar 1, bounded meters.
pub fn validate_meter_changes(changes: &[MeterChange]) -> Result<(), &'static str> {
    if changes.len() > MAX_METER_CHANGES {
        return Err("Too many meter changes (at most 256)");
    }
    let mut previous = 1;
    for change in changes {
        if !(1..=MAX_BEATS_PER_BAR).contains(&change.beats_per_bar) {
            return Err("Meter changes need 1–32 beats per bar");
        }
        if change.bar <= previous {
            return Err("Meter changes must start after bar 1 in increasing bar order");
        }
        previous = change.bar;
    }
    Ok(())
}

impl MeterMap {
    /// Build from the initial meter and changes. Invalid input is tolerated
    /// (out-of-order bars are skipped, zero meters become 1) so painting code
    /// never fails; persisted data is checked by `validate_meter_changes`.
    pub fn new(initial: u8, changes: &[MeterChange]) -> Self {
        let mut segments = vec![MeterSegment {
            start: 0.0,
            bar: 1,
            beats_per_bar: initial.max(1),
        }];
        for change in changes {
            let last = segments[segments.len() - 1];
            if change.bar <= last.bar {
                continue;
            }
            segments.push(MeterSegment {
                start: last.start
                    + f64::from(change.bar - last.bar) * f64::from(last.beats_per_bar),
                bar: change.bar,
                beats_per_bar: change.beats_per_bar.max(1),
            });
        }
        Self { segments }
    }

    pub fn uniform(beats_per_bar: u8) -> Self {
        Self::new(beats_per_bar, &[])
    }

    pub fn segments(&self) -> &[MeterSegment] {
        &self.segments
    }

    pub fn initial(&self) -> u8 {
        self.segments[0].beats_per_bar
    }

    pub fn is_uniform(&self) -> bool {
        self.segments.len() == 1
    }

    fn segment_at(&self, beat: f64) -> MeterSegment {
        let index = self
            .segments
            .partition_point(|segment| segment.start <= beat + EPSILON);
        self.segments[index.saturating_sub(1)]
    }

    pub fn meter_at(&self, beat: f64) -> u8 {
        self.segment_at(clean(beat)).beats_per_bar
    }

    /// One-based bar and the beat offset inside it (zero on the downbeat).
    pub fn position(&self, beat: f64) -> (u32, f64) {
        let beat = clean(beat);
        let segment = self.segment_at(beat);
        let meter = f64::from(segment.beats_per_bar);
        let elapsed = (beat - segment.start).max(0.0);
        let bars = ((elapsed + EPSILON) / meter).floor();
        let bar = segment
            .bar
            .saturating_add(bars.min(f64::from(u32::MAX)) as u32);
        (bar, (elapsed - bars * meter).max(0.0))
    }

    /// Start beat of a one-based bar.
    pub fn bar_start(&self, bar: u32) -> f64 {
        let bar = bar.max(1);
        let index = self.segments.partition_point(|segment| segment.bar <= bar);
        let segment = self.segments[index.saturating_sub(1)];
        segment.start + f64::from(bar - segment.bar) * f64::from(segment.beats_per_bar)
    }

    pub fn is_downbeat(&self, beat: f64) -> bool {
        self.position(beat).1 < EPSILON
    }

    pub fn nearest_bar_line(&self, beat: f64) -> f64 {
        let beat = clean(beat);
        let (bar, _) = self.position(beat);
        let (before, after) = (self.bar_start(bar), self.bar_start(bar.saturating_add(1)));
        if beat - before <= after - beat {
            before
        } else {
            after
        }
    }

    /// Continuous bar coordinate from the song start (0 at bar 1).
    pub fn bars(&self, beat: f64) -> f64 {
        let (bar, offset) = self.position(beat);
        f64::from(bar - 1) + offset / f64::from(self.meter_at(beat))
    }

    /// Bar lines `(bar, beat)` from `start` through `end`, inclusive.
    pub fn bar_lines(&self, start: f64, end: f64) -> impl Iterator<Item = (u32, f64)> + '_ {
        let (bar, offset) = self.position(start);
        let first = if offset < EPSILON {
            bar
        } else {
            bar.saturating_add(1)
        };
        (first..u32::MAX)
            .map(|bar| (bar, self.bar_start(bar)))
            .take_while(move |&(_, beat)| beat <= end + EPSILON)
    }

    /// The same song meter seen from a clip: local beat 0 is `origin` and is
    /// bar 1, while later meter changes stay at their absolute song positions.
    pub fn local(&self, origin: f64) -> Self {
        let origin = clean(origin);
        if origin == 0.0 {
            return self.clone();
        }
        let mut segments = vec![MeterSegment {
            start: 0.0,
            bar: 1,
            beats_per_bar: self.meter_at(origin),
        }];
        for segment in self.segments.iter().filter(|s| s.start > origin + EPSILON) {
            let last = segments[segments.len() - 1];
            let start = segment.start - origin;
            let elapsed = ((start - last.start) / f64::from(last.beats_per_bar) - EPSILON)
                .ceil()
                .max(1.0);
            segments.push(MeterSegment {
                start,
                bar: last.bar.saturating_add(elapsed as u32),
                beats_per_bar: segment.beats_per_bar,
            });
        }
        Self { segments }
    }
}

fn clean(beat: f64) -> f64 {
    if beat.is_finite() { beat.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> MeterMap {
        // 4/4 for bars 1–2, 3/4 for bars 3–4, then 7/4.
        MeterMap::new(
            4,
            &[
                MeterChange {
                    bar: 3,
                    beats_per_bar: 3,
                },
                MeterChange {
                    bar: 5,
                    beats_per_bar: 7,
                },
            ],
        )
    }

    #[test]
    fn positions_follow_meter_changes() {
        let map = map();
        assert_eq!(map.position(0.0), (1, 0.0));
        assert_eq!(map.position(7.5), (2, 3.5));
        assert_eq!(map.position(8.0), (3, 0.0));
        assert_eq!(map.position(11.0), (4, 0.0));
        assert_eq!(map.position(13.0), (4, 2.0));
        assert_eq!(map.position(14.0), (5, 0.0));
        assert_eq!(map.position(22.0), (6, 1.0));
        assert_eq!(map.position(-3.0), (1, 0.0));
        assert_eq!(map.position(f64::NAN), (1, 0.0));
        assert_eq!(map.position(7.999_999_9), (3, 0.0));
        assert_eq!(map.meter_at(7.9), 4);
        assert_eq!(map.meter_at(8.0), 3);
        assert_eq!(map.meter_at(100.0), 7);
    }

    #[test]
    fn bar_starts_and_downbeats_round_trip() {
        let map = map();
        for (bar, beat) in [
            (1, 0.0),
            (2, 4.0),
            (3, 8.0),
            (4, 11.0),
            (5, 14.0),
            (6, 21.0),
        ] {
            assert_eq!(map.bar_start(bar), beat);
            assert_eq!(map.position(beat), (bar, 0.0));
            assert!(map.is_downbeat(beat));
        }
        assert!(!map.is_downbeat(12.0));
        assert_eq!(map.bar_start(0), 0.0);
        assert_eq!(map.nearest_bar_line(9.4), 8.0);
        assert_eq!(map.nearest_bar_line(9.6), 11.0);
        assert_eq!(map.bars(12.5), 3.5);
    }

    #[test]
    fn bar_lines_cover_the_requested_range() {
        let map = map();
        let lines: Vec<_> = map.bar_lines(3.0, 21.0).collect();
        assert_eq!(lines, [(2, 4.0), (3, 8.0), (4, 11.0), (5, 14.0), (6, 21.0)]);
        assert_eq!(map.bar_lines(8.0, 8.0).collect::<Vec<_>>(), [(3, 8.0)]);
        assert_eq!(map.bar_lines(0.0, f64::NAN).count(), 0);
    }

    #[test]
    fn local_maps_start_a_clip_at_bar_one() {
        let local = map().local(4.0);
        assert_eq!(local.position(0.0), (1, 0.0));
        assert_eq!(local.position(4.0), (2, 0.0));
        assert_eq!(local.meter_at(4.0), 3);
        assert_eq!(local.bar_start(3), 7.0);
        // A clip starting off the bar line keeps the absolute change position.
        let offset = map().local(6.0);
        assert_eq!(offset.meter_at(0.0), 4);
        assert_eq!(offset.position(2.0), (2, 0.0));
        assert_eq!(offset.position(5.0), (3, 0.0));
        assert_eq!(MeterMap::uniform(4).local(0.0), MeterMap::uniform(4));
    }

    #[test]
    fn validation_rejects_unordered_or_invalid_changes() {
        let change = |bar, beats_per_bar| MeterChange { bar, beats_per_bar };
        assert!(validate_meter_changes(&[change(2, 3), change(9, 32)]).is_ok());
        assert!(validate_meter_changes(&[change(1, 3)]).is_err());
        assert!(validate_meter_changes(&[change(3, 3), change(3, 4)]).is_err());
        assert!(validate_meter_changes(&[change(5, 3), change(3, 4)]).is_err());
        assert!(validate_meter_changes(&[change(2, 0)]).is_err());
        assert!(validate_meter_changes(&[change(2, 33)]).is_err());
        let many: Vec<_> = (2..(MAX_METER_CHANGES as u32 + 3))
            .map(|bar| change(bar, 3))
            .collect();
        assert!(validate_meter_changes(&many).is_err());
    }
}
