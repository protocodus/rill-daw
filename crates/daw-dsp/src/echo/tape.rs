/// The transport of a tape echo: where the tape is over time. A head some
/// distance behind the record head plays what passed the record head when
/// the tape was that much further back, so one position history gives every
/// head its delay: repitch while the motor comes up to a new speed, wow
/// that grows with the distance, flutter that averages out over it.
///
/// Distances are in lengths of tape between the record head and the last
/// playback head, speeds in those per second.
#[derive(Clone, Debug, Default)]
pub struct TapeTransport {
    /// Position at every tick, newest at `cursor`.
    history: Vec<f64>,
    cursor: usize,
    tick_seconds: f64,
    position: f64,
    motor: f64,
    lag: f64,
}

impl TapeTransport {
    /// Samples between two calls of [`Self::advance`].
    pub const TICK: u32 = 16;
    /// Time constant of the motor reaching a new speed.
    pub const LAG_SECONDS: f64 = 0.12;

    /// Room for delays of up to `longest_seconds`. Call from `prepare`
    /// only.
    pub fn prepare(&mut self, sample_rate: f32, longest_seconds: f32) {
        self.tick_seconds = f64::from(Self::TICK) / f64::from(sample_rate);
        let ticks = (f64::from(longest_seconds) / self.tick_seconds).ceil() as usize + 4;
        self.history = vec![0.0; ticks];
        self.lag = 1.0 - (-self.tick_seconds / Self::LAG_SECONDS).exp();
        self.reset(1.0);
    }

    /// As if the tape had always run at `speed`.
    pub fn reset(&mut self, speed: f64) {
        self.motor = speed;
        self.position = 0.0;
        self.cursor = 0;
        let length = self.history.len();
        for (index, slot) in self.history.iter_mut().enumerate() {
            // Slot `index` is `length − index` ticks old once the cursor
            // has wrapped to 0.
            let age = if index == 0 { 0 } else { length - index };
            *slot = -(age as f64) * self.tick_seconds * speed;
        }
    }

    /// One tick: the motor follows `speed`, and the tape moves at the
    /// motor's speed times `1 + deviation` (wow and flutter).
    pub fn advance(&mut self, speed: f64, deviation: f64) {
        if self.history.is_empty() {
            return;
        }
        self.motor += (speed - self.motor) * self.lag;
        self.position += self.motor * (1.0 + deviation) * self.tick_seconds;
        self.cursor = (self.cursor + 1) % self.history.len();
        self.history[self.cursor] = self.position;
    }

    /// The motor's speed now.
    pub fn speed(&self) -> f64 {
        self.motor
    }

    /// Position and speed of the tape `seconds` ago.
    fn past(&self, seconds: f64) -> (f64, f64) {
        let length = self.history.len();
        let ticks = (seconds / self.tick_seconds).clamp(0.0, (length - 2) as f64);
        let whole = ticks as usize;
        let at = |back: usize| self.history[(self.cursor + length - back) % length];
        let (near, far) = (at(whole), at(whole + 1));
        (
            near + (far - near) * (ticks - whole as f64),
            (near - far) / self.tick_seconds,
        )
    }

    /// Refines the delay in seconds of a head `distance` behind the record
    /// head: one Newton step from `delay`, which a head that follows the
    /// tape tick by tick needs no more of.
    pub fn solve(&self, delay: f64, distance: f64) -> f64 {
        if self.history.is_empty() {
            return delay;
        }
        let (position, speed) = self.past(delay);
        let longest = (self.history.len() - 2) as f64 * self.tick_seconds;
        (delay + (position - (self.position - distance)) / speed.max(1e-6)).clamp(0.0, longest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn transport(speed: f64) -> TapeTransport {
        let mut transport = TapeTransport::default();
        transport.prepare(RATE, 4.2);
        transport.reset(speed);
        transport
    }

    #[test]
    fn a_steady_tape_delays_by_distance_over_speed() {
        let mut transport = transport(2.5);
        for distance in [1.0, 0.5, 0.382] {
            assert!((transport.solve(0.1, distance) - distance / 2.5).abs() < 1e-9);
        }
        for _ in 0..10_000 {
            transport.advance(2.5, 0.0);
        }
        assert!((transport.solve(0.3, 1.0) - 0.4).abs() < 1e-9);
        assert_eq!(transport.speed(), 2.5);
        // Longer than the history holds: the oldest there is.
        assert!(transport.solve(4.0, 100.0) < 4.21);
    }

    #[test]
    fn a_new_speed_is_reached_with_the_motor_lag_and_the_heads_follow() {
        let mut transport = transport(10.0);
        let mut delay = 0.1;
        let mut speeds = Vec::new();
        let mut delays = Vec::new();
        for _ in 0..6000 {
            transport.advance(5.0, 0.0);
            delay = transport.solve(delay, 1.0);
            speeds.push(transport.speed());
            delays.push(delay);
        }
        let tick = 16.0 / f64::from(RATE);
        // 63 % of the way after the time constant.
        let reached = speeds
            .iter()
            .position(|speed| *speed <= 5.0 + 5.0 * (-1.0_f64).exp())
            .unwrap();
        assert!(
            (reached as f64 * tick / TapeTransport::LAG_SECONDS - 1.0).abs() < 0.02,
            "{reached} ticks"
        );
        // The delay only grows, never faster than time passes (the tape
        // never runs backward), and ends on the new one.
        assert!(
            delays
                .windows(2)
                .all(|pair| { pair[1] >= pair[0] - 1e-12 && pair[1] - pair[0] < tick })
        );
        assert!((delay - 0.2).abs() < 1e-5, "{delay}");
    }

    #[test]
    fn wow_moves_a_far_head_more_than_a_near_one() {
        let swing = |speed: f64| {
            let mut transport = transport(speed);
            let mut delay = 1.0 / speed;
            let (mut low, mut high) = (f64::MAX, f64::MIN);
            for tick in 0..24_000 {
                let seconds = tick as f64 * 16.0 / f64::from(RATE);
                let wow = 0.015 * (std::f64::consts::TAU * 0.5 * seconds).sin();
                transport.advance(speed, wow);
                delay = transport.solve(delay, 1.0);
                if tick > 6000 {
                    low = low.min(delay);
                    high = high.max(delay);
                }
            }
            0.5 * (high - low)
        };
        // A speed error of 1.5 % at 0.5 Hz over the time the tape spends
        // between the heads.
        for seconds in [0.1_f64, 0.4] {
            let expected = 0.015 / (std::f64::consts::TAU * 0.5)
                * 2.0
                * (std::f64::consts::PI * 0.5 * seconds).sin();
            let measured = swing(1.0 / seconds);
            assert!(
                (measured / expected - 1.0).abs() < 0.03,
                "{seconds} s: {measured} against {expected}"
            );
        }
    }
}
