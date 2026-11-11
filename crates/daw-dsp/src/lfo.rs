use super::Random;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleAndHold,
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
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        Self {
            phase: 0.0,
            held: 0.0,
            random: Random::new(seed),
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.held = 0.0;
        self.random.reset();
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Advances by `increment` cycles (rate / sample rate).
    #[inline]
    pub fn advance(&mut self, increment: f64) {
        self.set_phase(self.phase + increment);
    }

    /// Jumps to an absolute phase in cycles; a new S&H value is drawn on
    /// each cycle boundary crossed.
    #[inline]
    pub fn set_phase(&mut self, phase: f64) {
        let wrapped = phase.rem_euclid(1.0);
        if phase.floor() != self.phase.floor() || wrapped < self.phase {
            self.held = self.random.bipolar();
        }
        self.phase = wrapped;
    }

    /// Value at the current phase plus an offset in cycles (stereo spread).
    #[inline]
    pub fn value(&self, shape: LfoShape, offset: f64) -> f32 {
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
            LfoShape::SampleAndHold => self.held,
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
