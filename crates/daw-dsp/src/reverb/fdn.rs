use super::{butterflies, hadamard};
use crate::{DelayLine, FadeTap, LinearRamp, Shelf1, ShelfGlide, flush, glide_length};

const GLIDE_SECONDS: f64 = 0.1;
/// Cosine and sine of no rotation.
const IDENTITY: [LinearRamp; 2] = [LinearRamp::new(1.0), LinearRamp::new(0.0)];

/// The core of a reverb: `N` delay lines (a power of two) whose outputs
/// pass a first-order absorption filter each and return through an
/// orthogonal matrix. With identity absorption it neither gains nor loses
/// energy; the absorption alone sets the decay, per frequency (see
/// [`super::absorption_shelf`]). The lines are read at whole taps, so
/// nothing is lost to interpolation; a length change glides.
///
/// The matrix is a signed permutation and the Hadamard matrix. Two optional
/// stages follow it: scattering (a short delay per channel, a second signed
/// permutation and a rotation of adjustable angle, after Schlecht and
/// Habets), which makes the echo density grow much faster, and a slow
/// rotation of each pair of channels, which moves the modes without
/// touching a delay.
#[derive(Clone, Debug)]
pub struct Fdn<const N: usize> {
    lines: [DelayLine; N],
    absorb: [ShelfGlide; N],
    /// In samples; equals the target when at rest.
    length: [f64; N],
    target: [u32; N],
    max_length: usize,
    glide: f64,
    /// Line `i` is fed from the absorbed output of line `permutation[i]`
    /// with `signs[i]`, before the Hadamard matrix.
    permutation: [usize; N],
    signs: [f32; N],
    scatter: Option<Scatter<N>>,
    /// Cosine and sine of each pair's rotation (the first `N/2` are used).
    rotation: [[LinearRamp; 2]; N],
    rotating: bool,
}

#[derive(Clone, Debug)]
struct Scatter<const N: usize> {
    lines: [DelayLine; N],
    taps: [FadeTap; N],
    permutation: [usize; N],
    signs: [f32; N],
    /// Cosine and sine of the mixing angle.
    angle: [LinearRamp; 2],
}

impl<const N: usize> Fdn<N> {
    /// Samples in which [`Self::set_rotation`] and
    /// [`Self::set_scatter_angle`] reach their values: call them that often
    /// for a continuous movement.
    pub const TICK: u32 = 32;
    /// Samples of length per sample: a line never stops or runs backward.
    pub const FASTEST_GLIDE: f64 = 0.5;

    /// Any permutation and signs are lossless; a good choice keeps a line's
    /// own output out of its input's largest term.
    pub fn new(permutation: [usize; N], signs: [f32; N]) -> Self {
        Self {
            lines: std::array::from_fn(|_| DelayLine::default()),
            absorb: std::array::from_fn(|_| ShelfGlide::default()),
            length: [2.0; N],
            target: [2; N],
            max_length: 0,
            glide: 1.0,
            permutation,
            signs,
            scatter: None,
            rotation: [IDENTITY; N],
            rotating: false,
        }
    }

    /// Room for lines of `max_length` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_length: usize, sample_rate: f32) {
        for line in &mut self.lines {
            line.allocate(max_length);
        }
        self.max_length = max_length;
        self.glide = 1.0 - (-1.0 / (GLIDE_SECONDS * f64::from(sample_rate))).exp();
    }

    /// Adds the scattering stage: delays of up to `max_delay` samples that
    /// crossfade over `fade` samples, and the signed permutation behind
    /// them. Call from `prepare` only.
    pub fn allocate_scatter(
        &mut self,
        max_delay: usize,
        fade: u32,
        permutation: [usize; N],
        signs: [f32; N],
    ) {
        let mut scatter = Scatter {
            lines: std::array::from_fn(|_| DelayLine::default()),
            taps: [FadeTap::default(); N],
            permutation,
            signs,
            angle: IDENTITY,
        };
        for (line, tap) in scatter.lines.iter_mut().zip(&mut scatter.taps) {
            line.allocate(max_delay);
            tap.set_fade(fade);
        }
        self.scatter = Some(scatter);
    }

    /// Clears the audio state and ends every ramp and crossfade; lengths
    /// and absorption stay.
    pub fn reset(&mut self) {
        for (line, absorb) in self.lines.iter_mut().zip(&mut self.absorb) {
            line.reset();
            absorb.reset();
        }
        if let Some(scatter) = &mut self.scatter {
            for (line, tap) in scatter.lines.iter_mut().zip(&mut scatter.taps) {
                line.reset();
                tap.snap(tap.target());
            }
            scatter.angle.iter_mut().for_each(LinearRamp::snap);
        }
        self.rotation
            .iter_mut()
            .flatten()
            .for_each(LinearRamp::snap);
    }

    /// New line lengths in samples (primes, so that the lines' echoes never
    /// coincide; see [`super::layout::prime_lengths`]). The lines glide
    /// there, which bends the pitch of what they hold instead of cutting
    /// it.
    pub fn set_lengths(&mut self, samples: &[u32; N]) {
        let longest = ((0.98 * self.max_length as f32) as u32).max(2);
        self.target = samples.map(|length| length.clamp(2, longest));
    }

    /// Jumps to the set lengths.
    pub fn snap_lengths(&mut self) {
        self.length = self.target.map(f64::from);
    }

    pub fn is_gliding(&self) -> bool {
        self.length
            .iter()
            .zip(&self.target)
            .any(|(length, target)| *length != f64::from(*target))
    }

    /// The lengths the lines are at or gliding to.
    pub fn lengths(&self) -> &[u32; N] {
        &self.target
    }

    /// The lengths now, on the way to [`Self::lengths`].
    pub fn current_lengths(&self) -> &[f64; N] {
        &self.length
    }

    /// Moves a line to the filter's coefficients over `samples` (at once
    /// with 0) and keeps its state.
    pub fn set_absorption(&mut self, line: usize, filter: &Shelf1, samples: u32) {
        self.absorb[line].set(filter, samples);
    }

    /// The scattering delay of each channel in samples (0 is none); the
    /// reads crossfade there.
    pub fn set_scatter_delays(&mut self, samples: &[u32; N]) {
        if let Some(scatter) = &mut self.scatter {
            for ((tap, line), samples) in scatter.taps.iter_mut().zip(&scatter.lines).zip(samples) {
                tap.set((*samples as usize).min(line.max_delay() as usize));
            }
        }
    }

    /// The scattering stage's mixing angle, reached in [`Self::TICK`]
    /// samples: at 0 the stage only delays and shuffles, at π/4 it mixes
    /// every channel into all.
    pub fn set_scatter_angle(&mut self, radians: f32) {
        if let Some(scatter) = &mut self.scatter {
            let (sin, cos) = radians.sin_cos();
            scatter.angle[0].set_target(cos, Self::TICK);
            scatter.angle[1].set_target(sin, Self::TICK);
        }
    }

    /// The rotation of channels `2·pair` and `2·pair + 1`, reached in
    /// [`Self::TICK`] samples. The interpolated cosine and sine have a norm
    /// of at most 1, so a moving rotation never gains.
    pub fn set_rotation(&mut self, pair: usize, radians: f32) {
        let (sin, cos) = radians.sin_cos();
        self.rotation[pair][0].set_target(cos, Self::TICK);
        self.rotation[pair][1].set_target(sin, Self::TICK);
        self.rotating = self.rotation[..N / 2]
            .iter()
            .any(|[cos, sin]| cos.value() != 1.0 || cos.target() != 1.0 || sin.target() != 0.0);
    }

    /// One sample: `input` is added to what each line stores; the lines'
    /// outputs are returned, for the caller to pick up.
    #[inline]
    pub fn process(&mut self, input: &[f32; N]) -> [f32; N] {
        let mut outputs = [0.0; N];
        for (index, output) in outputs.iter_mut().enumerate() {
            let target = f64::from(self.target[index]);
            let length = self.length[index];
            *output = if length == target {
                self.lines[index].tap(self.target[index] as usize - 1)
            } else {
                self.length[index] = glide_length(length, target, self.glide, Self::FASTEST_GLIDE);
                let whole = (length - 1.0).max(1.0).floor();
                self.lines[index].read_at(whole as usize, (length - 1.0 - whole).max(0.0) as f32)
            };
        }
        let mut mixed = [0.0; N];
        for (index, mixed) in mixed.iter_mut().enumerate() {
            let source = self.permutation[index];
            *mixed = self.signs[index] * self.absorb[source].process(outputs[source]);
        }
        hadamard(&mut mixed);
        if let Some(scatter) = &mut self.scatter {
            let mut delayed = [0.0; N];
            for index in 0..N {
                scatter.lines[index].write(mixed[index]);
                delayed[index] = scatter.taps[index].read(&scatter.lines[index]);
            }
            for index in 0..N {
                mixed[index] = scatter.signs[index] * delayed[scatter.permutation[index]];
            }
            let [cos, sin] = scatter.angle.each_mut().map(LinearRamp::next);
            butterflies(&mut mixed, cos, sin);
        }
        if self.rotating {
            for pair in 0..N / 2 {
                let [cos, sin] = self.rotation[pair].each_mut().map(LinearRamp::next);
                let (a, b) = (mixed[2 * pair], mixed[2 * pair + 1]);
                mixed[2 * pair] = cos * a + sin * b;
                mixed[2 * pair + 1] = cos * b - sin * a;
            }
        }
        for (line, (mixed, input)) in self.lines.iter_mut().zip(mixed.iter().zip(input)) {
            line.write(flush(mixed + input));
        }
        outputs
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ROOM_SCATTER_PERMUTATION, ROOM_SCATTER_SIGNS, nearest_prime};
    use super::*;
    use crate::Random;

    const LINES: usize = 16;
    const PERMUTATION: [usize; LINES] = [7, 12, 1, 10, 4, 15, 2, 9, 0, 13, 6, 11, 3, 14, 5, 8];
    const SIGNS: [f32; LINES] = [
        1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0,
    ];
    const LENGTHS: [u32; LINES] = [
        601, 683, 743, 811, 877, 947, 1019, 1087, 1163, 1229, 1301, 1381, 1453, 1531, 1607, 1693,
    ];

    fn network(lengths: &[u32; LINES]) -> Fdn<LINES> {
        let mut network = Fdn::new(PERMUTATION, SIGNS);
        network.allocate(4096, 48_000.0);
        network.set_lengths(lengths);
        network.snap_lengths();
        network
    }

    fn scattering(lengths: &[u32; LINES]) -> Fdn<LINES> {
        let mut network = network(lengths);
        network.allocate_scatter(256, 480, ROOM_SCATTER_PERMUTATION, ROOM_SCATTER_SIGNS);
        network.set_scatter_delays(&std::array::from_fn(|index| 9 * index as u32));
        network.set_scatter_angle(std::f32::consts::FRAC_PI_4);
        network.reset();
        network
    }

    /// The energy the network holds now: each stored sample leaves its line
    /// exactly once, within the line's length; what the scattering delays
    /// hold arrives in the lines first.
    fn stored_energy(network: &mut Fdn<LINES>) -> f64 {
        let lengths = *network.lengths();
        let mut energy = 0.0;
        for n in 0..*lengths.iter().max().unwrap() {
            let outputs = network.process(&[0.0; LINES]);
            for (output, length) in outputs.iter().zip(&lengths) {
                if n < *length {
                    energy += f64::from(*output).powi(2);
                }
            }
        }
        energy
    }

    #[test]
    fn network_without_absorption_keeps_its_energy() {
        assert!(LENGTHS.iter().all(|n| nearest_prime(*n) == *n));
        for (name, mut network) in [
            ("plain", network(&LENGTHS)),
            ("scattering", scattering(&LENGTHS)),
        ] {
            let mut random = Random::new(8);
            let impulse: [f32; LINES] = std::array::from_fn(|_| random.bipolar());
            let energy_in: f64 = impulse.iter().map(|x| f64::from(*x).powi(2)).sum();
            network.process(&impulse);
            for _ in 0..48_000 {
                network.process(&[0.0; LINES]);
            }
            let early = stored_energy(&mut network);
            for _ in 0..480_000 {
                network.process(&[0.0; LINES]);
            }
            let late = stored_energy(&mut network);
            let difference = 10.0 * (late / early).log10();
            assert!(difference.abs() < 0.02, "{name}: {difference} dB in 10 s");
            // The scattering delays hold a few per cent of it, which the
            // count of what leaves the lines misses.
            let held = 10.0 * (early / energy_in).log10();
            let bound = if name == "plain" { 0.01 } else { 0.4 };
            assert!(held.abs() < bound, "{name}: {held} dB of the input");
        }
    }

    #[test]
    fn a_moving_rotation_never_gains_and_loses_next_to_nothing() {
        let mut network = network(&LENGTHS);
        let mut impulse = [0.0; LINES];
        impulse[3] = 1.0;
        network.process(&impulse);
        // Full swing at 1.3 Hz for ten seconds.
        for tick in 0..15_000 {
            for pair in 0..LINES / 2 {
                let phase = 1.3 * tick as f32 * 32.0 / 48_000.0 + pair as f32 / 8.0;
                network.set_rotation(pair, 0.6 * (std::f32::consts::TAU * phase).sin());
            }
            for _ in 0..32 {
                network.process(&[0.0; LINES]);
            }
        }
        let difference = 10.0 * stored_energy(&mut network).log10();
        assert!(
            (-0.1..=0.001).contains(&difference),
            "{difference} dB after 10 s"
        );
    }

    #[test]
    fn absorption_sets_the_decay() {
        let mut network = network(&LENGTHS);
        // Half the amplitude per pass through a line of 1133 samples on
        // average: 6.02 dB per 23.6 ms.
        for line in 0..LINES {
            network.set_absorption(line, &Shelf1::with_gains(1.0, 0.5, 0.5), 0);
        }
        let mut impulse = [0.0; LINES];
        impulse[0] = 1.0;
        network.process(&impulse);
        let before = stored_energy(&mut network);
        for _ in 0..4800 {
            network.process(&[0.0; LINES]);
        }
        let after = stored_energy(&mut network);
        let passes = (4800.0 + 1693.0) / 1133.0;
        let decay_db = 10.0 * (before / after).log10();
        assert!(
            (decay_db / (6.02 * passes) - 1.0).abs() < 0.15,
            "{decay_db} dB for {passes} passes"
        );
    }

    #[test]
    fn scattering_fills_the_echoes_in_sooner() {
        // Echoes above a ten-thousandth in the first 75 ms of an impulse.
        let count = |mut network: Fdn<LINES>| {
            let mut impulse = [0.0; LINES];
            impulse[0] = 1.0;
            let mut echoes = 0;
            for n in 0..3600 {
                let input = if n == 0 { impulse } else { [0.0; LINES] };
                let outputs = network.process(&input);
                echoes += outputs.iter().filter(|x| x.abs() > 1e-4).count();
            }
            echoes
        };
        let (plain, scattered) = (count(network(&LENGTHS)), count(scattering(&LENGTHS)));
        assert!(scattered > 8 * plain, "{scattered} against {plain}");
    }

    #[test]
    fn length_changes_glide_at_a_bounded_rate() {
        let mut network = network(&LENGTHS);
        for line in 0..LINES {
            network.set_absorption(line, &Shelf1::with_gains(1.0, 0.6, 0.6), 0);
        }
        let mut n = 0;
        let mut largest_step = |network: &mut Fdn<LINES>, samples: usize| {
            let mut previous = [0.0; LINES];
            let mut lengths = *network.current_lengths();
            let mut largest = 0.0_f32;
            for index in 0..samples {
                let x = 0.1 * (0.03 * n as f32).sin();
                n += 1;
                let outputs = network.process(&[x; LINES]);
                if index > 0 {
                    for (output, previous) in outputs.iter().zip(&previous) {
                        largest = largest.max((output - previous).abs());
                    }
                }
                previous = outputs;
                for (now, before) in network.current_lengths().iter().zip(&lengths) {
                    assert!((now - before).abs() <= Fdn::<LINES>::FASTEST_GLIDE);
                }
                lengths = *network.current_lengths();
            }
            largest
        };
        let steady = largest_step(&mut network, 48_000);
        network.set_lengths(&LENGTHS.map(|length| nearest_prime(length / 2)));
        assert!(network.is_gliding());
        let gliding = largest_step(&mut network, 48_000);
        assert!(!network.is_gliding());
        assert!(
            network
                .lengths()
                .iter()
                .zip(&LENGTHS)
                .all(|(a, b)| *a < b * 6 / 10)
        );
        assert!(gliding <= 1.5 * steady + 0.01, "{gliding} against {steady}");
    }
}
