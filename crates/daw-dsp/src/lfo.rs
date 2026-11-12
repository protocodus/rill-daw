use super::Random;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleAndHold,
    /// A random curve without steps or corners: a new value each cycle,
    /// reached along a smooth ramp. Not among [`Self::LABELS`], which
    /// devices persist as a parameter's choices.
    Smooth,
}

impl LfoShape {
    pub const LABELS: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H"];

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Triangle,
            2 => Self::Saw,
            3 => Self::Square,
            4 => Self::SampleAndHold,
            _ => Self::Sine,
        }
    }
}

/// Bipolar (−1–1) low-frequency oscillator. Free-running from phase 0 after
/// `reset`, or locked to the song position when given a beat-derived phase.
#[derive(Clone, Debug)]
pub struct Lfo {
    phase: f64,
    held: f32,
    random: Random,
    /// [`LfoShape::Smooth`]: the value at the start of this cycle, of the
    /// next and of the one after.
    smooth: [f32; 3],
    /// Its own generator, so the S&H sequence does not depend on it.
    smooth_random: Random,
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        let mut lfo = Self {
            phase: 0.0,
            held: 0.0,
            random: Random::new(seed),
            smooth: [0.0; 3],
            smooth_random: Random::new(seed ^ 0x9E37_79B9),
        };
        lfo.reset();
        lfo
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.held = 0.0;
        self.random.reset();
        self.smooth_random.reset();
        // From rest, so a device that starts with it starts at its centre.
        self.smooth = [
            0.0,
            self.smooth_random.bipolar(),
            self.smooth_random.bipolar(),
        ];
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Advances by `increment` cycles (rate / sample rate).
    #[inline]
    pub fn advance(&mut self, increment: f64) {
        let next = self.phase + increment;
        // Within the cycle nothing wraps and nothing is drawn, which spares
        // the remainder on all but one sample of a cycle.
        if next >= self.phase && next < 1.0 {
            self.phase = next;
        } else {
            self.set_phase(next);
        }
    }

    /// Jumps to an absolute phase in cycles; a new S&H value is drawn on
    /// each cycle boundary crossed.
    #[inline]
    pub fn set_phase(&mut self, phase: f64) {
        let wrapped = phase.rem_euclid(1.0);
        if phase.floor() != self.phase.floor() || wrapped < self.phase {
            self.held = self.random.bipolar();
        }
        if wrapped < self.phase {
            if self.phase - wrapped > 0.5 {
                // Into the next cycle: its start is where the last one ended.
                self.smooth = [self.smooth[1], self.smooth[2], self.smooth_random.bipolar()];
            } else {
                // A jump backward (a loop or locate) starts a new curve.
                self.smooth = [0; 3].map(|_| self.smooth_random.bipolar());
            }
        }
        self.phase = wrapped;
    }

    /// Value at the current phase plus an offset in cycles (stereo spread).
    #[inline]
    pub fn value(&self, shape: LfoShape, offset: f64) -> f32 {
        if shape == LfoShape::Smooth {
            // The offset reaches into the next cycle instead of wrapping,
            // so an offset channel follows the same curve later.
            let u = (self.phase + offset.rem_euclid(1.0)) as f32;
            let (from, to, t) = if u < 1.0 {
                (self.smooth[0], self.smooth[1], u)
            } else {
                (self.smooth[1], self.smooth[2], u - 1.0)
            };
            return from + (to - from) * t * t * (3.0 - 2.0 * t);
        }
        let phase = (self.phase + offset).rem_euclid(1.0) as f32;
        match shape {
            LfoShape::Sine => (std::f32::consts::TAU * phase).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            LfoShape::Saw => 2.0 * phase - 1.0,
            LfoShape::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleAndHold | LfoShape::Smooth => self.held,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(phase: f64, shape: LfoShape) -> f32 {
        let mut lfo = Lfo::new(1);
        lfo.set_phase(phase);
        lfo.value(shape, 0.0)
    }

    #[test]
    fn shapes_hit_their_corners_at_the_expected_phases() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(at(0.0, LfoShape::Sine), 0.0));
        assert!(close(at(0.25, LfoShape::Sine), 1.0));
        assert!(close(at(0.75, LfoShape::Sine), -1.0));
        assert!(close(at(0.0, LfoShape::Triangle), -1.0));
        assert!(close(at(0.5, LfoShape::Triangle), 1.0));
        assert!(close(at(0.0, LfoShape::Saw), -1.0));
        assert!(close(at(0.5, LfoShape::Saw), 0.0));
        assert!(close(at(0.25, LfoShape::Square), 1.0));
        assert!(close(at(0.75, LfoShape::Square), -1.0));
        // The stereo offset wraps like the phase does.
        let mut lfo = Lfo::new(1);
        lfo.set_phase(0.9);
        assert!(close(lfo.value(LfoShape::Saw, 0.2), at(0.1, LfoShape::Saw)));
    }

    #[test]
    fn phase_wraps_and_song_positions_lock_it() {
        let mut lfo = Lfo::new(1);
        for _ in 0..10 {
            lfo.advance(0.3);
        }
        assert!((lfo.phase() - 0.0).abs() < 1e-9 || (lfo.phase() - 1.0).abs() < 1e-9);
        lfo.set_phase(5.25);
        assert_eq!(lfo.phase(), 0.25);
        lfo.set_phase(-0.25);
        assert_eq!(lfo.phase(), 0.75);
    }

    #[test]
    fn sample_and_hold_changes_only_on_cycle_boundaries() {
        let mut lfo = Lfo::new(99);
        lfo.advance(0.1);
        let mut held = lfo.value(LfoShape::SampleAndHold, 0.0);
        let mut changes = 0;
        for step in 0..1000 {
            let before = lfo.phase();
            lfo.advance(0.01);
            let value = lfo.value(LfoShape::SampleAndHold, 0.0);
            if lfo.phase() < before {
                changes += 1;
                assert!((-1.0..1.0).contains(&value));
            } else {
                assert_eq!(value, held, "step {step} changed mid-cycle");
            }
            held = value;
        }
        assert!((9..=11).contains(&changes), "{changes}");
        // Jumping backward (a loop or locate) is a new cycle too.
        lfo.set_phase(0.5);
        let before = lfo.value(LfoShape::SampleAndHold, 0.0);
        lfo.set_phase(0.2);
        assert_ne!(lfo.value(LfoShape::SampleAndHold, 0.0), before);
    }

    #[test]
    fn reset_replays_the_same_random_steps() {
        let run = |lfo: &mut Lfo| -> Vec<f32> {
            (0..20)
                .map(|_| {
                    lfo.advance(0.5);
                    lfo.value(LfoShape::SampleAndHold, 0.0)
                })
                .collect()
        };
        let mut lfo = Lfo::new(3);
        let first = run(&mut lfo);
        lfo.reset();
        assert_eq!(lfo.phase(), 0.0);
        assert_eq!(run(&mut lfo), first);
    }

    #[test]
    fn smooth_has_no_steps_or_corners_and_replays() {
        let run = |lfo: &mut Lfo, offset: f64| -> Vec<f32> {
            (0..5000)
                .map(|_| {
                    lfo.advance(0.001);
                    lfo.value(LfoShape::Smooth, offset)
                })
                .collect()
        };
        for offset in [0.0, 0.25, 0.5] {
            let mut lfo = Lfo::new(21);
            let values = run(&mut lfo, offset);
            let mut range = (0.0_f32, 0.0_f32);
            for triple in values.windows(3) {
                let (before, after) = (triple[1] - triple[0], triple[2] - triple[1]);
                // The steepest slope of a full swing is 3·10⁻³ per step; the
                // slope itself changes by at most 12·10⁻⁶.
                assert!(before.abs() < 3.1e-3, "step {before} at offset {offset}");
                assert!((after - before).abs() < 2e-5, "corner at offset {offset}");
                range = (range.0.min(triple[1]), range.1.max(triple[1]));
            }
            assert!(range.0 < -0.3 && range.1 > 0.3, "{range:?}");
            lfo.reset();
            assert_eq!(run(&mut lfo, offset), values);
        }
        // An offset channel is the same curve, later.
        let mut lfo = Lfo::new(21);
        let ahead = run(&mut lfo, 0.25);
        lfo.reset();
        let plain = run(&mut lfo, 0.0);
        assert!((ahead[100] - plain[350]).abs() < 1e-5);
    }

    #[test]
    fn sample_and_hold_sequence_is_the_one_devices_already_render() {
        let mut lfo = Lfo::new(0x005E_EDF1);
        let values: Vec<f32> = (0..4)
            .map(|_| {
                lfo.advance(1.0);
                lfo.value(LfoShape::SampleAndHold, 0.0)
            })
            .collect();
        let mut random = Random::new(0x005E_EDF1);
        let expected: Vec<f32> = (0..4).map(|_| random.bipolar()).collect();
        assert_eq!(values, expected);
    }

    #[test]
    fn shape_indices_match_their_labels() {
        for (index, label) in LfoShape::LABELS.iter().enumerate() {
            let shape = LfoShape::from_index(index);
            let expected = match *label {
                "Sine" => LfoShape::Sine,
                "Triangle" => LfoShape::Triangle,
                "Saw" => LfoShape::Saw,
                "Square" => LfoShape::Square,
                _ => LfoShape::SampleAndHold,
            };
            assert_eq!(shape, expected);
        }
        assert_eq!(LfoShape::from_index(99), LfoShape::Sine);
    }
}
