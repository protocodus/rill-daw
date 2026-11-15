# Device browser redesign

Status: agreed by all five UX designers and implemented; UX lead review completed 2026-11-15.

## Goal

Find an instrument or effect, understand where it will go, and add it without
interrupting the musical task. The browser is a full workspace with Back, not a
modal plug-in manager. Browsing and inspecting never modify the song.

The five-person UX team comprises a design lead and specialists in workflow,
information architecture, visual hierarchy, and accessibility. This proposal is
based on the current application, its supported catalogue metadata and its
existing insertion, replacement, send, persistence and navigation behavior.

## Problems to solve

The existing screen gives comparable weight to discovery, installation details,
editable categories, image-capture status and insertion. Developer filtering is
duplicated. Large image cards spend valuable space on missing screenshots. The
search field competes with several controls, useful details disappear at narrow
widths, and the actual assignment destination is easy to overlook. Enter can be
queued during a scan and applied later to a different result.

## Agreed screen structure

1. Retain the shared full-page header, title **Device browser**, and **Back**.
   The quiet trailing **Manage plug-ins** menu contains **Refresh** and
   **Plug-in folders…**. A compact scan/progress status never blocks browsing.
2. A dedicated search row receives the largest available width. Its hint names
   the current type: **Search instruments**, **Search effects**, or
   **Search unverified devices**. A visible **Clear** action clears only the
   query and restores search focus.
3. Type choices **Instruments / Effects / Unverified** remain explicit. A
   secondary controls row contains one **Category** chooser, one **Developer**
   chooser, **Sort**, and **List / Grid** view choices. No duplicate developer
   sidebar. Under constrained width controls wrap into bounded rows; search is
   never squeezed between the filters. An active-filter line exposes removable
   collection/category/developer filters and **Reset filters**, which clears
   the search and all facets, retains the current device type, and returns focus
   to search.
4. The body consists of a narrow **Library** collection rail, the dominant
   results pane, and a concise selected-device inspector when space permits.
   Collections are **All devices**, **Favorites**, **Recently used**, and
   **Built-in (R1)**. Counts are for the active device type, independent of the
   other facets. The results heading reports the actual filtered result count.
   Preview availability is maintenance metadata, not a top-level collection.
5. **List is the initial view.** Each stable, whole-row target shows device name
   first, developer/category second and a compact format label; an optional
   small image supports recognition. Missing images use a quiet device icon.
   Target row height is 60–73 logical points. The optional Grid preserves the
   visual discovery mode and its stable card geometry. Both remain virtualized.
6. The wide inspector is a read-only selection summary: device name, developer,
   type/format, category and an optional bounded image. Favorites are a visible
   control. **Organize** discloses the category field, **Save**, and a visible
   reset-to-default operation. **Device details** opens the existing parameter
   and technical inspector. Install path and capture diagnostics are secondary,
   never the initial browse surface. **More** retains optional image capture and
   alternate insertion actions. All organization and details operations remain
   available from the visible **Details** control when the side inspector does
   not fit; use the same summary/organization content in an on-demand popup.
7. A fixed bottom action area presents selected device and destination together,
   with one blue primary action. **Details** and **More** are quiet secondary
   controls. Keep **Favorite** in the inspector/Details popup and item context
   menu so the footer remains compact. Long names truncate with full text on hover and never
   push the primary action offscreen. Primary action is disabled with a readable
   reason when no device or compatible destination is available.

Approximate wide proportions: 196–233pt collection rail, flexible results,
233–280pt inspector with 21pt gaps. Show the inspector only while at least 440pt
of results remain. Hide the inspector first, then replace the rail with a
visible labelled **Collection** chooser. Never bury collection selection in
View. At short heights reserve the header/search/action area first; the body
scrolls independently and still shows complete result rows. Use shared theme,
typography, spacing, focus and 34pt control-target tokens from `daw-ui`.

## Finding and keeping a device

- Opening the browser focuses search. Search is case-insensitive AND matching of words in any order across name,
  developer, automatic/custom categories and format. Keep existing semantics.
- Changing type or collection retains the query. Type changes clear incompatible
  category/developer facets. A fresh intent-specific chooser sets the correct
  type and clears stale query/facets even when reopening the same target.
  Collection and view preference may remain only when visibly represented.
- Sort choices remain **Name**, **Developer**, and **Recently added**. Recently
  used always sorts newest use first and makes its sorting behavior explicit.
- Preserve selected identity while it remains in the result set; otherwise
  select the first match. Zero results clear selection. After filtering, sort,
  keyboard movement or reflow, keep the selected result fully in view.
- Single click selects only. Arrows move selection within results; Enter or a
  same-item double click invokes exactly the visible primary action. Two quick
  clicks on different items must not activate either. Search retains Left/Right
  for caret movement. Menus, category text and other controls own their keys.
- Never queue activation behind a scan. Already discovered devices and built-ins
  can be used immediately; no later scan result can cause an unsolicited load.
- Favorite/category edits keep existing persistent storage and rollback on save
  failure. A favorite change does not also select or load another device.

## Destination and completion

Pin the destination when opening the browser. Subsequent selection changes in the
application must not redirect an assignment. Show concise purpose/destination in
the action area; no separate permanent explanatory banner is needed.

| Intent | Primary action | Context |
| --- | --- | --- |
| New instrument with no existing destination | Add instrument track | New MIDI track |
| Provisional new instrument track | Set instrument | Track name |
| Existing MIDI track | Replace instrument | Track name and current instrument |
| Effect insert | Add to effect chain | Named Audio/MIDI/Bus/Master channel |
| Send effect | Create send track | Send from named Audio/MIDI/Bus channel |
| Missing/incompatible destination | Disabled relevant action | Destination no longer available / Choose a channel to add an effect |

Never silently turn Replace into Add. Master accepts inserts but cannot be the
source of a send; do not offer that invalid alternate. Browse mode freezes any
selected destination at entry too. Keep instrument/effect type switching
available, but resolve each primary action only against that pinned context.
Unknown/unverified devices never imply a verified kind: **Try as instrument**
and **Try as effect** stay explicit secondary actions; include compatible pinned
assignment semantics rather than silently creating an unrelated track.

Successful replacement returns to the caller; a new instrument track opens its
instrument panel; insert/send actions open the relevant Effects panel. Back or
Escape restores the caller without a browser-induced song edit. A provisional
new MIDI track and ready pattern created before the chooser intentionally survive
cancellation. Preserve normal Undo behavior,
provisional-track naming rules and existing native-host isolation.

## Feedback and empty states

- Filtered empty: **No matching instruments/effects/devices**, with **Clear
  search** when applicable and **Reset filters**. Preserve the query until the
  user chooses recovery.
- Empty Favorites: **No favorite instruments/effects yet**, with concise guidance
  and **Browse all**. Empty Recently used explains that added devices appear
  here and offers **Browse all**.
- Empty Unverified: **No unverified devices**. Do not call these effects or
  suggest that no instruments exist.
- No installed devices of a type: use format-neutral copy and **Plug-in
  folders…** / **Refresh**. Built-in devices remain discoverable throughout.
- Scanning: show truthful progress independently of available results; never
  cover or disable the current catalogue or delay a primary action.
- Image capture is an optional editor screenshot. A missing/failed screenshot
  does not imply load failure, and never disables an otherwise valid device.
- Missing destination and persistence failure retain actionable, readable
  feedback without changing the task or destination behind the user's back.

## Shared components and implementation boundaries

Reusable search/results/selection-summary components belong in `crates/daw-ui`
first. The application owns catalogue filtering, pinned destinations, commands,
settings and existing detail-inspection services. Reuse standard page, toolbar,
choice, filter-chip, footer, notice and theme primitives. Avoid hardcoded colors
and one-off application styling. This proposal adds no audition engine, ratings,
AI recommendations, invented categories or new discovery service.

## Acceptance and validation

Executable running-app tests must cover every new behavior, and existing
assignment, library, device-card, resizing and navigation regressions must pass.

- Open the browser and complete the four valid insertion/replacement workflows
  by mouse; Back leaves the song unchanged; Undo reverses successful changes.
- Verify pinned destination, deleted destination, Master insert and unavailable
  Master send. Reopening a chooser for the same target resets stale type/search.
- Verify initial List, optional Grid, all four collections, filter counts,
  matching query retained across type/collection, reset, deterministic sorting,
  and appropriate collection-specific/Unverified empty states.
- Verify keyboard selection/activation, no activation from category editing or
  menus, no delayed scan activation, no activation from clicks on two items,
  and selection visibility across filter/reflow/scroll.
- Verify Favorite persistence and category save/reset in both wide inspector and
  narrow Details popup; screenshots never affect action eligibility.
- Check light/dark and minimum/wide/short/scaled viewports. Search, rows, footer,
  controls and popup bounds must not overlap or leave the viewport. Primary
  action and mouse routes to every operation remain visible or explicitly
  reachable, and focused versus selected state remains discernible.
- Capture and inspect the real app and shared-component previews in both themes.

## Consensus record

The lead assembled this proposal from all four specialists' audits. Every
specialist reviewed the complete proposal and explicitly agreed before production
implementation. No unresolved design blockers remain.

| Designer | Decision | Reviewed responsibility |
| --- | --- | --- |
| UX lead (`ux_lead`) | Agree | Coherent end-to-end design and feasible implementation |
| Workflow (`ux_workflow`) | Agree | Pinned destinations, assignment safety, completion/cancellation |
| Information architecture (`ux_information`) | Agree | Search, collections, counts, metadata and recovery |
| Visual hierarchy (`ux_visual`) | Agree | List-first hierarchy, responsive panes and bounded action area |
| Accessibility (`ux_accessibility`) | Agree | Mouse/keyboard parity, focus, states and executable validation |

Implementation is assigned to three coding agents: shared UI components,
application integration, and real-app end-to-end coverage. The parent agent
coordinates integration, build verification, and visual inspection.

## Implementation review

Implemented by the three coding agents and reviewed by the UX lead on
2026-11-15. The design lead approves the implemented visual and interaction
direction; no unresolved UX review blockers remain. This is an expert review of
the running application, not a claim of measured user-study performance.

The shared [discovery components](../crates/daw-ui/src/device_browser.rs) implement
the dedicated search row, result count, concise identity and destination summary.
The application now uses List initially, the four agreed collections, one route
to each filter, progressive organization, and pinned insertion destinations.
Review caught and corrected misleading alternate-assignment labels, unverified
send behavior, clipped default filter labels, and insufficient result height in
compact layouts. Compact mode shares result counts with active filter chips and
shows scan activity beside Manage plug-ins. Recently used explicitly says
**Latest use first**. Grid images shrink on short screens while retaining device
identity; List retains the complete 73-point row.

The visual review inspected:

- Shared-component captures in [light](assets/ui-gallery/device-browser-light.png)
  and [dark](assets/ui-gallery/device-browser-dark.png).
- Native wide browser captures in
  [light](../artifacts/device-browser-redesign/library-light.png) and
  [dark](../artifacts/device-browser-redesign/library-dark.png).
- The native 800×600 layout, including a typed query, active category, filtered
  count, selected result and fixed actions; the same filtered layout in dark at
  125% scale retained a whole result and readable, bounded controls.
- The compact Details → Organize flow with a built-in editor image. Category
  save/reset, technical details and path actions remained reachable by scrolling,
  with Close fixed outside the scrolling content. Final polish removes the
  duplicate subject heading and brings the expanded category field into view.

The completed validation log reported 39 passing tests across the 13 relevant
browser, catalogue, details, resizing, naming, assignment and navigation E2E
suites. The built-in browser metadata/preview test also passed. The new
[`device_browser_redesign_e2e`](../crates/daw-app/tests/device_browser_redesign_e2e.rs)
suite covers initial List/query retention and empty states, compact organization,
retryable category-save failure, missing pinned destinations, Master insertion,
complete results with active filters, and unverified replacement intent. The
short-screen case exercises both List and Grid, including 125% scale. See
[the runnable validation commands](E2E.md) for reproduction.

Integration with the latest main retains the Samples tab, sample preview and
loading workflows, asynchronous waveform/spectrum display, and toolbar
improvements. The integrated workspace build passed, followed by 189 app unit
tests and 68 relevant E2E tests with no failures. Native browser screenshots were
refreshed from that build, and the Samples page was visually checked as well.
