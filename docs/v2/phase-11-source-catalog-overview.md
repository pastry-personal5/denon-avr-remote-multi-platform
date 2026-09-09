# Version 2 - Phase 11 Source Catalog Overview

**Status: Implemented, evidence-gated follow-on — diagnostic capture still required before enablement.**

This follow-on increment makes the desktop app use the source names and source
visibility chosen on the receiver. It is not a dependency of visual-identity
completion. It does not replace the canonical receiver-control boundary or
turn an unverified protocol observation into a writable capability.

Before enabling product reads, use the diagnostic-only read-only `source-catalog-probe`
with an explicit AppCommand endpoint. It records candidate request XML, HTTP
status, and raw response XML and cannot change receiver state. The required
trace sequence and its current status are in the
[source-catalog validation record](phase-11-source-catalog-validation-record.md).

## Objective

Present each Main Zone source with the receiver's current user-facing label, and offer only sources that the receiver has not hidden. A source's protocol identifier remains stable for selection; its display name is receiver-owned presentation data.

The AVR-X3800H owner can configure this on the receiver under **Settings → Inputs → Source Rename** and **Hide Sources**. Denon documents that renamed names appear on the receiver's display and menus, allows up to 16 characters, and may automatically adopt an HDMI device name. It also documents **Hide** as removing an unused source from the receiver's UI and front-panel displays. [Denon's X3800H manual](https://manuals.denon.com/AVRX3800h/NA/EN/download.php?filename=%2FAVRX3800H%2FNA%2FEN%2Fpdf%2FAVRX3800H_NA_EN.pdf) is the normative user-behavior reference.

## Scope and outcomes

- Read the X3800H's source names and Show/Hide choices after connecting and on an explicit refresh.
- Show the resulting label anywhere a source is rendered: Dashboard current source button and context, source picker, command feedback, Quick Select source summaries, Diagnostics, and accessible labels.
- Exclude hidden sources from the normal picker and all selectable source lists. Do not silently delete their configuration, historical observations, or protocol identifiers.
- Keep the active source visible if it was selected before being hidden, with an explicit “hidden on receiver” qualifier; never force a source change.
- Preserve a raw canonical source identifier for commands, reconciliation, diagnostics, and fallback labels. A display name must never be sent in a Telnet `SI` command.
- When catalog retrieval is unavailable, retain the last confirmed catalog for its connection generation and label it as last known; otherwise use the existing validated canonical list without inventing aliases or hide states.

This phase must not infer source names from connector assignments, signal format, app artwork, or a local alias. It must not treat a missing/partial catalog reply as proof that an input is hidden.

## User experience

The source picker remains a vertically long dropdown below the current-source button. It contains receiver-visible entries only, one display label per row; the current item is marked rather than duplicated. Long labels wrap rather than truncate, and the canonical identifier is available in an accessible description and Diagnostics, not as competing display text.

The Dashboard header, Quick Select summary, and feedback say the receiver's chosen display name. For example, canonical `GAME` renamed to `PlayStation 5` is displayed as “PlayStation 5” throughout, while selection continues to send `SIGAME`. A hidden `AUX3` appears nowhere selectable. If `AUX3` is presently active, the header reads its configured name plus “Hidden on receiver”; it is not added back to the picker.

Until desktop write operations are independently validated, the management surface is informative: it explains that names and visibility are managed on the receiver at **Settings → Inputs → Source Rename / Hide Sources**, and offers Refresh source list. This satisfies the receiver owner's ability to edit or hide sources without falsely claiming the desktop can write an undocumented setting.

## Evidence and delivery gates

The X3800H manual establishes the feature and its user intent, but does not document the network payload. A mature Denon integration reports a read-only `AppCommand.xml` request containing `GetRenameSource` and `GetDeletedSource`, with name/rename and FuncName/use rows respectively. Treat that as a candidate shape only, not as X3800H validation. See its [implementation](https://github.com/JPHutchins/denonavr2016/blob/master/denonavr2016/denonavr.py).

Before enabling source-catalog reads for the X3800H profile, record a live,
sanitized fixture including model, firmware, date/timezone, endpoint, timing,
request XML, complete response XML, renamed entries, shown entries, hidden
entries, default names, an HDMI-discovered label if available, and a currently
selected hidden source. Capture baseline; rename; shown; selected-then-hidden;
hidden; shown/restored; reconnect; and restoration confirmation. Validate
malformed XML, absent functions, unknown identifiers, and a mid-session
rename/hide refresh.

Do not add Rename, Reset name, Hide, or Show desktop controls until a separate live record establishes the exact X3800H mutation request, response/acknowledgment, persistence across reconnect, and rollback/error behavior. The planned read-only source catalog is independently useful and safe; write support is a subsequent evidence-gated increment.

## Acceptance criteria

- A receiver rename is shown identically in every source-bearing desktop surface after a successful catalog refresh.
- A receiver-hidden source is absent from every selectable list, while an active hidden source remains accurately reported without becoming selectable.
- All source commands use canonical identifiers, never human display labels.
- Catalog failures cannot replace a known label with a guessed one or expose an input as hidden; lifecycle and freshness language explain the state.
- Diagnostics reports catalog freshness and entry count, and Quick Select source summaries use the same receiver-owned labels when a preset includes an input.
- Fixture, parser, controller, GUI reducer, accessibility, and visual cases cover renamed, hidden, active-hidden, defaults, partial replies, failure, reconnect, and stale-result rejection.

The detailed layer design and implementation order are in [the source-catalog architecture](phase-11-source-catalog-architecture.md).
