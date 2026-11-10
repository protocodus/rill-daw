<picture>
  <source media="(prefers-color-scheme: dark)" srcset="crates/daw-ui/assets/brand/rill-logo-dark.svg">
  <img alt="Rill" src="crates/daw-ui/assets/brand/rill-logo-light.svg" width="320">
</picture>

**From the first loop to the final mix.**

Rill is a free, open-source desktop music workstation for **macOS, Linux, and Windows**. Record audio and MIDI, develop a phrase with the integrated AI assistant, and bring your song together with your favorite VST3, CLAP and (on Linux) LV2 instruments and effects. Focused workspaces keep arranging, editing, and mixing close to the music.

[Download](#download) · [Features](#make-it-your-song) · [AI assistant](#make-music-with-ai) · [Screenshots](#screenshots) · [User guide](docs/USER_GUIDE.md) · [Website](https://protocodus.cz/product/rill/)

![Rill Arrangement with audio and MIDI parts, colored tracks, and a song timeline](artifacts/readme-arrangement.png)

## Make it your song

- **Arrange freely.** Import WAV, MP3, FLAC, OGG or MIDI. Trim, split, stretch and layer clips; organize tracks with colors and nested folders. Large audio streams from disk, and edits stay reversible.
- **Capture a performance.** Record audio and MIDI with a pre-count and metronome. Keep audio loop passes as takes, comp your favorite passages with crossfades, or record separate overdub layers. MIDI loop passes build one overdubbed phrase.
- **Develop the notes.** Write in the piano roll, preview chords, arpeggios and melodies, and refine timing with quantize and humanize. Edit velocity and supported note expression while retaining richer performance data in the project.
- **Work with AI in context.** Ask for a new part, a phrase variation, an audio edit or a mix adjustment. Keep a conversation scoped to the open clip, channel or processor, with changes available in normal Undo/Redo history.
- **Shape the mix.** Host VST3, CLAP and LV2 (Linux) instruments and effects in isolated processes. Connect buses, pre/post-fader sends, sidechains and grouped instrument outputs. Draw automation for plug-in parameters, sends, and channel/Master gain, volume and pan; inspect spectrum, loudness and stereo levels.
- **Freeze and return.** Print clips or tracks to new audio, including configured instrument outputs together. Originals remain retained and muted; **Return to originals** brings them back while keeping your prints and edits.
- **Take the song with you.** Save portable `.rill` projects with referenced audio, retained takes and Freeze originals embedded. Export songs or loops as 16/24-bit PCM or 32-bit float WAV, FLAC, Ogg Vorbis or MP3, write aligned stems of tracks, buses and folders in one step, or make a MIDI file.

Independent instrument and effect chains can run across multiple CPU cores. Shared-memory audio reduces plug-in communication overhead without reducing sample precision, and lightweight graphs retain a serial path. Performance depends on the song, plug-ins and hardware; see the [architecture and measured results](docs/ARCHITECTURE.md#scope-and-verification).

## Download

Get a development build for your platform. These links serve the latest successful `main` build through [nightly.link](https://nightly.link/protocodus/rill-daw/workflows/ci/main), without a GitHub login.

| Platform | Download | Launch |
| --- | --- | --- |
| macOS · Apple silicon | [Download for arm64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-aarch64-apple-darwin.zip) | Move `Rill.app` to Applications. |
| macOS · Intel | [Download for x86-64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-apple-darwin.zip) | Move `Rill.app` to Applications. |
| Linux · x86-64 | [Download for x86-64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-unknown-linux-gnu.zip) | Run `./rill`. |
| Linux · arm64 | [Download for arm64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-aarch64-unknown-linux-gnu.zip) | Run `./rill`. |
| Windows 10/11 · x86-64 | [Download for Windows](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-pc-windows-msvc.zip) | Run `rill.exe`. |

**Installing:** extract the download ZIP, then the platform archive inside it. Keep the application and `daw-plugin-host` helper together; keep the macOS app bundle intact. Each package includes a SHA-256 checksum and installation notes. Builds are unsigned and macOS builds are not notarized. macOS requires version 11 or newer; Linux requires glibc 2.35 or newer. See [platform requirements and installation](docs/RELEASING.md).

**Bring your own instruments:** VST3, CLAP and LV2 instruments and effects are installed separately. Rill includes an original audio drum loop to try immediately.

Build artifacts expire after 14 days. If a link is unavailable, visit the [successful main builds](https://github.com/protocodus/rill-daw/actions/workflows/ci.yml?query=branch%3Amain+is%3Asuccess) and open a run's **Artifacts** section; GitHub sign-in is required for that route. A newer build can be in progress while the links serve an earlier successful version.

## Make music with AI

Describe a musical change and hear it in your current song. The assistant can inspect the arrangement, create MIDI parts, transform notes, adjust plug-in parameters and the mix, and process or analyze selected audio. Its edits use the same **Undo/Redo** history as your own.

Open **AI for this context** from an editor to limit the conversation to that clip, channel or processor. Its scope stays pinned when you change selection, and Rill checks edits against that boundary. Open **AI** in the menu bar for work across the song.

Try prompts such as:

- “Create a four-bar D-minor arpeggio using one of my installed instruments.”
- “Humanize this phrase without changing its pitches.”
- “Lower the drum bus by 3 dB and pan the shaker 20% right.”
- “Normalize the selected audio clip to −1 dBFS.”

![Rill's AI Assistant with a draft request pinned to the open Glass chords MIDI clip](artifacts/readme-ai-assistant.png)

*A prepared request pinned to the open MIDI clip. Connect your AI provider to send it; no request has been sent in this capture.*

Choose **OpenAI** or **Anthropic** in **Window → Setup → AI**, enter your model and your own API key, then click **Apply**. Requests go to your chosen provider; its charges and usage limits apply. The rest of Rill works without AI setup.

Prefer an external AI client? Rill also exposes the live song through **MCP**. See [AI assistant setup and MCP tools](docs/AI_TOOLS.md).

## Screenshots

From the arrangement above to notes, audio and mixing below: these are captures of the running desktop app.

### Write the parts

Edit melodies and chords in the piano roll, with note lengths, velocities, scale highlighting, and phrase previews close to the notes.

![Rill piano roll showing MIDI notes and velocity editing in the demo project](artifacts/readme-piano-roll.png)

### Shape the audio

Work directly on the waveform with non-destructive trimming, fades, gain, source slip, and processing tools. Retained takes let you revisit a performance and refine the comp.

![Rill audio Clip Editor showing the waveform and audio editing tools](artifacts/readme-audio-editor.png)

### Bring the mix together

Balance channels and Master, place sounds in stereo, and reach instruments, effects, routing and automation from the mixer.

![Rill Mixer showing the demo project's audio and instrument channels alongside its buses and master](artifacts/readme-mixer.png)

### Give the song movement

Draw volume changes, bend individual curve segments, and enter precise point values. Temporarily override automation without losing the curve, then resume it when ready.

![Rill Automation editor showing the Atmosphere track's volume curve, editable points and segment controls](artifacts/readme-automation.png)

## Start making music

1. Launch Rill and click **Play** to hear the included drum loop, or choose **File → New** for an empty song.
2. Drop WAV, MP3, FLAC or OGG files onto an audio track or empty arranger space, or choose **File → Import audio…** to create a track at the cursor. For MIDI, choose **Track → Add MIDI track** and assign a VST3, CLAP or LV2 instrument.
3. Double-click a clip to edit its audio or notes. Open **Window → Mixer** to balance the tracks.
4. Save a portable `.rill` project, then use **File → Export WAV…** to share a mix.

Use the menus and on-screen tools, or speed up familiar actions with keyboard shortcuts. See the [user guide](docs/USER_GUIDE.md) for recording, loop takes, generation, effects, routing, automation, and recovery.

## Current scope

Audio runs through the platform driver (ALSA, WASAPI or CoreAudio) or, from Setup → Audio → Driver, JACK on Linux. ASIO on Windows is an opt-in build (`--features asio`, see [releasing](docs/RELEASING.md)); no release archive includes it, and it has not been tested on hardware.

Rill is in active development and focuses on audio/MIDI music production. It hosts **VST3** and **CLAP** on every platform and **LV2** on Linux (through the system lilv library); instruments and effects are installed separately. LV2 plug-ins use Rill's generic parameter browser instead of their own editor windows. Songs can change meter at bar lines and carry named markers; audio clips can be warped with markers and quantized to the grid. Tempo maps, surround mixing, Audio Units/VST2/AAX/ARA, and video/scoring-to-picture workflows are not supported. See the [current capabilities and limits](docs/PRODUCT_DIRECTION.md).

`.rill` projects preserve high-resolution MIDI note/controller data. Native UMP input targets macOS 11 and newer; Linux and Windows use MIDI 1.0 input. Audible expression depends on the instrument's supported mappings (VST3 note expressions, CLAP note expressions, MIDI for LV2); per-note pitch-bend packets are retained but not currently played. Exporting richer performances as `.mid` requires an explicit MIDI 1.0 copy. This is not universal MIDI 2.0 device or plug-in compatibility; see [MIDI recording and expression](docs/USER_GUIDE.md#midi-recording-and-pre-count).

## Explore further

- [User guide](docs/USER_GUIDE.md) — the full composing, recording, and mixing reference.
- [Audio editing](docs/AUDIO_EDITOR.md) — clip processing, comping, stretching, and workflow details.
- [Editing tools audit](docs/EDITING_TOOLS_AUDIT.md) — implemented MIDI/audio tools, verification and remaining gaps.
- [AI assistant and MCP](docs/AI_TOOLS.md) — optional OpenAI/Anthropic assistance and tools for the current song.
- [Project format](docs/PROJECT_FORMAT.md) — portable projects, stored assets, and limits.
- [DAW comparison and priorities](docs/PRODUCT_DIRECTION.md) — current capabilities, market gaps, evidence and limits.
- [Architecture](docs/ARCHITECTURE.md) · [UI components](docs/UI.md) · [End-to-end testing](docs/E2E.md) — for contributors.

## Build from source

Requires Rust 1.92 or newer and the platform's native build tools. Build the whole workspace so the isolated plug-in helper is available beside the app:

```sh
cargo build --locked --release --workspace
./target/release/rill
```

On Windows, launch `target\release\rill.exe`. Linux also needs audio and desktop development libraries. See [build prerequisites and packaging](docs/RELEASING.md#build-locally) for all platforms, or use `./scripts/run.sh` for a local development launch.

Rill is written in Rust and released under the [MIT license](LICENSE). Bug reports, workflow feedback, and contributions are welcome through [Issues](https://github.com/protocodus/rill-daw/issues) and [pull requests](https://github.com/protocodus/rill-daw/pulls).
