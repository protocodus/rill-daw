# Rill project files

`.rill` is a portable ZIP archive. Version 6 contains `manifest.json` and the WAV
files referenced by audio clips. MIDI performance data, edits, routing, automation, and opaque
instrument/effect state live in the manifest. Public `Project::save` always writes
this portable format, even when an explicit caller chooses another extension.
Load detects ZIP contents rather than relying on the filename; legacy JSON
projects remain readable. `save_legacy_json` is an explicit external-reference
format used for internal recovery and legacy interchange.

The manifest is UTF-8 JSON with this envelope:

```json
{"version":6,"project":{"name":"Song","tracks":[]}}
```

The example omits the remaining required project fields. The `project` schema is
the same versioned model used by legacy JSON. Opaque VST component/controller state
is encoded as JSON byte arrays in `instrument_state` or an effect slot's `state`.
Normalized parameter overrides are restored after opaque state. Effect slot IDs
remain stable across saves, allowing routing/editor and automation references to
identify the same instance.

Clip `name` remains a required string but may be empty. Empty or whitespace-only
names suppress the arrangement title band without changing timing, audio/MIDI
data or clip identity. Rename edits normalize all-blank text to an empty string;
linked copies retain their shared name. No format-version change is required.

Automation points store an outgoing `curve` value from −1 (Log) through 0
(Linear) to +1 (Exp). The `Spline` interpolation mode uses that value for a
bounded cubic Bézier segment. New lanes default to Linear with zero bend;
changing a bend stores a per-point `interpolation: "Spline"` override and does
not change the lane default or neighboring segments. Missing `curve` fields read as zero; existing
Step, Linear and Smooth modes preserve their behavior. A lane's optional
normalized `override_value` holds a manual value without changing its points;
omission or null resumes the curve. Both shapes and override values are
validated before applying edits. The first and last point values extend beyond
their times; automation points do not determine the song's duration.

Version 2 adds the per-instance `source_rate` field (source beats per arrangement beat, default `1.0`, range `0.25`–`4.0`). The source window is `[offset, offset + length * source_rate)`. Stretch changes the visible duration and rate while preserving that window; source notes/audio and ghost membership stay unchanged. Version-1 files load with unity rate.

Audio clips may carry an optional `warp_markers` array, for example `[{"source":0.0,"beat":0.0},{"source":2.1,"beat":2.0},{"source":6.0,"beat":6.0}]`. `source` is a source position in beats at the song tempo (the unit of `offset`); `beat` is the forward clip-relative beat where it plays. Markers are strictly increasing in both values, every segment between neighbours plays at 0.25×–4× speed, and at most 2,048 are stored. Outside the markers the clip's `source_rate` applies, and `offset` equals the source position at clip beat 0. Reversed clips play the same map mirrored. An absent or empty array means ordinary whole-clip stretch; the field is omitted when empty, so older files load unchanged and the format version is not bumped. Warp markers are local to each ghost occurrence, like `source_rate`.

Version 3 adds audio `processing`: `algorithm` (`Signalsmith` or `Bungee`) and `transpose_semitones` (−24 to +24, fractional values supported). Missing processing defaults to Signalsmith and zero transposition. Pitch and algorithm are shared source edits for ghosts; stretch rate remains local.

Version 4 adds optional audio clip `audio_take` membership. A retained candidate uses `{"group":42,"role":"Source"}`; an audible comp region uses `{"group":42,"role":{"Comp":{"source":17}}}`, where `17` is the retained source clip's ID. Clips without this field are ordinary independent or ghost clips. Group IDs share the document's object-ID namespace, are reserved when loading, and cannot span tracks. Each comp source reference must identify a retained candidate in the same group and audio track. A group supports up to 64 candidates.

Retained candidates remain in `track.clips` with their original source windows and edits. They are silent: playback, overlap scheduling, and export duration exclude them. Their media remains bundled even when no comp region currently uses it. Comp regions are ordinary editable audio clips with local bounds, source offsets, rates, fades, and optional crossfades. Selecting a passage preserves other regions and adds smooth complementary crossfades limited to available source handles; choosing a whole take replaces the group's audible comp. Independent copies and overdubs clear take membership, while splits preserve it. Detaching a group removes retained candidates from the document and leaves the current comp as ordinary clips without deleting any source files.

The optional project fields `meter_changes` and `markers` add song timing
context without a version change; both are omitted when empty, so older songs
save unchanged. `meter_changes` is a list of `{"bar":5,"beats_per_bar":3}` sorted
by bar: each change starts at a bar line after bar 1 (whose meter remains
`beats_per_bar`) and holds 1–32 beats per bar, up to 256 changes. Clip and note
positions stay in beats. `markers` holds up to 1024 `{"id":7,"beat":16.0,
"name":"Chorus","color":[220,120,40]}` entries; IDs share the object-ID
namespace, names are 1–256 characters and not blank, and `color` is optional.

Versions 1–6 remain readable. New saves use version 6 so older applications
reject unsupported routing, Master, Freeze or MIDI performance semantics instead
of silently dropping them. These version rules apply to ZIP manifests and legacy
JSON alike; missing optional fields retain their legacy defaults.

## Master, plug-in buses and Freeze

Version 5 adds `Project.master`, a channel outside `project.tracks` with the
reserved ID `0`. Ordinary object IDs remain positive. Its default is a unity
stereo sum with centered pan and no inserts, preserving older projects' sound.
Master supports `input_gain`, ordered insert slots, `gain` (volume), `pan` and
mute. It cannot contain clips, an instrument, sends, parent folders, input
assignments or outgoing routing, and cannot be soloed. `project.track(0)` and
the channel iterators include it; arrangement track ordering excludes it.

All audio channels gain a finite `input_gain` field in `0..=16`, defaulting to
`1.0`. This is pre-insert trim, separate from the existing post-insert `gain`
volume and `pan`. `AutomationParameter::TrackInputGain` automates it; ordinary
automation targets use `track: 0` for Master, including its insert parameters.

`Track.instrument_inputs` and `EffectSlot.inputs` contain `PluginInputRoute`
records: `source_track`, destination `bus` and `first_channel`, linear `gain`
and `pre_fader`. Bus/channel indices are zero-based native plug-in addresses;
`first_channel` starts an even-indexed stereo pair, with a final odd channel
handled as mono. A source tap is after its inserts and either before or after
volume/pan. Distinct sources can sum into one input pair. An insert's actual
primary input pair remains its channel signal; extra routes cannot replace it.
Master cannot be a source, but its inserts can receive auxiliary input.

An instrument output channel is a `Bus` track with
`instrument_output: {"track": owner_id, "bus": 1, "first_channel": 0}`.
It receives that raw native output from the owner's single instrument instance,
before the owner's trim/inserts/fader, and has its own processing and routing.
Output channels are not summing-bus destinations. The main native output is
resolved from plug-in metadata rather than assumed to be bus zero. Input,
output and owner dependencies participate in feedback/cycle validation even
when muted or at zero gain. Selected pairs are checked against the restored
plug-in's actual bus layout during preparation. Model bounds are 64 buses,
128 channels per bus, 128 input routes per processor and 128 output selections
per creation batch; native allocation limits are enforced separately.

Clips gain occurrence-local `muted` (default false) and optional `freeze`
provenance. `FreezeLink` stores a group ID, original owner/clip IDs, channel
mute states changed by Freeze, any parallel sidechain route copies, and an
`active` flag. Printed pieces share the group's provenance; splitting an
original updates active return references. These fields do not propagate a
mute to other ghost occurrences. References to deleted originals remain valid
provenance, but Return refuses to mute a print if its required original is
missing. Load reserves referenced IDs, preventing accidental reuse.

Freeze files are managed 32-bit float WAV assets bundled like other sources.
Undo/Redo and Return retain media. Return restores the recorded original mutes,
mutes the printed clips, and removes only unchanged sidechain copies made by
Freeze. User-edited routes and printed audio edits remain. See the
[Freeze workflow](USER_GUIDE.md#freeze-and-return-to-originals).

## MIDI 2.0 performance data

Version 6 adds optional `Note.midi2`, `Clip.midi_events` and the incoming
`Track.midi_input_group` selector (zero-based `0..15`, default `0`). The input
group selector does not rewrite or filter already stored performance data.

`Midi2NoteData` retains group/channel, wire note index, 16-bit attack/release
velocities, attack/release attribute types and data, and relative-source-beat
per-note expression packets. `protocol: 1` preserves an original MIDI 1.0
attack's exact `u7 / 127` playback normalization; `protocol: 2` uses `u16 / 65535`.
Release, pitch or expression editing does not silently change that attack flag.
MIDI 2.0 velocity zero remains zero even though the legacy piano-roll preview
uses a playable minimum. Pitch attribute type 3 retains absolute Pitch 7.9
precision (1/512 semitone); the wire note index remains its expression address.

`UmpPacket` stores four exact `u32` words plus `word_count` (1–4), validates the
message type's defined packet length, and requires unused storage words to be
zero. `MidiClipEvent` adds a stable ID and source beat; per-note events instead
belong to their note and use relative source beats. The model retains hidden
events when notes/clips are trimmed and validates their address and IDs.
Per-clip performance data is bounded to 262,144 events and per-note expressions
to 32,768 points; the manifest size limit remains an aggregate bound.
Automatic containment cleanup does not merge expressive notes into a matching
legacy pitch: their distinct channel, attributes, IDs and expression remain.

This is project persistence, not a claim to implement raw UMP stream files,
the standard MIDI Clip File format, MIDI-CI/profile negotiation or every SysEx
family in playback. Standard `.mid` delivery remains MIDI 1.0 note export;
the app requires an explicit **Export MIDI 1.0 copy…** decision when richer
data would be reduced or omitted. Save `.rill` to retain the original data.

Audio tracks also save an optional `audio_input` channel assignment: for example
`{"Mono":{"channel":0}}` or `{"Stereo":{"left":0,"right":1}}`. Indices are
zero-based channels of the input device chosen in application Setup. Stereo
channels must be distinct. Missing assignments load as `null`; new audio tracks
default to mono input 1. Channel availability is checked against the actual device
when its stream opens, so moving a project to a smaller interface does not make
its existing recorded audio unreadable. Input-device identity remains a local
application preference. Record arm, monitoring, and an in-progress recording are
session state and are never restored automatically from a project.

Finalized recordings use ordinary source WAVs. A non-loop recording creates an
independent clip; loop recording creates a take group, retaining complete and
partial passes as separate sources. The newest complete pass is initially audible.
If none completed, the longest partial pass is selected (latest on a tie), so a
short final pass does not silence the rest of a longer recording. Existing clips
and edited comps remain intact; a later recording creates a new recoverable group.
Overdub placement creates a separate, distinctly colored audio channel with the
same output bus, folder, input assignment, and gain/pan settings. Inserts and sends
are not duplicated onto the new layer. A recording batch publishes
atomically; Undo removes its project references without deleting captured files.
This optional input metadata does not require another format bump: older
version-4 readers can ignore channel assignments without changing recorded audio.

Audio entries are named `assets/00000000.wav`, `assets/00000001.wav`, and so on.
Clips reference these relative names in the manifest. Sources are deduplicated by
canonical source path and file identity, including hard links; two different
files with the same basename remain separate.
Referenced files are included in full, preserving non-destructive offsets, trims,
and duplicate clips. Normal Save uses ZIP Stored compression for assets to avoid
compressing large audio files; the manifest uses Deflate.
Every referenced audio clip is included, including muted clips, retained takes,
frozen originals and printed audio. Save does not trim away hidden source audio.
There are no external audio paths in a successful portable manifest.

**Save and optimize** (`Project::save_optimized`) rewrites the archive with the
same referenced-asset rules, combines separate source files when their complete
bytes are identical, and uses lossless ZIP Deflate for audio entries. Both save
modes omit assets referenced only by deleted clips. Optimization preserves the
complete WAV bytes and clip edits; it does not trim source ranges, delete muted
material, or discard takes or Freeze originals. Comparison and copying use
bounded buffers rather than retaining entire audio files. The current project's
source paths and Undo/Redo remain unchanged, and no source or scratch file is
deleted. Optimization targets the saved archive, not global scratch storage.

WAV/MP3/FLAC/OGG imports are normalized into managed 32-bit float WAV assets
before they enter the project. Source sample rate and mono/stereo channels are
preserved without re-encoding to MP3/Vorbis; normalized samples use 32-bit float
precision. Bundles contain this decoded audio, so opening a project does not
require the original compressed files or invoke their decoders. Original tags
and cover art are not stored in the WAV.

Instrument and insert references (`"Vst3": {"path", "class_id"}`) hold any hosted
format; the format follows the path's extension (`.vst3`, `.clap`, `.lv2`, otherwise
VST3). `class_id` is a 32-hex VST3 class ID, a CLAP plug-in ID or an LV2 plug-in URI.
CLAP state is the plug-in's own stream; LV2 state is lilv's Turtle text. No
project-version change was needed.

Rill's built-in devices are a separate variant, `"Builtin": {"device": "r1-filter"}`,
for both instruments and inserts. `device` is 1–32 characters of `a-z`, `0-9`
and `-`; their state is only the normalized `parameters` overrides keyed by the
device's stable parameter IDs, so `state` must be empty. A device ID this
version does not know stays in the project unchanged: an unknown insert passes
audio through and an unknown instrument is silent. Older Rill versions reject
the variant.

Plug-in binaries and external sample libraries managed by a plug-in are not
included. Plug-in paths are made absolute when saving a bundle and retained when
loading. A different machine still needs compatible plug-ins and their external
data installed. Opaque state can contain plug-in-owned paths which Rill cannot
rewrite or discover reliably.

`Project::load_with_assets(path, scratch_root)` validates the archive, then extracts
assets to a private `scratch_root/rill-project-<random>/assets/` directory. Returned
clip paths are absolute. The archive's directory is never used for extraction.
`Project::load` uses the system temporary directory's `rill-project-assets` folder
as a fallback for CLI/core callers; the application supplies its configured scratch
directory. Successful extraction directories persist while projects may reference
them. Each load creates a fresh directory, so scratch usage grows. There is no
automatic media garbage collection: active projects, Undo/Redo, recovery snapshots
and pending jobs may retain media after it disappears from the visible song.
Failed loads remove only their incomplete private directory and leave unrelated
scratch files alone.

The format enforces these limits:

| Item | Limit |
| --- | --- |
| Archive entries, including manifest | 4,096 |
| ZIP metadata read during parsing/header validation | 4 MiB |
| Expanded manifest | 256 MiB |
| Each expanded WAV | Just below 4 GiB (`u32::MAX - 128` bytes), compatible with the current RIFF writer |
| Total expanded data and archive file | 64 GiB each |
| Opaque state per instrument/effect | 64 MiB |

The manifest limit includes the JSON byte-array representation of plug-in states,
so several large states can exceed the aggregate limit before reaching the asset
budget. Save reports that failure without replacing the previous project.
Media copying uses a fixed 64 KiB buffer; manifest parsing is streamed rather
than retaining another complete JSON byte buffer. ZIP64 entries/offsets support
projects larger than 4 GiB. Open-file metadata and identity are checked before
and after copying, and referenced metadata is checked again before publication.
Detected source changes or truncation fail the save. Cooperative cancellation
APIs stop between bounded chunks without replacing the existing destination.

Only the manifest and referenced, numbered asset files are accepted. Absolute paths,
traversal, backslashes, unexpected entries, duplicate names, symlinks, special files,
encryption, unsupported compression, inconsistent ZIP headers, overlapping entries,
and missing/unreferenced media are rejected. Declared sizes are checked before
extraction; actual reads are capped at each declared size plus one byte and verified
against both size and CRC. Metadata parsing itself has a bounded read budget.

Saving builds a temporary archive beside the destination and flushes it before an
atomic replacement. Missing/changing media, serialization, or write failures leave
an existing destination intact and remove the temporary file.

## Application I/O

Explicit Save, Save and optimize, and Open run on one background document pipeline. Saving first flushes native plugin state, then writes an immutable project snapshot atomically. Continued edits remain unsaved; completing an older write never marks later names, colors, notes, or plugin edits as saved. New/Open/Save/Quit requests are serialized, and a failed operation cancels queued document replacement/shutdown. Quit waits for pending writes. Both JSON and bundle saves flush the replacement directory entry on Unix after flushing file contents.

Every committed UI or tool edit queues a JSON recovery checkpoint independently of native editor state and the document pipeline. Recovery writes run on a background worker, coalesce pending revisions of the same song, and preserve an outgoing song's pending checkpoint across a load completing. Native state refreshes still run every 30 seconds, after first checkpointing the last acknowledged state. `recovery.daw.json` holds the latest complete snapshot; `recovery.previous.daw.json` retains its valid predecessor. **Recover last session…** falls back to the predecessor if the latest is missing or corrupt. Quit waits for recovery and remains open if it fails. Failure does not retry every frame; new commits and periodic refreshes retry. These snapshots reference existing media paths rather than copying audio. There is a short in-flight window before an asynchronous checkpoint reaches disk; unacknowledged native state, interrupted writes, storage failure, and deleted source media cannot be guaranteed recoverable.

Opening extracts the bundle and admits source metadata on background workers before flushing the outgoing native editor and replacing the document. Duplicate paths share descriptors. Only files below 32 MiB decoded PCM are retained in the application preview map, within 128 MiB total; larger valid files remain usable through disk-backed descriptors and bounded waveform caches. Missing/invalid sources keep their clips linked and report a warning. Canonical imported float WAVs are bounded to `u32::MAX - 4096` decoded bytes per source and an 8 GiB import batch; archive byte limits above are separate. No full large-source PCM allocation is needed to open a portable project.

## MIDI input and recording timing

Since project format 4, optional `Track.midi_input` stores `"All"`,
`{"Channel":{"channel":0}}` (zero-based channel 0–15), or `null`. New MIDI
tracks default to All; older manifests without the field load with no assignment.
Only MIDI tracks accept this field. The global MIDI device identifier is stored
in application settings, not in portable projects; arm/monitor choices remain
session-only.

Project fields `metronome_enabled`, `metronome_gain`, and `precount_bars` default
to false, 0.25, and 0 when absent. Gain is finite in 0–1; pre-count accepts 0, 1,
or 2 bars. MIDI recordings use ordinary independent MIDI clips, with legacy
MIDI 1.0 sustain represented by note duration and loop passes merged into that
clip. MIDI 2.0 retains key-release timing and the sustain controller. Version 6
also saves rich performance data in the fields described above. There is no
extra source file for MIDI. The selected input device and platform transport
are not a promise that a saved packet maps to every VST3 destination. Mixed
audio/MIDI recording publishes one validated project edit and archives its audio
sources through the existing path.
