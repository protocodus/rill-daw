# vst3-host 0.10.0: isolated audio transport, native menus and stable loading

Source: the published `vst3-host` 0.10.0 crate from crates.io, licensed under MIT
(see LICENSE). Upstream repository: https://github.com/HelgeSverre/rust-vst3-host/.
Upstream commit: `7bece4e39cff2925a0a33b40092f2c2dcaaea9ac`.
Published crate SHA-256: `57a1b316200861ab8b9f634c587e882b796cb2fa8005d9f6c6d21df42386e896`.

The root Cargo manifest patches the exact existing 0.10.0 dependency to this copy.
Source, published manifests, README, license, and VCS provenance are retained;
registry bookkeeping, the independent lockfile, changelog, and convenience shell
script are omitted. Unused upstream example-only dev-dependencies are removed from
the normalized Cargo manifest so host unit tests run with the root workspace lockfile;
Cargo.toml.orig preserves the upstream manifest. No build artifacts are included.
Three upstream trailing-whitespace lines were trimmed for repository diff checks.

The local patch adds `Plugin::set_parameter_context_menu_actions` and
`HostNotification::ParameterAutomationRequested`. Native editors can obtain a
host-prefilled `IContextMenuTarget` through `IComponentHandler3`, whether they
render copied host items themselves or ask the host to render `popup()`. Existing
plugin-provided entries and targets remain supported. Actions are opt-in,
validated against current controller metadata, and execute only on the control
thread. Changes to the configured actions and component-handler destruction invalidate copied
targets. Weak references prevent pending-menu ownership cycles. The isolated host
command protocol forwards the same configuration and notification.

Plugin loading and `Plugin::has_editor` never call `IEditController::createView`.
Controller presence supplies an optimistic editor capability hint, as it already
does during discovery; requesting the editor validates whether a view is available.
This avoids initializing GUI frameworks inside an audio-only worker. An eager
load-time editor probe caused the reported Hammond B-3X crash; native editors still
open explicitly in Rill's separate editor helper.

Isolated lifecycle and configuration commands use the configured bounded slow
operation deadline (30 seconds in Rill), including start/stop, sample rate, block
size, bus reconfiguration and complete parameter enumeration. Native instruments can legitimately spend longer
than one audio-block deadline in these calls; AAS initialization took about 200 ms.
Audio `Process`, `ProcessBuses` and `ProcessBlock` retain Rill's 100 ms live deadline, so a stalled
processor still terminates promptly. The native fixture delays lifecycle callbacks
by 200 ms and checks both successful initialization and enforced timeout bounds.
Offline audio uses Rill's separate five-second processing deadline. These bounds
contain hung helpers; they do not establish hardware callback schedulability.

The isolated command pipe uses one persistent writer and a one-request queue.
The response deadline covers request writes as well as response reads, including
large state transfers to a helper that leaves stdin open without reading it.
Shutdown never writes directly to that pipe, and kills an unresponsive child
before bounded writer/reader cleanup. Protocol tests exercise a blocked 4 MiB
state write and shutdown while the command pipe is full.

The additive `ProcessingCapabilities` handshake negotiates `ProcessBlock` and
shared PCM. A block contains a bounded, ordered `BlockCommand` list for transport,
parameters and sample-offset musical events followed by one audio processing
request. Complete value, offset, count and bus-shape validation precedes native
state changes. The list preserves same-offset ordering and supports at most
16,384 operations, 4,096 explicit parameter points and 8 MiB of owned native
event payload, sharing bounds with the native queues. The parameter queue also
rejects mapped MIDI fanout beyond capacity before mirroring the rejected value
to the controller; accepted points are never silently discarded. Batching does
not combine lifecycle, metadata or editor calls.
`BlockTransport::Auto` uses negotiated shared PCM when available, `Batched`
retains lossless inline PCM, and `Legacy` retains immediate commands and the
previous processing request. Older helpers can reject the capability query and
continue through the original protocol. In-process implementations apply the
same admitted operations without IPC. `IpcStats` counts actual requests and
serialized bytes rather than local queued operations.

`shared_audio` provides reusable host/helper mappings backed by a private named
temporary file using `memmap2` and `tempfile`. The header verifies identity,
layout and capacity. Fixed bounds are 64 buses per direction, 256 aggregate
channels, 65,536 frames per channel and 64 MiB of PCM. Mapping setup is outside
block processing and touches the PCM pages. Blocks exchange generation/frame
identities over the control pipe; atomics transfer ownership before either
endpoint reads or writes PCM. Native plug-ins see separate local buffers.
Transfers retain all sample bits, and inactive channels and unused tails become
silence. The fallback inline codec remains exact IEEE-754 data.

Failed or stale exchanges permanently poison a mapping rather than recycle
storage that a late helper could still write. A sticky instance failure also
rejects legacy processing calls until recovery, so partially applied operations
cannot leak into a later block through another API. Reconfiguration acknowledges
`ReleaseSharedAudio` before dropping the old owner; shutdown and failed setup
reap the child before temporary-file cleanup where necessary. Windows file
deletion therefore follows both peers' unmapping. Unix mappings also retain
their backing storage after the owner closes or removes its pathname. Mapping
creation/admission failures can select inline batching, but a failed native
processing call is never retried via a second transport.

This patch preserves one helper-owned lane for native lifecycle, control and
DSP calls. Shared PCM removes audio encoding and large pipe payloads; it does
not turn the synchronous adapter into a hard real-time callback API. Graph and
helper threads retain ordinary scheduling until a dedicated bounded DSP lane
with correct platform deadline coordination exists.

Regression coverage lives with the upstream COM implementation unit tests and
process-isolation serialization tests. A native test plugin terminates its worker
if `createView` is reached: `native_dsp_load` checks DSP/state and capability queries,
while the app test checks parameter editing, audible export, and archive recall.
Run from the repository root:

```sh
cargo test --locked -p vst3-host --lib component_handler_tests
cargo test --locked -p vst3-host --lib host_notifications_and_restart_requests_round_trip_across_the_wire
cargo test --locked -p vst3-host --lib slow_commands_are_classified_apart_from_the_per_block_ones
cargo test --locked -p vst3-host --lib process_isolation::block
cargo test --locked -p vst3-host --lib shared_audio
cargo build --locked --workspace --bins
cargo test --locked -p daw-plugins --test native_dsp_load
cargo test --locked -p daw-plugins --test block_transport
cargo test --locked -p daw-app --test plugin_loading_e2e
```

Remove the local patch once an upstream release provides equivalent host-prefilled
actions, retained-target validation, side-effect-free editor capability queries,
separate bounded lifecycle/audio deadlines, ordered block processing and shared
PCM ownership. Verify native menus, headless loading, transport equivalence,
stale-response rejection and process cleanup before removing it.

Rill also exposes native bus names and Main/Aux roles through the additive
`AudioBusDetailsLayout` metadata query. The isolation protocol forwards that query,
explicit bus activation and bus arrangements, preserving native indices and
inactive slots. The Rill adapter admits bounded layouts before allocating audio,
selects the actual Main role (which need not be index zero), and reapplies selected
auxiliary activation after state restoration or helper recovery. Existing stereo
processing entry points keep their previous default routing.

The isolated Rill helper now forwards channel-specific Note Expression metadata,
controller mappings and sample-offset controller parameter changes. High-resolution
musical events reuse the existing owned `PluginEvent` wire representation: float
note velocity, host note IDs, double normalized expression values, integer note
attributes, and native float poly pressure. MIDI-1 attacks retain their original
seven-bit normalization. These are explicit VST3 musical mappings, not raw UMP or
universal MIDI-2 protocol support. Expression enumeration rejects malformed counts
before allocation. The mapping ABI and existing VST3 event structures are retained;
no alternative plug-in format is introduced. Rill ships the host and helper from
the same executable build; existing command names and default processing behavior
remain intact.

The MIDI-2 per-note IDs follow the official **VST SDK 3.8.1 build 84**
[`ivstnoteexpression.h`, pinned commit `4f547e8e`](https://github.com/steinbergmedia/vst3_pluginterfaces/blob/4f547e8e102b47de4a8b8aaf343c73b700786372/vst/ivstnoteexpression.h#L61-L88):
assignable controllers use `201000 + index`, registered controllers
`202000 + index`, and note attributes the reserved `203000`/`204000` ranges.
Older Rust bindings omit these constants; their native `u32` type IDs remain ABI
compatible. Rill forwards only IDs the plug-in explicitly advertises for that
event bus/channel, without inferring mappings for ordinary vendor custom IDs.
Unsigned 32-bit controller payloads become `f64::from(data) / f64::from(u32::MAX)`,
preserving all 32-bit levels in the SDK's
[absolute normalized `[0, 1]` expression value](https://github.com/steinbergmedia/vst3_pluginterfaces/blob/4f547e8e102b47de4a8b8aaf343c73b700786372/vst/ivstnoteexpression.h#L114-L128).

Additional native regressions:

```sh
cargo test --locked -p daw-plugins --test multi_bus
cargo test --locked -p daw-plugins --test expressive
cargo test --locked -p daw-engine --lib midi2_render_tests
```

Isolated `Vst3Host::load_plugin_class` forwards the class identifier to the helper
without requiring a VST3 32-hex UID. Rill's helper hosts CLAP and LV2 behind the
same command protocol, where identifiers are reverse-DNS IDs or URIs; in-process
loads and the helper's own VST3 path still validate VST3 UIDs.

`window::HostWindow` is the native top-level window that `PluginWindow` used
privately, made public and format-neutral: it creates the NSWindow/HWND/X11 window,
exposes its embedding handle, follows editor resizes and DPI changes, and reports a
user close. `PluginWindow` keeps its API and behaviour on top of it. Rill's CLAP
editor embeds into the same window, so close handling, titles and capture match
VST3 editors on every platform.
