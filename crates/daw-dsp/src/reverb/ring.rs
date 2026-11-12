//! The Hall model: a ring of absorbent allpasses and delays after Dattorro,
//! with the decay law of Dahl and Jot.
use super::rooms::Move;
use super::{HALL_TAPS, Layout, TAP_FADE_SECONDS, absorption_shelf};
use crate::{
    AbsorbentAllpass, DelayAllpass, FadeTap, Lfo, LfoShape, LinearRamp, Shelf1, ShelfGlide,
    soft_limit,
};

pub(super) const DIFFUSER_SECONDS: [[f32; 4]; 2] = [
    [0.004_77, 0.003_60, 0.012_73, 0.009_31],
    [0.005_03, 0.003_83, 0.013_37, 0.009_87],
];
const DIFFUSER_GAINS: [f32; 4] = [0.75, 0.75, 0.625, 0.625];
/// Each branch at Size 50 %: a modulated allpass, a delay, an allpass, a
/// delay (Dattorro's lengths at his 29 761 Hz).
pub(super) const ELEMENT_SECONDS: [[f32; 4]; 2] = [
    [0.022_6, 0.149_6, 0.060_5, 0.125_0],
    [0.030_5, 0.141_7, 0.089_2, 0.106_3],
];
/// Gains of the four elements of a branch at full Diffusion; the delays
/// are allpasses without gain.
const ELEMENT_GAINS: [f32; 4] = [0.7, 0.0, 0.5, 0.0];
/// Room in each element's line against its length at Size 50 %: the Size
/// range and the seed's jitter.
pub(super) const ELEMENT_ROOM: f32 = 2.7;
/// The output taps of each channel as `(branch, element, sign)`; where in
/// the element is the layout's.
pub(super) const TAPS: [[(usize, usize, f32); HALL_TAPS]; 2] = [
    [
        (1, 1, 1.0),
        (1, 1, 1.0),
        (1, 2, -1.0),
        (1, 3, 1.0),
        (0, 1, -1.0),
        (0, 2, -1.0),
        (0, 3, -1.0),
        (0, 1, 1.0),
        (0, 1, -1.0),
        (1, 2, 1.0),
        (0, 3, 1.0),
    ],
    [
        (0, 1, 1.0),
        (0, 1, 1.0),
        (0, 2, -1.0),
        (0, 3, 1.0),
        (1, 1, -1.0),
        (1, 2, -1.0),
        (1, 3, -1.0),
        (1, 1, 1.0),
        (1, 1, -1.0),
        (0, 2, 1.0),
        (1, 3, 1.0),
    ],
];
const TAP_GAIN: f32 = 0.6;
/// A modulated allpass is not strictly energy-preserving: the ring's
/// input is held, far above any programme.
const RING_LIMIT: f32 = 8.0;
const MOVEMENT_SECONDS: f32 = 0.0005;
const SEED_FADE_SECONDS: f32 = 0.05;
const TICK: u32 = 32;

#[derive(Clone, Debug)]
pub(super) struct Hall {
    diffusers: [[DelayAllpass; 4]; 2],
    ring: [[AbsorbentAllpass; 4]; 2],
    taps: [[FadeTap; HALL_TAPS]; 2],
    /// The decay each tap's signal has not yet received.
    tap_shelves: [[ShelfGlide; HALL_TAPS]; 2],
    lfos: [Lfo; 2],
    phases: [f32; 2],
    rates: [f32; 2],
    /// Offset of each branch's modulated read in samples.
    offsets: [LinearRamp; 2],
    moving: bool,
}

impl Hall {
    pub fn new() -> Self {
        Self {
            diffusers: std::array::from_fn(|_| std::array::from_fn(|_| DelayAllpass::default())),
            ring: std::array::from_fn(|_| std::array::from_fn(|_| AbsorbentAllpass::default())),
            taps: [[FadeTap::default(); HALL_TAPS]; 2],
            tap_shelves: std::array::from_fn(|_| std::array::from_fn(|_| ShelfGlide::default())),
            lfos: [Lfo::new(0x00AE_0001), Lfo::new(0x00AE_0002)],
            phases: [0.0; 2],
            rates: [0.0; 2],
            offsets: [LinearRamp::new(0.0); 2],
            moving: false,
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        let samples = |seconds: f32| (seconds * sample_rate).ceil() as usize;
        for branch in 0..2 {
            for (diffuser, seconds) in self.diffusers[branch]
                .iter_mut()
                .zip(&DIFFUSER_SECONDS[branch])
            {
                diffuser.allocate(samples(*seconds) + 1);
                diffuser.snap_length((seconds * sample_rate).round() as usize);
            }
            for (element, seconds) in self.ring[branch].iter_mut().zip(&ELEMENT_SECONDS[branch]) {
                element.allocate(samples(ELEMENT_ROOM * seconds), sample_rate);
                element.set_fade(samples(SEED_FADE_SECONDS) as u32);
            }
        }
        for tap in self.taps.iter_mut().flatten() {
            tap.set_fade(samples(TAP_FADE_SECONDS) as u32);
        }
    }

    pub fn configure(&mut self, layout: &Layout, how: Move) {
        for (index, element) in self.ring.iter_mut().flatten().enumerate() {
            let length = layout.elements[index];
            match how {
                Move::Snap => element.snap_to(length),
                Move::Glide => element.glide_to(length),
                Move::Fade => element.fade_to(length),
            }
        }
        for channel in 0..2 {
            for (tap, place) in self.taps[channel].iter_mut().zip(&layout.taps[channel]) {
                tap.set(*place as usize);
            }
        }
        self.phases = [layout.phases[0], layout.phases[1]];
        self.rates = [layout.rates[0], layout.rates[1]];
    }

    fn aim(&mut self, movement: f32, sample_rate: f32) {
        let depth = movement * MOVEMENT_SECONDS * sample_rate;
        for branch in 0..2 {
            let lfo = &self.lfos[branch];
            let value = 0.7 * lfo.value(LfoShape::Sine, f64::from(self.phases[branch]))
                + 0.3 * lfo.value(LfoShape::Smooth, 0.0);
            self.offsets[branch].set_target(depth * value, TICK);
        }
        self.moving = movement > 0.0 || self.offsets.iter().any(|offset| offset.value() != 0.0);
    }

    pub fn tick(&mut self, movement: f32, sample_rate: f32) {
        for (lfo, rate) in self.lfos.iter_mut().zip(&self.rates) {
            lfo.advance(f64::from(rate * TICK as f32 / sample_rate));
        }
        self.aim(movement, sample_rate);
    }

    /// Clears the audio state and settles on the controls.
    pub fn reset(&mut self, movement: f32, sample_rate: f32) {
        self.lfos.iter_mut().for_each(Lfo::reset);
        self.aim(movement, sample_rate);
        self.offsets.iter_mut().for_each(LinearRamp::snap);
        self.diffusers
            .iter_mut()
            .flatten()
            .for_each(DelayAllpass::reset);
        self.ring
            .iter_mut()
            .flatten()
            .for_each(AbsorbentAllpass::reset);
        for tap in self.taps.iter_mut().flatten() {
            tap.snap(tap.target());
        }
        self.tap_shelves
            .iter_mut()
            .flatten()
            .for_each(ShelfGlide::reset);
    }

    /// New absorption and tap gains for the lengths and tap places as
    /// they are now, reached in `ramp` samples; whether they still move.
    pub fn absorb(
        &mut self,
        decay: f32,
        high: f32,
        crossover_hz: f32,
        sample_rate: f32,
        ramp: u32,
    ) -> bool {
        let mut gliding = false;
        for element in self.ring.iter_mut().flatten() {
            let samples = element.heard_length().round() as u32;
            let shelf: Shelf1 = absorption_shelf(samples, decay, high, crossover_hz, sample_rate);
            element.set_absorption(&shelf, ramp);
            gliding |= element.is_gliding() || element.is_fading();
        }
        // The absorption sits at the end of an element: a tap part-way
        // along it would hear the decay as a staircase, and a decay shorter
        // than the ring not at all. Each gets the absorption of the length
        // its signal has passed since the last one, and a tap that
        // crossfades to another place takes its absorption along.
        for channel in 0..2 {
            for (tap, shelf) in self.taps[channel]
                .iter()
                .zip(&mut self.tap_shelves[channel])
            {
                let place = tap.place().round() as u32;
                shelf.set(
                    &absorption_shelf(place, decay, high, crossover_hz, sample_rate),
                    ramp,
                );
                gliding |= tap.is_fading();
            }
        }
        gliding
    }

    pub fn lengths(&self) -> [f64; 8] {
        std::array::from_fn(|index| self.ring[index / 4][index % 4].current_length())
    }

    /// One sample: the late output, left and right. `diffusion` is 0–1.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32, diffusion: f32) -> [f32; 2] {
        let mut inputs = [left, right];
        for (signal, diffusers) in inputs.iter_mut().zip(&mut self.diffusers) {
            for (diffuser, gain) in diffusers.iter_mut().zip(DIFFUSER_GAINS) {
                *signal = diffuser.process(*signal, diffusion * gain);
            }
        }
        // Each branch ends in the other's input.
        let ends = [self.ring[0][3].read(None), self.ring[1][3].read(None)];
        for branch in 0..2 {
            let offset = self.offsets[branch].next();
            let input = inputs[branch] + ends[1 - branch];
            let mut signal = RING_LIMIT * soft_limit(input / RING_LIMIT, 2.0);
            let [first, delay, second, last] = &mut self.ring[branch];
            signal = first.process(
                signal,
                diffusion * ELEMENT_GAINS[0],
                self.moving.then_some(offset),
            );
            signal = delay.process(signal, 0.0, None);
            signal = second.process(signal, diffusion * ELEMENT_GAINS[2], None);
            last.write(signal, ends[branch], 0.0);
        }
        let mut outputs = [0.0; 2];
        for channel in 0..2 {
            for (index, (branch, element, sign)) in TAPS[channel].iter().enumerate() {
                let line = self.ring[*branch][*element].line();
                let heard = self.taps[channel][index].read(line);
                outputs[channel] +=
                    sign * TAP_GAIN * self.tap_shelves[channel][index].process(heard);
            }
        }
        outputs
    }
}
