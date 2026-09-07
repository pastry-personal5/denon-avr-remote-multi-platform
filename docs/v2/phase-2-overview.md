# Version 2 - Phase 2 Overview

## Objective

Deliver a buildable native desktop application that presents the existing
read-only receiver workflow without weakening the v1 protocol, capability, or
partial-status guarantees. The GUI consumes the canonical domain and
application interfaces established in Phase 1.

## Scope

Phase 2 adds an Iced desktop GUI for Windows, macOS, and Linux. It uses the
persistent AVR session to connect automatically to a saved receiver, discover
and select another receiver, or accept a manual host. The main screen displays
main-zone power, input, volume, mute, and surround mode together with explicit
connection, freshness, authority, and per-field availability states.

Both the CLI and GUI move to one platform-native configuration location. If
that file does not exist, a valid legacy `config/denon-avr-remote.yaml` is
imported once and left intact. An existing platform configuration always wins.

Phase 2 is read-only. Receiver control, HEOS, additional zones, JSON or HTTP
APIs, browser and mobile clients, credentials, installers, signing,
notarization, auto-update, and a cross-platform CI build matrix remain out of
scope. The deliverable is buildable source, not packaged release artifacts.

## Key Deliverables

- A separate Iced GUI binary while preserving the existing CLI binary and
  public v1 library behavior.
- Saved-receiver auto-connect with discovery, explicit receiver selection,
  manual-host fallback, disconnect, reselection, and manual refresh.
- A read-only five-field main-zone view that preserves partial failures and
  distinguishes event-derived state from an authoritative query.
- Connected, reconnecting, disconnected, loading, empty, and actionable error
  presentations.
- Authoritative refresh after initial connection and every reconnect; cached
  state is invalidated while authority is being re-established.
- Keyboard-accessible controls, visible focus, scalable layout, meaningful
  labels, and state communication that does not rely on color alone.
- Documented CLI and GUI development commands plus deterministic GUI-state,
  configuration-migration, and local fake-server tests.

## Acceptance Criteria

- Starting with a saved receiver attempts connection without first requiring
  user interaction; failure leaves discovery and manual-host recovery visible.
- Starting without saved configuration provides discovery and manual-host
  paths without mutating receiver state.
- An initial connection or reconnect never presents stale state as
  authoritative and performs a fresh five-field query.
- Unsolicited validated events update the relevant field while unknown lines
  remain preserved outside the typed state.
- The existing CLI commands and tests remain operational after the second
  binary and configuration migration are introduced.
