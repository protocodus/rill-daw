# Rill: AI and tool architecture rules

> **GENERAL RULE:** "all actions that user can do should also be available as
> tools. the only exceptions are view-related operations that exist only because
> of screen size and eye limitations."

> **PRIME DIRECTIVE:** "creating music through mcp/ai tools should be easier than
> creating it by hand and also easier and much faster than using application
> control (computer use) with native gui"

These are product requirements for new functionality and for the migration of
existing functionality. A tool that merely clicks the same panels as a person
does not satisfy them. Follow [AGENTS.md](AGENTS.md) as well.

## Keep the layers separate

1. **Musical editing foundation.** Put validated, UI-independent musical edits
   in `daw-core`. Operate on stable track, clip, note, effect, bus, and parameter
   IDs. Keep units explicit. Reuse existing source-preserving MIDI/audio edits,
   routing validation, and project validation.
2. **Session application service.** Bind edits to the active song and its
   revision; validate and commit atomic changes; coordinate undo/redo, selection,
   document guards, engine updates, and background work. UI and automation share
   this execution path. A revision conflict must not overwrite a newer edit.
3. **Canonical tool layer.** Define tool names, musical descriptions, input
   schemas, capabilities, validation, and structured results once. Its handlers
   call the application service. Keep discovery and invocation consistent.
4. **MCP wrapper.** Adapt that registry to MCP discovery and execution. Do not
   create a second implementation of tools, validation, authorization, or edits.
5. **Embedded agent.** A provider-neutral agent loop uses the same tool executor
   from inside Rill. Provider adapters handle API formats and model capabilities;
   they do not edit songs. The internal executor may be in process; MCP is an
   interchangeable transport, not a reason to duplicate the tool surface.
6. **Assistant UI.** Present prompts, musical scope, progress, cancellation,
   results, and undo in the DAW. Keep provider logic and mutations out of widgets.

The dependency direction is UI/agent/MCP → tools → session → core. Provider
adapters, the AI UI, and MCP handlers must not mutate `Project` directly. Existing
GUI mutation paths remain migration work until they use the shared boundary;
do not claim complete parity merely because the first registry exists.

The implemented foundation currently exposes 33 current-song tools, four
additional application-profile tools, and 66 typed core edits.
It includes bulk composition/mixing, an atomic notes-plus-instrument workflow,
selection, parameter discovery, analysis-backed clip processing, transport,
MIDI/audio joining, and five recording tools. Enter through the menu bar’s **AI** button or
**Window → Assistant**, with provider credentials under **Setup → AI**. See
[docs/AI_TOOLS.md](docs/AI_TOOLS.md) for the current inventory and focused E2E
commands; this count is not proof of full GUI parity.

## Tools express musical intent

- Prefer “create this eight-bar pattern,” “replace these notes,” “apply these
  channel settings,” and “edit this region” over one operation per pointer move,
  note, or slider tick. Add high-level musical tools alongside composable edits.
- Support bounded bulk operations and atomic command batches. A failed batch
  leaves the song and undo history unchanged. Report created IDs and useful
  change summaries so the next call need not rediscover everything.
- Expose concise song overviews, selected scope, and detailed reads by ID and
  musical range. Parameter discovery must return real IDs, names, units, ranges,
  and choices. Do not require opening panels, using screen coordinates, or
  guessing normalized values.
- Use one history entry per meaningful committed operation. Do not hold a
  transaction open across a model/network request. Validate again when delayed
  work completes, and reject results from another song or an obsolete revision.
- Use bounded payloads, finite numbers, explicit errors, cancellation, and run
  budgets. Retries must not silently duplicate a committed musical operation.
- Keep network requests, plugin discovery, rendering, and analysis outside the
  realtime audio callback. Use the established engine edit/preparation paths.

## Coverage and authority are different

The general tool catalog must cover meaningful GUI functionality. Four
application tools expose explicit document/media I/O and redacted Setup access
through the opt-in `--mcp-application` profile; ordinary `--mcp` and the built-in
assistant retain the current-song profile. The built-in
assistant is a restricted client of that catalog: it may inspect, edit, and
audition **only the current song**. It may not create/open/replace a document,
load/save projects, export to arbitrary files, change application settings, read
credentials, or execute arbitrary code.

Enforce the current-song capability at discovery **and execution**, including
when the same tools are reached through MCP. Bind runs to a host-issued session
identity and revision; invalidate stale work when the user changes documents.
Do not expose a generic “run GUI action,” script, shell, or filesystem tool that
bypasses these limits. Internal bounded scratch/render work is a host service,
not authority to write an arbitrary user path.

Selection reads and explicit musical ranges should support the same scope a
person can choose. Pure zoom, scroll, window placement, theme, and visual aids
need no musical tool when their only effect is presentation. Do not broaden that
exception to editing, audition, transport, recording, or parameter controls.

## Provider credentials and listening

Use user-provided API keys. The requested settings format stores them encrypted
with AES-GCM and a built-in application key, alongside the other settings. Keep
plaintext keys out of project data, prompts, tool results, diagnostics, and logs;
decrypt only for provider requests. Use fresh nonces and a versioned envelope.
An encryption key shipped in the binary can be recovered: this scheme obscures
the stored secret and protects against casual inspection, not someone who has
the binary and settings file. An OS credential-store backend is a possible
future improvement, not a substitute for the currently requested format.

Model capabilities vary. Local preview rendering and deterministic audio
measurements are useful independently of provider audio input. Do not describe
text-only models as hearing audio, or claim audio understanding because a render
or waveform exists. Expose supported capability information and bounded audio
handles; avoid leaking arbitrary paths or sources through tools.

## Definition of done

- Implement the operation in the shared foundation/service and expose its tool
  in the same change as its user-facing functionality. For legacy work, identify
  any remaining parity gaps in [docs/AI_TOOLS.md](docs/AI_TOOLS.md).
  Remaining take/preset workflows, quit, and other uncovered legacy actions
  still require parity work. Recording control, MIDI/audio joining, device
  discovery, and separately authorized application/project/Setup operations are
  already in the current surface.
- Add new UI components to `crates/daw-ui` first, use its theme/tokens, and reuse
  them from application views; see [docs/UI.md](docs/UI.md).
- Every new user-facing feature needs an executable end-to-end test through the
  running app. Keep it container-compatible and run relevant E2Es; see
  [docs/E2E.md](docs/E2E.md). Test meaningful musical results, not only successful
  tool responses. Tool/MCP tests complement, rather than replace, running-app
  coverage.
- Verify normal GUI editing and undo/redo still share the resulting state and
  engine behavior. Document the actual supported tools, restrictions, and
  remaining work; never infer full feature coverage from a broad schema.
