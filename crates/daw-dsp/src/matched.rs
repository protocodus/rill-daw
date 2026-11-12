//! Matched second-order filters: biquads whose magnitude follows an analog
//! prototype up to 20 kHz at any sample rate, instead of being squeezed
//! against Nyquist as bilinear designs are (a bell near 20 kHz at 44.1 kHz is
//! up to 15 dB off with the bilinear transform).
//!
//! Poles come from impulse invariance, which places them exactly; the
//! numerator is then fitted to the prototype's magnitude on a fixed grid, in
//! the squared-magnitude form of Vicanek ("Matched Second Order Digital
//! Filters", 2016), where a biquad's `|H|²` is linear in three coefficients.
//! Everything is `f64`, closed-form (the notch runs three passes) and free of
//! allocation, so a design can run on the audio thread.
use super::Biquad;

/// An analog second-order section in the normalized variable `s = j·f/f0`:
/// coefficients of `s²`, `s` and 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prototype {
    pub numerator: [f64; 3],
    pub denominator: [f64; 3],
}

/// Section Qs of Butterworth cascades with one, two and four sections
/// (12, 24 and 48 dB per octave).
pub const BUTTERWORTH_Q: [&[f64]; 3] = [
    &[std::f64::consts::FRAC_1_SQRT_2],
    &[0.541_196_100_146_197, 1.306_562_964_876_376],
    &[
        0.509_795_579_104_159,
        0.601_344_886_935_045,
        0.899_976_223_136_416,
        2.562_915_447_741_505,
    ],
];

impl Prototype {
    /// Boosting bell; `amplitude` is `10^(dB/40)`, at least 1. A cut is the
    /// [`Self::reciprocal`].
    pub fn bell(amplitude: f64, q: f64) -> Self {
        Self {
            numerator: [1.0, amplitude / q, 1.0],
            denominator: [1.0, 1.0 / (amplitude * q), 1.0],
        }
    }

    /// Boosting low shelf with separate Qs for its zeros and poles. Equal Qs
    /// give the usual shelf; a larger zero Q dips the response on the far
    /// side of the transition.
    pub fn low_shelf(amplitude: f64, zero_q: f64, pole_q: f64) -> Self {
        let root = amplitude.sqrt();
        Self {
            numerator: [amplitude, amplitude * root / zero_q, amplitude * amplitude],
            denominator: [amplitude, root / pole_q, 1.0],
        }
    }

    pub fn high_shelf(amplitude: f64, zero_q: f64, pole_q: f64) -> Self {
        let root = amplitude.sqrt();
        Self {
            numerator: [amplitude * amplitude, amplitude * root / zero_q, amplitude],
            denominator: [1.0, root / pole_q, amplitude],
        }
    }

    pub fn notch(q: f64) -> Self {
        Self {
            numerator: [1.0, 0.0, 1.0],
            denominator: [1.0, 1.0 / q, 1.0],
        }
    }

    pub fn low_pass(q: f64) -> Self {
        Self {
            numerator: [0.0, 0.0, 1.0],
            denominator: [1.0, 1.0 / q, 1.0],
        }
    }

    pub fn high_pass(q: f64) -> Self {
        Self {
            numerator: [1.0, 0.0, 0.0],
            denominator: [1.0, 1.0 / q, 1.0],
        }
    }

    /// Numerator and denominator exchanged: the cut that undoes a boost.
    pub fn reciprocal(&self) -> Self {
        Self {
            numerator: self.denominator,
            denominator: self.numerator,
        }
    }

    /// `|H|²` at `ratio` = f/f0.
    pub fn magnitude_squared(&self, ratio: f64) -> f64 {
        let square =
            |[p0, p1, p2]: [f64; 3]| (p2 - p0 * ratio * ratio).powi(2) + (p1 * ratio).powi(2);
        square(self.numerator) / square(self.denominator)
    }
}

/// How a design was reached. The two methods give different coefficients for
/// neighbouring settings, so a caller that interpolates coefficients treats a
/// change of method as a jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Fitted,
    /// The fit had no realizable solution; a closed form took its place.
    Fallback,
}

/// The two feedback coefficients `(a1, a2)` of the poles that the roots of
/// `polynomial` map to by impulse invariance, with the prototype's `f0` at
/// `omega0` radians per sample. Always stable.
pub fn impulse_invariant(polynomial: [f64; 3], omega0: f64) -> (f64, f64) {
    let [p0, p1, p2] = polynomial;
    let omega = omega0 * (p2 / p0).sqrt();
    let damping = 0.5 * p1 / (p0 * p2).sqrt();
    let radius = (-damping * omega).exp();
    let a1 = if damping <= 1.0 {
        -2.0 * radius * ((1.0 - damping * damping).sqrt() * omega).cos()
    } else {
        -2.0 * radius * ((damping * damping - 1.0).sqrt() * omega).cosh()
    };
    (a1, radius * radius)
}

/// Decay rate, in nepers per sample, of the slowest root of `polynomial`
/// placed as in [`impulse_invariant`]: a filter with these poles falls by
/// 120 dB in `13.8155 / rate` samples.
pub fn decay_rate(polynomial: [f64; 3], omega0: f64) -> f64 {
    let [p0, p1, p2] = polynomial;
    let omega = omega0 * (p2 / p0).sqrt();
    let damping = 0.5 * p1 / (p0 * p2).sqrt();
    omega * (damping - (damping * damping - 1.0).max(0.0).sqrt())
}

const GRID: usize = 32;

/// Designs matched biquads for one sample rate.
#[derive(Clone, Debug)]
pub struct MatchedDesigner {
    sample_rate: f64,
    /// Fit grid: log-spaced to 20 kHz (or just below Nyquist).
    omega: [f64; GRID],
    /// `|H|²` basis at each grid point: cos²(ω/2), sin²(ω/2), sin²(ω).
    basis: [[f64; 3]; GRID],
    cosine: [f64; GRID],
}

/// The basis in which a biquad's squared magnitude is linear.
fn basis(omega: f64) -> [f64; 3] {
    let phi1 = (0.5 * omega).sin().powi(2);
    let phi0 = 1.0 - phi1;
    [phi0, phi1, 4.0 * phi0 * phi1]
}

/// Squared-magnitude coefficients of `1 + a1·z⁻¹ + a2·z⁻²` in [`basis`].
fn squared(a1: f64, a2: f64) -> [f64; 3] {
    [(1.0 + a1 + a2).powi(2), (1.0 - a1 + a2).powi(2), -4.0 * a2]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// False for NaN as well, which is what a failed fit produces.
fn positive(value: f64) -> bool {
    value > 0.0
}

/// The minimum-phase polynomial with these squared-magnitude coefficients.
fn factor([b0, b1, b2]: [f64; 3]) -> Option<[f64; 3]> {
    if b0.is_nan() || b1.is_nan() || b0 < 0.0 || b1 < 0.0 {
        return None;
    }
    let w = 0.5 * (b0.sqrt() + b1.sqrt());
    let discriminant = w * w + b2;
    if discriminant.is_nan() || discriminant < 0.0 {
        return None;
    }
    let c0 = 0.5 * (w + discriminant.sqrt());
    if !positive(c0) {
        return None;
    }
    Some([c0, 0.5 * (b0.sqrt() - b1.sqrt()), -b2 / (4.0 * c0)])
}

/// Least squares for two unknowns by QR. The normal equations square the
/// condition number, and the two columns are nearly parallel next to a
/// resonant pole: a 20 Hz, Q 18 bell comes out 18 dB wrong with them.
fn solve(u: &[f64; GRID], v: &[f64; GRID], r: &[f64; GRID]) -> Option<(f64, f64)> {
    let inner =
        |a: &[f64; GRID], b: &[f64; GRID]| -> f64 { a.iter().zip(b).map(|(a, b)| a * b).sum() };
    let norm_u = inner(u, u).sqrt();
    if !positive(norm_u) {
        return None;
    }
    let q1 = u.map(|u| u / norm_u);
    let mut r12 = inner(&q1, v);
    let mut rest: [f64; GRID] = std::array::from_fn(|i| v[i] - r12 * q1[i]);
    // Orthogonalize twice: once leaves a component along q1 of the size of
    // the rounding in `r12`, which is all that `rest` consists of here.
    let again = inner(&q1, &rest);
    for (rest, q1) in rest.iter_mut().zip(&q1) {
        *rest -= again * q1;
    }
    r12 += again;
    let r22 = inner(&rest, &rest).sqrt();
    if !positive(r22) {
        return None;
    }
    let q2 = rest.map(|rest| rest / r22);
    let x2 = inner(&q2, r) / r22;
    let x1 = (inner(&q1, r) - r12 * x2) / norm_u;
    (x1.is_finite() && x2.is_finite()).then_some((x1, x2))
}

impl MatchedDesigner {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = f64::from(sample_rate);
        let top =
            (std::f64::consts::TAU * 20_000.0 / sample_rate).min(0.999 * std::f64::consts::PI);
        let omega: [f64; GRID] =
            std::array::from_fn(|i| top * 1000.0_f64.powf(i as f64 / (GRID - 1) as f64 - 1.0));
        Self {
            sample_rate,
            omega,
            basis: omega.map(basis),
            cosine: omega.map(f64::cos),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Radians per sample of a centre or corner frequency, held between
    /// 10 Hz and just below Nyquist.
    pub fn omega(&self, frequency: f64) -> f64 {
        std::f64::consts::TAU * frequency.clamp(10.0, 0.49 * self.sample_rate) / self.sample_rate
    }

    /// A bell or shelf prototype (boost or cut) with its `f0` at `frequency`.
    pub fn design(&self, prototype: &Prototype, frequency: f64) -> (Biquad, Fit) {
        let omega0 = self.omega(frequency);
        let corner = |[p0, _, p2]: [f64; 3]| (p2 / p0).sqrt();
        let q = |[p0, p1, p2]: [f64; 3]| (p0 * p2).sqrt() / p1;
        let (zero, pole) = (corner(prototype.numerator), corner(prototype.denominator));
        let same_corner = (pole - zero).abs() <= zero * 1e-12;
        // Always design the variant whose poles are the lower, more resonant
        // pair (bell boost, low-shelf boost, high-shelf cut) and invert it for
        // the other. Designing the other directly is several dB worse, and
        // the inversion makes cuts mirror boosts exactly.
        let swap = if same_corner {
            q(prototype.denominator) < q(prototype.numerator)
        } else {
            pole > zero
        };
        let oriented = if swap {
            prototype.reciprocal()
        } else {
            *prototype
        };
        let (biquad, fit) = match self.fit_numerator(&oriented, omega0) {
            Some(biquad) => (biquad, Fit::Fitted),
            None => {
                // Only a bell has the closed form.
                let closed = same_corner
                    .then(|| peak_closed_form(&oriented, omega0))
                    .flatten();
                (
                    closed.unwrap_or_else(|| matched_z(&oriented, omega0)),
                    Fit::Fallback,
                )
            }
        };
        (if swap { biquad.inverse() } else { biquad }, fit)
    }

    /// One section of a low-pass.
    pub fn low_pass(&self, q: f64, frequency: f64) -> (Biquad, Fit) {
        let omega0 = self.omega(frequency);
        let prototype = Prototype::low_pass(q);
        match self.fit_numerator(&prototype, omega0) {
            Some(biquad) => (biquad, Fit::Fitted),
            None => {
                // Vicanek's low-pass: exact at DC and at the corner.
                let (a1, a2) = impulse_invariant(prototype.denominator, omega0);
                let poles = squared(a1, a2);
                let [phi0, phi1, _] = basis(omega0);
                let at_corner = dot(poles, basis(omega0)) * q * q;
                let b1_squared = ((at_corner - poles[0] * phi0) / phi1).max(0.0);
                let b0 = 0.5 * (poles[0].sqrt() + b1_squared.sqrt());
                let biquad = Biquad {
                    b0,
                    b1: poles[0].sqrt() - b0,
                    b2: 0.0,
                    a1,
                    a2,
                };
                (biquad, Fit::Fallback)
            }
        }
    }

    /// One section of a high-pass. Closed form: a double zero at DC, the
    /// gain set at the corner; within 0.001 dB of the prototype.
    pub fn high_pass(&self, q: f64, frequency: f64) -> Biquad {
        let omega0 = self.omega(frequency);
        let (a1, a2) = impulse_invariant(Prototype::high_pass(q).denominator, omega0);
        let at_corner = basis(omega0);
        let b0 = q * dot(squared(a1, a2), at_corner).sqrt() / (4.0 * at_corner[1]);
        Biquad {
            b0,
            b1: -2.0 * b0,
            b2: b0,
            a1,
            a2,
        }
    }

    /// A notch: zeros on the unit circle at `frequency`, unity gain at DC,
    /// and poles fitted so that the width follows the prototype.
    pub fn notch(&self, q: f64, frequency: f64) -> (Biquad, Fit) {
        let omega0 = self.omega(frequency);
        let prototype = Prototype::notch(q);
        let (a1, a2) = impulse_invariant(prototype.denominator, omega0);
        let cosine = omega0.cos();
        let scale = 1.0 / (2.0 - 2.0 * cosine);
        let with_poles = |poles: [f64; 3]| Biquad {
            b0: scale / poles[0],
            b1: -2.0 * scale * cosine / poles[0],
            b2: scale / poles[0],
            a1: poles[1] / poles[0],
            a2: poles[2] / poles[0],
        };
        let fitted = || {
            let target: [f64; GRID] =
                std::array::from_fn(|i| prototype.magnitude_squared(self.omega[i] / omega0));
            let numerator: [f64; GRID] =
                std::array::from_fn(|i| (scale * (2.0 * self.cosine[i] - 2.0 * cosine)).powi(2));
            let start = squared(a1, a2);
            let mut solution = [1.0, start[1] / start[0], start[2] / start[0]];
            // The error is weighted by the current denominator, so that it is
            // relative to the response; three rounds settle the weights.
            for _ in 0..3 {
                let (mut u, mut v, mut r) = ([0.0; GRID], [0.0; GRID], [0.0; GRID]);
                for i in 0..GRID {
                    let [phi0, phi1, phi2] = self.basis[i];
                    let denominator = dot(solution, self.basis[i]);
                    if !positive(denominator) {
                        return None;
                    }
                    let weight = 1.0 / (denominator * target[i].max(1e-3));
                    u[i] = target[i] * phi1 * weight;
                    v[i] = target[i] * phi2 * weight;
                    r[i] = (numerator[i] - target[i] * phi0) * weight;
                }
                let (x1, x2) = solve(&u, &v, &r)?;
                solution = [1.0, x1, x2];
            }
            if !self.basis.iter().all(|basis| dot(solution, *basis) > 0.0) {
                return None;
            }
            let biquad = with_poles(factor(solution)?);
            (biquad.is_finite() && biquad.is_stable()).then_some(biquad)
        };
        match fitted() {
            Some(biquad) => (biquad, Fit::Fitted),
            None => {
                let gain = 1.0 + a1 + a2;
                (
                    with_poles([1.0 / gain, a1 / gain, a2 / gain]),
                    Fit::Fallback,
                )
            }
        }
    }

    /// Impulse-invariant poles, exact DC gain, and the numerator that fits
    /// the prototype's magnitude best in the relative sense. `None` when
    /// that numerator is not a real, minimum-phase polynomial.
    fn fit_numerator(&self, prototype: &Prototype, omega0: f64) -> Option<Biquad> {
        let (a1, a2) = impulse_invariant(prototype.denominator, omega0);
        let poles = squared(a1, a2);
        let dc = prototype.numerator[2] / prototype.denominator[2];
        let b0_squared = dc * dc * poles[0];
        let (mut u, mut v, mut r) = ([0.0; GRID], [0.0; GRID], [0.0; GRID]);
        for i in 0..GRID {
            let [phi0, phi1, phi2] = self.basis[i];
            let target =
                prototype.magnitude_squared(self.omega[i] / omega0) * dot(poles, self.basis[i]);
            if !positive(target) {
                return None;
            }
            u[i] = phi1 / target;
            v[i] = phi2 / target;
            r[i] = 1.0 - b0_squared * phi0 / target;
        }
        let (b1_squared, b2_squared) = solve(&u, &v, &r)?;
        let [b0, b1, b2] = factor([b0_squared, b1_squared, b2_squared])?;
        let biquad = Biquad { b0, b1, b2, a1, a2 };
        (biquad.is_finite() && biquad.is_minimum_phase()).then_some(biquad)
    }
}

/// Vicanek's matched bell: exact at DC, at the centre and in its slope there.
fn peak_closed_form(prototype: &Prototype, omega0: f64) -> Option<Biquad> {
    let (a1, a2) = impulse_invariant(prototype.denominator, omega0);
    let poles = squared(a1, a2);
    let peak = prototype.magnitude_squared(1.0);
    let [phi0, phi1, _] = basis(omega0);
    let r1 = dot(poles, basis(omega0)) * peak;
    let r2 = (-poles[0] + poles[1] + 4.0 * (phi0 - phi1) * poles[2]) * peak;
    let b0_squared = poles[0];
    let b2_squared = (r1 - r2 * phi1 - b0_squared) / (4.0 * phi1 * phi1);
    let b1_squared = r2 + b0_squared + 4.0 * (phi1 - phi0) * b2_squared;
    let [b0, b1, b2] = factor([b0_squared, b1_squared, b2_squared])?;
    let biquad = Biquad { b0, b1, b2, a1, a2 };
    (biquad.is_finite() && biquad.is_minimum_phase()).then_some(biquad)
}

/// Matched-Z: zeros placed like the poles, DC gain exact. Always realizable
/// and minimum phase; the last resort.
fn matched_z(prototype: &Prototype, omega0: f64) -> Biquad {
    let (a1, a2) = impulse_invariant(prototype.denominator, omega0);
    let (z1, z2) = impulse_invariant(prototype.numerator, omega0);
    let gain =
        prototype.numerator[2] / prototype.denominator[2] * (1.0 + a1 + a2) / (1.0 + z1 + z2);
    Biquad {
        b0: gain,
        b1: gain * z1,
        b2: gain * z2,
        a1,
        a2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Random, SvfMix};

    const RATES: [f32; 4] = [44_100.0, 48_000.0, 96_000.0, 192_000.0];
    const FREQUENCIES: [f64; 14] = [
        20.0, 50.0, 100.0, 400.0, 1000.0, 2500.0, 5000.0, 8000.0, 10_000.0, 12_000.0, 14_000.0,
        16_000.0, 18_000.0, 20_000.0,
    ];
    const QS: [f64; 7] = [0.1, 0.3, 0.71, 1.0, 2.0, 5.0, 18.0];
    const GAINS: [f64; 14] = [
        -24.0, -18.0, -12.0, -6.0, -3.0, -1.0, -0.01, 0.01, 1.0, 3.0, 6.0, 12.0, 18.0, 24.0,
    ];

    fn amplitude(gain_db: f64) -> f64 {
        10.0_f64.powf(gain_db.abs() / 40.0)
    }

    fn signed(prototype: Prototype, gain_db: f64) -> Prototype {
        if gain_db < 0.0 {
            prototype.reciprocal()
        } else {
            prototype
        }
    }

    fn db(magnitude: f64) -> f64 {
        20.0 * magnitude.max(1e-300).log10()
    }

    /// Measurement points from `low` Hz to 20 kHz (or just below Nyquist).
    fn points(low: f64, rate: f64, count: usize) -> impl Iterator<Item = f64> {
        let high = 20_000.0_f64.min(0.499 * rate);
        (0..count).map(move |i| low * (high / low).powf(i as f64 / (count - 1) as f64))
    }

    /// Largest deviation in dB of a cascade from its prototypes, wherever the
    /// prototype is above `floor_db`.
    fn error_db(
        sections: &[(Biquad, Prototype)],
        frequency: f64,
        designer: &MatchedDesigner,
        low: f64,
        floor_db: f64,
    ) -> f64 {
        let rate = designer.sample_rate();
        let centre = frequency.clamp(10.0, 0.49 * rate);
        points(low, rate, 1500)
            .filter_map(|f| {
                let omega = std::f64::consts::TAU * f / rate;
                let (mut digital, mut analog) = (0.0, 0.0);
                for (biquad, prototype) in sections {
                    digital += db(biquad.response(omega).norm());
                    analog += 0.5 * db(prototype.magnitude_squared(f / centre));
                }
                (analog > floor_db).then_some((digital - analog).abs())
            })
            .fold(0.0, f64::max)
    }

    fn assert_realizable(biquad: &Biquad, minimum_phase: bool, context: &str) {
        assert!(
            biquad.is_finite() && biquad.is_stable(),
            "{context}: {biquad:?}"
        );
        if minimum_phase {
            assert!(biquad.is_minimum_phase(), "{context}: {biquad:?}");
        }
        let mix = SvfMix::from_biquad(biquad);
        assert!(
            mix.g > 0.0 && mix.k > 0.0 && mix.g.is_finite() && mix.k.is_finite(),
            "{context}: {mix:?}"
        );
    }

    /// Worst error and number of fallbacks over the grid of frequencies, Qs
    /// and gains of both signs.
    fn sweep(rate: f32, qs: &[f64], prototype: impl Fn(f64, f64) -> Prototype) -> (f64, usize) {
        let designer = MatchedDesigner::new(rate);
        let (mut worst, mut fallbacks) = (0.0_f64, 0);
        for frequency in FREQUENCIES {
            for &q in qs {
                for gain in GAINS {
                    let prototype = signed(prototype(amplitude(gain), q), gain);
                    let (biquad, fit) = designer.design(&prototype, frequency);
                    assert_realizable(&biquad, true, &format!("{rate} {frequency} {q} {gain}"));
                    fallbacks += usize::from(fit == Fit::Fallback);
                    let error = error_db(&[(biquad, prototype)], frequency, &designer, 10.0, -1e9);
                    worst = worst.max(error);
                }
            }
        }
        (worst, fallbacks)
    }

    /// The Analog bell of an equalizer: half the Q near 0 dB, double at 24.
    fn proportional(amplitude: f64, q: f64) -> Prototype {
        let gain_db = 40.0 * amplitude.log10();
        Prototype::bell(
            amplitude,
            (q * 10.0_f64.powf((gain_db - 12.0) / 40.0)).clamp(0.05, 40.0),
        )
    }

    #[test]
    fn bells_follow_the_prototype_and_never_fall_back() {
        for (rate, limit) in RATES.into_iter().zip([0.65, 0.35, 0.02, 0.005]) {
            for (name, prototype) in [
                ("bell", Prototype::bell as fn(f64, f64) -> Prototype),
                ("proportional bell", proportional),
            ] {
                let (worst, fallbacks) = sweep(rate, &QS, prototype);
                assert!(worst <= limit, "{name} at {rate}: {worst} dB");
                assert_eq!(fallbacks, 0, "{name} at {rate}");
            }
        }
    }

    #[test]
    fn shelves_follow_the_prototype() {
        for (rate, limit) in RATES.into_iter().zip([0.55, 0.35, 0.5, 0.005]) {
            let (low, _) = sweep(rate, &QS[..5], |a, q| Prototype::low_shelf(a, q, q));
            let (high, _) = sweep(rate, &QS[..5], |a, q| Prototype::high_shelf(a, q, q));
            assert!(low <= limit && high <= limit, "{rate}: {low} {high} dB");
        }
    }

    #[test]
    fn dipped_shelves_follow_the_prototype() {
        for (rate, limit) in RATES.into_iter().zip([1.3, 0.8, 0.6, 0.005]) {
            let (low, _) = sweep(rate, &QS[..5], |a, q| {
                Prototype::low_shelf(a, q * a.sqrt(), q)
            });
            let (high, _) = sweep(rate, &QS[..5], |a, q| {
                Prototype::high_shelf(a, q * a.sqrt(), q)
            });
            assert!(low <= limit && high <= limit, "{rate}: {low} {high} dB");
        }
    }

    #[test]
    fn low_pass_cascades_follow_the_prototype() {
        let limits = [
            [0.3, 0.55, 1.05],
            [0.15, 0.25, 0.5],
            [0.005, 0.005, 0.005],
            [0.002, 0.002, 0.002],
        ];
        for (rate, limits) in RATES.into_iter().zip(limits) {
            let designer = MatchedDesigner::new(rate);
            for (qs, limit) in BUTTERWORTH_Q.into_iter().zip(limits) {
                for frequency in [
                    200.0, 1000.0, 5000.0, 10_000.0, 14_000.0, 16_000.0, 18_000.0, 20_000.0,
                ] {
                    let sections: Vec<_> = qs
                        .iter()
                        .map(|&q| (designer.low_pass(q, frequency).0, Prototype::low_pass(q)))
                        .collect();
                    for (biquad, _) in &sections {
                        assert_realizable(biquad, false, &format!("{rate} {frequency}"));
                    }
                    let error = error_db(&sections, frequency, &designer, 10.0, -60.0);
                    assert!(
                        error <= limit,
                        "{} sections, {frequency} Hz at {rate}: {error} dB",
                        qs.len()
                    );
                }
            }
        }
    }

    #[test]
    fn high_pass_cascades_follow_the_prototype() {
        for rate in [32_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let designer = MatchedDesigner::new(rate);
            for qs in BUTTERWORTH_Q {
                for frequency in [20.0, 100.0, 500.0, 2000.0] {
                    let sections: Vec<_> = qs
                        .iter()
                        .map(|&q| (designer.high_pass(q, frequency), Prototype::high_pass(q)))
                        .collect();
                    for (biquad, _) in &sections {
                        assert_realizable(biquad, false, &format!("{rate} {frequency}"));
                    }
                    let error = error_db(&sections, frequency, &designer, 5.0, -60.0);
                    assert!(
                        error <= 0.002,
                        "{} sections, {frequency} Hz at {rate}: {error} dB",
                        qs.len()
                    );
                }
            }
        }
    }

    #[test]
    fn notches_follow_the_prototype() {
        for (rate, limit) in RATES.into_iter().zip([2.2, 0.95, 0.02, 0.005]) {
            let designer = MatchedDesigner::new(rate);
            for frequency in [
                20.0, 100.0, 1000.0, 5000.0, 10_000.0, 14_000.0, 16_000.0, 18_000.0, 20_000.0,
            ] {
                for q in QS {
                    let (biquad, _) = designer.notch(q, frequency);
                    assert_realizable(&biquad, false, &format!("{rate} {frequency} {q}"));
                    let prototype = Prototype::notch(q);
                    let error = error_db(&[(biquad, prototype)], frequency, &designer, 10.0, -30.0);
                    assert!(
                        error <= limit,
                        "{frequency} Hz, Q {q} at {rate}: {error} dB"
                    );
                    // The zero is on the unit circle at the notch frequency.
                    let depth = biquad.response(designer.omega(frequency)).norm();
                    assert!(depth < 1e-6, "{frequency} Hz, Q {q} at {rate}: {depth}");
                }
            }
        }
    }

    #[test]
    fn every_design_is_realizable() {
        let mut random = Random::new(0x00E0_5EED);
        let mut uniform = move |low: f64, high: f64| {
            low + (high - low) * 0.5 * (f64::from(random.bipolar()) + 1.0)
        };
        for rate in [32_000.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
            let designer = MatchedDesigner::new(rate);
            for case in 0..4000 {
                // Past both ends of what a device asks for.
                let frequency = 5.0 * 2.0_f64.powf(uniform(0.0, 13.0));
                let q = 0.05 * 2.0_f64.powf(uniform(0.0, 9.7));
                let gain = uniform(-30.0, 30.0);
                let a = amplitude(gain);
                let context = format!("{rate} Hz: {frequency} Hz, Q {q}, {gain} dB, case {case}");
                let (biquad, minimum_phase) = match case % 8 {
                    0 => (
                        designer
                            .design(&signed(Prototype::bell(a, q), gain), frequency)
                            .0,
                        true,
                    ),
                    1 => (
                        designer
                            .design(&signed(proportional(a, q), gain), frequency)
                            .0,
                        true,
                    ),
                    2 => (
                        designer
                            .design(&signed(Prototype::low_shelf(a, q, q), gain), frequency)
                            .0,
                        true,
                    ),
                    3 => (
                        designer
                            .design(&signed(Prototype::high_shelf(a, q, q), gain), frequency)
                            .0,
                        true,
                    ),
                    4 => {
                        let shelf = if case % 16 == 4 {
                            Prototype::low_shelf(a, q * a.sqrt(), q)
                        } else {
                            Prototype::high_shelf(a, q * a.sqrt(), q)
                        };
                        (designer.design(&signed(shelf, gain), frequency).0, true)
                    }
                    5 => (designer.low_pass(q, frequency).0, false),
                    6 => (designer.high_pass(q, frequency), false),
                    _ => (designer.notch(q, frequency).0, false),
                };
                assert_realizable(&biquad, minimum_phase, &context);
            }
        }
    }

    #[test]
    fn a_cut_is_the_exact_inverse_of_the_boost() {
        let designer = MatchedDesigner::new(44_100.0);
        for frequency in [30.0, 1000.0, 16_000.0] {
            for gain in [0.5, 6.0, 24.0] {
                for prototype in [
                    Prototype::bell(amplitude(gain), 2.0),
                    Prototype::low_shelf(amplitude(gain), 0.71, 0.71),
                    Prototype::high_shelf(amplitude(gain), 1.2, 0.71),
                ] {
                    let (boost, _) = designer.design(&prototype, frequency);
                    let (cut, _) = designer.design(&prototype.reciprocal(), frequency);
                    let inverse = boost.inverse();
                    for (cut, inverse) in [
                        (cut.b0, inverse.b0),
                        (cut.b1, inverse.b1),
                        (cut.b2, inverse.b2),
                        (cut.a1, inverse.a1),
                        (cut.a2, inverse.a2),
                    ] {
                        assert!((cut - inverse).abs() < 1e-14, "{frequency} Hz, {gain} dB");
                    }
                    // Evaluating a direct form loses digits towards DC; the
                    // coefficients above are what is exact there.
                    for f in points(500.0, 44_100.0, 200) {
                        let omega = std::f64::consts::TAU * f / 44_100.0;
                        let product = boost.response(omega) * cut.response(omega);
                        assert!(
                            (product - crate::Complex::ONE).norm() < 1e-12,
                            "{frequency} Hz, {gain} dB at {f} Hz"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fallbacks_are_reported_and_still_close() {
        // The one case in the shelf grid whose fit is not realizable.
        let designer = MatchedDesigner::new(96_000.0);
        let prototype = Prototype::low_shelf(amplitude(24.0), 2.0, 2.0);
        let (biquad, fit) = designer.design(&prototype, 20_000.0);
        assert_eq!(fit, Fit::Fallback);
        assert!(error_db(&[(biquad, prototype)], 20_000.0, &designer, 10.0, -1e9) < 0.5);
        let (_, fit) = designer.design(&prototype, 1000.0);
        assert_eq!(fit, Fit::Fitted);
    }

    #[test]
    fn poles_decay_at_the_reported_rate() {
        let omega0 = std::f64::consts::TAU * 100.0 / 48_000.0;
        for q in [0.3, 0.71, 5.0] {
            let polynomial = Prototype::low_pass(q).denominator;
            let (a1, a2) = impulse_invariant(polynomial, omega0);
            // Radius of the slowest pole.
            let discriminant = a1 * a1 - 4.0 * a2;
            let radius = if discriminant > 0.0 {
                0.5 * (-a1 + discriminant.sqrt())
            } else {
                a2.sqrt()
            };
            let rate = decay_rate(polynomial, omega0);
            assert!((radius - (-rate).exp()).abs() < 1e-12, "Q {q}");
        }
    }

    #[test]
    fn frequencies_are_held_inside_the_designable_range() {
        let designer = MatchedDesigner::new(32_000.0);
        assert_eq!(designer.omega(20_000.0), designer.omega(15_680.0));
        assert_eq!(designer.omega(1.0), designer.omega(10.0));
        assert_eq!(BUTTERWORTH_Q.map(<[f64]>::len), [1, 2, 4]);
        for qs in BUTTERWORTH_Q {
            // A Butterworth cascade is 3 dB down at its corner.
            let at_corner: f64 = qs
                .iter()
                .map(|&q| Prototype::low_pass(q).magnitude_squared(1.0))
                .product();
            assert!((at_corner - 0.5).abs() < 1e-12);
        }
    }
}
