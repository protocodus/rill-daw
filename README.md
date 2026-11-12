<picture>
  <source media="(prefers-color-scheme: dark)" srcset="crates/daw-ui/assets/brand/rill-logo-dark.svg">
  <img alt="Rill" src="crates/daw-ui/assets/brand/rill-logo-light.svg" width="320">
</picture>

**From the first loop to the final mix.**

Rill is a free, open-source desktop music workstation for **macOS, Linux, and Windows**. Record audio and MIDI, develop a phrase with the integrated AI assistant, and bring your song together with your favorite VST3, CLAP, (on Linux) LV2 and (on macOS) Audio Unit instruments and effects. Focused workspaces keep arranging, editing, and mixing close to the music.

[Download](#download) · [Features](#make-it-your-song) · [AI assistant](#make-music-with-ai) · [Screenshots](#screenshots) · [All screens](#every-screen-and-dialog) · [User guide](docs/USER_GUIDE.md) · [Website](https://protocodus.cz/product/rill/)

![Rill Arrangement with audio and MIDI parts, colored tracks, and a song timeline](artifacts/readme-arrangement.png)

## Make it your song

- **Arrange freely.** Import WAV, MP3, FLAC, OGG or MIDI. Trim, split, stretch and layer clips; organize tracks with colors and nested folders. Large audio streams from disk, and edits stay reversible.
- **Capture a performance.** Record audio and MIDI with a pre-count and metronome. Keep audio loop passes as takes, comp your favorite passages with crossfades, or record separate overdub layers. MIDI loop passes build one overdubbed phrase.
- **Develop the notes.** Write in the piano roll, preview chords, arpeggios and melodies, and refine timing with quantize and humanize. Edit velocity and supported note expression while retaining richer performance data in the project.
- **Work with AI in context.** Ask for a new part, a phrase variation, an audio edit or a mix adjustment. Keep a conversation scoped to the open clip, channel or processor, with changes available in normal Undo/Redo history.
- **Shape the mix.** Host VST3, CLAP, LV2 (Linux) and Audio Unit (macOS) instruments and effects in isolated processes. Connect buses, pre/post-fader sends, sidechains and grouped instrument outputs. Draw automation for plug-in parameters, sends, and channel/Master gain, volume and pan; inspect spectrum, loudness and stereo levels.
- **Freeze and return.** Print clips or tracks to new audio, including configured instrument outputs together. Originals remain retained and muted; **Return to originals** brings them back while keeping your prints and edits.
- **Take the song with you.** Save portable `.rill` projects with referenced audio, retained takes and Freeze originals embedded. Export songs or loops as 16/24-bit PCM or 32-bit float WAV, FLAC, Ogg Vorbis or MP3, write aligned stems of tracks, buses and folders in one step, or make a MIDI file.

Independent instrument and effect chains can run across multiple CPU cores. Shared-memory audio reduces plug-in communication overhead without reducing sample precision, and lightweight graphs retain a serial path. Performance depends on the song, plug-ins and hardware; see the [architecture and measured results](docs/ARCHITECTURE.md#scope-and-verification).

## Download

Get a development build for your platform. These links serve the latest successful nightly build of `main` through [nightly.link](https://nightly.link/protocodus/rill-daw/workflows/ci/main), without a GitHub login.

| Platform | Download | Launch |
| --- | --- | --- |
| macOS · Apple silicon | [Download for arm64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-aarch64-apple-darwin.zip) | Move `Rill.app` to Applications. |
| macOS · Intel | [Download for x86-64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-apple-darwin.zip) | Move `Rill.app` to Applications. |
| Linux · x86-64 | [Download for x86-64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-unknown-linux-gnu.zip) | Run `./rill`. |
| Linux · arm64 | [Download for arm64](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-aarch64-unknown-linux-gnu.zip) | Run `./rill`. |
| Windows 10/11 · x86-64 | [Download for Windows](https://nightly.link/protocodus/rill-daw/workflows/ci/main/rill-x86_64-pc-windows-msvc.zip) | Run `rill.exe`. |

**Installing:** extract the download ZIP, then the platform archive inside it. Keep the application and `daw-plugin-host` helper together; keep the macOS app bundle intact. Each package includes a SHA-256 checksum and installation notes. Builds are unsigned and macOS builds are not notarized. macOS requires version 11 or newer; Linux requires glibc 2.35 or newer. See [platform requirements and installation](docs/RELEASING.md).

**Sound out of the box:** Rill includes its own [R1 synth and filter](#built-in-devices) and an original audio drum loop to try immediately. Add VST3, CLAP, LV2 and AU instruments and effects whenever you like.

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

## Every screen and dialog

Rendered from the current `main` by CI on every build ([`scripts/capture-screens.sh`](scripts/capture-screens.sh)), light theme first and dark beside it. Click an image for full size.

<table>
<tr><th width="50%">Light</th><th width="50%">Dark</th></tr>
<tr><td colspan="2"><b>Arranger</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/arrangement.png"><img alt="Rill Arranger, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/arrangement.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/arrangement.png"><img alt="Rill Arranger, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/arrangement.png"></a></td></tr>
<tr><td colspan="2"><b>Mixer</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/mixer.png"><img alt="Rill Mixer, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/mixer.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/mixer.png"><img alt="Rill Mixer, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/mixer.png"></a></td></tr>
<tr><td colspan="2"><b>Piano roll</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/piano.png"><img alt="Rill Piano roll, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/piano.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/piano.png"><img alt="Rill Piano roll, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/piano.png"></a></td></tr>
<tr><td colspan="2"><b>Audio clip editor</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/audio.png"><img alt="Rill Audio clip editor, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/audio.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/audio.png"><img alt="Rill Audio clip editor, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/audio.png"></a></td></tr>
<tr><td colspan="2"><b>Automation editor</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/automation.png"><img alt="Rill Automation editor, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/automation.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/automation.png"><img alt="Rill Automation editor, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/automation.png"></a></td></tr>
<tr><td colspan="2"><b>Note expression</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/expression.png"><img alt="Rill Note expression, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/expression.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/expression.png"><img alt="Rill Note expression, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/expression.png"></a></td></tr>
<tr><td colspan="2"><b>Device browser</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/library.png"><img alt="Rill Device browser, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/library.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/library.png"><img alt="Rill Device browser, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/library.png"></a></td></tr>
<tr><td colspan="2"><b>AI assistant</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/assistant.png"><img alt="Rill AI assistant, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/assistant.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/assistant.png"><img alt="Rill AI assistant, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/assistant.png"></a></td></tr>
<tr><td colspan="2"><b>Signal analysis</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/analysis.png"><img alt="Rill Signal analysis, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/analysis.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/analysis.png"><img alt="Rill Signal analysis, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/analysis.png"></a></td></tr>
<tr><td colspan="2"><b>Instrument panel (R1 Mini)</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/instrument.png"><img alt="Rill Instrument panel (R1 Mini), light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/instrument.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/instrument.png"><img alt="Rill Instrument panel (R1 Mini), dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/instrument.png"></a></td></tr>
<tr><td colspan="2"><b>Effects</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/effects.png"><img alt="Rill Effects, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/effects.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/effects.png"><img alt="Rill Effects, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/effects.png"></a></td></tr>
<tr><td colspan="2"><b>Effect panel (R1 Filter)</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/effect-editor.png"><img alt="Rill Effect panel (R1 Filter), light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/effect-editor.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/effect-editor.png"><img alt="Rill Effect panel (R1 Filter), dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/effect-editor.png"></a></td></tr>
<tr><td colspan="2"><b>Routing</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/routing.png"><img alt="Rill Routing, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/routing.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/routing.png"><img alt="Rill Routing, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/routing.png"></a></td></tr>
<tr><td colspan="2"><b>Clip properties</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/properties.png"><img alt="Rill Clip properties, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/properties.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/properties.png"><img alt="Rill Clip properties, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/properties.png"></a></td></tr>
<tr><td colspan="2"><b>Channel properties</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/channel.png"><img alt="Rill Channel properties, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/channel.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/channel.png"><img alt="Rill Channel properties, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/channel.png"></a></td></tr>
<tr><td colspan="2"><b>Recording</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/recording.png"><img alt="Rill Recording, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/recording.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/recording.png"><img alt="Rill Recording, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/recording.png"></a></td></tr>
<tr><td colspan="2"><b>Export audio</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/export.png"><img alt="Rill Export audio, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/export.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/export.png"><img alt="Rill Export audio, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/export.png"></a></td></tr>
<tr><td colspan="2"><b>History</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/history.png"><img alt="Rill History, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/history.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/history.png"><img alt="Rill History, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/history.png"></a></td></tr>
<tr><td colspan="2"><b>Setup · Audio</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup.png"><img alt="Rill Setup · Audio, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup.png"><img alt="Rill Setup · Audio, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup.png"></a></td></tr>
<tr><td colspan="2"><b>Setup · Appearance</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-appearance.png"><img alt="Rill Setup · Appearance, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-appearance.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-appearance.png"><img alt="Rill Setup · Appearance, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-appearance.png"></a></td></tr>
<tr><td colspan="2"><b>Setup · Storage</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-storage.png"><img alt="Rill Setup · Storage, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-storage.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-storage.png"><img alt="Rill Setup · Storage, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-storage.png"></a></td></tr>
<tr><td colspan="2"><b>Setup · Plug-ins</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-plugins.png"><img alt="Rill Setup · Plug-ins, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-plugins.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-plugins.png"><img alt="Rill Setup · Plug-ins, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-plugins.png"></a></td></tr>
<tr><td colspan="2"><b>Setup · AI</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-ai.png"><img alt="Rill Setup · AI, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/setup-ai.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-ai.png"><img alt="Rill Setup · AI, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/setup-ai.png"></a></td></tr>
<tr><td colspan="2"><b>Save changes?</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/unsaved-changes.png"><img alt="Rill Save changes?, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/unsaved-changes.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/unsaved-changes.png"><img alt="Rill Save changes?, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/unsaved-changes.png"></a></td></tr>
<tr><td colspan="2"><b>About</b></td></tr>
<tr><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/about.png"><img alt="Rill About, light theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/light/about.png"></a></td><td width="50%"><a href="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/about.png"><img alt="Rill About, dark theme" src="https://raw.githubusercontent.com/protocodus/rill-daw/screens/dark/about.png"></a></td></tr>
</table>

## Built-in devices

Rill ships its own instruments and effects, so a new song makes sound before any plug-in is installed. Find them in **Window → Device browser** under **Built-in (R1)**. Each has factory presets, automation on every control, and tools the AI assistant can use in plain units.

| Device | What it does |
| --- | --- |
| ![R1 Mini panel](artifacts/readme-r1-mini.png) **R1 Mini** · instrument | A two-oscillator subtractive synth and the default instrument of new MIDI tracks. Circuit-modelled VCO and DCO oscillators, a sub and noise source, a choice of five analog filter circuits, two envelopes, an LFO, and poly, mono and legato voicing. Play it from the panel keyboard with pitch and mod wheels. |
| ![R1 Filter panel](artifacts/readme-r1-filter.png) **R1 Filter** · effect | A filter for inserts and sends. Pick the transparent Clean model or one of eight characters (transistor ladder, Sallen-Key, two OTA designs, diode pair, state variable, comb and vowel), then move it with the built-in LFO and envelope follower. |
| ![R1 Equalizer panel](artifacts/readme-r1-equalizer.png) **R1 Equalizer** · effect | A six-band equalizer: high-pass, four bands (bell, low shelf, high shelf or notch) and low-pass, with slopes up to 48 dB per octave. Drag the bands in the plot over a live spectrum of the track, or set them in the table below. Digital mode is exact and neutral; Analog adds proportional-Q bells, passive-style shelves and a trace of colour. Auto gain keeps the level steady while you compare. |

The full design, including the devices still to come, is in [R1 built-in devices](docs/R1_DEVICES.md).

### Preset compatibility tests

Every version of Rill must load presets saved by any earlier version: each parameter a preset names keeps its value, and each parameter added since takes its default. `crates/daw-devices/tests/preset_compatibility.rs` enforces this against frozen presets in `crates/daw-devices/tests/presets/<device>/<version>/`. Every built-in instrument and effect with presets follows the same rules:

- **Adding a device:** copy its factory presets into `tests/presets/<device-id>/v1/`, plus an `every-parameter.rillpreset` that names every parameter key. The test fails for any device with presets but no `v1` folder.
- **Changing a device's parameters:** never edit or delete a frozen preset. If old presets still load, nothing else is needed. When the change alters what presets mean, freeze the presets from just before it in a new `v2`, `v3`… folder.
- **Renaming a choice option:** presets store choices by label, so add `(key, old label, current label)` to the descriptor's `legacy_labels`. Never rename or reuse a parameter key, and never change a parameter's unit or range so that an old value means something else.

Run them with `cargo test -p daw-devices --test preset_compatibility`.

## Start making music

1. Launch Rill and click **Play** to hear the included drum loop, or choose **File → New** for an empty song.
2. Drop WAV, MP3, FLAC or OGG files onto an audio track or empty arranger space, or choose **File → Import audio…** to create a track at the cursor. For MIDI, choose **Track → Add MIDI track**: it plays R1 Mini at once, or pick a VST3, CLAP, LV2 or AU instrument instead.
3. Double-click a clip to edit its audio or notes. Open **Window → Mixer** to balance the tracks.
4. Save a portable `.rill` project, then use **File → Export WAV…** to share a mix.

Use the menus and on-screen tools, or speed up familiar actions with keyboard shortcuts. See the [user guide](docs/USER_GUIDE.md) for recording, loop takes, generation, effects, routing, automation, and recovery.

## Current scope

Audio runs through the platform driver (ALSA, WASAPI or CoreAudio) or, from Setup → Audio → Driver, JACK on Linux. Windows builds also offer ASIO, built with Steinberg's ASIO SDK under its GPLv3 option, so the Windows executable is distributed under the GPLv3 while Rill's source stays MIT (see [releasing](docs/RELEASING.md)). ASIO has not been tested on hardware.

Rill is in active development and focuses on audio/MIDI music production. It hosts **VST3** and **CLAP** on every platform, **LV2** on Linux (through the system lilv library) and **Audio Units** (AUv2, and AUv3 through Apple's bridge) on macOS, alongside its own [built-in devices](#built-in-devices). LV2 plug-ins use Rill's generic parameter browser instead of their own editor windows. Songs can change meter at bar lines and carry named markers; audio clips can be warped with markers and quantized to the grid. Tempo maps, surround mixing, VST2/AAX/ARA, and video/scoring-to-picture workflows are not supported. See the [current capabilities and limits](docs/PRODUCT_DIRECTION.md).

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
