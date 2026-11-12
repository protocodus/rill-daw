//! Chorus, flanger and phaser as one kernel: an LFO sweeps a short delay
//! (chorus, flanger) or a chain of allpass stages (phaser). The kernel gives
//! the wet signal alone; whoever uses it mixes it with a dry signal delayed
//! by [`LATENCY`].
use super::{
    AlgebraicSigmoid, Allpass1, AllpassChain, Complex, DelayLine, Lfo, LfoShape, LinearRamp,
    Oversampler2x, Shape, Shelf1, Smoothed, Svf, SvfMix, flush, prewarp, tan_fast,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModulationMode {
    Chorus,
    Flanger,
    Phaser,
}

/// Plain-unit settings of a [`ModulationKernel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModulationSettings {
    pub mode: ModulationMode,
    /// Bucket-brigade (chorus, flanger) or JFET (phaser) colour.
    pub analog: bool,
    pub shape: LfoShape,
    pub rate_hz: f32,
    /// Length of an LFO cycle in beats when it follows the tempo.
    pub sync_beats: Option<f64>,
    /// 0–1.
    pub depth: f32,
    /// LFO offset of the right channel, 0–180.
    pub stereo_degrees: f32,
    /// −0.95–0.95.
    pub feedback: f32,
    /// Seconds of delay (chorus, flanger) or hertz (phaser).
    pub center: f32,
    /// 1–4, chorus only.
    pub voices: usize,
    /// 2–12, phaser only.
    pub stages: usize,
}

impl Default for ModulationSettings {
    fn default() -> Self {
        Self {
            mode: ModulationMode::Chorus,
            analog: false,
            shape: LfoShape::Sine,
            rate_hz: 0.6,
            sync_beats: None,
            depth: 0.5,
            stereo_degrees: 90.0,
            feedback: 0.0,
            center: 0.012,
            voices: 2,
            stages: 4,
        }
    }
}

/// The wet path's delay against the input, in samples at the base rate.
pub const LATENCY: u32 = Oversampler2x::LATENCY;
pub const MAX_VOICES: usize = 4;
pub const MAX_STAGES: usize = 12;
pub const MAX_FEEDBACK: f32 = 0.95;

/// Chorus: the delay swings by this much either way at full depth.
const CHORUS_SWING_SECONDS: f32 = 0.004;
/// Flanger and phaser sweep evenly in pitch, this far either way.
const FLANGER_OCTAVES: f32 = 2.0;
pub const PHASER_OCTAVES: f32 = 2.5;
const SHORTEST_CHORUS_SECONDS: f32 = 0.001;
/// In samples at the doubled rate.
const SHORTEST_DELAY: f32 = 2.0;
const MAX_DELAY_SECONDS: f32 = 0.045;
const LOWEST_CORNER_HZ: f32 = 20.0;
const HIGHEST_CORNER_HZ: f32 = 20_000.0;
/// Of the base rate: as far as [`tan_fast`] reaches at the doubled one.
const HIGHEST_CORNER_SHARE: f32 = 0.44;

/// Bandwidth of a bucket-brigade line with its filters.
const ANALOG_CUTOFF_HZ: f32 = 9500.0;
const ANALOG_BIAS: f64 = 0.12;
const ANALOG_DC_HZ: f32 = 5.0;
const SATURATOR_LIMIT: f64 = 1e4;
/// Scales the signal into a phaser stage's curve, so that full scale is
/// coloured and not flattened.
const STAGE_DRIVE: f32 = 0.5;
/// Corner mismatch of the analog phaser's stages: finite notches.
const STAGE_SPREAD: [f32; MAX_STAGES] = [
    0.021, -0.017, 0.009, -0.028, 0.030, -0.006, 0.014, -0.023, 0.004, -0.011, 0.026, -0.019,
];

/// Samples of delay per sample. A delay that grows faster than time runs
/// reads the line backwards and the same sound several times over: with
/// feedback and the analog curve that holds a tone up for ever. Only a
/// flanger with a long centre at full depth above about 5 Hz gets here.
const MAX_SLEW: f32 = 0.75;
/// The analog curve is a straight line to `f32` precision below this, and
/// in `f64` it rounds to steps of 1e-17 around its bias, which a loop would
/// hold up for ever.
const SATURATOR_FLOOR: f32 = 1e-8;
const SWEEP_SMOOTHING_SECONDS: f32 = 0.003;
const GLIDE_SECONDS: f32 = 0.15;
const DUCK_SECONDS: f32 = 0.005;
/// In cycles: more than the phase moves by itself between two samples.
const JUMP: f64 = 1e-3;
const LFO_SEED: u32 = 0x0C40_2A5E;

/// Voices that sound: one outside chorus.
fn voice_count(settings: &ModulationSettings) -> usize {
    if settings.mode == ModulationMode::Chorus {
        settings.voices.clamp(1, MAX_VOICES)
    } else {
        1
    }
}

/// LFO offset of the right channel in cycles. A share of the voice spacing:
/// a plain offset would put the right channel's voices on the left's at 180°
/// with two voices.
pub fn channel_offset(settings: &ModulationSettings) -> f64 {
    f64::from(settings.stereo_degrees) / (360.0 * voice_count(settings) as f64)
}

fn sweep_law(mode: ModulationMode, depth: f32, center: f32, lfo: f32, sample_rate: f32) -> f32 {
    let doubled = 2.0 * sample_rate;
    let longest = MAX_DELAY_SECONDS * doubled;
    match mode {
        ModulationMode::Chorus => {
            ((center + depth * CHORUS_SWING_SECONDS * lfo).max(SHORTEST_CHORUS_SECONDS) * doubled)
                .min(longest)
        }
        ModulationMode::Flanger => (center * (FLANGER_OCTAVES * depth * lfo).exp2() * doubled)
            .clamp(SHORTEST_DELAY, longest),
        ModulationMode::Phaser => (center * (PHASER_OCTAVES * depth * lfo).exp2()).clamp(
            LOWEST_CORNER_HZ,
            HIGHEST_CORNER_HZ.min(HIGHEST_CORNER_SHARE * sample_rate),
        ),
    }
}

/// Where an LFO value (−1–1) puts the sweep: the delay in samples at twice
/// `sample_rate` (chorus, flanger) or the stages' corner in hertz (phaser).
/// The one law for the audio path and for drawing.
pub fn swept(settings: &ModulationSettings, lfo: f32, sample_rate: f32) -> f32 {
    sweep_law(
        settings.mode,
        settings.depth,
        settings.center,
        lfo,
        sample_rate,
    )
}

/// Integrator gains of the phaser's stages for a corner.
#[inline]
fn stage_gains(corner_hz: f32, analog: bool, doubled_rate: f32) -> [f32; MAX_STAGES] {
    let g = tan_fast((std::f32::consts::PI * corner_hz / doubled_rate).min(0.7));
    if analog {
        STAGE_SPREAD.map(|spread| g * (1.0 + spread))
    } else {
        [g; MAX_STAGES]
    }
}

/// Complex response of the wet path at `hz` with the LFO at `lfo` for each
/// voice (one value outside chorus), small signals, without the kernel's
/// latency: what is mixed with the dry signal.
pub fn wet_response(
    settings: &ModulationSettings,
    lfo: &[f32],
    sample_rate: f32,
    hz: f64,
) -> Complex {
    let doubled = 2.0 * sample_rate;
    let omega = std::f64::consts::TAU * hz / f64::from(doubled);
    let feedback = f64::from(settings.feedback.clamp(-MAX_FEEDBACK, MAX_FEEDBACK));
    let looped = |path: Complex| path / (Complex::ONE - path.scale(feedback));
    if settings.mode == ModulationMode::Phaser {
        let corner = swept(settings, lfo.first().copied().unwrap_or(0.0), sample_rate);
        let gains = stage_gains(corner, settings.analog, doubled);
        return looped(
            gains[..settings.stages.clamp(1, MAX_STAGES)].iter().fold(
                Complex::ONE,
                |product, g| {
                    product * Allpass1::response(Allpass1::coefficient(f64::from(*g)), omega)
                },
            ),
        );
    }
    let voices = voice_count(settings).min(lfo.len()).max(1);
    let longest = MAX_DELAY_SECONDS * doubled;
    let mut sum = Complex::new(0.0, 0.0);
    for value in lfo.iter().take(voices) {
        // As the audio path reads: one sample less than the delay, the
        // write that follows the read making up for it.
        let position = (swept(settings, *value, sample_rate) - 1.0).clamp(1.0, longest);
        let whole = position.floor();
        sum = sum
            + DelayLine::lagrange_response(position - whole, omega)
                * Complex::unit(-omega * (f64::from(whole) + 1.0));
    }
    let mut path = sum.scale(1.0 / voices as f64);
    if settings.analog {
        let low_pass = SvfMix::<f64>::new(
            f64::from(prewarp(ANALOG_CUTOFF_HZ, doubled)),
            std::f64::consts::SQRT_2,
            0.0,
            0.0,
            1.0,
        )
        .response(omega);
        let blocker = Shelf1::high_pass(ANALOG_DC_HZ, doubled).response(omega);
        path = path * low_pass * low_pass * blocker;
    }
    // The loop runs on the voices' average; the output is their sum at the
    // level of one voice.
    looped(path).scale((voices as f64).sqrt())
}

/// Seconds until the wet signal has fallen by 60 dB after the input stops,
/// without the kernel's latency, at most 10.
pub fn tail_seconds(settings: &ModulationSettings) -> f32 {
    let pass = match settings.mode {
        ModulationMode::Chorus => settings.center + settings.depth * CHORUS_SWING_SECONDS,
        ModulationMode::Flanger => settings.center * (FLANGER_OCTAVES * settings.depth).exp2(),
        ModulationMode::Phaser => {
            // 1/(π·corner) is the longest group delay of a stage.
            let lowest =
                (settings.center * (-PHASER_OCTAVES * settings.depth).exp2()).max(LOWEST_CORNER_HZ);
            settings.stages as f32 / (std::f32::consts::PI * lowest)
        }
    };
    let feedback = settings.feedback.abs().min(MAX_FEEDBACK);
    let passes = if feedback > 0.0 {
        1.0 + (3.0 / -feedback.log10()).ceil()
    } else {
        1.0
    };
    (pass * passes).min(10.0)
}

/// What the audio path is switched between under a short duck, because it
/// cannot change smoothly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Core {
    mode: ModulationMode,
    analog: bool,
    voices: usize,
    stages: usize,
}

impl Core {
    fn of(settings: &ModulationSettings) -> Self {
        Self {
            mode: settings.mode,
            analog: settings.analog,
            voices: voice_count(settings),
            // Only the phaser has stages; elsewhere the count changes nothing.
            stages: if settings.mode == ModulationMode::Phaser {
                settings.stages.clamp(1, MAX_STAGES)
            } else {
                0
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModulationKernel {
    sample_rate: f32,
    settings: ModulationSettings,
    core: Core,
    pending: Option<Core>,
    /// Wet gain around a switch of the core.
    duck: LinearRamp,
    lfo: Lfo,
    /// Per channel and voice: the LFO value last used, the same after the
    /// sweep smoothing, and the swept delay or corner of the last frame.
    blended: [[f32; MAX_VOICES]; 2],
    sweep: [[f32; MAX_VOICES]; 2],
    previous: [[f32; MAX_VOICES]; 2],
    sweep_coefficient: f32,
    /// LFO values of the moment before a discontinuity, and the blend from
    /// them to the live ones.
    held: [[f32; MAX_VOICES]; 2],
    glide: LinearRamp,
    /// Where the phase should be at the next frame if nothing jumps.
    expected_phase: f64,
    started: bool,
    stereo: Smoothed,
    depth: Smoothed,
    center: Smoothed,
    feedback: Smoothed,
    over: [Oversampler2x; 2],
    lines: [DelayLine; 2],
    low_pass_in: [Svf; 2],
    low_pass_out: [Svf; 2],
    blocker: [Shelf1; 2],
    analog_g: f32,
    saturator: AlgebraicSigmoid,
    chain: [AllpassChain<MAX_STAGES>; 2],
}

impl Default for ModulationKernel {
    fn default() -> Self {
        let settings = ModulationSettings::default();
        Self {
            sample_rate: 48_000.0,
            settings,
            core: Core::of(&settings),
            pending: None,
            duck: LinearRamp::new(1.0),
            lfo: Lfo::new(LFO_SEED),
            blended: [[0.0; MAX_VOICES]; 2],
            sweep: [[0.0; MAX_VOICES]; 2],
            previous: [[0.0; MAX_VOICES]; 2],
            sweep_coefficient: 1.0,
            held: [[0.0; MAX_VOICES]; 2],
            glide: LinearRamp::new(1.0),
            expected_phase: 0.0,
            started: false,
            stereo: Smoothed::new(settings.stereo_degrees),
            depth: Smoothed::new(settings.depth),
            center: Smoothed::new(settings.center),
            feedback: Smoothed::new(settings.feedback),
            over: [Oversampler2x::new(), Oversampler2x::new()],
            lines: [DelayLine::default(), DelayLine::default()],
            low_pass_in: [Svf::default(), Svf::default()],
            low_pass_out: [Svf::default(), Svf::default()],
            blocker: [Shelf1::default(), Shelf1::default()],
            analog_g: 0.0,
            saturator: AlgebraicSigmoid::new(1.0, 1.0, ANALOG_BIAS),
            chain: [AllpassChain::default(), AllpassChain::default()],
        }
    }
}

impl ModulationKernel {
    /// Allocates for a sample rate and resets.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let doubled = 2.0 * sample_rate;
        for line in &mut self.lines {
            line.allocate((MAX_DELAY_SECONDS * doubled).ceil() as usize);
        }
        self.sweep_coefficient = 1.0 - (-1.0 / (SWEEP_SMOOTHING_SECONDS * sample_rate)).exp();
        self.analog_g = prewarp(ANALOG_CUTOFF_HZ, doubled);
        let blocker = Shelf1::high_pass(ANALOG_DC_HZ, doubled);
        self.blocker = [blocker.clone(), blocker];
        for smoother in [
            &mut self.stereo,
            &mut self.depth,
            &mut self.center,
            &mut self.feedback,
        ] {
            smoother.set_time(20.0, sample_rate);
        }
        self.reset();
    }

    /// New targets. Continuous values glide; mode, colour, voices and
    /// stages are exchanged under a short duck of the wet signal.
    pub fn set(&mut self, settings: &ModulationSettings) {
        let shape_changed = settings.shape != self.settings.shape;
        self.settings = *settings;
        if shape_changed && self.started {
            self.hold();
        }
        self.stereo.set_target(settings.stereo_degrees);
        self.depth.set_target(settings.depth.clamp(0.0, 1.0));
        self.feedback
            .set_target(settings.feedback.clamp(-MAX_FEEDBACK, MAX_FEEDBACK));
        let core = Core::of(settings);
        // The centre is a delay in one mode and a frequency in another: it
        // changes with the core, not before.
        if core.mode == self.core.mode {
            self.center.set_target(settings.center);
        }
        let fade = (DUCK_SECONDS * self.sample_rate) as u32;
        let aim = if core == self.core {
            self.pending = None;
            1.0
        } else {
            self.pending = Some(core);
            0.0
        };
        if self.duck.target() != aim {
            self.duck.set_target(aim, fade);
        }
    }

    /// Clears the audio state and settles on the settings.
    pub fn reset(&mut self) {
        self.core = Core::of(&self.settings);
        self.pending = None;
        self.duck = LinearRamp::new(1.0);
        self.lfo.reset();
        self.glide = LinearRamp::new(1.0);
        self.started = false;
        self.center.set_target(self.settings.center);
        for smoother in [
            &mut self.stereo,
            &mut self.depth,
            &mut self.center,
            &mut self.feedback,
        ] {
            smoother.snap();
        }
        for over in &mut self.over {
            over.reset();
        }
        self.clear_core();
    }

    fn clear_core(&mut self) {
        for channel in 0..2 {
            self.lines[channel].reset();
            self.low_pass_in[channel].reset();
            self.low_pass_out[channel].reset();
            self.blocker[channel].reset();
            self.chain[channel].reset();
        }
    }

    pub fn latency(&self) -> u32 {
        LATENCY
    }

    /// LFO phase in cycles.
    pub fn phase(&self) -> f64 {
        self.lfo.phase()
    }

    /// Where each channel's voices are in their sweep, as LFO values
    /// (−1–1); only the voices that sound are kept up to date.
    pub fn sweep(&self) -> &[[f32; MAX_VOICES]; 2] {
        &self.sweep
    }

    /// Keeps the LFO values of the moment and blends to the live ones: a
    /// jump of the phase or a new shape would otherwise move every delay
    /// within the sweep smoothing, a chirp.
    fn hold(&mut self) {
        self.held = self.blended;
        self.glide = LinearRamp::new(0.0);
        self.glide
            .set_target(1.0, (GLIDE_SECONDS * self.sample_rate) as u32);
    }

    fn live(&self, channel: usize, voice: usize, stereo: f32) -> f32 {
        let offset =
            (voice as f64 + channel as f64 * f64::from(stereo) / 360.0) / self.core.voices as f64;
        self.lfo.value(self.settings.shape, offset)
    }

    /// Puts the sweep where the LFO is, without smoothing or glide.
    fn snap_sweep(&mut self) {
        let (stereo, depth, center) =
            (self.stereo.value(), self.depth.value(), self.center.value());
        for channel in 0..2 {
            for voice in 0..self.core.voices {
                let value = self.live(channel, voice, stereo);
                self.blended[channel][voice] = value;
                self.sweep[channel][voice] = value;
                self.previous[channel][voice] =
                    sweep_law(self.core.mode, depth, center, value, self.sample_rate);
            }
        }
        self.glide = LinearRamp::new(1.0);
    }

    fn switch(&mut self, core: Core) {
        let voices_only = Core {
            voices: core.voices,
            ..self.core
        } == core;
        self.core = core;
        self.center.set_target(self.settings.center);
        self.center.snap();
        if !voices_only {
            self.clear_core();
        }
        // The phase of every voice depends on the count.
        self.snap_sweep();
    }

    /// One frame: the song position drives a synced LFO while `playing`
    /// (`beat` is this frame's); returns the wet signal, [`LATENCY`] samples
    /// late.
    #[inline]
    pub fn process(
        &mut self,
        tempo: f64,
        beat: f64,
        playing: bool,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        let rate = f64::from(self.sample_rate);
        let increment = match self.settings.sync_beats {
            Some(beats) => tempo / (60.0 * rate * beats),
            None => f64::from(self.settings.rate_hz) / rate,
        };
        match self.settings.sync_beats {
            Some(beats) if playing => self.lfo.set_phase(beat / beats),
            _ => self.lfo.advance(increment),
        }
        let phase = self.lfo.phase();
        // Both are phases, so one turn brings the difference into 0..1
        // without a remainder.
        let away = phase - self.expected_phase;
        let away = if away < 0.0 { away + 1.0 } else { away };
        let jumped = self.started && away.min(1.0 - away) > JUMP;
        self.expected_phase = phase + increment;
        if jumped {
            self.hold();
        }

        let duck_before = self.duck.value();
        let duck = self.duck.next();
        let mut switched = false;
        if duck == 0.0
            && let Some(core) = self.pending.take()
        {
            switched = true;
            self.switch(core);
            self.duck
                .set_target(1.0, (DUCK_SECONDS * self.sample_rate) as u32);
        }
        let stereo = self.stereo.next_value();
        let depth = self.depth.next_value();
        let center = self.center.next_value();
        let feedback = self.feedback.next_value();
        if !self.started {
            self.started = true;
            self.snap_sweep();
        }
        let blend = self.glide.next();
        let core = self.core;
        let voice_gain = (core.voices as f32).sqrt();
        let doubled = 2.0 * self.sample_rate;
        // Nothing of the new core sounds in the frame of the switch.
        let ducks = if switched {
            [0.0; 2]
        } else {
            [0.5 * (duck_before + duck), duck]
        };
        let input = [left, right];
        let mut wet = [0.0; 2];
        for channel in 0..2 {
            let mut positions = [[0.0; MAX_VOICES]; 2];
            let [first, second] = &mut positions;
            for (voice, (first, second)) in first
                .iter_mut()
                .zip(second.iter_mut())
                .enumerate()
                .take(core.voices)
            {
                let live = self.live(channel, voice, stereo);
                let held = self.held[channel][voice];
                let value = if blend < 1.0 {
                    held + blend * (live - held)
                } else {
                    live
                };
                self.blended[channel][voice] = value;
                let sweep = &mut self.sweep[channel][voice];
                *sweep += self.sweep_coefficient * (value - *sweep);
                let mut now = sweep_law(core.mode, depth, center, *sweep, self.sample_rate);
                let before = self.previous[channel][voice];
                if core.mode != ModulationMode::Phaser {
                    now = now.clamp(before - 2.0 * MAX_SLEW, before + 2.0 * MAX_SLEW);
                }
                self.previous[channel][voice] = now;
                *first = 0.5 * (before + now);
                *second = now;
            }
            let mut pair = self.over[channel].upsample(flush(input[channel]));
            for (index, sample) in pair.iter_mut().enumerate() {
                let x = *sample;
                let y = if core.mode == ModulationMode::Phaser {
                    let mut gains = stage_gains(positions[index][0], core.analog, doubled);
                    for g in &mut gains[..core.stages] {
                        *g /= 1.0 + *g;
                    }
                    let chain = &mut self.chain[channel];
                    ducks[index]
                        * if core.analog {
                            chain.process_saturated(x, &gains, core.stages, feedback, STAGE_DRIVE)
                        } else {
                            chain.process(x, &gains, core.stages, feedback)
                        }
                } else {
                    let line = &mut self.lines[channel];
                    // Read before the write: a read one sample short makes a
                    // loop of exactly the delay.
                    let mut delayed = 0.0;
                    for position in &positions[index][..core.voices] {
                        delayed += line.read(position - 1.0);
                    }
                    // Ducked where it is read, so that nothing that jumps at
                    // a switch is heard or written back, and where it is
                    // written, so that a cleared line fills without an edge.
                    delayed *= ducks[index] / core.voices as f32;
                    if core.analog {
                        delayed = self.low_pass_out[channel]
                            .process(delayed, self.analog_g, std::f32::consts::SQRT_2)
                            .low;
                    }
                    let mut stored = ducks[index] * x + feedback * delayed;
                    if core.analog {
                        stored = self.low_pass_in[channel]
                            .process(stored, self.analog_g, std::f32::consts::SQRT_2)
                            .low;
                        if stored.abs() > SATURATOR_FLOOR {
                            stored = self
                                .saturator
                                .value(f64::from(stored).clamp(-SATURATOR_LIMIT, SATURATOR_LIMIT))
                                as f32;
                        }
                        // The curve's bias turns signal into an offset, which
                        // a circuit's coupling capacitors would block.
                        stored = self.blocker[channel].process(stored);
                    }
                    line.write(flush(stored));
                    // The loop runs on the voices' average, which never
                    // exceeds one voice; the output on their sum at the
                    // level of one, as uncorrelated signals add.
                    delayed * voice_gain
                };
                *sample = flush(y);
            }
            wet[channel] = self.over[channel].downsample(pair);
        }
        (wet[0], wet[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn kernel(settings: &ModulationSettings) -> ModulationKernel {
        let mut kernel = ModulationKernel::default();
        kernel.prepare(RATE);
        kernel.set(settings);
        kernel.reset();
        kernel
    }

    fn still(mode: ModulationMode, center: f32) -> ModulationSettings {
        ModulationSettings {
            mode,
            center,
            depth: 0.0,
            voices: 1,
            ..ModulationSettings::default()
        }
    }

    /// Level of the settled wet signal for a sine, by quadrature.
    fn wet_gain(kernel: &mut ModulationKernel, hz: f64, amplitude: f64) -> f64 {
        const WINDOW: usize = 9600;
        let (mut sine, mut cosine) = (0.0, 0.0);
        for n in 0..48_000 {
            let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(RATE);
            let x = (amplitude * phase.sin()) as f32;
            let (y, _) = kernel.process(120.0, 0.0, false, x, x);
            if n >= 48_000 - WINDOW {
                sine += f64::from(y) * phase.sin();
                cosine += f64::from(y) * phase.cos();
            }
        }
        2.0 * sine.hypot(cosine) / (WINDOW as f64 * amplitude)
    }

    #[test]
    fn the_wet_path_is_the_delay_it_says_plus_the_latency() {
        let settings = still(ModulationMode::Flanger, 0.002);
        let mut kernel = kernel(&settings);
        assert!((swept(&settings, 0.7, RATE) - 192.0).abs() < 1e-3);
        let response: Vec<f32> = (0..400)
            .map(|n| {
                kernel
                    .process(120.0, 0.0, false, if n == 0 { 1.0 } else { 0.0 }, 0.0)
                    .0
            })
            .collect();
        let peak = (0..400).max_by(|a, b| response[*a].abs().total_cmp(&response[*b].abs()));
        assert_eq!(peak, Some(LATENCY as usize + 96));
        assert!(response[LATENCY as usize + 96] > 0.8);
    }

    #[test]
    fn measured_gain_matches_the_response_in_every_mode() {
        let cases = [
            (ModulationMode::Chorus, 0.012, 0.0),
            (ModulationMode::Flanger, 0.0013, 0.7),
            (ModulationMode::Flanger, 0.0013, -0.7),
            (ModulationMode::Phaser, 800.0, 0.0),
            (ModulationMode::Phaser, 800.0, 0.7),
        ];
        for (mode, center, feedback) in cases {
            for analog in [false, true] {
                let settings = ModulationSettings {
                    feedback,
                    analog,
                    stages: 6,
                    ..still(mode, center)
                };
                let mut kernel = kernel(&settings);
                for hz in [90.0, 440.0, 1000.0, 3300.0, 9000.0] {
                    kernel.reset();
                    let measured = wet_gain(&mut kernel, hz, 0.01);
                    let expected = wet_response(&settings, &[0.0], RATE, hz).norm();
                    let error_db = 20.0 * (measured / expected).log10();
                    assert!(
                        error_db.abs() < 0.1,
                        "{mode:?} analog {analog} feedback {feedback} at {hz} Hz: {error_db} dB"
                    );
                }
            }
        }
    }

    #[test]
    fn sweep_laws_cover_their_ranges() {
        let chorus = ModulationSettings {
            depth: 1.0,
            ..ModulationSettings::default()
        };
        assert!((swept(&chorus, 1.0, RATE) - 0.016 * 96_000.0).abs() < 0.01);
        assert!((swept(&chorus, -1.0, RATE) - 0.008 * 96_000.0).abs() < 0.01);
        let flanger = ModulationSettings {
            mode: ModulationMode::Flanger,
            center: 0.01,
            depth: 1.0,
            ..chorus
        };
        assert!((swept(&flanger, 1.0, RATE) - 0.04 * 96_000.0).abs() < 0.01);
        assert!((swept(&flanger, -1.0, RATE) - 0.0025 * 96_000.0).abs() < 0.01);
        let tiny = ModulationSettings {
            center: 0.000_02,
            ..flanger
        };
        assert_eq!(swept(&tiny, -1.0, 44_100.0), SHORTEST_DELAY);
        let phaser = ModulationSettings {
            mode: ModulationMode::Phaser,
            center: 8000.0,
            depth: 1.0,
            ..chorus
        };
        assert_eq!(swept(&phaser, 1.0, RATE), 20_000.0);
        assert_eq!(swept(&phaser, 1.0, 44_100.0), 0.44 * 44_100.0);
        assert!((swept(&phaser, -1.0, RATE) - 8000.0 / 5.656_854).abs() < 0.1);
        // 180° puts the right voices midway between the left ones.
        assert_eq!(
            channel_offset(&ModulationSettings {
                stereo_degrees: 180.0,
                ..chorus
            }),
            0.25
        );
        assert_eq!(
            channel_offset(&ModulationSettings {
                stereo_degrees: 180.0,
                ..flanger
            }),
            0.5
        );
    }

    #[test]
    fn tails_follow_the_feedback() {
        let chorus = ModulationSettings::default();
        assert!((tail_seconds(&chorus) - 0.014).abs() < 1e-6);
        let flanger = ModulationSettings {
            mode: ModulationMode::Flanger,
            center: 0.01,
            depth: 1.0,
            feedback: 0.95,
            ..chorus
        };
        // 135 passes of 40 ms to fall by 60 dB.
        assert!((tail_seconds(&flanger) - 0.04 * 136.0).abs() < 1e-3);
        let phaser = ModulationSettings {
            mode: ModulationMode::Phaser,
            center: 100.0,
            depth: 1.0,
            feedback: -0.95,
            stages: 12,
            ..chorus
        };
        assert_eq!(tail_seconds(&phaser), 10.0);
    }

    fn noise(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / 8_388_608.0 - 1.0
    }

    /// RMS of the wet signal over `frames` of noise or of silence.
    fn wet_level(kernel: &mut ModulationKernel, frames: usize, seed: Option<&mut u32>) -> f64 {
        let mut seed = seed;
        let mut sum = 0.0;
        for _ in 0..frames {
            let x = seed.as_deref_mut().map_or(0.0, noise);
            let (y, _) = kernel.process(120.0, 0.0, false, x, x);
            sum += f64::from(y) * f64::from(y);
        }
        (sum / frames as f64).sqrt()
    }

    #[test]
    fn the_fastest_sweep_falls_silent_within_the_tail() {
        for analog in [true, false] {
            for shape in [LfoShape::Triangle, LfoShape::Sine] {
                for feedback in [MAX_FEEDBACK, -MAX_FEEDBACK] {
                    let settings = ModulationSettings {
                        mode: ModulationMode::Flanger,
                        analog,
                        shape,
                        rate_hz: 20.0,
                        depth: 1.0,
                        feedback,
                        center: 0.01,
                        ..ModulationSettings::default()
                    };
                    let mut kernel = kernel(&settings);
                    let mut seed = 1;
                    let playing = wet_level(&mut kernel, 2 * RATE as usize, Some(&mut seed));
                    let tail = (tail_seconds(&settings) * RATE) as usize + LATENCY as usize;
                    wet_level(&mut kernel, tail, None);
                    let after = wet_level(&mut kernel, RATE as usize, None);
                    assert!(
                        after < 1e-3 * playing,
                        "analog {analog} {shape:?} feedback {feedback}: {after} after {playing}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_analog_loop_decays_to_exact_silence() {
        for (mode, center) in [
            (ModulationMode::Chorus, 0.012),
            (ModulationMode::Flanger, 0.002),
        ] {
            let settings = ModulationSettings {
                mode,
                center,
                analog: true,
                feedback: 0.9,
                ..ModulationSettings::default()
            };
            let mut kernel = kernel(&settings);
            let mut seed = 1;
            wet_level(&mut kernel, RATE as usize / 2, Some(&mut seed));
            wet_level(&mut kernel, 12 * RATE as usize, None);
            assert_eq!(wet_level(&mut kernel, RATE as usize, None), 0.0, "{mode:?}");
        }
    }

    #[test]
    fn switches_duck_and_changes_glide() {
        let settings = ModulationSettings {
            depth: 1.0,
            rate_hz: 3.0,
            ..ModulationSettings::default()
        };
        let mut kernel = kernel(&settings);
        let mut n = 0;
        let mut largest_step = |kernel: &mut ModulationKernel, frames: usize| {
            let mut previous: Option<f32> = None;
            let mut largest = 0.0_f32;
            for _ in 0..frames {
                let x = 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / RATE).sin();
                n += 1;
                let (y, _) = kernel.process(120.0, 0.0, false, x, x);
                if let Some(previous) = previous {
                    largest = largest.max((y - previous).abs());
                }
                previous = Some(y);
            }
            largest
        };
        largest_step(&mut kernel, 4800);
        let steady = largest_step(&mut kernel, 48_000);
        let changes = [
            ModulationSettings {
                voices: 4,
                ..settings
            },
            ModulationSettings {
                analog: true,
                ..settings
            },
            ModulationSettings {
                mode: ModulationMode::Phaser,
                center: 800.0,
                ..settings
            },
            ModulationSettings {
                mode: ModulationMode::Phaser,
                center: 800.0,
                stages: 12,
                ..settings
            },
            ModulationSettings {
                shape: LfoShape::Triangle,
                ..settings
            },
            ModulationSettings {
                shape: LfoShape::Smooth,
                ..settings
            },
            ModulationSettings {
                mode: ModulationMode::Flanger,
                center: 0.002,
                ..settings
            },
            settings,
        ];
        for change in changes {
            kernel.set(&change);
            let step = largest_step(&mut kernel, 24_000);
            assert!(
                step <= 1.5 * steady + 0.02,
                "{change:?}: {step} against {steady}"
            );
        }
    }

    #[test]
    fn a_synced_lfo_follows_the_song_and_stops_with_it() {
        let settings = ModulationSettings {
            sync_beats: Some(4.0),
            ..ModulationSettings::default()
        };
        let mut kernel = kernel(&settings);
        let beat = |n: usize| 3.0 + n as f64 * 2.0 / f64::from(RATE);
        for n in 0..=24_000 {
            kernel.process(120.0, beat(n), true, 0.0, 0.0);
        }
        // Beat 4 is a bar line: the cycle starts again.
        assert!(kernel.phase() < 1e-9 || kernel.phase() > 1.0 - 1e-9);
        // Stopped, it runs on at the tempo's rate: a quarter cycle per beat.
        for _ in 0..24_000 {
            kernel.process(120.0, 0.0, false, 0.0, 0.0);
        }
        assert!((kernel.phase() - 0.25).abs() < 1e-6, "{}", kernel.phase());
    }
}
