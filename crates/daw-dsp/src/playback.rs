//! Where a voice is in its recording: start and end markers, reverse, and
//! the loop modes of a sampler, with an equal-power crossfade at a forward
//! loop's seam. The reads it asks for go through [`crate::MipMap`].
//!
//! Positions are worked out in a played direction: with reverse, position
//! 0 is the recording's last sample, so every mode runs the same way.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
    /// Plays from the start to the end marker; a released note fades with
    /// its envelope.
    #[default]
    Off,
    /// Repeats the loop while the voice sounds, also after its release.
    Forward,
    /// Plays the loop forward and backward in turn.
    PingPong,
    /// Plays from start to end whatever the note's length.
    OneShot,
}

impl LoopMode {
    pub const LABELS: &[&str] = &["Off", "Forward", "Ping-pong", "One-shot"];

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Forward,
            2 => Self::PingPong,
            3 => Self::OneShot,
            _ => Self::Off,
        }
    }

    pub fn loops(self) -> bool {
        matches!(self, Self::Forward | Self::PingPong)
    }
}

/// The shortest loop played, in samples of the recording; a shorter one
/// plays as no loop.
pub const MIN_LOOP: f64 = 32.0;

/// What a voice plays of a recording, in samples of it along the played
/// direction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Region {
    frames: f64,
    reverse: bool,
    start: f64,
    end: f64,
    /// `None` when the mode does not loop or the loop is too short or lies
    /// before the start.
    looped: Option<Loop>,
    mode: LoopMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Loop {
    start: f64,
    end: f64,
    /// Samples before the seam that fade into the samples before the loop
    /// start, so the loop is as long as it is set and needs no crossfade
    /// material inside it.
    crossfade: f64,
    ping_pong: bool,
}

/// Markers as shares (0–1) of the recording, which are its parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Markers {
    pub start: f32,
    pub end: f32,
    pub loop_start: f32,
    pub loop_end: f32,
    /// In samples of the recording.
    pub crossfade: f64,
    pub mode: LoopMode,
    pub reverse: bool,
}

impl Region {
    /// The markers in a recording of `frames` samples. Start and end, and
    /// the loop's two ends, may be given in either order. The crossfade is
    /// limited to what lies before the loop start and to the loop's length.
    pub fn new(frames: usize, markers: &Markers) -> Self {
        let frames = frames as f64;
        let place = |share: f32| {
            let at = f64::from(share.clamp(0.0, 1.0)) * frames;
            if markers.reverse { frames - at } else { at }
        };
        let ordered = |a: f64, b: f64| (a.min(b), a.max(b));
        let (start, end) = ordered(place(markers.start), place(markers.end));
        let (loop_start, loop_end) = ordered(place(markers.loop_start), place(markers.loop_end));
        let loop_end = loop_end.min(end);
        let looped = (markers.mode.loops()
            && loop_end - loop_start >= MIN_LOOP
            && start < loop_end)
            .then(|| Loop {
                start: loop_start,
                end: loop_end,
                crossfade: match markers.mode {
                    LoopMode::Forward => markers
                        .crossfade
                        .max(0.0)
                        .min(loop_start)
                        .min(loop_end - loop_start),
                    _ => 0.0,
                },
                ping_pong: markers.mode == LoopMode::PingPong,
            });
        Self {
            frames,
            reverse: markers.reverse,
            start,
            end,
            looped,
            mode: markers.mode,
        }
    }

    pub fn mode(&self) -> LoopMode {
        self.mode
    }

    /// The loop in samples of the recording, in its own order, if it plays.
    pub fn loop_range(&self) -> Option<(f64, f64)> {
        self.looped.map(|looped| {
            let (a, b) = (self.recording(looped.start), self.recording(looped.end));
            (a.min(b), a.max(b))
        })
    }

    /// From a played position to one in the recording.
    #[inline]
    fn recording(&self, played: f64) -> f64 {
        if self.reverse {
            self.frames - played
        } else {
            played
        }
    }
}

/// Up to two reads of the recording and their gains: two inside a
/// crossfade, the second with a gain of 0 otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reads {
    pub first: (f64, f32),
    pub second: (f64, f32),
}

/// One voice's place in its recording.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playhead {
    /// Along the played direction.
    position: f64,
    /// In a ping-pong loop, on its way back.
    backward: bool,
    /// Gain of the fade before the end marker; it falls no faster than the
    /// fade allows, also when the marker moves behind the playhead.
    level: f64,
    done: bool,
}

impl Playhead {
    pub fn new(region: &Region) -> Self {
        Self {
            position: region.start,
            backward: false,
            level: 1.0,
            done: region.end <= region.start,
        }
    }

    /// Played to the end, or nothing to play.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Where the playhead is in the recording, 0 to its length.
    pub fn position(&self, region: &Region) -> f64 {
        region.recording(self.position)
    }

    /// What to read now.
    #[inline]
    pub fn reads(&self, region: &Region) -> Reads {
        let position = self.position;
        let mut reads = Reads {
            first: (region.recording(position), 1.0),
            second: (0.0, 0.0),
        };
        match region.looped {
            Some(looped) if looped.crossfade > 0.0 && position >= looped.end - looped.crossfade => {
                // Equal power: the two sides of a seam are not correlated.
                let t = ((position - (looped.end - looped.crossfade)) / looped.crossfade) as f32;
                let (fade_in, fade_out) = (t.min(1.0) * std::f32::consts::FRAC_PI_2).sin_cos();
                reads.first.1 = fade_out;
                reads.second = (
                    region.recording(position - (looped.end - looped.start)),
                    fade_in,
                );
            }
            Some(_) => {}
            None => reads.first.1 = self.level as f32,
        }
        reads
    }

    /// Moves on by `step` samples of the recording. Without a loop, the
    /// last `fade` steps before the end marker fade out, so the end does not
    /// click; the voice is done when the fade is.
    #[inline]
    pub fn advance(&mut self, region: &Region, step: f64, fade: f64) {
        let Some(looped) = region.looped else {
            self.position += step;
            let (target, drop) = if fade > 0.0 {
                (
                    ((region.end - self.position) / step.max(1e-9) / fade).clamp(0.0, 1.0),
                    1.0 / fade,
                )
            } else {
                (f64::from(self.position < region.end), 1.0)
            };
            self.level = target.max(self.level - drop);
            // Steps of 1/fade do not add up to exactly 1.
            self.done = self.level <= 1e-9;
            return;
        };
        self.level = 1.0;
        let length = looped.end - looped.start;
        if looped.ping_pong {
            // Before the loop, a voice plays on toward it.
            if !self.backward && self.position + step < looped.end {
                self.position += step;
            } else {
                // Unfolded, the loop is one cycle of 2 × length there and back;
                // a step longer than the loop folds more than once.
                let offset = self.position - looped.start;
                let unfolded = if self.backward {
                    2.0 * length - offset
                } else {
                    offset
                } + step;
                let phase = unfolded.rem_euclid(2.0 * length);
                self.backward = phase > length;
                self.position = looped.start
                    + if self.backward {
                        2.0 * length - phase
                    } else {
                        phase
                    };
            }
        } else {
            self.position += step;
            if self.position >= looped.end {
                self.position = looped.start + (self.position - looped.start).rem_euclid(length);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(mode: LoopMode) -> Markers {
        Markers {
            start: 0.0,
            end: 1.0,
            loop_start: 0.5,
            loop_end: 0.75,
            crossfade: 0.0,
            mode,
            reverse: false,
        }
    }

    /// Recording positions a voice visits, `steps` of `step`.
    fn walk(region: &Region, step: f64, steps: usize) -> Vec<f64> {
        let mut playhead = Playhead::new(region);
        (0..steps)
            .map(|_| {
                let at = playhead.position(region);
                playhead.advance(region, step, 0.0);
                at
            })
            .collect()
    }

    #[test]
    fn off_and_one_shot_play_from_start_to_end() {
        for mode in [LoopMode::Off, LoopMode::OneShot] {
            let region = Region::new(
                1024,
                &Markers {
                    start: 0.125,
                    end: 0.25,
                    ..markers(mode)
                },
            );
            let mut playhead = Playhead::new(&region);
            assert_eq!(playhead.position(&region), 128.0);
            let mut steps = 0;
            while !playhead.is_done() {
                playhead.advance(&region, 1.5, 10.0);
                steps += 1;
            }
            assert_eq!(steps, 86, "{mode:?}");
            // The last steps fade out, the rest plays at full level.
            let mut playhead = Playhead::new(&region);
            assert_eq!(playhead.reads(&region).first.1, 1.0);
            for _ in 0..82 {
                playhead.advance(&region, 1.5, 10.0);
            }
            let gain = playhead.reads(&region).first.1;
            assert!(gain > 0.0 && gain < 0.5, "{gain}");
            // An end marker moved behind the playhead fades it as fast, no faster.
            let mut playhead = Playhead::new(&region);
            playhead.advance(&region, 1.5, 10.0);
            let shortened = Region::new(
                1024,
                &Markers {
                    start: 0.125,
                    end: 0.0,
                    ..markers(mode)
                },
            );
            let mut gains = Vec::new();
            while !playhead.is_done() {
                playhead.advance(&shortened, 1.5, 10.0);
                gains.push(playhead.reads(&shortened).first.1);
            }
            assert_eq!(gains.len(), 10, "{gains:?}");
        }
    }

    #[test]
    fn reverse_plays_the_same_markers_backward() {
        let region = Region::new(
            1024,
            &Markers {
                start: 0.125,
                end: 0.25,
                reverse: true,
                ..markers(LoopMode::Off)
            },
        );
        let positions = walk(&region, 1.0, 3);
        assert_eq!(positions, [256.0, 255.0, 254.0]);
        // A reversed loop runs from its end back to its start.
        let region = Region::new(
            1000,
            &Markers {
                reverse: true,
                ..markers(LoopMode::Forward)
            },
        );
        assert_eq!(region.loop_range(), Some((500.0, 750.0)));
        let positions = walk(&region, 10.0, 60);
        assert_eq!(positions[25], 750.0);
        assert_eq!(positions[50], 750.0);
        assert!(positions.iter().all(|at| *at > 500.0));
    }

    #[test]
    fn forward_loops_hold_their_length_and_ping_pong_turns() {
        let region = Region::new(1000, &markers(LoopMode::Forward));
        let positions = walk(&region, 1.0, 2000);
        assert_eq!(positions[749], 749.0);
        assert_eq!(positions[750], 500.0);
        assert_eq!(positions[1000], 500.0);
        let region = Region::new(1000, &markers(LoopMode::PingPong));
        let positions = walk(&region, 1.0, 1000);
        assert_eq!(positions[749], 749.0);
        assert_eq!(positions[750], 750.0);
        assert_eq!(positions[751], 749.0);
        assert!(positions.iter().all(|at| *at <= 750.0));
        assert!(positions[750..].iter().all(|at| *at >= 500.0));
        // Steps longer than the loop fold back into it.
        let mut playhead = Playhead::new(&region);
        playhead.advance(&region, 333.0, 0.0);
        for _ in 0..100 {
            playhead.advance(&region, 333.0, 0.0);
            let at = playhead.position(&region);
            assert!((500.0..=750.0).contains(&at), "{at}");
        }
    }

    #[test]
    fn a_crossfaded_seam_is_continuous_at_equal_power() {
        // A loop whose ends do not meet in phase: without the crossfade the
        // seam jumps; with it, the output moves no faster than the tone.
        let tone = |at: f64| (at * 0.0731).sin();
        let render = |crossfade: f64| {
            let region = Region::new(
                10_000,
                &Markers {
                    loop_start: 0.4,
                    loop_end: 0.6,
                    crossfade,
                    ..markers(LoopMode::Forward)
                },
            );
            let mut playhead = Playhead::new(&region);
            (0..20_000)
                .map(|_| {
                    let reads = playhead.reads(&region);
                    let (gains, value) = (
                        reads.first.1.powi(2) + reads.second.1.powi(2),
                        reads.first.1 as f64 * tone(reads.first.0)
                            + reads.second.1 as f64 * tone(reads.second.0),
                    );
                    playhead.advance(&region, 1.0, 0.0);
                    (gains, value)
                })
                .collect::<Vec<_>>()
        };
        let largest_step = |output: &[(f32, f64)]| {
            output
                .windows(2)
                .map(|pair| (pair[1].1 - pair[0].1).abs())
                .fold(0.0, f64::max)
        };
        assert!(largest_step(&render(0.0)) > 0.5);
        let faded = render(400.0);
        assert!(
            largest_step(&faded) < 0.0731 * 1.2,
            "{}",
            largest_step(&faded)
        );
        assert!(faded.iter().all(|(power, _)| (power - 1.0).abs() < 1e-5));
        // Limited by what lies before the loop: none at a loop from the start.
        let region = Region::new(
            1000,
            &Markers {
                loop_start: 0.0,
                crossfade: 100.0,
                ..markers(LoopMode::Forward)
            },
        );
        let playhead = Playhead::new(&region);
        assert_eq!(playhead.reads(&region).second.1, 0.0);
    }

    #[test]
    fn short_or_misplaced_loops_play_through() {
        for (loop_start, loop_end, start) in [(0.5, 0.501, 0.0), (0.2, 0.4, 0.5)] {
            let region = Region::new(
                10_000,
                &Markers {
                    start,
                    loop_start,
                    loop_end,
                    ..markers(LoopMode::Forward)
                },
            );
            assert_eq!(region.loop_range(), None);
            let mut playhead = Playhead::new(&region);
            for _ in 0..10_001 {
                playhead.advance(&region, 1.0, 0.0);
            }
            assert!(playhead.is_done());
        }
    }
}
