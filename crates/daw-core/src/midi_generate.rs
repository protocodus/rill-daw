//! Bounded, deterministic note drafts for the piano-roll generator.
//!
//! Drafting never allocates project IDs or mutates a clip. Callers allocate IDs
//! only when accepting the exact draft that was shown in the preview.
use crate::{Chord, MAX_BEAT, Scale};
use anyhow::{Result, ensure};

pub const MAX_GENERATED_NOTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerateKind {
    Chord,
    Arpeggio,
    Melody,
}

impl GenerateKind {
    pub const ALL: [Self; 3] = [Self::Chord, Self::Arpeggio, Self::Melody];

    pub fn name(self) -> &'static str {
        match self {
            Self::Chord => "Chord",
            Self::Arpeggio => "Arpeggio",
            Self::Melody => "Melody",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerateHarmony {
    Chord(Chord),
    Scale(Scale),
}

impl GenerateHarmony {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chord(chord) => chord.name(),
            Self::Scale(scale) => scale.name(),
        }
    }

    pub fn contains(self, root: u8, pitch: u8) -> bool {
        match self {
            Self::Chord(chord) => chord.contains(root, pitch),
            Self::Scale(scale) => scale.contains(root, pitch),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpeggioPattern {
    Up,
    Down,
    UpDown,
    DownUp,
    OutsideIn,
    InsideOut,
    Random,
}

impl ArpeggioPattern {
    pub const ALL: [Self; 7] = [
        Self::Up,
        Self::Down,
        Self::UpDown,
        Self::DownUp,
        Self::OutsideIn,
        Self::InsideOut,
        Self::Random,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "Up",
            Self::Down => "Down",
            Self::UpDown => "Up / down",
            Self::DownUp => "Down / up",
            Self::OutsideIn => "Outside in",
            Self::InsideOut => "Inside out",
            Self::Random => "Random",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MelodyAlgorithm {
    RandomWalk,
    Motif,
    Euclidean,
}

impl MelodyAlgorithm {
    pub const ALL: [Self; 3] = [Self::RandomWalk, Self::Motif, Self::Euclidean];

    pub fn name(self) -> &'static str {
        match self {
            Self::RandomWalk => "Random walk",
            Self::Motif => "Motif variation",
            Self::Euclidean => "Euclidean rhythm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accompaniment {
    None,
    Counterpoint,
    Tintinnabuli,
}

impl Accompaniment {
    pub const ALL: [Self; 3] = [Self::None, Self::Counterpoint, Self::Tintinnabuli];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Counterpoint => "Counterpoint",
            Self::Tintinnabuli => "Tintinnabuli",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenerateSettings {
    pub kind: GenerateKind,
    pub harmony: GenerateHarmony,
    pub root: u8,
    /// Audible, clip-local beats. The application converts at the model boundary.
    pub start: f64,
    pub length: f64,
    pub octaves: u8,
    /// Scientific pitch notation: C4 is MIDI pitch 60.
    pub start_octave: i8,
    /// Inversion in harmony degrees; positive shifts raise the lowest tones.
    pub shift: i8,
    pub arp: ArpeggioPattern,
    /// One is an eighth note (half a quarter-note beat).
    pub speed_multiplier: f64,
    pub melody: MelodyAlgorithm,
    pub accompaniment: Accompaniment,
    pub additional_voices: u8,
    pub seed: u64,
    pub gate: f64,
    pub density: f64,
    pub step_size: u8,
    pub motif_steps: u8,
    pub velocity: u8,
}

impl Default for GenerateSettings {
    fn default() -> Self {
        Self {
            kind: GenerateKind::Chord,
            harmony: GenerateHarmony::Chord(Chord::Major),
            root: 0,
            start: 0.0,
            length: 4.0,
            octaves: 1,
            start_octave: 4,
            shift: 0,
            arp: ArpeggioPattern::Up,
            speed_multiplier: 1.0,
            melody: MelodyAlgorithm::RandomWalk,
            accompaniment: Accompaniment::None,
            additional_voices: 1,
            seed: 1,
            gate: 0.9,
            density: 0.8,
            step_size: 2,
            motif_steps: 8,
            velocity: 100,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedNote {
    pub pitch: u8,
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
    /// Zero is the lead; additional voices have stable distinct preview colors.
    pub voice: u8,
}

/// Return (start beat, length in beats) for bars entirely inside the viewport.
/// No outward epsilon is used: even slightly clipped bars remain excluded.
pub fn complete_visible_bars(start: f64, end: f64, beats_per_bar: u8) -> Option<(f64, f64)> {
    if !start.is_finite() || !end.is_finite() || end <= start || beats_per_bar == 0 {
        return None;
    }
    let bar = f64::from(beats_per_bar);
    let mut first = (start.max(0.0) / bar).ceil() * bar;
    let mut last = (end / bar).floor() * bar;
    // Division can round to an integer at very large positions. Retain the
    // inward-only contract after multiplication as well.
    if first < start {
        first += bar;
    }
    if last > end {
        last -= bar;
    }
    (first.is_finite() && last.is_finite() && first >= start && last <= end && last > first)
        .then_some((first, last - first))
}

/// Build an immutable draft. Invalid or excessively large requests fail before
/// allocating notes; output never silently truncates the requested duration.
pub fn generate_notes(settings: &GenerateSettings) -> Result<Vec<GeneratedNote>> {
    validate(settings)?;
    let pool = pitch_pool(settings)?;
    if settings.kind == GenerateKind::Chord {
        return Ok(pool
            .into_iter()
            .map(|pitch| GeneratedNote {
                pitch,
                start: settings.start,
                length: settings.length,
                velocity: settings.velocity,
                voice: 0,
            })
            .collect());
    }
    let step = 0.5 / settings.speed_multiplier;
    ensure!(
        settings.start + step * settings.gate > settings.start,
        "The generation start is too large for the chosen note length"
    );
    let slots = (settings.length / step).ceil();
    let voices =
        if settings.kind == GenerateKind::Melody && settings.accompaniment != Accompaniment::None {
            usize::from(settings.additional_voices) + 1
        } else {
            1
        };
    ensure!(
        slots <= (MAX_GENERATED_NOTES / voices) as f64,
        "Too many generated notes; shorten Length or reduce Speed or Voices (maximum {MAX_GENERATED_NOTES})"
    );
    let slots = slots as usize;
    let mut rng = Random(settings.seed);
    let mut notes = Vec::with_capacity(slots * voices);
    let mut current = 0;
    let order = arpeggio_order(settings.arp, pool.len());
    let motif =
        if settings.kind == GenerateKind::Melody && settings.melody == MelodyAlgorithm::Motif {
            make_motif(settings, pool.len(), &mut rng)
        } else {
            Vec::new()
        };
    let mut variation = 0;
    for slot in 0..slots {
        let pitch_index = if settings.kind == GenerateKind::Arpeggio {
            if settings.arp == ArpeggioPattern::Random {
                current = if slot == 0 {
                    rng.below(pool.len())
                } else {
                    let next = rng.below(pool.len().saturating_sub(1).max(1));
                    if pool.len() > 1 && next >= current {
                        next + 1
                    } else {
                        next
                    }
                };
                current
            } else {
                order[slot % order.len()]
            }
        } else {
            let cycle = usize::from(settings.motif_steps);
            let active = match settings.melody {
                MelodyAlgorithm::Euclidean => {
                    let pulses = (settings.density * cycle as f64).round() as usize;
                    (slot % cycle * pulses) % cycle < pulses
                }
                MelodyAlgorithm::Motif => motif[slot % cycle].1,
                MelodyAlgorithm::RandomWalk => {
                    settings.density > 0.0 && (slot == 0 || rng.unit() < settings.density)
                }
            };
            if !active {
                continue;
            }
            match settings.melody {
                MelodyAlgorithm::Motif => {
                    if slot % cycle == 0 {
                        variation = if slot == 0 {
                            0
                        } else {
                            rng.below(5) as i32 - 2
                        };
                    }
                    let degree = motif[slot % cycle].0 as i32 + variation;
                    degree.clamp(0, pool.len() as i32 - 1) as usize
                }
                MelodyAlgorithm::RandomWalk | MelodyAlgorithm::Euclidean => {
                    if slot > 0 {
                        current = walk(current, pool.len(), settings.step_size, &mut rng);
                    }
                    current
                }
            }
        };
        let start = settings.start + slot as f64 * step;
        let length = (step * settings.gate).min(settings.start + settings.length - start);
        if length > 0.0 {
            notes.push(GeneratedNote {
                pitch: pool[pitch_index],
                start,
                length,
                velocity: settings.velocity,
                voice: 0,
            });
        }
    }
    if voices > 1 {
        add_voices(&mut notes, settings, &pool);
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.voice.cmp(&b.voice)));
    Ok(notes)
}

fn validate(s: &GenerateSettings) -> Result<()> {
    ensure!(
        s.start.is_finite()
            && s.start >= 0.0
            && s.length.is_finite()
            && s.length > 0.0
            && (s.start + s.length).is_finite()
            && s.start + s.length <= MAX_BEAT
            && s.start + s.length > s.start,
        "Choose a finite, positive generation range"
    );
    ensure!(s.root < 12, "Choose a valid root note");
    ensure!((1..=8).contains(&s.octaves), "Choose 1–8 octaves");
    ensure!(
        (-1..=9).contains(&s.start_octave),
        "Choose a start octave from −1 to 9"
    );
    ensure!(
        (-24..=24).contains(&s.shift),
        "Choose a shift from −24 to 24 degrees"
    );
    ensure!(
        s.speed_multiplier.is_finite() && (0.125..=16.0).contains(&s.speed_multiplier),
        "Choose a speed from ⅛× to 16×"
    );
    ensure!(
        s.gate.is_finite() && (0.05..=1.0).contains(&s.gate),
        "Choose a gate from 5% to 100%"
    );
    ensure!(
        s.density.is_finite() && (0.0..=1.0).contains(&s.density),
        "Choose a density from 0% to 100%"
    );
    ensure!(
        s.additional_voices <= 3,
        "Choose at most 3 additional voices"
    );
    ensure!(
        (1..=12).contains(&s.step_size),
        "Choose a step size from 1 to 12"
    );
    ensure!(
        (2..=32).contains(&s.motif_steps),
        "Choose a phrase length from 2 to 32 steps"
    );
    ensure!(
        (1..=127).contains(&s.velocity),
        "Choose a velocity from 1 to 127"
    );
    Ok(())
}

fn pitch_pool(s: &GenerateSettings) -> Result<Vec<u8>> {
    let intervals: Vec<_> = (0..12)
        .filter(|interval| s.harmony.contains(s.root, (s.root + interval) % 12))
        .map(i16::from)
        .collect();
    let base = (i16::from(s.start_octave) + 1) * 12 + i16::from(s.root);
    let count = intervals.len() * usize::from(s.octaves);
    let shift = if s.kind == GenerateKind::Chord {
        i16::from(s.shift)
    } else {
        0
    };
    (0..count)
        .map(|index| {
            let degree = index as i16 + shift;
            let pitch = base
                + degree.div_euclid(intervals.len() as i16) * 12
                + intervals[degree.rem_euclid(intervals.len() as i16) as usize];
            ensure!(
                (0..=127).contains(&pitch),
                "Pitch range exceeds MIDI limits; adjust Start octave, Octaves or Shift"
            );
            Ok(pitch as u8)
        })
        .collect()
}

fn arpeggio_order(pattern: ArpeggioPattern, count: usize) -> Vec<usize> {
    match pattern {
        ArpeggioPattern::Up | ArpeggioPattern::Random => (0..count).collect(),
        ArpeggioPattern::Down => (0..count).rev().collect(),
        ArpeggioPattern::UpDown => (0..count)
            .chain((1..count.saturating_sub(1)).rev())
            .collect(),
        ArpeggioPattern::DownUp => (0..count).rev().chain(1..count.saturating_sub(1)).collect(),
        ArpeggioPattern::OutsideIn | ArpeggioPattern::InsideOut => {
            let mut order: Vec<_> = (0..count)
                .map(|i| if i % 2 == 0 { i / 2 } else { count - 1 - i / 2 })
                .collect();
            if pattern == ArpeggioPattern::InsideOut {
                order.reverse();
            }
            order
        }
    }
}

fn walk(current: usize, count: usize, step_size: u8, rng: &mut Random) -> usize {
    let step = 1 + rng.below(usize::from(step_size)) as i32;
    let next = current as i32 + if rng.below(2) == 0 { -step } else { step };
    // Reflection creates melodic return motion instead of long clamped runs.
    let period = (count.saturating_sub(1) * 2).max(1) as i32;
    let reflected = next.rem_euclid(period);
    reflected.min(period - reflected).min(count as i32 - 1) as usize
}

fn make_motif(s: &GenerateSettings, count: usize, rng: &mut Random) -> Vec<(usize, bool)> {
    let mut current = 0;
    (0..s.motif_steps)
        .map(|index| {
            if index > 0 {
                current = walk(current, count, s.step_size, rng);
            }
            (
                current,
                s.density > 0.0 && (index == 0 || rng.unit() < s.density),
            )
        })
        .collect()
}

fn add_voices(notes: &mut Vec<GeneratedNote>, s: &GenerateSettings, pool: &[u8]) {
    let lead_count = notes.len();
    let low = i16::from(pool[0]).saturating_sub(24).max(0) as u8;
    let high = (u16::from(*pool.last().unwrap()) + 24).min(127) as u8;
    let candidates: Vec<_> = (low..=high)
        .filter(|pitch| s.harmony.contains(s.root, *pitch))
        .collect();
    let minor = s.harmony.contains(s.root, (s.root + 3) % 12)
        && !s.harmony.contains(s.root, (s.root + 4) % 12);
    let triad = if minor { Chord::Minor } else { Chord::Major };
    let triad_pitches: Vec<_> = (0..=127)
        .filter(|pitch| triad.contains(s.root, *pitch))
        .collect();
    let mut previous = [None; 3];
    let mut previous_lead = None;
    for index in 0..lead_count {
        let lead = notes[index].clone();
        let mut used = vec![lead.pitch];
        for (voice, previous_voice) in previous
            .iter_mut()
            .enumerate()
            .take(usize::from(s.additional_voices))
        {
            let pitch = match s.accompaniment {
                Accompaniment::None => continue,
                Accompaniment::Tintinnabuli => {
                    // Alternating nearest lower/upper triad tones, then the
                    // second lower tone. This is a simple fixed-position T
                    // voice, not an emulation of an individual composition.
                    nearest_triad(&triad_pitches, lead.pitch, voice, &used)
                }
                Accompaniment::Counterpoint => counterpoint_pitch(
                    &candidates,
                    lead.pitch,
                    previous_lead,
                    *previous_voice,
                    voice,
                    &used,
                ),
            };
            if let Some(pitch) = pitch {
                used.push(pitch);
                *previous_voice = Some(pitch);
                notes.push(GeneratedNote {
                    pitch,
                    start: lead.start,
                    length: lead.length,
                    velocity: lead.velocity.saturating_sub(12 + voice as u8 * 3).max(1),
                    voice: voice as u8 + 1,
                });
            }
        }
        previous_lead = Some(lead.pitch);
    }
}

fn nearest_triad(pitches: &[u8], lead: u8, voice: usize, used: &[u8]) -> Option<u8> {
    pitches
        .iter()
        .copied()
        .filter(|pitch| !used.contains(pitch))
        .min_by_key(|pitch| {
            let wrong_side = if voice.is_multiple_of(2) {
                *pitch >= lead
            } else {
                *pitch <= lead
            };
            (
                u16::from(wrong_side) * 256 + u16::from(pitch.abs_diff(lead)),
                *pitch,
            )
        })
}

/// A useful contrary-motion/consonance heuristic, deliberately not advertised
/// as a complete species-counterpoint solver.
fn counterpoint_pitch(
    pitches: &[u8],
    lead: u8,
    previous_lead: Option<u8>,
    previous: Option<u8>,
    voice: usize,
    used: &[u8],
) -> Option<u8> {
    let lead = i16::from(lead);
    let below = voice.is_multiple_of(2);
    let target = lead
        + if below {
            -(7 + 12 * (voice / 2) as i16)
        } else {
            7
        };
    let motion = previous_lead.map_or(0, |previous| (lead - i16::from(previous)).signum());
    pitches
        .iter()
        .copied()
        .filter(|pitch| !used.contains(pitch))
        .min_by_key(|pitch| {
            let candidate = i16::from(*pitch);
            let interval = (candidate - lead).unsigned_abs() % 12;
            let dissonant = ![0, 3, 4, 7, 8, 9].contains(&interval);
            let wrong_side = if below {
                candidate >= lead
            } else {
                candidate <= lead
            };
            let motion_penalty = previous.map_or(0, |previous| {
                let direction = (candidate - i16::from(previous)).signum();
                match (motion, direction) {
                    (0, _) => 0,
                    (_, 0) => 12,
                    _ if direction == motion => 36,
                    _ => 0,
                }
            });
            let leap = previous.map_or(0, |previous| (candidate - i16::from(previous)).abs());
            (
                i16::from(dissonant) * 1000
                    + i16::from(wrong_side) * 500
                    + motion_penalty
                    + leap * 2
                    + (candidate - target).abs(),
                *pitch,
            )
        })
}

// SplitMix64: deterministic across architectures and independent of global RNG.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_bars_round_strictly_inward_and_reject_empty_views() {
        assert_eq!(complete_visible_bars(0.0, 16.0, 4), Some((0.0, 16.0)));
        assert_eq!(complete_visible_bars(0.5, 15.5, 4), Some((4.0, 8.0)));
        assert_eq!(complete_visible_bars(4.0, 12.0, 4), Some((4.0, 8.0)));
        assert_eq!(
            complete_visible_bars(0.001, 12.0 - 1e-10, 4),
            Some((4.0, 4.0))
        );
        assert_eq!(complete_visible_bars(-1.0, 6.0, 3), Some((0.0, 6.0)));
        for (start, end, meter) in [
            (0.2, 3.9, 4),
            (0.0, 0.0, 4),
            (0.0, 4.0, 0),
            (f64::NAN, 8.0, 4),
            (0.0, f64::INFINITY, 4),
        ] {
            assert_eq!(complete_visible_bars(start, end, meter), None);
        }
    }

    #[test]
    fn sustained_chords_span_the_exact_window_and_invert_by_degrees() {
        let mut s = GenerateSettings {
            start: 4.0,
            length: 12.0,
            ..Default::default()
        };
        let notes = generate_notes(&s).unwrap();
        assert_eq!(
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            [60, 64, 67]
        );
        assert!(notes.iter().all(|n| n.start == 4.0 && n.length == 12.0));
        s.shift = 1;
        assert_eq!(
            generate_notes(&s)
                .unwrap()
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            [64, 67, 72]
        );
        s.shift = -1;
        assert_eq!(
            generate_notes(&s)
                .unwrap()
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            [55, 60, 64]
        );
        s.octaves = 2;
        s.shift = 0;
        s.root = 11;
        assert_eq!(
            generate_notes(&s)
                .unwrap()
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            [71, 75, 78, 83, 87, 90]
        );
    }

    #[test]
    fn arp_patterns_are_distinct_and_speed_and_gate_keep_the_exact_bounds() {
        let mut s = GenerateSettings {
            kind: GenerateKind::Arpeggio,
            start: 3.25,
            length: 2.13,
            speed_multiplier: 2.0,
            gate: 1.0,
            ..Default::default()
        };
        let expected = [
            (ArpeggioPattern::Up, vec![60, 64, 67, 60, 64]),
            (ArpeggioPattern::Down, vec![67, 64, 60, 67, 64]),
            (ArpeggioPattern::UpDown, vec![60, 64, 67, 64, 60]),
            (ArpeggioPattern::DownUp, vec![67, 64, 60, 64, 67]),
            (ArpeggioPattern::OutsideIn, vec![60, 67, 64, 60, 67]),
            (ArpeggioPattern::InsideOut, vec![64, 67, 60, 64, 67]),
        ];
        for (arp, pitches) in expected {
            s.arp = arp;
            let notes = generate_notes(&s).unwrap();
            assert_eq!(notes.len(), 9);
            assert_eq!(
                notes.iter().take(5).map(|n| n.pitch).collect::<Vec<_>>(),
                pitches
            );
            assert!(
                (notes.last().unwrap().start + notes.last().unwrap().length - (s.start + s.length))
                    .abs()
                    < 1e-12
            );
        }
        s.arp = ArpeggioPattern::Random;
        let notes = generate_notes(&s).unwrap();
        assert!(notes.windows(2).all(|pair| pair[0].pitch != pair[1].pitch));
    }

    #[test]
    fn melodic_algorithms_are_deterministic_bounded_and_harmonic() {
        for melody in MelodyAlgorithm::ALL {
            for accompaniment in Accompaniment::ALL {
                let s = GenerateSettings {
                    kind: GenerateKind::Melody,
                    harmony: GenerateHarmony::Scale(Scale::Minor),
                    root: 2,
                    start: 4.0,
                    length: 15.7,
                    octaves: 2,
                    melody,
                    accompaniment,
                    additional_voices: 3,
                    seed: 572,
                    ..Default::default()
                };
                let notes = generate_notes(&s).unwrap();
                assert!(!notes.is_empty());
                assert_eq!(notes, generate_notes(&s).unwrap());
                assert!(notes.iter().all(|n| n.start >= s.start
                    && n.start + n.length <= s.start + s.length + 1e-12
                    && n.length > 0.0
                    && n.pitch <= 127
                    && n.velocity > 0
                    && n.voice <= 3));
                assert!(notes.iter().all(|n| s.harmony.contains(s.root, n.pitch)));
                let expected_voices = if accompaniment == Accompaniment::None {
                    1
                } else {
                    4
                };
                assert_eq!(notes.len() % expected_voices, 0);
                if accompaniment != Accompaniment::None {
                    assert!(notes.iter().any(|n| n.voice == 3));
                }
            }
        }
    }

    #[test]
    fn seed_changes_melody_and_density_controls_rests() {
        for melody in MelodyAlgorithm::ALL {
            let mut s = GenerateSettings {
                kind: GenerateKind::Melody,
                harmony: GenerateHarmony::Scale(Scale::Major),
                length: 16.0,
                melody,
                density: 1.0,
                ..Default::default()
            };
            let original = generate_notes(&s).unwrap();
            assert_eq!(original.len(), 32);
            s.seed += 1;
            assert_ne!(original, generate_notes(&s).unwrap());
            s.density = 0.0;
            assert!(generate_notes(&s).unwrap().is_empty());
        }
    }

    #[test]
    fn hidden_melody_parameters_do_not_change_random_arpeggios() {
        let mut s = GenerateSettings {
            kind: GenerateKind::Arpeggio,
            arp: ArpeggioPattern::Random,
            ..Default::default()
        };
        let original = generate_notes(&s).unwrap();
        s.motif_steps = 32;
        s.step_size = 12;
        s.density = 0.0;
        assert_eq!(original, generate_notes(&s).unwrap());
    }

    #[test]
    fn counterpoint_prefers_consonant_contrary_motion_when_available() {
        let pitches: Vec<_> = (48..=84)
            .filter(|pitch| Scale::Major.contains(0, *pitch))
            .collect();
        // Lead rises C5→D5; the companion has a nearby consonant descending E4→D4.
        let pitch = counterpoint_pitch(&pitches, 74, Some(72), Some(64), 0, &[74]).unwrap();
        assert!(pitch < 64);
        assert!([0, 3, 4, 7, 8, 9].contains(&(74_u8.abs_diff(pitch) % 12)));
    }

    #[test]
    fn euclidean_rhythm_distributes_a_chosen_number_of_pulses_per_phrase() {
        let s = GenerateSettings {
            kind: GenerateKind::Melody,
            melody: MelodyAlgorithm::Euclidean,
            length: 4.0,
            density: 3.0 / 8.0,
            motif_steps: 8,
            ..Default::default()
        };
        assert_eq!(
            generate_notes(&s)
                .unwrap()
                .iter()
                .map(|n| n.start)
                .collect::<Vec<_>>(),
            [0.0, 1.5, 3.0]
        );
    }

    #[test]
    fn tintinnabuli_voices_use_nearest_tonic_triad_tones_without_duplicates() {
        let s = GenerateSettings {
            kind: GenerateKind::Melody,
            harmony: GenerateHarmony::Scale(Scale::Dorian),
            root: 2,
            accompaniment: Accompaniment::Tintinnabuli,
            additional_voices: 3,
            density: 1.0,
            ..Default::default()
        };
        let notes = generate_notes(&s).unwrap();
        for voices in notes.chunks(4) {
            assert_eq!(voices.len(), 4);
            let lead = voices[0].pitch;
            assert!(
                voices[1..]
                    .iter()
                    .all(|n| Chord::Minor.contains(s.root, n.pitch))
            );
            assert!(voices[1].pitch < lead);
            assert!(voices[2].pitch > lead);
            assert!(voices[3].pitch < voices[1].pitch);
            assert_eq!(
                voices
                    .iter()
                    .map(|n| n.pitch)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                4
            );
        }
    }

    #[test]
    fn invalid_ranges_and_extreme_workloads_fail_without_partial_drafts() {
        for (start, length) in [
            (f64::NAN, 4.0),
            (0.0, f64::INFINITY),
            (-1.0, 4.0),
            (0.0, 0.0),
            (f64::MAX, f64::MAX),
        ] {
            assert!(
                generate_notes(&GenerateSettings {
                    start,
                    length,
                    ..Default::default()
                })
                .is_err()
            );
        }
        assert!(
            generate_notes(&GenerateSettings {
                kind: GenerateKind::Arpeggio,
                length: 1e100,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            generate_notes(&GenerateSettings {
                kind: GenerateKind::Melody,
                length: 2048.0,
                accompaniment: Accompaniment::Counterpoint,
                additional_voices: 3,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            generate_notes(&GenerateSettings {
                speed_multiplier: f64::NAN,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            generate_notes(&GenerateSettings {
                start: MAX_BEAT - 1.0,
                length: 2.0,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            generate_notes(&GenerateSettings {
                start_octave: -1,
                shift: -1,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            generate_notes(&GenerateSettings {
                start_octave: 9,
                octaves: 2,
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(
            generate_notes(&GenerateSettings {
                start_octave: 9,
                ..Default::default()
            })
            .unwrap()
            .last()
            .unwrap()
            .pitch,
            127
        );
    }

    #[test]
    fn valid_harmonies_and_registers_never_escape_midi_limits() {
        for harmony in Chord::ALL
            .into_iter()
            .map(GenerateHarmony::Chord)
            .chain(Scale::ALL.into_iter().map(GenerateHarmony::Scale))
        {
            for root in 0..12 {
                for start_octave in [0, 4, 7] {
                    let s = GenerateSettings {
                        harmony,
                        root,
                        start_octave,
                        ..Default::default()
                    };
                    let notes = generate_notes(&s).unwrap();
                    assert!(notes.iter().all(|n| harmony.contains(root, n.pitch)));
                    assert!(notes.windows(2).all(|pair| pair[0].pitch < pair[1].pitch));
                }
            }
        }
    }
}
