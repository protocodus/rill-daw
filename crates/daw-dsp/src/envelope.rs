//! Exponential-segment ADSR, shaped like an analog RC envelope: the attack
//! charges toward a level above full scale and stops at it, decay and
//! release discharge exponentially. A retrigger starts the attack from the
//! current level, so it never clicks.
use super::flush;

/// The attack aims this far above full scale, as an analog envelope charging
/// toward its supply does, so it rises almost linearly and arrives firmly.
const ATTACK_TARGET: f32 = 1.3;
/// Below this the release is over (−80 dB).
const SILENT: f32 = 1e-4;
/// Decay and release times are where 99 % of the way is covered.
const TIME_CONSTANTS: f32 = 4.6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Release,
}

#[derive(Clone, Debug, Default)]
pub struct Adsr {
    stage: Stage,
    level: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

fn coefficient(seconds: f32, rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * rate).max(1.0)).exp()
}

impl Adsr {
    /// Times in milliseconds, `sustain` 0–1, `rate` the calls per second of
    /// [`Self::next_value`].
    pub fn set(&mut self, attack: f32, decay: f32, sustain: f32, release: f32, rate: f32) {
        let charge = (ATTACK_TARGET / (ATTACK_TARGET - 1.0)).ln();
        self.attack = coefficient(attack * 0.001 / charge, rate);
        self.decay = coefficient(decay * 0.001 / TIME_CONSTANTS, rate);
        self.sustain = sustain.clamp(0.0, 1.0);
        self.release = coefficient(release * 0.001 / TIME_CONSTANTS, rate);
    }

    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// Starts the attack from wherever the level is.
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    #[inline]
    pub fn next_value(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => return 0.0,
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * self.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = flush(self.level + (self.sustain - self.level) * self.decay);
            }
            Stage::Release => {
                self.level -= self.level * self.release;
                if self.level < SILENT {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn samples(milliseconds: f32) -> usize {
        (milliseconds * 0.001 * RATE) as usize
    }

    #[test]
    fn segments_take_their_set_times() {
        let mut envelope = Adsr::default();
        envelope.set(10.0, 100.0, 0.5, 200.0, RATE);
        envelope.gate_on();
        let mut peak_at = 0;
        for index in 0..samples(50.0) {
            if envelope.next_value() >= 1.0 && peak_at == 0 {
                peak_at = index;
            }
        }
        assert!(peak_at.abs_diff(samples(10.0)) <= 2, "{peak_at}");
        for _ in 0..samples(100.0) {
            envelope.next_value();
        }
        assert!((envelope.level() - 0.5).abs() < 0.01);
        envelope.gate_off();
        for _ in 0..samples(200.0) {
            envelope.next_value();
        }
        assert!(envelope.level() < 0.5 * 0.011);
        for _ in 0..samples(400.0) {
            envelope.next_value();
        }
        assert_eq!(envelope.stage(), Stage::Idle);
        assert_eq!(envelope.next_value(), 0.0);
    }

    #[test]
    fn retrigger_continues_from_the_current_level() {
        let mut envelope = Adsr::default();
        envelope.set(5.0, 50.0, 0.8, 300.0, RATE);
        envelope.gate_on();
        for _ in 0..samples(100.0) {
            envelope.next_value();
        }
        envelope.gate_off();
        for _ in 0..samples(20.0) {
            envelope.next_value();
        }
        let before = envelope.level();
        envelope.gate_on();
        let after = envelope.next_value();
        assert!(
            after >= before && after - before < 0.01,
            "{before} → {after}"
        );
    }
}
