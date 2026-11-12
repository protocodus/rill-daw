//! Dynamics building blocks in the log domain: the static curve of a
//! compressor, limiter or expander, the attack and release smoothing of a
//! detector, and the slow cell of a program-dependent release.
use super::flush;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveKind {
    /// Reduces what is above the threshold.
    Compress,
    /// Reduces what is below the threshold.
    Expand,
}

/// Largest reduction of an expander: its floor.
const EXPANDER_FLOOR_DB: f32 = 80.0;

/// Static gain curve with a quadratic knee. Levels and reductions are in dB;
/// a reduction is positive, input level minus output level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainComputer {
    pub kind: CurveKind,
    pub threshold_db: f32,
    /// At least 1; `f32::INFINITY` limits (compression only).
    pub ratio: f32,
    /// Width of the knee, centred on the threshold.
    pub knee_db: f32,
}

impl GainComputer {
    /// Share of the overshoot that is taken away: 1 − 1/ratio.
    fn slope(&self) -> f32 {
        1.0 - 1.0 / self.ratio.max(1.0)
    }

    pub fn reduction_db(&self, level_db: f32) -> f32 {
        let (threshold, knee) = (self.threshold_db, self.knee_db.max(0.0));
        match self.kind {
            CurveKind::Compress => {
                let over = level_db - (threshold - 0.5 * knee);
                if over <= 0.0 {
                    0.0
                } else if over < knee {
                    self.slope() * over * over / (2.0 * knee)
                } else {
                    self.slope() * (level_db - threshold)
                }
            }
            CurveKind::Expand => {
                let under = threshold + 0.5 * knee - level_db;
                let steepness = self.ratio.max(1.0) - 1.0;
                let reduction = if under <= 0.0 {
                    0.0
                } else if under < knee {
                    steepness * under * under / (2.0 * knee)
                } else {
                    steepness * (threshold - level_db)
                };
                reduction.min(EXPANDER_FLOOR_DB)
            }
        }
    }

    pub fn output_db(&self, level_db: f32) -> f32 {
        level_db - self.reduction_db(level_db)
    }

    /// The compressor's reduction as a function of the level *after* the
    /// gain element. A feedback detector using it settles on exactly the
    /// feed-forward curve. Finite ratios only.
    pub fn feedback_reduction_db(&self, output_db: f32) -> f32 {
        let (threshold, knee, ratio) = (self.threshold_db, self.knee_db.max(0.0), self.ratio);
        let over = output_db - (threshold - 0.5 * knee);
        if over <= 0.0 {
            0.0
        } else if output_db >= threshold + 0.5 * knee / ratio {
            (ratio - 1.0) * (output_db - threshold)
        } else {
            // Inside the knee the output is d + a·d² for an input d above
            // the knee's foot; solve for d.
            let a = -self.slope() / (2.0 * knee);
            let d = 2.0 * over / (1.0 + (1.0 + 4.0 * a * over).max(0.0).sqrt());
            -a * d * d
        }
    }

    /// The reduction `G ≥ 0` that solves
    /// `G = a + b·feedback_reduction_db(level_db − G)` for `a ≥ 0` and
    /// `0 ≤ b ≤ 1`: one step of a one-pole detector inside a feedback loop
    /// without a unit delay, which would be unstable at high ratios and short
    /// attacks. The residual is monotone in `G`, so the root is unique.
    pub fn solve_feedback(&self, level_db: f32, a: f32, b: f32) -> f32 {
        let (threshold, knee, ratio) = (self.threshold_db, self.knee_db.max(0.0), self.ratio);
        let foot = threshold - 0.5 * knee;
        let over = level_db - a - foot;
        if over <= 0.0 {
            return a;
        }
        let steepness = b * (ratio - 1.0);
        let linear = (a + steepness * (level_db - threshold)) / (1.0 + steepness);
        if knee <= 0.0 || level_db - linear >= threshold + 0.5 * knee / ratio {
            return linear;
        }
        let curvature = -self.slope() / (2.0 * knee);
        let p = curvature * (b - 1.0);
        let d = 2.0 * over / (1.0 + (1.0 - 4.0 * p * over).max(0.0).sqrt());
        a - b * curvature * d * d
    }

    /// Half of the reduction a full-scale signal gets: makeup that keeps
    /// typical material near its level. Full compensation overshoots.
    pub fn auto_makeup_db(&self) -> f32 {
        0.5 * self.reduction_db(0.0)
    }
}

/// One-pole smoother with separate coefficients for rising and falling
/// targets, in any domain: dB of reduction, linear level, power. Branching
/// on the smoothed value itself makes the 63 % times equal the set times.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ballistics {
    /// Coefficients `1 − exp(−1/(τ·fs))`; 1 is instant.
    pub rise: f32,
    pub fall: f32,
    state: f32,
}

impl Ballistics {
    /// For a time constant in milliseconds; instant for zero.
    pub fn coefficient(time_ms: f32, sample_rate: f32) -> f32 {
        let samples = time_ms * 0.001 * sample_rate;
        if samples > 0.0 {
            1.0 - (-1.0 / samples).exp()
        } else {
            1.0
        }
    }

    pub fn set_times(&mut self, rise_ms: f32, fall_ms: f32, sample_rate: f32) {
        self.rise = Self::coefficient(rise_ms, sample_rate);
        self.fall = Self::coefficient(fall_ms, sample_rate);
    }

    #[inline]
    pub fn process(&mut self, target: f32) -> f32 {
        let coefficient = if target > self.state {
            self.rise
        } else {
            self.fall
        };
        self.state = flush(self.state + (target - self.state) * coefficient);
        self.state
    }

    pub fn value(&self) -> f32 {
        self.state
    }

    pub fn set(&mut self, value: f32) {
        self.state = value;
    }

    pub fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// The slow cell of a two-stage release. A short burst barely charges it, so
/// recovery is the fast release; sustained reduction charges it, and recovery
/// is fast down to `weight` of the reduction, then follows the slow tail.
/// The cell is read before it is updated, so a detector inside a feedback
/// loop can still be solved in closed form.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgramRelease {
    /// 0 disables the cell.
    pub weight: f32,
    cell: Ballistics,
}

impl ProgramRelease {
    pub fn configure(&mut self, weight: f32, charge_ms: f32, release_ms: f32, sample_rate: f32) {
        self.weight = weight;
        self.cell.set_times(charge_ms, release_ms, sample_rate);
    }

    /// The reduction to apply for the fast detector's `fast`.
    #[inline]
    pub fn process(&mut self, fast: f32) -> f32 {
        let output = fast.max(self.weight * self.cell.value());
        self.cell.process(output);
        output
    }

    pub fn set(&mut self, value: f32) {
        self.cell.set(value);
    }

    pub fn reset(&mut self) {
        self.cell.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn compressor(threshold_db: f32, ratio: f32, knee_db: f32) -> GainComputer {
        GainComputer {
            kind: CurveKind::Compress,
            threshold_db,
            ratio,
            knee_db,
        }
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    #[test]
    fn regions_join_in_value_and_slope() {
        for kind in [CurveKind::Compress, CurveKind::Expand] {
            for knee in [0.0, 6.0, 24.0] {
                for ratio in [1.0, 2.0, 4.0, 20.0] {
                    let curve = GainComputer {
                        kind,
                        threshold_db: -18.0,
                        ratio,
                        knee_db: knee,
                    };
                    let step = 1e-2;
                    let mut previous = curve.reduction_db(-48.0);
                    let mut previous_slope = None;
                    let mut level = -48.0 + step;
                    while level < 0.0 {
                        let reduction = curve.reduction_db(level);
                        let slope = (reduction - previous) / step;
                        assert!(reduction >= 0.0);
                        // Monotone, and the output never falls as the input
                        // rises.
                        match kind {
                            CurveKind::Compress => assert!((-1e-3..=1.0 + 1e-3).contains(&slope)),
                            CurveKind::Expand => assert!(slope <= 1e-3),
                        }
                        // The expander's floor is a corner by design.
                        let floored = reduction.max(previous) >= EXPANDER_FLOOR_DB;
                        if let (Some(before), true) = (previous_slope, knee > 0.0 && !floored) {
                            let before: f32 = before;
                            // The slope changes by ratio-dependent steps of
                            // at most step/knee per sample of the sweep.
                            assert!(
                                (slope - before).abs() < 20.0 * step / knee + 5e-3,
                                "{kind:?} {knee} {ratio} at {level}: {before} → {slope}"
                            );
                        }
                        previous = reduction;
                        previous_slope = (!floored).then_some(slope);
                        level += step;
                    }
                }
            }
        }
        // The textbook values.
        let curve = compressor(-18.0, 4.0, 6.0);
        assert_eq!(curve.reduction_db(-21.0), 0.0);
        assert!((curve.reduction_db(-18.0) - 0.75 * 9.0 / 12.0).abs() < 1e-6);
        assert!((curve.reduction_db(-6.0) - 9.0).abs() < 1e-6);
        assert!((curve.output_db(-6.0) + 15.0).abs() < 1e-6);
        assert_eq!(compressor(-18.0, 1.0, 6.0).reduction_db(0.0), 0.0);
    }

    #[test]
    fn limiter_never_exceeds_its_threshold() {
        for knee in [0.0, 3.0, 24.0] {
            let limiter = compressor(-1.0, f32::INFINITY, knee);
            let mut level = -60.0;
            while level < 24.0 {
                assert!(limiter.output_db(level) <= -1.0 + 1e-5, "{knee} {level}");
                level += 0.01;
            }
            assert!((limiter.output_db(12.0) + 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn expander_mirrors_the_compressor_and_stops_at_its_floor() {
        let expander = GainComputer {
            kind: CurveKind::Expand,
            threshold_db: -40.0,
            ratio: 2.0,
            knee_db: 12.0,
        };
        assert_eq!(expander.reduction_db(-34.0), 0.0);
        assert_eq!(expander.reduction_db(0.0), 0.0);
        // Ten below the threshold comes out twenty below.
        assert!((expander.output_db(-50.0) + 60.0).abs() < 1e-5);
        assert!((expander.reduction_db(-40.0) - 36.0 / 24.0).abs() < 1e-5);
        assert_eq!(expander.reduction_db(-200.0), 80.0);
        let gate = GainComputer {
            ratio: 20.0,
            knee_db: 0.0,
            ..expander
        };
        assert_eq!(gate.reduction_db(-40.0), 0.0);
        assert!((gate.reduction_db(-42.0) - 38.0).abs() < 1e-4);
        assert_eq!(gate.reduction_db(-50.0), 80.0);
    }

    #[test]
    fn feedback_form_settles_on_the_feed_forward_curve() {
        for knee in [0.0, 6.0, 24.0] {
            for ratio in [1.0, 1.5, 4.0, 20.0] {
                let curve = compressor(-18.0, ratio, knee);
                let mut level = -80.0;
                while level < 12.0 {
                    let reduction = curve.reduction_db(level);
                    let back = curve.feedback_reduction_db(curve.output_db(level));
                    assert!(
                        (back - reduction).abs() < 1e-4,
                        "{knee} {ratio} at {level}: {back} against {reduction}"
                    );
                    level += 0.037;
                }
            }
        }
    }

    #[test]
    fn feedback_solve_leaves_no_residual() {
        let mut random = Lcg(5);
        for _ in 0..200_000 {
            let curve = compressor(
                -60.0 + 60.0 * random.next(),
                1.0 + 19.0 * random.next().powi(2),
                24.0 * random.next() * if random.next() < 0.2 { 0.0 } else { 1.0 },
            );
            let level = -90.0 + 110.0 * random.next();
            let a = 40.0 * random.next().powi(2);
            let b = if random.next() < 0.1 {
                1.0
            } else {
                random.next()
            };
            let solved = curve.solve_feedback(level, a, b);
            let residual = solved - a - b * curve.feedback_reduction_db(level - solved);
            assert!(
                solved >= a && residual.abs() < 1e-4 * (1.0 + solved.abs()),
                "{curve:?} level {level} a {a} b {b}: {solved}, residual {residual}"
            );
        }
        // With b = 1 and nothing stored it is the static curve.
        let curve = compressor(-18.0, 4.0, 6.0);
        for level in [-30.0, -19.0, -16.0, -6.0, 6.0] {
            assert!(
                (curve.solve_feedback(level, 0.0, 1.0) - curve.reduction_db(level)).abs() < 1e-4
            );
        }
    }

    #[test]
    fn auto_makeup_is_half_the_reduction_at_full_scale() {
        let curve = compressor(-18.0, 4.0, 6.0);
        assert!((curve.auto_makeup_db() - 6.75).abs() < 1e-6);
        assert_eq!(compressor(0.0, 4.0, 0.0).auto_makeup_db(), 0.0);
    }

    /// Samples until a step from 0 to 1 (or back) has covered 63 %.
    fn time_to_63(ballistics: &mut Ballistics, from: f32, to: f32) -> usize {
        ballistics.set(from);
        let goal = from + (to - from) * (1.0 - (-1.0_f32).exp());
        (1..)
            .find(|_| {
                let value = ballistics.process(to);
                if to > from {
                    value >= goal
                } else {
                    value <= goal
                }
            })
            .unwrap()
    }

    #[test]
    fn ballistics_take_their_set_times() {
        let mut ballistics = Ballistics::default();
        for (rise_ms, fall_ms) in [(1.0, 50.0), (10.0, 120.0), (100.0, 1000.0)] {
            ballistics.set_times(rise_ms, fall_ms, RATE);
            for (time_ms, from, to) in [(rise_ms, 0.0, 1.0), (fall_ms, 1.0, 0.0)] {
                let samples = time_to_63(&mut ballistics, from, to) as f32;
                let expected = time_ms * 0.001 * RATE;
                // One sample, or what f32 steps of a slow pole add up to.
                assert!(
                    (samples - expected).abs() <= (0.005 * expected).max(1.0),
                    "{time_ms}: {samples}"
                );
            }
        }
    }

    #[test]
    fn ballistics_coefficients_stay_valid_and_decays_end_in_zero() {
        for rate in [8000.0, 48_000.0, 192_000.0] {
            for time_ms in [0.0, 0.02, 1.0, 2500.0] {
                let coefficient = Ballistics::coefficient(time_ms, rate);
                assert!(coefficient > 0.0 && coefficient <= 1.0, "{time_ms} {rate}");
            }
        }
        assert_eq!(Ballistics::coefficient(0.0, RATE), 1.0);
        let mut ballistics = Ballistics::default();
        ballistics.set_times(0.0, 5.0, RATE);
        assert_eq!(ballistics.process(0.7), 0.7);
        assert_eq!(ballistics.value(), 0.7);
        let mut last = 1.0;
        for _ in 0..48_000 {
            last = ballistics.process(0.0);
        }
        assert_eq!(last, 0.0);
        ballistics.set(3.0);
        ballistics.reset();
        assert_eq!(ballistics.value(), 0.0);
    }

    /// Samples until the released reduction is below `share` of its start.
    fn recovery(burst: usize, share: f32) -> usize {
        let mut fast = Ballistics::default();
        fast.set_times(0.0, 60.0, RATE);
        let mut slow = ProgramRelease::default();
        slow.configure(0.5, 300.0, 1500.0, RATE);
        let mut reduction = 0.0;
        for _ in 0..burst {
            reduction = slow.process(fast.process(10.0));
        }
        assert_eq!(reduction, 10.0);
        (1..)
            .find(|_| slow.process(fast.process(0.0)) <= share * 10.0)
            .unwrap()
    }

    #[test]
    fn program_release_is_slower_after_sustained_reduction() {
        let (short, long) = ((0.02 * RATE) as usize, (2.0 * RATE) as usize);
        // The first half of the recovery is the fast release either way.
        let half = (recovery(short, 0.5), recovery(long, 0.5));
        assert!(half.1 < half.0 * 13 / 10, "{half:?}");
        // The tail is the slow cell's, only after the long burst.
        let tail = (recovery(short, 0.1), recovery(long, 0.1));
        assert!(tail.1 > 5 * tail.0, "{tail:?}");
        // Without weight it passes the fast detector through.
        let mut off = ProgramRelease::default();
        off.configure(0.0, 300.0, 1500.0, RATE);
        for value in [4.0, 9.0, 2.0, 0.0] {
            assert_eq!(off.process(value), value);
        }
        off.set(5.0);
        off.reset();
        assert_eq!(off.process(0.0), 0.0);
    }
}
