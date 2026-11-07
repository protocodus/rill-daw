# vst3-host 0.10.0: native automation menus and stable plugin loading

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

Plugin activation, deactivation, reconfiguration, complete parameter enumeration,
and other lifecycle operations use the existing bounded 30-second control deadline.
AAS Player can take longer than the 100-millisecond audio deadline to activate, and
enumerating its thousands of parameters can approach that bound. These control
operations must not be mistaken for a hung audio callback. `Process` and
`ProcessBuses` retain their short deadline, and exceeding either deadline still
terminates the worker. Native fixtures exercise delayed lifecycle callbacks and
metadata enumeration, along with both lifecycle and audio timeout enforcement.

Isolated lifecycle and configuration commands use the configured bounded slow
operation deadline (30 seconds in Rill), including start/stop, sample rate, block
size, and bus reconfiguration. Native instruments can legitimately spend longer
than one audio-block deadline in these calls; AAS initialization took about 200 ms.
Audio `Process` and `ProcessBuses` retain Rill's 100 ms deadline, so a stalled
processor still terminates promptly. The native fixture delays lifecycle callbacks
by 200 ms and checks both successful initialization and enforced timeout bounds.

The isolated command pipe uses one persistent writer and a one-request queue.
The response deadline covers request writes as well as response reads, including
large state transfers to a helper that leaves stdin open without reading it.
Shutdown never writes directly to that pipe, and kills an unresponsive child
before bounded writer/reader cleanup. Protocol tests exercise a blocked 4 MiB
state write and shutdown while the command pipe is full.

Regression coverage lives with the upstream COM implementation unit tests and
process-isolation serialization tests. A native test plugin terminates its worker
if `createView` is reached: `native_dsp_load` checks DSP/state and capability queries,
while the app test checks parameter editing, audible export, and archive recall.
Run from the repository root:

```sh
cargo test --locked -p vst3-host --lib component_handler_tests
cargo test --locked -p vst3-host --lib host_notifications_and_restart_requests_round_trip_across_the_wire
cargo test --locked -p vst3-host --lib slow_commands_are_classified_apart_from_the_per_block_ones
cargo build --locked --workspace --bins
cargo test --locked -p daw-plugins --test native_dsp_load
cargo test --locked -p daw-app --test plugin_loading_e2e
```

Remove the local patch once an upstream release provides equivalent host-prefilled
actions, retained-target validation, side-effect-free editor capability queries,
and separate bounded lifecycle/audio deadlines,
after verifying both native-menu paths and headless plugin loading.

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
