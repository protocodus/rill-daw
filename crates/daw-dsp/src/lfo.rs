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
