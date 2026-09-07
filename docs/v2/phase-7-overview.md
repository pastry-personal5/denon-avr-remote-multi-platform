# Version 2 - Phase 7 Overview

## Objective

Stabilize the controller lifecycle after the read-only and control workflows
are behaviorally complete, without breaking the layered Phase 1 architecture,
the canonical CLI or layered APIs.

## Scope

Main Zone is the only supported zone in all v2 phases. Additional zones are
excluded from this lifecycle phase and the wider v2 roadmap.

Phase 7 extracts the receiver lifecycle and state machine from the temporary
Iced worker, completes the application interfaces needed for deterministic
lifecycle control, and adds graceful cancellation, shutdown, and observability.
The GUI becomes one consumer of reusable application operations rather than
the owner of receiver policy.

This phase may deprecate redundant paths but does not remove or rename the v1
synchronous APIs, change established CLI output, broaden receiver capabilities,
or add new user-facing receiver features.

## Key Deliverables

- A reusable application-owned `ReceiverController` with typed commands,
  events, lifecycle state, snapshots, and control outcomes.
- Controller composition over the Phase 1 typed query, parsing, state-reduction,
  discovery, configuration, and transport boundaries.
- Injectable timing/backoff and observability boundaries alongside the Phase 1
  infrastructure ports.
- Explicit session cancellation and graceful shutdown semantics for queued and
  in-flight work.
- Structured observability for connection changes, reconnects, timeouts,
  malformed frames, command confirmation, and dropped events.
- Regression coverage proving continued CLI and layered-library behavior.

## Acceptance Criteria

- Iced code contains presentation state and message mapping but no receiver
  selection, retry, command sequencing, or confirmation policy.
- Controller tests run deterministically without live hardware, real discovery,
  real time, or user configuration directories.
- Shutdown resolves all queued work and distinguishes cancelled queries from
  controls whose delivery became indeterminate.
- Controller integration does not duplicate Phase 1 status parsing or
  reduction.
- Existing CLI commands, output contracts, and the synchronous adapter
  remain usable.
