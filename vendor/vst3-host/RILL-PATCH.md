# vst3-host 0.10.0: native parameter automation menus

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

Regression coverage lives with the upstream COM implementation unit tests and
process-isolation serialization tests. Run from the repository root:

```sh
cargo test --locked -p vst3-host --lib component_handler_tests
cargo test --locked -p vst3-host --lib host_notifications_and_restart_requests_round_trip_across_the_wire
```

Remove the local patch once an upstream release provides equivalent host-prefilled
actions and retained-target validation, after verifying both native-menu paths.
