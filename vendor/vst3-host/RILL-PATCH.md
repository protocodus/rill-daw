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
