# R1 built-in devices: design draft

Status: **draft for review, nothing is implemented.** This document proposes the
architecture, the shared UI additions and the per-device specifications for
Rill's first built-in instruments and effects. Implementation follows in
separate changes once the open decisions in [Decisions needed](#decisions-needed)
are settled.

This reverses the current product position ("Built-in instruments and effects
remain outside its scope", [PRODUCT_DIRECTION.md](PRODUCT_DIRECTION.md)). The
reason is the one that document already names as the cost of the plug-in-only
scope: a new user has no sound until they install and scan third-party
plug-ins. A small, good built-in set lets a MIDI part be audible immediately,
for a person and for the AI tools alike.

## Contents

1. [Goals and limits](#goals-and-limits)
2. [Architecture](#architecture)
3. [Shared UI library additions](#shared-ui-library-additions)
4. [Shared DSP toolkit](#shared-dsp-toolkit)
5. [Effects](#effects): Spacer, Modulator, Saturator, Compressor, Equalizer, Filter
6. [Instruments](#instruments): Mini, Maxi, Sampler, Drums
7. [Verification](#verification)
8. [Implementation order](#implementation-order)
9. [Decisions needed](#decisions-needed)

## Goals and limits

- **Few controls, high sound quality.** Each device covers its job with a
  panel that fits one window without scrolling (Maxi uses three pages).
  Quality comes from the DSP (alias-free oscillators and nonlinearities,
  uncramped filters, click-free parameter changes), not from parameter count.
- **Code separation.** DSP lives in a crate that knows nothing about the
  project model, the engine, egui or the app. The DAW reaches devices only
  through one small trait and a registry.
- **One UI library.** Device panels are built only from `daw-ui` components.
  The same panels run inside the app and, if the devices are later wrapped as
  plug-ins, inside the plug-in editor.
- **A built-in device looks like a plug-in to the rest of Rill.** It has
  stable `u32` parameter IDs with normalized 0–1 values, so the existing
  automation lanes, `set_plugin_parameters`, `plugin_describe_parameters`,
  Undo, the live parameter queue and the parameter browser work without a
  second implementation.
- **Not in this design:** multi-zone sampling, user wavetable import, drawable
  LFO shapes, linear-phase EQ, per-pad outputs for Drums, preset sharing, and
  shipping the devices as VST3/CLAP plug-ins. The architecture leaves room for
  each; none is specified here.

## Architecture

### Crates

Two new crates; `daw-ui` is extended, not split.

| Crate | Contents | Depends on |
| --- | --- | --- |
| `daw-devices` (new) | `Device` trait, parameter descriptors, device registry, DSP toolkit, the ten devices, factory presets, pure response functions used for drawing (EQ curve, compressor transfer curve, envelope shape). | `std`, `rustfft` (already pinned in the workspace, used to build band-limited wavetables and for the UI-side spectrum helper). No `daw-*` crate, no egui, no file or network I/O, no `unsafe`. |
| `daw-ui` (extended) | New generic device controls and visualizations ([below](#shared-ui-library-additions)). They take plain data (slices, numbers, closures) and do not know `daw-devices`. | unchanged |
| `daw-devices-ui` (new) | One panel function per device, composed only from `daw-ui` components, plus the `DeviceHost` trait that binds a panel to whatever owns the parameter values. | `daw-devices`, `daw-ui` |

```mermaid
flowchart LR
    Devices[daw-devices<br/>DSP, parameters, registry]
    UI[daw-ui<br/>controls, visualizations]
    Panels[daw-devices-ui<br/>device panels, DeviceHost]
    Core[daw-core<br/>device id strings only]
    Engine[daw-engine]
    App[daw-app]
    Wrapper[future plug-in wrapper]
    Panels --> Devices
    Panels --> UI
    Engine --> Devices
    Engine --> Core
    App --> Panels
    App --> Engine
    Wrapper -.-> Panels
```

`daw-core` gains no dependency. It stores a device as a short identifier
string and validates only its form, the way it already repeats the Audio Unit
locator constant instead of depending on `daw-plugins`.

Why not put panels in `daw-devices` behind a feature: the DSP crate then
compiles with or without egui depending on who asks, and the "no UI in DSP"
rule becomes a convention instead of a dependency fact. Why not split `daw-ui`
into a "plug-in subset": `daw-ui` already has no audio, app or window-runtime
dependency, so a plug-in editor can link it as it is.

### The device contract

```rust
// daw-devices
pub struct DeviceDescriptor {
    pub id: &'static str,            // "r1-mini"; persisted, never renamed
    pub name: &'static str,          // "R1 Mini"
    pub kind: DeviceKind,            // Instrument | Effect
    pub parameters: &'static [ParamSpec],
    pub sample_slots: u32,           // 0, 1 (Sampler) or 16 (Drums)
    pub sidechain_input: bool,       // Compressor only
}

pub struct ParamSpec {
    pub id: u32,                     // stable; never reused or renumbered
    pub name: &'static str,          // "Filter cutoff"
    pub group: &'static str,         // "Filter"
    pub unit: &'static str,          // "Hz", "dB", "ms", "%", ""
    pub range: ParamRange,           // Linear{min,max} | Log{min,max} | Choice(&[&str]) | Toggle
    pub default: f32,                // plain value
    pub modulatable: bool,           // Maxi mod-matrix destination
}

pub trait Device: Send {
    fn descriptor(&self) -> &'static DeviceDescriptor;
    /// The only place that may allocate.
    fn prepare(&mut self, sample_rate: f32, max_block: usize);
    fn set_parameter(&mut self, id: u32, normalized: f32);
    fn set_sample(&mut self, slot: u32, sample: Option<Arc<SampleData>>);
    fn process(&mut self, context: &ProcessContext, events: &[Event], io: StereoIo<'_>);
    fn reset(&mut self);
    fn latency_samples(&self) -> u32;
    fn tail_samples(&self) -> u32;
    fn telemetry(&self) -> Arc<Telemetry>;
}

pub fn registry() -> &'static [&'static DeviceDescriptor];
pub fn create(id: &str) -> Option<Box<dyn Device>>;
```

- `ParamSpec` owns the plain↔normalized mapping and the display text
  (`format(normalized) -> "1.2 kHz"`, `parse("1.2k") -> normalized`). The panel,
  the parameter browser and the tools all use it, so a value reads the same
  everywhere.
- `process` never allocates, locks, or does I/O. Parameter changes are smoothed
  inside the device (5 ms ramps for gains and mixes, per-sample coefficient
  interpolation for filter frequencies) so block-rate automation, which is what
  the engine delivers to plug-ins today (256-frame blocks), does not zipper.
- `Event` carries a sample offset and one of: note on/off (with note ID, pitch
  as a float so Pitch 7.9 tuning survives, velocity as a float), per-note
  pressure, channel pitch bend, mod wheel, sustain. The engine translates from
  the events it already prepares for hosted instruments; kinds a device does
  not use are ignored.
- `ProcessContext` carries tempo and beat position for synced delays and LFOs.
- `SampleData` is decoded stereo `f32` at its source rate. The engine supplies
  it from its existing decoded-source cache; devices never read files.
- `Telemetry` is the lock-free path from the audio instance to its panel:
  atomics for scalars (gain reduction, input/output peak, effective cutoff,
  LFO phase, up to 32 voice playheads) and a fixed 4,096-frame ring of the
  device's output (and input, for effects) for scope and spectrum. A torn read
  costs one odd frame in a drawing and is accepted. The FFT for the spectrum
  runs on the UI side.

### State and presets

Every device in this design is fully described by its parameter values plus,
for Sampler and Drums, its sample references. No device needs an opaque state
blob, so:

- The existing `instrument_parameters` / `EffectSlot.parameters` lists
  (normalized overrides by ID) are the saved state. `instrument_state` /
  `EffectSlot.state` stay empty for built-ins.
- A preset is a named list of `(id, plain value)` pairs in JSON. Factory
  presets are compiled into `daw-devices`; user presets are files in Rill's
  data directory. Loading a preset is one ordinary `set_plugin_parameters`
  batch: one Undo step, automation-safe, available to tools.
- Maxi's modulation matrix is 24 fixed slots of three parameters each (source,
  destination, amount), so it is automatable and tool-editable like everything
  else.

A snapshot test in `daw-devices` lists every `(device, id, name, range)` and
fails when an ID is changed or reused, because saved projects and automation
lanes refer to those IDs.

### Project model (`daw-core`)

```rust
pub enum Instrument {
    None,
    Vst3 { path, class_id },
    Builtin { device: String, samples: Vec<DeviceSample> },   // new
}
pub enum Effect {
    Vst3 { path, class_id },
    Builtin { device: String },                                // new
}
pub struct DeviceSample { pub slot: u32, pub path: String }
```

- An explicit variant instead of a pseudo-path in `Vst3` (the Audio Unit
  locator approach): built-ins have no bundle, never go through path checks,
  scanning or the helper, and "is this built in" should be a type, not a
  string comparison.
- Sample paths live in the model, not in an opaque blob, so project validation
  can bound them, `.rill` saves can bundle them with the other media, and
  missing-media handling applies to them. Trim points, pitch and the rest are
  ordinary parameters.
- Validation: `device` is 1–32 characters of `[a-z0-9-]`; at most 16 samples
  with unique slots. An unknown `device` loads, stays in the project unchanged
  and renders silence with a visible "Unknown device" advisory, like a missing
  plug-in.
- Older Rill versions reject the new variants, which matches how removed
  variants are handled today. [PROJECT_FORMAT.md](PROJECT_FORMAT.md) gets the
  new variants and a version note.

### Engine (`daw-engine`)

Built-ins run **in process** on the graph workers, not in a `daw-plugin-host`
child. The child process exists to contain third-party native code; this is
our own safe Rust, and running it directly removes IPC cost and lets many
instances stay cheap.

- `Processor` gains `Builtin(Box<dyn Device>)` next to `Vst`, and `Insert`
  gains a `Builtin` arm. Preparation creates the device from the registry,
  calls `prepare`, applies saved parameters, hands over samples, and reads
  latency and tail for the existing delay compensation and tail calculation.
- Automation and manual edits go through the same `PluginAutomation` and live
  parameter queue used for hosted plug-ins; the closure calls
  `device.set_parameter` instead of `queue_parameter_for_block`.
- Parameter edits on a built-in are always live-compatible (no graph rebuild).
  Changing a sample is a controlled preparation, like changing an audio clip
  source.
- The Compressor's external sidechain uses the existing
  `EffectSlot.inputs` routes and their latency alignment; its descriptor
  declares one auxiliary stereo input.
- A device error cannot occur in `process` (no fallible operations). Nonfinite
  output is guarded once, at the device boundary, by replacing the block with
  silence and resetting the device; the existing final-output guard stays.

### App, Library and tools

- **Library.** Built-ins appear as verified device cards with producer
  "Rill", without scanning, and before third-party results. The empty-library
  text no longer says that MIDI is silent until a plug-in is installed.
- **Instrument and Effects windows.** For a built-in, the window body is the
  device panel itself instead of the overview with **Open editor**. The
  shared `utility_header`, **Automate…**, **Pick parameter** and **AI for this
  context** stay as they are.
- **Parameter metadata** comes straight from the registry. No helper process,
  no asynchronous inspection, no "unknown until read" state.
- **Tools.** `create_instrument_part`, the instrument and insert tools accept
  built-in catalog IDs (`rill:r1-mini`). `plugin_describe_parameters` reports,
  for built-ins, the plain range, unit, choice labels and formatted current
  value in addition to the normalized value, and `set_plugin_parameters`
  accepts a plain value as an alternative to the normalized one. This meets
  the rule in `claude.md` that tools must not require guessing normalized
  values. A new `set_device_sample` tool assigns an already imported project
  audio source to a sample slot; like other current-song tools it accepts no
  arbitrary path.
- **Audition.** Panel keyboards and drum pads use the existing held-note
  audition path to the render worker.
- Every device ships with an end-to-end test through the running app, per
  `AGENTS.md` ([Verification](#verification)).

### Plug-in readiness

A panel talks to its owner only through this trait:

```rust
// daw-devices-ui
pub trait DeviceHost {
    fn value(&self, id: u32) -> f64;                    // normalized
    fn begin_edit(&mut self, id: u32);                  // one gesture = one Undo step
    fn set_value(&mut self, id: u32, normalized: f64);
    fn end_edit(&mut self, id: u32);
    fn automation(&self, id: u32) -> Option<AutomationState>;
    fn telemetry(&self) -> Option<&Telemetry>;
    fn sample(&self, slot: u32) -> Option<SampleView<'_>>;   // name, peaks for drawing
    fn request_sample(&mut self, slot: u32);            // host opens its chooser
    fn audition(&mut self, note: Option<(u8, f32)>);
}

pub fn show(ui: &mut egui::Ui, device: &DeviceDescriptor, host: &mut dyn DeviceHost, state: &mut PanelState);
```

The app implements it over the session service. A later VST3/CLAP wrapper
would implement it over the plug-in parameter API and call the same `show`,
with `daw_ui::theme::apply` on its own egui context. That wrapper (window
embedding, format glue, packaging) is not part of this design.

## Shared UI library additions

All of these go into `daw-ui` first, with gallery entries in
`examples/component_gallery`, light and dark previews in `docs/assets/ui-gallery`
and a section in [UI.md](UI.md), following the existing rule. They use the
existing palette and tokens; automation orange, AI pink and command blue keep
their meanings. A new **modulation** accent is needed for Maxi (proposal:
violet), added to `Palette`.

Every draggable element also has a non-drag path (numeric field or context
menu), so each operation stays possible with plain clicks, and every canvas
handle uses the 21-point `HIT_TARGET`.

### Controls

| Component | Purpose | Notes |
| --- | --- | --- |
| `Knob` (extend) | The main device control. | Add `.bipolar()` (arc grows from center), `.taper(Taper::Log)` for frequency and time, `.default_value(v)` with double-click and context-menu **Reset**, `.modulation(range)` drawing a thin outer arc for modulation depth, `.format(fn)`/`.parse(fn)` so "1.2 kHz" displays and types correctly. Existing compact and detailed modes stay. |
| `labeled_knob` | Knob with caption above and value below in a fixed cell. | Device panels are grids of these; the cell reserves height so rows align (the `AGENTS.md` centering rule). Sizes: 34 (dense), 48 (standard), 64 (hero). |
| `segmented` | Exclusive choice of 2–5 short options (Digital/Tube/Opto/FET). | Radio semantics. Falls back to the existing `select` when options do not fit. |
| `stepped_selector` | Previous/next arrows around a current-value menu (wavetable, preset, division). | Reuses `choice_menu`. |
| `toggle_led` | Small on/off with a lit state for section enables (Echo on, Band on). | Uses the existing `switch` semantics; compact visual. |
| `xy_pad` | Two parameters on one surface (cutoff × resonance). | Returns per-axis changes; relative Shift-fine drag like `Knob`. |
| `envelope_editor` | ADSR / AHDSR with draggable break points and optional per-segment curve. | Plain data in, `EnvelopeEdit` out. Optional playhead dots from telemetry. |
| `pad_grid` | 4×4 pads with name, activity flash, selection and drop target. | Click height on the pad sets audition velocity. Returns pressed/released/selected/dropped. |
| `keyboard_strip` | Horizontal audition keyboard with held-note and root-key display. | Reuses the `piano_key` painting. |
| `mod_source_chip` and `mod_slot_row` | Draggable modulation source; one row of the matrix (source, destination, amount, remove). | Dropping a chip on a knob reports `(source, destination)`; a context menu on any modulatable knob offers the same without dragging. |
| `preset_bar` | Preset name, previous/next, menu (factory/user, Save as…, Reset). | Part of `device_panel`. |
| `device_panel` and `device_section` | Panel frame: name, preset bar, bypass, input/output meters, optional Mix and Output knobs; titled sections with spacing instead of nested outlines. | Gives all ten devices one header and one rhythm. |

### Visualizations

| Component | Shows | Data it takes |
| --- | --- | --- |
| `frequency_plot` | Log-frequency magnitude graph: one or more response curves, optional spectrum backdrop, optional draggable band nodes. | Curves as `&[f32]` dB values over a shared log grid; nodes as `(freq, gain, selected, color)`. Returns node drags. Used by Equalizer, Filter, Mini, Maxi, Sampler, Modulator. |
| `transfer_plot` | Input→output curve with a live operating point. | Curve samples and the current input level. Compressor (dB/dB) and Saturator (linear). |
| `gain_reduction_history` | Scrolling input envelope with the gain-reduction trace over it. | Ring of `(input_peak, reduction_db)` columns. |
| `reduction_meter` | Vertical gain-reduction bar reading downward. | One value plus hold; reuses `channel_meter` styling. |
| `scope` | Oscilloscope with optional zero-cross sync. | Sample slice. |
| `spectrum` | Spectrum as a standalone or as a backdrop for `frequency_plot`. | Magnitude bins; reuses the `signal_analyzer` painting where possible. |
| `wavetable_view` | Current wavetable frame, with neighboring frames stacked faintly behind it. | Frames as slices plus position. |
| `lfo_view` | One LFO cycle with a moving phase marker. | Shape samples and phase. |
| `sample_view` | Sample waveform with draggable start, end and loop markers, and voice playheads. | Peak columns (reuses `waveform.rs`), marker positions; returns marker drags. |
| `delay_view` | Echo taps as decaying bars (left/right), with the reverb decay drawn behind them. | Tap times and gains, decay time, pre-delay. |
| `harmonics_view` | Bars for harmonics 1–10. | Ten values in dB. |

The drawing data for static graphs (EQ curve, filter response, compressor
curve, saturation curve and harmonics, envelope shapes) comes from pure
functions in `daw-devices` that use the same coefficient code as the audio
path, so a drawn curve cannot drift from what is heard.

## Shared DSP toolkit

`daw-devices::dsp`, used by all devices and unit-tested on its own:

| Block | Approach |
| --- | --- |
| Parameter smoothing | Linear ramp (gains, mixes) and one-pole (times); filter coefficients interpolated per sample. |
| Oversampling | 2×/4×/8× polyphase half-band FIR, linear phase, latency reported to the engine. Used around every nonlinearity. |
| Linear filters | Zero-delay-feedback (TPT) state-variable filter as the default; biquads with matched magnitude at Nyquist (Vicanek's matched design) so bells and shelves near 10–20 kHz are not cramped at 44.1/48 kHz. |
| Nonlinear filters | ZDF transistor ladder (4-pole, `tanh` stages, resonance to self-oscillation) and Sallen-Key with clipped feedback, each solved with one Newton step per sample, run at 2×. |
| Waveshapers | Static curves with first-order antiderivative anti-aliasing, inside the oversampler. |
| Delay lines | Power-of-two ring buffers with third-order Lagrange (modulated delays) or allpass (fixed fractional) interpolation. |
| Reverb core | 16-line feedback delay network, Hadamard mixing, per-line absorption filters for frequency-dependent decay, slow delay modulation against metallic ringing, allpass input diffusion, multitap early reflections. |
| Oscillators | PolyBLEP saw/pulse/triangle (Mini); wavetable with one band-limited mip level per octave built by FFT at load, crossfaded between levels and frames (Maxi). |
| Envelopes and LFOs | Exponential-segment ADSR/AHDSR with click-free retrigger; LFO shapes sine, triangle, saw, square, sample-and-hold, with tempo sync. |
| Level detection | Peak and RMS followers, log-domain gain computer with soft knee. |
| Sample playback | 16-tap windowed-sinc polyphase interpolation whose cutoff lowers when pitching up, to keep aliasing out. |
| Hygiene | Denormal guard, DC blocker after asymmetric shapers, equal-power dry/wet. |

Processing is stereo `f32`. All devices are deterministic: the same events and
parameters give bit-identical output, including "analog" drift and noise,
which use a seeded generator reset by `reset()`.

## Effects

All effects share the `device_panel` header (preset, bypass, input/output
meters) and end with **Mix** (where parallel blending makes sense) and
**Output** (−24 to +24 dB). Those two are not repeated in the tables. Panels
are 610 points wide (`POPUP_WIDTH_MD`) unless noted.

### R1 Spacer

Echo and reverb in one device: an echo section and a reverb section that can
run in series or in parallel. One device covers a slapback, a tempo delay, a
room, a hall, and a delay washing into a hall.

| Section | Parameter | Range / choices | Default |
| --- | --- | --- | --- |
| Echo | On | toggle | on |
| | Type | Digital, Tape, Analog | Digital |
| | Sync | toggle | on |
| | Time | 1 ms – 2 s (log) | 350 ms |
| | Division | 1/32 – 1/1, straight, dotted, triplet | 1/8 dotted |
| | Feedback | 0 – 100 % | 35 % |
| | Stereo | Mono, Ping-pong, Wide | Ping-pong |
| | Low cut / High cut (in the feedback path) | 20 Hz – 2 kHz / 500 Hz – 20 kHz | 120 Hz / 8 kHz |
| | Wobble | 0 – 100 % | 0 % |
| | Level | −∞ – 0 dB | −6 dB |
| Reverb | On | toggle | on |
| | Type | Room, Chamber, Hall, Plate | Hall |
| | Size | 0 – 100 % | 50 % |
| | Decay | 0.1 – 20 s (log) | 1.8 s |
| | Pre-delay | 0 – 250 ms | 15 ms |
| | Damping | 0 – 100 % | 40 % |
| | Low cut / High cut | 20 Hz – 1 kHz / 1 – 20 kHz | 100 Hz / 12 kHz |
| | Diffusion | 0 – 100 % | 80 % |
| | Movement | 0 – 100 % | 25 % |
| | Width | 0 – 150 % | 100 % |
| | Level | −∞ – 0 dB | −9 dB |
| Global | Routing | Echo → Reverb, Parallel | Echo → Reverb |
| | Duck | 0 – 100 % | 0 % |

**DSP.** Echo: one interpolated delay line per channel. Digital is clean.
Tape adds wow and flutter (two slow LFOs plus noise on the delay time, scaled
by Wobble), soft saturation and progressive high-frequency loss in the
feedback loop. Analog models a bucket-brigade line: band-limited before and
after the delay, companded soft clipping, darker repeats. Changing Time glides
(pitch-bending like tape) in Tape and Analog and crossfades in Digital.
Reverb: the shared FDN. Type selects delay-length sets, early-reflection
patterns and diffusion; Size scales lengths from a small room to a large hall
continuously, so "room to hall" is one knob with Type as a starting character.
Duck lowers the wet signal while the dry input is loud, so echoes fill the
gaps instead of masking the source.

**Panel.** `delay_view` across the top (taps and decay tail, updates as you
turn anything). Left half Echo, right half Reverb, each a `device_section`
with its enable. Bottom strip: Routing, Duck, Mix, Output.

### R1 Modulator

Chorus, flanger and phaser in one device with shared controls.

| Parameter | Range / choices | Default |
| --- | --- | --- |
| Mode | Chorus, Flanger, Phaser | Chorus |
| Character | Clean, Analog | Clean |
| Rate | 0.01 – 20 Hz (log) | 0.6 Hz |
| Sync / Division | toggle; 8 bars – 1/16 | off |
| Depth | 0 – 100 % | 50 % |
| Feedback | −95 – +95 % | 0 % (Chorus), 50 % (Flanger, Phaser) |
| Center | Chorus 5 – 30 ms; Flanger 0.1 – 10 ms; Phaser 100 Hz – 8 kHz | 12 ms / 1.5 ms / 800 Hz |
| Voices / Stages | Chorus 1 – 4 voices; Phaser 2, 4, 6, 8, 12 stages | 2 / 4 |
| Shape | Sine, Triangle, Random | Sine |
| Stereo | 0 – 180° LFO offset between channels | 90° |
| Wet low cut | 20 – 500 Hz | 20 Hz |

**DSP.** Chorus: up to four modulated Lagrange-interpolated delay voices per
channel with evenly spread LFO phases. Flanger: one short modulated delay with
signed feedback (through-zero is left out). Phaser: a chain of first-order
allpass stages with signed feedback. Analog character band-limits and softly
saturates the delay path (bucket-brigade style) for chorus and flanger and
adds stage nonlinearity for the phaser; it runs at 2×. Mix at 100 % with
Chorus gives vibrato. Mode-specific defaults apply when the mode is changed
from the panel, as one Undo step.

**Panel.** `frequency_plot` showing the moving comb or notch response with
two phase dots for left and right LFO positions. One row of knobs: Rate,
Depth, Feedback, Center, Stereo; Mode and Character as `segmented` above.

### R1 Saturator

| Parameter | Range / choices | Default |
| --- | --- | --- |
| Model | Transistor, Tube, Tape | Tube |
| Drive | 0 – 36 dB | 6 dB |
| Character | 0 – 100 % (Transistor: hardness; Tube: bias; Tape: compression) | 50 % |
| Tone | −6 – +6 dB tilt around 1 kHz | 0 dB |
| Focus | 20 Hz – 1 kHz; content below it passes clean | 20 Hz |
| Auto gain | toggle | on |
| Quality | 2×, 4×, 8× oversampling | 4× |

**DSP.** All models: pre-emphasis, oversampled anti-aliased waveshaper,
de-emphasis, DC blocker. *Transistor*: asymmetric diode-style clipper whose
knee hardens with Character; odd harmonics dominate, edgy. *Tube*: triode-style
asymmetric soft curve; Character moves the bias point, trading even against
odd harmonics, with a slight level-dependent low-end bloom. *Tape*: symmetric
soft saturation where high frequencies saturate earlier than lows, a gentle
head-bump low shelf, and program-dependent compression; this is a tape-style
model, not a physical hysteresis simulation. Focus is a crossover that keeps
bass out of the shaper so low end stays solid under heavy drive. Auto gain
compensates the static loudness change from Drive. Latency from oversampling
is reported.

**Panel.** `transfer_plot` with the live operating point on the left,
`harmonics_view` on the right. Model as `segmented`; hero Drive knob; Character,
Tone, Focus, Mix, Output in a row.

### R1 Compressor

| Parameter | Range / choices | Default |
| --- | --- | --- |
| Mode | Compressor, Limiter, Expander | Compressor |
| Character | Digital, Tube, Opto, FET | Digital |
| Threshold | −60 – 0 dB | −18 dB |
| Ratio | 1:1 – 20:1 (Compressor, Expander); fixed ∞:1 in Limiter | 4:1 |
| Attack | 0.02 – 250 ms (log) | 10 ms |
| Release | 5 ms – 2.5 s (log) | 120 ms |
| Auto release | toggle | off |
| Knee | 0 – 24 dB | 6 dB |
| Makeup | −12 – +24 dB | 0 dB |
| Auto makeup | toggle | off |
| Lookahead | 0 – 10 ms (Digital character and Limiter) | 0 ms; 5 ms in Limiter |
| Ceiling | −12 – 0 dB (Limiter) | −0.3 dB |
| Sidechain | Internal, External | Internal |
| Sidechain high-pass | 20 – 500 Hz | 20 Hz |
| Stereo link | 0 – 100 % | 100 % |

**DSP.** A shared log-domain gain computer with soft knee; Character changes
the detector topology and timing law, which is where these designs actually
differ:

- *Digital*: feed-forward peak detection, exact ratio and times, optional
  lookahead. Transparent.
- *Tube* (variable-mu style): feedback detection, ratio that rises with level,
  a knee that is never narrower than 12 dB, slow smooth
  timing, mild even-harmonic color that grows with reduction.
- *Opto*: feedback detection with a two-stage, program-dependent release (fast
  initial recovery, slow tail that lengthens after sustained compression),
  attack limited to about 10 ms and slower.
- *FET*: feedback detection with very fast attack (down to 20 µs), release
  curve with program dependence, and odd-harmonic color at the gain element.

The colored characters run at 2×. *Limiter*: lookahead brickwall with
4× oversampled peak detection to control inter-sample peaks at the Ceiling;
this is not a certified true-peak limiter. *Expander*: downward expansion
below Threshold; high ratios act as a gate. Lookahead and oversampling latency
is reported and compensated by the engine. External sidechain uses the
insert's existing auxiliary input routing.

**Panel.** `gain_reduction_history` across the top with `reduction_meter` at
its right edge; `transfer_plot` at the left with draggable threshold. Mode and
Character as two `segmented` rows; knob row: Threshold, Ratio, Attack, Release,
Knee, Makeup; a second, quieter row for sidechain, link, lookahead/ceiling.
Controls that do not apply in the current mode are hidden, not disabled.

### R1 Equalizer

Eight bands: a high-pass, six parametric bands, a low-pass.

| Band | Parameter | Range / choices | Default |
| --- | --- | --- | --- |
| 1 (HP) | On, Frequency, Slope, Q | 20 Hz – 2 kHz; 6, 12, 18, 24, 36, 48 dB/oct | off, 30 Hz, 12 dB/oct |
| 2 – 7 | On, Type, Frequency, Gain, Q | Bell, Low shelf, High shelf, Notch; 20 Hz – 20 kHz; ±24 dB; Q 0.1 – 18 | on, 0 dB; band 2 low shelf 100 Hz, bands 3 – 6 bells at 250 Hz, 800 Hz, 2.5 kHz, 6 kHz, band 7 high shelf 10 kHz |
| 8 (LP) | On, Frequency, Slope, Q | 200 Hz – 20 kHz; same slopes | off, 18 kHz, 12 dB/oct |
| Global | Mode | Digital, Analog | Digital |
| | Auto gain | toggle | off |
| | Analyzer | Off, Input, Output | Output |

**DSP.** *Digital*: minimum-phase matched-magnitude filters, so the audible
curve equals the drawn curve up to 20 kHz at any sample rate; precise and
neutral. *Analog*: proportional-Q bells (broad at small gains, narrower as
gain rises), shelves with the gentle opposite dip of passive designs, and a
subtle output-stage saturation; realized with SVFs. Coefficients are
interpolated per sample, so sweeping a band is click-free. Zero latency in
both modes. A band at 0 dB gain is skipped.

**Panel.** A large `frequency_plot` (about 60 % of the height) with the
`spectrum` backdrop and one draggable node per enabled band: drag for
frequency and gain, vertical drag with the secondary button or the band strip
knob for Q, double-click to toggle the band. Below it a strip of eight band
columns (enable, type, three numeric knobs), with the selected band
highlighted. Width 987 points (`POPUP_WIDTH_LG`).

### R1 Filter

| Parameter | Range / choices | Default |
| --- | --- | --- |
| Model | Clean, Ladder, Sallen-Key, State Variable, Comb, Vowel | Clean |
| Type | Low-pass, High-pass, Band-pass, Notch (Clean, Sallen-Key LP/HP only); Morph for State Variable | Low-pass |
| Slope | 12, 24 dB/oct (Clean) | 12 |
| Cutoff | 20 Hz – 20 kHz (log) | 1 kHz |
| Resonance | 0 – 100 % | 20 % |
| Drive | 0 – 24 dB | 0 dB |
| Morph | 0 – 100 % (State Variable: LP → notch → HP; Vowel: A-E-I-O-U) | 0 % |
| LFO rate, Sync, Shape | 0.01 – 20 Hz; sine, triangle, saw, square, S&H | 1 Hz, sine |
| LFO depth | ±5 octaves | 0 |
| LFO stereo | 0 – 180° | 0° |
| Envelope amount | ±5 octaves | 0 |
| Envelope attack / release | 0.1 – 100 ms / 10 ms – 2 s | 5 ms / 150 ms |

**DSP.** *Clean*: linear ZDF state-variable filter, stable under fast
modulation. *Ladder*: nonlinear four-pole transistor ladder with the
characteristic bass thinning at high resonance and self-oscillation.
*Sallen-Key*: two-pole with clipped feedback, aggressive and screaming at high
resonance. *State Variable*: two-pole with continuous low-pass/notch/high-pass
morph, smooth resonance. *Comb*: tuned feedback delay, Cutoff sets the pitch,
Resonance the (signed) feedback. *Vowel*: three parallel formant band-passes
morphing between vowels. Nonlinear models run at 2×. The same kernels are the
filters inside Mini, Maxi and Sampler.

**Panel.** `frequency_plot` with one node (drag = cutoff and resonance) whose
curve moves with the modulated cutoff from telemetry. Model as a
`stepped_selector`; hero Cutoff knob; Resonance, Drive, Morph; an LFO
`device_section` with `lfo_view`; an Envelope section.

## Instruments

All instruments share: **Volume**, **Voice mode** (Poly, Mono, Legato),
**Voices**, **Glide** (0 – 2 s), **Bend range** (0 – 24 semitones), and
velocity as a float from the engine. Voice stealing takes the quietest
releasing voice first, then the oldest, with a 5 ms fade to avoid clicks.
Each panel has a `keyboard_strip` at the bottom for mouse audition.

### R1 Mini

A small subtractive synth in the classic two-oscillator, ladder-filter
layout. Everything is visible at once; about 45 parameters.

| Section | Parameters |
| --- | --- |
| Oscillator 1 | Wave (saw, pulse, triangle, sine), Octave (−2 – +2), Pulse width (5 – 95 %) |
| Oscillator 2 | Wave, Octave, Semitones (±12), Detune (±50 cents), Pulse width, Sync to oscillator 1 (toggle) |
| Mixer | Oscillator 1, Oscillator 2, Sub (square one octave down), Noise levels |
| Filter | Cutoff, Resonance, Envelope amount (±), Key tracking (0 – 100 %), Drive |
| Filter envelope | Attack, Decay, Sustain, Release |
| Amp envelope | Attack, Decay, Sustain, Release |
| LFO | Rate, Sync, Shape, → Pitch, → Cutoff, → Pulse width; mod wheel scales LFO depth |
| Velocity | → Amp, → Cutoff |
| Voice | Mode, Voices (1 – 8), Glide, Bend range, Drift (0 – 100 %), Volume |

**DSP.** PolyBLEP oscillators with hard sync, mixed into the shared nonlinear
ladder filter, whole voice at 2× oversampling. Drift adds a slow, seeded
per-voice random walk to pitch and cutoff so stacked notes do not sound
static. The mixer can overdrive the filter input, as on the hardware this
layout comes from.

**Panel** (987 points, one page):

```text
┌ R1 Mini ── preset ◂ ▸ ─────────────────────────────── scope ── Volume ┐
│ OSC 1        OSC 2          MIXER      FILTER              AMP        │
│ wave  oct    wave oct semi  o1 o2      [response plot]     [envelope] │
│ pw           det  pw  sync  sub noise  cut res env key drv A D S R    │
│ LFO  [shape] rate sync →pitch →cut →pw   FILTER ENV [envelope] A D S R│
│ VOICE mode voices glide bend drift       velocity →amp →cut           │
├ keyboard ─────────────────────────────────────────────────────────────┤
```

### R1 Maxi

A wavetable synth in the spirit of the current generation of "everything
visible" synths: two wavetable oscillators, a third analog/noise oscillator,
two filters, drag-and-drop modulation and a built-in effects rack. It is by
far the largest device (about 280 parameters), so it is specified as three
deliverable stages. Each stage is a usable instrument, and all parameter IDs
are reserved from stage 1 so later stages do not break saved projects.

**Stage 1: sound engine.**

| Section | Parameters |
| --- | --- |
| Oscillators A and B | Wavetable (about 40 factory tables, generated in code: analog shapes, harmonic series, formants, digital), Position, Warp mode (None, Bend, Sync, Squeeze, Mirror, FM from the other oscillator), Warp amount, Octave, Semitones, Fine, Unison voices (1 – 16), Unison detune, Unison blend, Stereo spread, Phase, Random phase, Level, Pan, Filter routing (Filter 1 ↔ Filter 2) |
| Oscillator C | Wave (sine, triangle, saw, square, white noise, pink noise), Octave, Semitones, Level, Pan, Filter routing |
| Filters 1 and 2 | Model and Type (the R1 Filter kernels), Cutoff, Resonance, Drive, Morph, Key tracking; Filter routing Serial / Parallel |
| Envelope 1 (amp) | Attack, Hold, Decay, Sustain, Release, with a curve per segment |
| Voice | Mode, Voices (1 – 32), Glide, Bend range, Quality (1×, 2×), Volume |

**Stage 2: modulation.**

| Section | Parameters |
| --- | --- |
| Envelopes 2 and 3 | as Envelope 1 |
| LFOs 1 – 4 | Shape (sine, triangle, saw, square, S&H), Rate, Sync, Trigger (Free, Retrigger, One-shot), Fade in, Phase |
| Macros 1 – 4 | 0 – 100 %, renamable |
| Matrix | 24 slots: Source, Destination, Amount (bipolar). Sources: envelopes, LFOs, macros, velocity, key, mod wheel, per-note pressure, pitch bend, per-note random. Destinations: every parameter marked `modulatable`. |

Modulation is evaluated per voice at control rate (every 16 samples) and
interpolated, with per-sample evaluation for amp and pitch.

**Stage 3: effects and presets.** A fixed-order rack of six slots, each with
an enable and a reduced parameter set, reusing the effect kernels: Saturator
(Model, Drive, Mix), Modulator (Mode, Rate, Depth, Mix), Echo (Division,
Feedback, Mix), Reverb (Size, Decay, Mix), Compressor (Threshold, Ratio,
Makeup), Equalizer (Low, Mid, High). Plus a factory preset bank that covers
bass, lead, pad, pluck, keys and effects.

**DSP.** Mip-mapped band-limited wavetables with frame and level
crossfading, so a saw stays alias-free across the keyboard and Position sweeps
are smooth. Unison voices are spread in pitch and pan with level compensation.
FM is phase modulation between A and B. Per-note pressure and Pitch 7.9
tuning from Rill's MIDI 2.0 model reach the voice directly, so expressive
data recorded in Rill is audible without controller mapping.

**Panel** (987 points, three pages as `tab_choice`: **Synth**, **Matrix**,
**Effects**):

```text
┌ R1 Maxi ── preset ◂ ▸ ── [Synth] Matrix Effects ── scope/spectrum ─ Vol ┐
│ OSC A [wavetable view] table pos warp  unison det  lvl │ FILTER 1 [plot] │
│ OSC B [wavetable view] table pos warp  unison det  lvl │ FILTER 2 [plot] │
│ OSC C  wave oct lvl                                    │ routing         │
├─────────────────────────────────────────────────────────────────────────┤
│ ENV 1 ENV 2 ENV 3  [envelope editor]   │ LFO 1 2 3 4  [lfo view]         │
│ sources: (ENV1)(ENV2)(ENV3)(LFO1)…(VEL)(KEY)(WHEEL)(PRESS)  MACROS ◉◉◉◉  │
├ keyboard ───────────────────────────────────────────────────────────────┤
```

Source chips drag onto any modulatable knob; the knob then shows the
modulation arc and its context menu lists its modulations with amounts. The
Matrix page shows the same 24 slots as rows.

### R1 Sampler

One sample played across the keyboard.

| Section | Parameters |
| --- | --- |
| Sample | slot 0 (project audio source); Root key, Fine tune (±100 cents), Start, End, Reverse, Gain |
| Loop | Mode (Off, Forward, Ping-pong, One-shot), Loop start, Loop end, Crossfade (0 – 500 ms) |
| Pitch | Key tracking (toggle), Transpose (±24) |
| Filter | Type (LP, HP, BP; 12, 24 dB/oct), Cutoff, Resonance, Envelope amount, Key tracking, Velocity → Cutoff |
| Filter envelope | Attack, Decay, Sustain, Release |
| Amp envelope | Attack, Hold, Decay, Sustain, Release |
| LFO | Rate, Sync, Shape, → Pitch, → Cutoff, → Level, → Pan |
| Voice | Mode, Voices (1 – 32), Glide, Bend range, Velocity → Level, Pan, Volume |

Start, End and loop points are normalized positions in the sample, so they
are ordinary automatable parameters and survive a sample swap sensibly.

**DSP.** Windowed-sinc interpolation with pitch-dependent band-limiting;
equal-power loop crossfade; sample-rate conversion is implicit in the playback
ratio. Samples above the engine's resident-PCM limit (32 MiB decoded) are
refused with a clear message instead of being streamed; streaming sample
playback is not part of this design.

**Sample handling.** **Load sample…**, a drop from the OS or the Library, or
**Use as sampler instrument** on an audio clip. The file goes through the
existing import worker (WAV, MP3, FLAC, OGG) and is saved with the project
like clip media.

**Panel** (610 points): `sample_view` across the top with draggable start,
end and loop markers and live voice playheads; rows for Sample/Loop,
Filter + envelope, Amp envelope, LFO; `keyboard_strip` showing the root key.

### R1 Drums

Sixteen pads, one sample per pad, a short shaping chain per pad tuned for
one-shots.

- Pads map to MIDI notes 36 – 51 (C1 – D#2), bottom-left to top-right.
- Sample slots 0 – 15.

| Per pad (×16) | Range / choices | Default |
| --- | --- | --- |
| Start, End | normalized positions | 0, 1 |
| Reverse | toggle | off |
| Pitch, Fine | ±24 semitones, ±100 cents | 0 |
| Pitch envelope amount, decay | ±48 semitones, 5 – 500 ms | 0, 50 ms |
| Attack, Hold, Decay | 0 – 100 ms, 0 – 2 s, 5 ms – 10 s | 0, full length, full length |
| Trigger | One-shot, Gate | One-shot |
| Filter | one bipolar control: low-pass to the left, off at center, high-pass to the right | center |
| Drive | 0 – 24 dB (Saturator kernel, Transistor model) | 0 dB |
| Velocity → Level | 0 – 100 % | 80 % |
| Choke group | Off, 1 – 4 | Off |
| Gain, Pan | −∞ – +12 dB, L – R | 0 dB, center |
| Mute | toggle | off |

Global: Volume. That is 18 parameters per pad; each pad owns a fixed block of
32 IDs so pads can gain parameters later without renumbering.

**DSP.** The Sampler's playback kernel without looping; attack/hold/decay
envelope; one SVF; optional drive at 2×; choke groups fade the choked pad over
5 ms. Up to 4 overlapping voices per pad so fast rolls do not cut themselves
off unless Trigger is Gate or a choke group says so. One stereo output; the
descriptor format allows per-pad outputs later through the existing
multi-output instrument routing.

**Panel** (987 points): `pad_grid` on the left (pad name, activity flash,
drop target, click to audition with height-based velocity, context menu:
Load, Clear, Copy/Paste pad, Swap with…). Right: the selected pad's
`sample_view` with start/end markers and the amp envelope drawn over the
waveform, then one knob row in signal order: Pitch, Pitch env, Attack, Hold,
Decay, Filter, Drive, Gain, Pan.

Without samples Drums is silent, which undercuts the reason for built-ins;
see the factory content question in [Decisions needed](#decisions-needed).

## Verification

**DSP unit tests in `daw-devices`** (fast, no audio device):

- Parameter-ID snapshot (IDs never change or get reused).
- Response accuracy: each linear filter and EQ band matches its analytic
  magnitude within 0.1 dB up to 20 kHz at 44.1 and 48 kHz; the drawing
  functions and the audio path agree.
- Aliasing: oscillators and each saturator model at specified drive keep
  aliased components at least 80 dB (oscillators) and 70 dB (nonlinearities at
  4×) below the fundamental for fundamentals up to 5 kHz at 48 kHz.
- Compressor: measured static curve matches threshold/ratio/knee; attack and
  release reach 63 % in the set time (Digital); limiter output never exceeds
  the Ceiling on an inter-sample-peak test signal measured at 4×.
- Reverb: measured RT60 within 10 % of Decay at 1 kHz; no DC or runaway at
  maximum feedback and size.
- Robustness: silence in gives exact silence out after the tail; no denormal
  slowdown; no NaN/Inf for any parameter combination in a randomized sweep;
  click-free parameter jumps (bounded sample-to-sample delta under a full-range
  step).
- Determinism: two runs give bit-identical output.
- No allocation in `process`, using the engine's existing allocation counter.
- A `devices_bench` example reporting per-device CPU time per block, published
  as measurements, not as guarantees.

**Engine tests:** built-in instrument and insert render, automation of a
built-in parameter equals the manual value path, latency compensation with
Compressor lookahead, save/reopen round trip, unknown device ID.

**UI tests in `daw-ui`:** each new component gets the same kind of interaction
and geometry tests the existing ones have, and a gallery page.

**End-to-end, through the running app** (required by `AGENTS.md`): for each
device, add it to a track from the Library, play a MIDI part or audio clip,
change parameters through the panel and through tools, check the rendered
audio changed in the expected direction (for example, a low-pass lowers
high-band energy; Compressor lowers crest factor), Undo, save and reopen.

## Implementation order

Suggested split into separate threads, foundation first:

1. **UI foundation** (`daw-ui`): the controls and visualizations above, with
   gallery pages and tests. No DSP needed; uses fixed example data.
2. **Device foundation and vertical slice**: `daw-devices` skeleton (trait,
   parameters, registry, DSP toolkit basics, test harness), `daw-devices-ui`
   with `DeviceHost`, the model/engine/app/tools integration, and **R1
   Filter** end to end as the proof. After this, adding a device touches only
   the two device crates plus its E2E test.
3. **Effects**, in three independent pieces: Equalizer; Saturator and
   Compressor; Modulator and Spacer.
4. **R1 Mini.**
5. **Sample plumbing, R1 Sampler and R1 Drums.**
6. **R1 Maxi** in its three stages.
7. Documentation: PRODUCT_DIRECTION, ARCHITECTURE, PROJECT_FORMAT, USER_GUIDE,
   UI and AI_TOOLS updates travel with the change that makes each statement
   true.

Steps 1 and 2 can run in parallel. Step 3 onward needs both. The DSP-heavy
pieces (toolkit, nonlinear filters, compressor characters, reverb, Maxi) and
the engine integration are the ones where the stronger model is worth it; the
UI components, panels, gallery and documentation are well-bounded work.

## Decisions needed

1. **What "plugins" means.** This design builds the devices into Rill and
   keeps them wrapper-ready ([Plug-in readiness](#plug-in-readiness)). If you
   also want them shipped as VST3/CLAP plug-ins for other DAWs now, that adds
   a wrapper crate, native window embedding for egui, and packaging per
   platform. Recommendation: in-app first.
2. **Equalizer band count.** Specified as six parametric bands plus high-pass
   and low-pass, eight in total. The alternative reading is eight parametric
   bands plus the two filters (ten nodes). Recommendation: eight in total; it
   fits the panel and covers ordinary mixing.
3. **Default instrument.** Should a new MIDI track, and `create_midi_part`
   without an instrument, start with R1 Mini instead of silence?
   Recommendation: yes, once Mini exists; it is the main payoff of built-ins
   for a first session and for the assistant.
4. **Factory samples for Drums and Sampler.** Options: ship empty; ship a
   small kit rendered by us from R1 Mini/Maxi patches (no licensing question,
   a few hundred kilobytes); or source a CC0 kit. Recommendation: the
   self-rendered kit, after Maxi stage 1.
5. **Maxi scope.** Confirm the three-stage plan and the stage 1 limits (no
   user wavetable import, no drawable LFOs) or name what must be in the first
   version.
6. **Naming.** Crates `daw-devices` and `daw-devices-ui`; device IDs
   `r1-spacer`, `r1-modulator`, `r1-saturator`, `r1-compressor`,
   `r1-equalizer`, `r1-filter`, `r1-mini`, `r1-maxi`, `r1-sampler`,
   `r1-drums`. IDs are persisted in projects, so they are settled before the
   first device ships.
