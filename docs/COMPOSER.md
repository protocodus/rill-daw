# Native composition tools

Rill's embedded assistant and MCP clients share a native composition module,
`daw-composer`, adapted from
[midi-composer-mcp 0.2.0](https://github.com/voho/midi-composer-mcp/tree/72a25722f12fed96e8e19c637f0b15260fcb51a7).
The tools produce deterministic musical data and commit parts through Rill's
normal song editing service. End users need no Python runtime or extra MCP server.

## One catalogue with the piano roll

`daw-core::music_theory` owns the shared chord and scale catalogue. The piano
roll, its Generate dialog, the existing `generate_part` tool, and the native
composition module all use these definitions and the same name/alias resolver.
All 50 scales and 37 chords are available in the editor selectors; longer menus
scroll within the available workspace.

The catalogue keeps full compound intervals for spelled chord voicings. The
piano roll derives sorted, unique pitch classes from those intervals for
highlighting, scale filtering, multi-draw, and its octave-based generator.
For example, C4 Add9 is voiced as C4–E4–G4–D5 in a theory lookup, while an Add9
stack remains C4–D4–E4–G4 inside the clicked octave. Both come from the same
definition. Existing editor choices and saved scale identifiers remain valid.

## Available tools

| Tool | Result |
| --- | --- |
| `composer_catalog` | All 50 scales and 37 chord types, with names, aliases, degree spellings, descriptions, and module limits. |
| `composer_scale` | Spelled scale notes, including the octave; concrete roots also return MIDI pitches. |
| `composer_chord` | Spelled chord tones and, for concrete roots, MIDI pitches. |
| `composer_progression` | Diatonic triads or sevenths for integer scale degrees, optionally voiced with smooth, close inversions. |
| `composer_euclidean` | A rhythm with evenly distributed pulses: `O` is the first accent, `o` another onset, and `.` a rest. Positive rotation shifts left. |
| `compose_progression` | Chord, arpeggio, and bass clips created together in one atomic, undoable edit. |

The five `composer_*` queries do not change the song or its history.
`compose_progression` requires fresh `session_id` and `expected_revision` values
from `get_song_context`; stale requests are rejected before committing.

## Try it with the assistant

Configure OpenAI or Anthropic in **Window → Setup → AI**, then open **AI** or
**Window → Assistant**. Example prompts:

- “Show me the notes of F major and a C dominant ninth chord.”
- “Create pads, an Alberti arpeggio, and a root/fifth bass over D minor degrees 1, 6, 3, 7, four beats per chord. Use my existing instrument tracks.”
- “Find a Euclidean rhythm with five hits over thirteen steps.”

The native queries can also be chained with existing `create_midi_part`,
`create_instrument_part`, `transform_phrase`, and mixing tools. The new module
and Rill's existing seeded chord/arpeggio/melody generator reuse the piano-roll
catalogue.

## Notes and timing

- Pitch spelling is octave-aware: `C4` is MIDI 60. `F4` major includes `Bb4`,
  and a C4 dominant ninth extends to D5. Abstract roots such as `F` return
  pitch-class names without MIDI numbers.
- Progressions require a concrete root such as `D4` and a seven-note scale.
  `degrees` contains integer positions 1–7. These are diatonic positions;
  this interface does not interpret Roman-numeral case, borrowed chords,
  applied dominants, or figured bass. For example, degree 5 in natural minor
  is a minor chord; choose harmonic minor for its diatonic major dominant.
- All durations and positions are zero-based quarter-note beats. Rill retains
  the project's tempo and time signature. Set `beats_per_chord` explicitly
  when the harmonic rhythm differs from the default four quarter notes.
- Notes in a new clip start at source beat zero. `start` places that clip in
  the arrangement. A part retains the whole progression's duration, including
  the gap after its last gated note.

## Commit several parts together

Read `get_song_context` and `list_tracks` first. This illustrative tool call
creates three new MIDI tracks; replace the session and revision with the
current values:

```json
{
  "session_id": 1,
  "expected_revision": 0,
  "name": "D minor accompaniment",
  "start": 0,
  "root": "D4",
  "scale": "natural minor",
  "degrees": [1, 6, 3, 7],
  "smooth": true,
  "beats_per_chord": 4,
  "parts": [
    {"part": {"kind": "chords", "name": "Pads", "velocity": 78}},
    {"part": {"kind": "arpeggio", "name": "Arpeggio", "style": "alberti", "step": 0.5}},
    {"part": {"kind": "bass", "name": "Bass", "style": "root_fifth", "octave": 2}}
  ]
}
```

Supply `track_id` beside `part` to append a clip to an existing MIDI track:
`{"track_id": 7, "part": {"kind": "bass", "style": "root_fifth"}}`.
Its notes, instrument, mix, and routing remain intact. Omitting `track_id`
creates a new MIDI track. New tracks need an installed VST3 instrument before
they can be heard; assign one through Rill or `assign_plugin`.

Chord parts support `gate`, `velocity`, and `octave_shift`. Arpeggios additionally
support `step` and `up`, `down`, `up_down`, or `alberti` styles. Bass parts use
`root`, `root_fifth`, or `root_octave`, with `step`, `gate`, `velocity`, and a
base `octave`. Read the published schemas for numeric bounds and defaults.
For arpeggios and alternating bass styles, `step` must evenly divide
`beats_per_chord`. Root bass holds one note per chord and ignores that grid.

The result returns the created track/clip IDs, part lengths, note counts, and
instrument-assignment status. It does not resend every generated note.
One successful call creates one history entry; invalid inputs or targets leave
all parts and history unchanged.

## Scope and limits

A call supports up to 32 chords, eight parts, 8,192 notes in total, and 2,048
quarter-note beats. Rhythms support up to 256 steps. Inputs and generated pitches
are validated before committing; notes outside MIDI 0–127 are rejected.

This is a native adaptation of selected upstream capabilities. The upstream
project's full Roman grammar, detailed species counterpoint, SATB voicing,
advanced key/voice-leading analysis, and file/audio rendering tools are not part
of this module. Composition tools cannot access files, change Setup, or replace
the current document. Rill's own import/export, audio engine, and VST3 hosting
remain responsible for playback and files.

See [upstream attribution and parity scope](../crates/daw-composer/UPSTREAM.md).
Native golden tests compare representative musical results with the pinned
upstream source. Running-app E2Es cover assistant/MCP access, exact notes,
existing instrument tracks, complete clip ranges, atomic failures, and GUI
Undo/Redo.
