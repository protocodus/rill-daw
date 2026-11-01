# Fibonacci spacing audit

Date: 2026-11-01. Baseline: `68062d0`, following the expert UI refinements.

This change gives RILL workspaces, utilities, dialogs, and controls more room,
especially around labels. Shared geometry lives in `daw-ui`; application views
reserve useful content and reachable actions when that geometry grows.

The [component reference and gallery](UI.md), [native review](../artifacts/fibonacci-spacing-2026-11-01/native-report.txt),
and [evidence manifest](../artifacts/fibonacci-spacing-2026-11-01/native-manifest.json)
document the result. The pull request and CI records provide the test outcomes
for the reviewed revision. This audit assigns no new usability scores.

## Spacing contract

Values are logical points, mapped to physical pixels at the display density.
Measured bounds round outward where captions or keys must fit completely.

| Value | Relationship | Application |
| --- | --- | --- |
| 5 | Tight details | Caption pairs and small internal offsets |
| 8 | Related compact controls | Toolbar groups, keyboard gaps, sidebar entries |
| 13 | Ordinary rows and insets | Item spacing, form rows, card padding, menu margins, vertical window padding |
| 21 | Separate sections | Pane gutters, horizontal window padding, section boundaries |
| 34 | Full control target | Buttons, inputs, and count-item rows |
| 55 | Captioned parameter row | Control line plus scale captions |
| 89 | Larger content unit | Complete take-lane row and minimum compact Generate parameter viewport |

Ordinary gaps, form-row gaps, menu insets, and vertical window margins increase
from 8 to 13. Form label/control columns retain a 21-point gutter; compact
toolbars retain 8-point internal gaps. The scale establishes relationships:
musical coordinates, plugin units, image ratios, and measured text dimensions
remain independent of it.

## Implementation

| Surface | Before | Result |
| --- | --- | --- |
| Workspace headers and Mixer | Fixed header bands and local insets competed with context lines and captions. | Measured title/context rows, shared panel insets, clear strip gutters, and complete knob/caption reservations. Mixer keeps input → processing → measuring → output order. |
| Knobs, sliders, cards, and counts | Captions or second text lines could outgrow their allocation. | Reserve label/value bounds before artwork or rails; captioned controls align using their whole row. Count items reserve 34 points, and take lanes reserve 89. |
| Library | Tight sidebar rows, fixed footer height, and optional artwork competed with selected-device information. | Related rows use 8-point gaps. Compact selection identity sits inline with the footer; details prioritize kind/categories. Viewport-derived budgeting stays stable during idle frames. |
| Instrument and Effects | Navigation, parameter details, and action geometry varied locally. | Shared section gaps and bounded parameter details. Returning from automation resets the details scroll once so the selected parameter and action are reachable. |
| Routing | Local spacing and growing content could hide lower destinations. | Shared section spacing and bounded content preserve send controls and the output destination after resizing. |
| Generate | Header/footer reservations could reduce parameters to a sliver; narrow grid cells wrapped labels excessively. | Compact range controls share one row with measured, non-wrapping labels. Preview/Fit share the status row, primary actions stay fixed, and the parameter body retains at least 89 points at 800×600. |
| Assistant | Stacked context, status, and prompt chrome consumed the browser or diagnostics. | Compact prompt labels sit beside the editor; status shares header space, with full context on hover. Examples, tools, and diagnostics scroll above a stable composer/footer. |
| Audio processing | Fixed footer and list sizes competed in short windows. | Takes scrolls lanes and settings above pinned actions. Silence reserves feedback before analysis completes. Join budgets its list against the footer; Crossfade and boundary fades use shared form/section spacing. |
| Automation | A short graph crowded percentage labels and competed with inspectors. | Preserve a useful plot, select ticks using measured text spacing, and scroll point/segment inspectors separately. |
| Setup, Recording, Export, History, and document guards | Long warnings or growing lists could displace actions. | Reserve actual footer geometry and scroll variable content within stable viewport capacity. |
| Menus and text/number entry | Larger margins could hide final commands or keyboard rows. | Menu capacity derives from the viewport, avoiding feedback from the previous popup position or size. Naming keyboards and numeric editors reserve complete key, field, validation, and action regions. |

## Geometry and interaction guarantees

Geometry checks inspect rendered glyph bounds as well as widget rectangles.
Labels retain clearance from artwork, rails, and adjacent controls in selected,
disabled, and automated states. A compact 48-point knob reserves 61 points with
its separate 13-point caption allowance.

Footer budgets include actual font rows, control heights, native gaps, and
separators. Popup capacity must remain stable across repeated idle frames rather
than shrink from its previously allocated body. The shared regression observes
40 frames. Async feedback and repeated generation retain commit-button screen
coordinates across pointer-down and pointer-up.

Scroll bodies allocate their full visible width so wheel input works over blank
space as well as populated rows. Tests scroll to later examples, menu commands,
and parameter controls through the running app. Resizing an already-open window
is covered alongside opening directly at a compact size.

## Coverage and retained evidence

The native review covers dark/light themes at 1440×1000 and 800×600, with a
1.25× sample using 1000×750 physical pixels for 800×600 logical content. Shared
geometry tests also exercise 2× density where applicable.

The retained artifact set contains 20 unmodified PNGs: nine before/after pairs
for Arrangement, MIDI, Audio, Mixer, Routing, Generate, Automation, Takes, and
long Properties titles, plus Notes-menu scrolling and Library typography.
The manifest identifies each image hash, binary hash, viewport, scale, and
theme. The main matrix uses build 5; Takes uses revision 6, Library revision 8,
and final menus revision 9. Earlier captures are not presented as later builds.

Additional native checks cover editing tools, Quantize preview/cancel, nested
choices, color/numeric editors, device parameters, Setup, Assistant Tools,
History, About, Export, crossfades, Join, naming, and error details. All 18
1120×987 component-gallery captures were visually reviewed in both themes for
complete sections, label clearance, and clipping.

Native interaction checks used copied projects and isolated settings; original
project hashes remained unchanged. Audio was not auditioned. OS dialogs and
plugin-owned editor chrome are outside RILL's styling scope.

## Reproducing validation

From the repository root, use the dependencies and environment described in
[CI](../.github/workflows/ci.yml):

```sh
cargo fmt --all -- --check
cargo build --locked --workspace
cargo test --locked --workspace --exclude daw-app
cargo test --locked -p daw-app --bin rill
bash scripts/test-e2e.sh
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Focused mouse journeys can be rerun independently:

```sh
cargo test --locked -p daw-app --test generation_workflow_e2e --test dialog_spacing_e2e --test command_menu_spacing_e2e
cargo test --locked -p daw-app --test library_e2e --test audio_takes_e2e --test assistant_guidance_e2e --test export_sequence_e2e
```

On Linux, regenerate the component gallery with a private display:

```sh
LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a cargo run --locked -p daw-ui --example component_gallery -- --capture docs/assets/ui-gallery
```

These commands describe reproducible checks, not a claim that a particular
revision passed. Consult the PR/CI results alongside the linked native evidence.

## Separate numeric synchronization correction

The change also includes a residual correction from the earlier controls work.
Controls preserve active exact-value drafts through acknowledged model round
trips, including `f32` storage and unit conversions; actual external changes
still invalidate those drafts. Dedicated `parameter_value_sync`,
`parameter_roundtrip_draft_e2e`, and numeric-entry coverage address this behavior
separately from spacing.
