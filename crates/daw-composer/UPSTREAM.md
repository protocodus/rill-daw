# Composition rules adapted from midi-composer-mcp

`daw-composer` is a native Rust adaptation of selected deterministic music
operations from [midi-composer-mcp](https://github.com/voho/midi-composer-mcp),
by [voho](https://github.com/voho) and its contributors.

- Upstream package version: **0.2.0**.
- Pinned commit: **`72a25722f12fed96e8e19c637f0b15260fcb51a7`**.
- License declaration: **MIT**, in the pinned
  [pyproject.toml](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/pyproject.toml).

The pinned repository has no standalone license file or explicit copyright
notice. [LICENSE-MIT.txt](LICENSE-MIT.txt) supplies the standard MIT permission
text for redistribution, with attribution and a separate notice for Rill's
Rust adaptation. It is not presented as a verbatim upstream license file.

## Adapted material

The shared catalogue and spelling implementation live in Rill's
`daw-core/src/music_theory/`. The piano roll owns the canonical `Chord` and
`Scale` choices over this data; `daw-composer` re-exports and consumes those
definitions. UI membership is a cached pitch-class projection of the same
compound intervals used for theoretical voicings. There is no independent
composition-module catalogue.

The scale and chord vocabulary, interval/degree data, aliases, and descriptions
come from upstream
[scales.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/scales.py)
and
[chords.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/chords.py).
Octave-aware spelling follows
[notes.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/notes.py).

Diatonic degree realization and close voicings adapt selected rules from
[diatonic.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/diatonic.py),
[harmony.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/harmony.py),
and the pure event-building helpers in
[midi_io.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/midi_io.py).
The Euclidean rhythm implementation follows the Bjorklund grouping in
[generate.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/generate.py).
Accompaniment uses selected patterns described in
[accompany.py](https://github.com/voho/midi-composer-mcp/blob/72a25722f12fed96e8e19c637f0b15260fcb51a7/src/midi_composer_mcp/accompany.py).

This is a selected subset, rather than a port of all 63 upstream MCP tools.
It provides 50 scale types, 37 chord types, ordinary diatonic degree
progressions with optional sevenths and smooth close voicings, Euclidean
rhythms, and bounded chord/arpeggio/bass drafts. It does not include the full
literal Roman-numeral grammar, advanced historical composition rules, or
upstream file/audio renderers. The music21-derived key profiles in upstream
`analysis.py` identify separate BSD attribution; those profiles and algorithms
are not copied here.

## Rill-specific behavior

The module produces musical data without opening files, launching Python/MCP,
or accessing projects, plug-ins, providers, or audio devices. Rill's existing
session service applies drafts through its validated, undoable edit boundary.
Musical positions and lengths are zero-based quarter-note beats; `C4` is MIDI
60. Requested draft lengths include rests.

Rill supplies explicit register, velocity, and gate controls. Bass drafts keep
the requested register rather than upstream's walking-register folding and
nearest-register decisions. Up/down/up-down arpeggios cycle the chosen
voicing; they do not add the extra octave-top step of upstream's named
`chord_pattern` presets. Alberti retains the low/high/middle/high figure.
General MIDI program numbers do not select VST3 instruments: instrument
assignment remains an explicit Rill operation.

## Golden reference data

`tests/fixtures/` contains small, pinned outputs generated from the upstream
Python functions, using Python 3.12 and `mido==1.3.3` only for importing the
event helpers:

- `lookups.json`: every canonical scale from `scale_info(type, "F4")`, every
  canonical chord from `chord_info(type, "Bb3")`, and spelling/alias cases.
- `progressions.json`: `degrees_to_chords` with a concrete root; smooth cases
  then pass the chord symbols through `voice_leading` at that root's octave.
- `rhythms.json`: exact `euclidean_rhythm` patterns, including rotations and
  empty/full rhythms.
- `accompaniment.json`: matching Alberti and bass cases from `chord_pattern`
  or `bass_line`, then `build_track_events`. All velocities are fixed to 90
  and reference event lengths are explicitly multiplied by Rill's 0.9 gate.
  Bass cases keep one repeated root so the two register policies coincide.

Each fixture records the source commit. The executable comparison tests check
musical pitches, spelling, timing, lengths, and velocity, allowing only normal
floating-point tolerance for beat lengths. These references establish the
documented subset; they do not claim full upstream tool compatibility.
