# Rill project files

`.rill` is a portable ZIP archive. Version 1 contains `manifest.json` and the WAV
files referenced by audio clips. MIDI notes, edits, routing, automation, and opaque
instrument/effect state live in the manifest. Legacy JSON projects remain readable
and writable when their filename has another extension, such as `.daw.json`.

The manifest is UTF-8 JSON with this envelope:

```json
{"version":1,"project":{"name":"Song","tracks":[]}}
```

The example omits the remaining required project fields. The `project` schema is
the same versioned model used by legacy JSON. Opaque VST component/controller state
is encoded as JSON byte arrays in `instrument_state` or an effect slot's `state`.
Normalized parameter overrides are restored after opaque state. Effect slot IDs
remain stable across saves, allowing routing/editor and automation references to
identify the same instance.

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

Version 1 enforces these limits:

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
