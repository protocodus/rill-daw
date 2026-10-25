# Rill project files

`.rill` is a portable ZIP archive. Version 4 contains `manifest.json` and the WAV
files referenced by audio clips. MIDI notes, edits, routing, automation, and opaque
instrument/effect state live in the manifest. Legacy JSON projects remain readable
and writable when their filename has another extension, such as `.daw.json`.

The manifest is UTF-8 JSON with this envelope:

```json
{"version":4,"project":{"name":"Song","tracks":[]}}
```

The example omits the remaining required project fields. The `project` schema is
the same versioned model used by legacy JSON. Opaque VST component/controller state
is encoded as JSON byte arrays in `instrument_state` or an effect slot's `state`.
Normalized parameter overrides are restored after opaque state. Effect slot IDs
remain stable across saves, allowing routing/editor and automation references to
identify the same instance.

Version 2 adds the per-instance `source_rate` field (source beats per arrangement beat, default `1.0`, range `0.25`–`4.0`). The source window is `[offset, offset + length * source_rate)`. Stretch changes the visible duration and rate while preserving that window; source notes/audio and ghost membership stay unchanged. Version-1 files load with unity rate.

Version 3 adds audio `processing`: `algorithm` (`Signalsmith` or `Bungee`) and `transpose_semitones` (−24 to +24, fractional values supported). Missing processing defaults to Signalsmith and zero transposition. Pitch and algorithm are shared source edits for ghosts; stretch rate remains local.

Version 4 adds optional audio clip `audio_take` membership. A retained candidate uses `{"group":42,"role":"Source"}`; an audible comp region uses `{"group":42,"role":{"Comp":{"source":17}}}`, where `17` is the retained source clip's ID. Clips without this field are ordinary independent or ghost clips. Group IDs share the document's object-ID namespace, are reserved when loading, and cannot span tracks. Each comp source reference must identify a retained candidate in the same group and audio track. A group supports up to 64 candidates.

Retained candidates remain in `track.clips` with their original source windows and edits. They are silent: playback, overlap scheduling, and export duration exclude them. Their media remains bundled even when no comp region currently uses it. Comp regions are ordinary editable audio clips with local bounds, source offsets, rates, fades, and optional crossfades. Selecting a passage preserves other regions and adds smooth complementary crossfades limited to available source handles; choosing a whole take replaces the group's audible comp. Independent copies and overdubs clear take membership, while splits preserve it. Detaching a group removes retained candidates from the document and leaves the current comp as ordinary clips without deleting any source files.

Versions 1–3 remain readable. New saves use version 4 so older applications reject unsupported take metadata instead of playing retained candidates as ordinary overlapping clips. These version rules apply to ZIP manifests and legacy JSON alike.

Audio entries are named `assets/00000000.wav`, `assets/00000001.wav`, and so on.
Clips reference these relative names in the manifest. Sources are deduplicated by
canonical source path; two different files with the same basename remain separate.
Referenced files are included in full, preserving non-destructive offsets, trims,
and duplicate clips. Assets use ZIP Stored compression to avoid compressing large
audio files during save; the manifest uses Deflate.

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
them. This first version creates a fresh directory on every load, so scratch usage
grows; old extraction directories can be removed after all projects using them are
closed. Failed loads remove their incomplete directory and leave unrelated scratch
files alone.

The format enforces these limits:

| Item | Limit |
| --- | --- |
| Archive entries, including manifest | 4,096 |
| ZIP metadata read during parsing/header validation | 4 MiB |
| Expanded manifest | 256 MiB |
| Each expanded WAV | 2 GiB |
| Total expanded data and archive file | 8 GiB each |
| Opaque state per instrument/effect | 64 MiB |

The manifest limit includes the JSON byte-array representation of plug-in states,
so several large states can exceed the aggregate limit before reaching the asset
budget. Save reports that failure without replacing the previous project.

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

Explicit Save and Open run on one background document pipeline. Saving first flushes native plugin state, then writes an immutable project snapshot atomically. Continued edits remain unsaved; completing an older write never marks later names, colors, notes, or plugin edits as saved. New/Open/Save/Quit requests are serialized, and a failed operation cancels queued document replacement/shutdown. Quit waits for pending writes. Existing outgoing JSON recovery remains in place.

Opening extracts the bundle and prepares waveform data on the worker before flushing the outgoing native editor and replacing the document. Duplicate audio paths decode once. Waveform preparation is limited to 256 MiB per source and 512 MiB total; unavailable/invalid/oversized previews leave their clips linked and display a warning. General WAV decoding rejects declarations exceeding 512 MiB of decoded samples before allocating sample buffers. Streaming sources larger than this is not implemented.
