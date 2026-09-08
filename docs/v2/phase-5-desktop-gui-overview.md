# Version 2 - Phase 5 Overview

**Status: Complete — 2026-09-08**

Phase 5 delivers the Version 2 native desktop GUI and integrates it with the
application-owned `ReceiverController` from Phase 4. It consolidates the
former read-only GUI milestone and the retired GUI-integration milestone.

## Objective

Deliver a buildable Iced 0.14 GUI for Windows, macOS, and Linux that presents
and controls the Main Zone through typed application APIs, without moving
receiver policy or protocol behavior into the presentation layer.

## Scope

Phase 5 adds a separate GUI binary while preserving the existing CLI and
public library APIs. The GUI supports saved-receiver auto-connect, discovery,
manual host entry, explicit receiver selection, disconnect, reconnect,
refresh, configuration migration, and bounded close/quit.

The Dashboard presents power, source, volume, mute, and sound mode, including
partial field availability, authority, freshness, lifecycle, pending,
confirmed, rejected, unsupported, transport-failure, and unconfirmed states.
Phase 3 Main Zone controls are exposed only for validated receiver
capabilities. Unknown and unvalidated models remain read-only.

The layout follows the Phase 2 information architecture and GUI design
specifications. Keyboard navigation, visible focus, scalable text, contrast,
reduced motion, accessible labels, and status announcements are required.

Additional zones, HEOS, raw commands, browser/mobile clients, credentials,
installers, signing, notarization, auto-update, and unvalidated models remain
out of scope.

## Key Deliverables

- A separate Iced GUI target and documented GUI development command.
- A controller bridge that maps typed GUI intents and `ReceiverEvent` values.
- Saved configuration, discovery, manual-host, selection, refresh, and
  shutdown workflows with usable recovery states.
- A responsive Main Zone dashboard and Settings/Diagnostics destinations.
- Controller-backed Phase 3 controls with execute-once confirmation semantics.
- Deterministic GUI-state and controller-bridge tests plus host-platform
  usability verification.

## Acceptance Criteria

- GUI code contains presentation state and message mapping only; it does not
  frame protocol commands, retry controls, or implement confirmation policy.
- Initial connection and reconnect perform fresh authoritative status queries;
  stale state is never relabelled as current.
- Late tasks and old-generation events cannot overwrite a newer selection.
- Partial failures affect only their own fields and retain recovery actions.
- Control outcomes distinguish pending, confirmed, rejected, unsupported,
  transport-failure, and unconfirmed delivery.
- Close/quit performs bounded controller shutdown without sending standby or
  replaying a state-changing command.
- Existing CLI behavior, public APIs, and boundary checks remain valid.
