# Version 2 - Phase 10 Source Catalog Architecture

**Status: Planned — implementation follows live X3800H read validation.**

## Research summary

Denon's X3800H manual separates two receiver-owned settings under **Inputs**:

| Receiver setting | Intent | Constraints relevant to the app |
| --- | --- | --- |
| Source Rename | Change an input's display name. | The displayed name may be manually edited, may be overwritten by an acquired HDMI device name, and is limited to 16 characters. |
| Hide Sources | Remove unused inputs from receiver selection surfaces. | `Show` is the default; `Hide` means the source is not used. |

The manual states that these names are displayed by the receiver itself and that hidden sources are removed from receiver UI/front-panel displays. [X3800H Source Rename and Hide Sources](https://manuals.denon.com/AVRX3800h/NA/EN/download.php?filename=%2FAVRX3800H%2FNA%2FEN%2Fpdf%2FAVRX3800H_NA_EN.pdf) therefore governs the intended desktop behavior.

A community implementation uses a POST to `/goform/AppCommand.xml` containing `GetRenameSource` and `GetDeletedSource`. Its parser expects `functionrename/list/{name,rename}` and `functiondelete/list/{FuncName,use}`, interpreting `use == "0"` as hidden. This is useful candidate evidence, but it is not vendor documentation nor proof that an AVR-X3800H with the target firmware returns the same shape. [Candidate parser and request](https://github.com/JPHutchins/denonavr2016/blob/master/denonavr2016/denonavr.py) must be confirmed by a recorded live fixture before use.

No reliable, model-specific network mutation request was found for changing either setting. The plan consequently separates receiver-side editing (already documented and user-accessible) from future desktop-side writing (unapproved until verified).

## Domain model

Add a presentation catalog alongside, not inside, `Input`:

```text
SourceId            canonical command/status identifier, e.g. GAME
SourceVisibility    Shown | Hidden | Unknown
SourceEntry         { id, display_name, visibility }
SourceCatalog       { entries, freshness, generation, observed_at/error }
```

`Input` and `MainZoneControl::Input` retain the canonical identifier. The catalog maps that identifier to a receiver-reported display name. Model capabilities remain a conservative protocol allowlist, not a source-label store. Unknown source IDs found in replies are retained in diagnostic raw data but cannot become selectable without explicit capability validation.

`SourceCatalog` needs independent freshness/error state so failure to read aliases does not invalidate power, volume, or the canonical selected source. The catalog's equality/versioning must include the receiver generation to prevent a late reply from a prior connection overwriting a newer receiver.

## Ports and application use cases

Introduce a read-only `SourceCatalogReader` port with:

```text
refresh_source_catalog() -> Result<SourceCatalogObservation, OperationError>
```

The observation preserves raw response evidence separately from parsed entries and identifies complete, partial, unsupported, malformed, timeout, and disconnected outcomes. The controller calls it after the normal initial Main Zone snapshot has established a generation, on user-requested refresh, and after reconnect. It publishes a dedicated `ReceiverEvent::SourceCatalog`; it does not overload `MainZoneSnapshot`.

The controller reconciles entries as follows:

1. Canonicalize receiver identifiers using the same mapping as Main Zone input status.
2. For each known identifier, use a non-empty receiver rename as the display name; otherwise use the receiver/default canonical label.
3. Treat explicit `use == 0` as `Hidden`; any absent or malformed visibility row is `Unknown`, not hidden.
4. Keep an active hidden source in the observed catalog but filter it from the selectable projection.
5. On failure, preserve the prior catalog only within its original connection generation and mark it last-known; do not merge a partial response into a fresh catalog without per-entry provenance.

## Protocol and infrastructure plan

1. Add candidate AppCommand request constructors and XML DTOs in `crates/protocol`, independent of HTTP and Tokio. Cover exact request XML, names, Unicode/escaped 16-character labels, duplicated identifiers, empty rename values, `use` values, missing fields, and unknown XML.
2. Add fixture-backed parser tests before calling a receiver. Store the sanitized X3800H trace and its provenance in the existing active evidence style; do not promote fabricated examples to validated fixtures.
3. Extend the existing bounded AppCommand HTTP adapter in `crates/infrastructure` to issue the two read commands as one bounded request when the validated profile says it is supported. Route transport failure and malformed XML through typed `OperationError`.
4. Add the application adapter/port implementation and deterministic fake session tests. Session serialization, cancellation, reconnect generation, and unsolicited Telnet routing must remain unchanged.
5. Add an X3800H `source_catalog_read` capability flag, default false. It is enabled only by the recorded fixture/live validation, never merely because the model name resembles X3800H.

Future writes use separate ports—`rename_source(SourceId, label)` and `set_source_visibility(SourceId, Shown | Hidden)`—and separate capability flags. They require an exact request/response trace, input validation (the receiver's 16-character limit), optimistic-state policy, authoritative read-back, timeout/error behavior, and persistence across reconnect. They are not part of this plan's implementation milestone.

## Desktop integration plan

1. Replace GUI-only `source_label` aliases with a single catalog-aware presentation helper. It receives `SourceId`, catalog entry/freshness, and the context (current status, option, feedback, Quick Select, diagnostic).
2. Route the Dashboard source button, its anchored dropdown, source-control feedback, Quick Select labels, Diagnostics, and accessible descriptions through that helper. Never let a raw source code leak as the primary label when a confirmed receiver name is available.
3. Build the picker from `catalog.selectable_entries()`: `Shown` entries only; `Unknown` entries may be included only when the model's validated base capability already permits their canonical selection. The active hidden item appears as a non-selectable status row outside the options.
4. Add a Source presentation section to Settings/Advanced that shows refresh state and provides the receiver menu path for rename/hide. Do not show editable text or Hide/Show switches until the write capability has passed its separate gate.
5. On catalog updates, leave the dropdown open only if its generation is current; preserve keyboard focus where possible and close it when the selected option disappears. Announce a catalog refresh accessibly without announcing every source row.

## Test and review matrix

| Layer | Required cases |
| --- | --- |
| Protocol | Complete candidate reply; malformed XML; missing/empty label; `use=0`; unknown identifier; duplicate/conflicting row; special characters. |
| Infrastructure | Bounded request; transport/HTTP/XML failure mapping; fixture request shape; no socket behavior in protocol tests. |
| Application | Generation rejection; partial/failure freshness; canonical command preserved; active-hidden handling; reconnect refresh. |
| GUI | Renamed labels in every surface; hidden option absent; active hidden source visibly qualified; no catalog fallback; stale event ignored; focus/order and accessible names. |
| Visual/manual | 1400 × 880 and 200% text with a 16-character name; vertical source dropdown scrolling; default, HDMI-renamed, hidden, and failure states on supported platforms. |

Completion requires passing the normal workspace checks, committed deterministic fixtures/tests, a recorded X3800H validation artifact, and manual comparison against the receiver after rename, hide, show, HDMI auto-rename, reconnect, and selection of a source that is subsequently hidden. A desktop write feature cannot be called complete merely because the receiver's on-screen menu works.

