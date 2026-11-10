use super::flush;

/// Linear zero-delay-feedback (TPT) state-variable filter after Simper. Stable
/// under per-sample coefficient changes, so cutoff can be modulated freely.
#[derive(Clone, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SvfOutputs {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl SvfOutputs {
    pub fn notch(self) -> f32 {
        self.low + self.high
    }
}

impl Svf {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `g` from [`super::prewarp`]; `k` = 1/Q.
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32) -> SvfOutputs {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        SvfOutputs {
            low: v2,
            band: v1,
            high: input - k * v1 - v2,
        }
    }

    /// [`Self::process`] with the band state soft-clipped, like a diode pair
    /// in the resonance path: bounded at any damping, including slightly
    /// negative damping, which self-oscillates.
    #[inline]
    pub fn process_clipped(&mut self, input: f32, g: f32, k: f32) -> SvfOutputs {
        let outputs = self.process(input, g, k);
        self.ic1 = self.ic1.tanh();
        outputs
    }
}

/// Zero-delay-feedback one-pole low-pass; high-pass is `input - low`.
/// `low = G·input + β·state` with `G = g/(1+g)`, `β = 1/(1+g)`.
#[derive(Clone, Debug, Default)]
pub struct OnePole {
    pub state: f32,
}

impl OnePole {
    #[inline]
    pub fn low(&mut self, input: f32, big_g: f32) -> f32 {
        let v = (input - self.state) * big_g;
        let low = v + self.state;
        self.state = flush(low + v);
        low
    }
}
