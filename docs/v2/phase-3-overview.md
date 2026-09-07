# Version 2 - Phase 3 Overview

## Status

Completed. Power-on and standby were manually verified against the AVR. Volume
control is implemented and partially manually verified; its normalized level
mapping remains a follow-up validation item.

## Objective

Add safe, capability-gated Main Zone controls to the application API and CLI
for the validated AVR/AVC-X3800H target. GUI integration is a later milestone.

## Scope

Phase 3 controls power, input, normalized volume level, mute, and surround
mode. The application API uses typed commands and confirmed outcomes. The
one-shot CLI performs a fresh status preflight, checks an optional snapshot
resource version when supplied, and dispatches an accepted command once; it exits with an
explicitly unconfirmed result and asks the user to run `get status`.

Controls are enabled only when the connected receiver maps to the validated
X3800H capability record. Unknown models and other Denon or Marantz receivers
remain read-only. Input and surround selectors contain only values whose set
command and confirmation query pass live X3800H validation. Reference protocol
documentation supplies candidates, not capability claims.

Additional zones are excluded from all v2 phases. All controls remain Main Zone
only. HEOS, raw-command entry, relative volume commands, macros,
automation, scheduling, remote/cloud access, and controls for unvalidated
models remain out of scope.

## Key Deliverables

- Typed power, input, absolute-volume, mute, and surround-mode operations.
- A distinct execute-once transport path that cannot inherit reconnect retry
  behavior intended for read-only queries.
- Structured confirmed, rejected, unconfirmed, unsupported, and transport
  failure outcomes.
- Optimistic-concurrency resource versions and rejection of stale commands.
- Normalized Volume level values from 0.0 to 100.0 in 0.5 steps, mapped to the
  validated native receiver code range.
- CLI `get capabilities` and `set ... --dry-run` workflows.
- Automated protocol fixtures for X3800H input and surround-mode allowlists.
- A one-second quiet period after application-API power-on before confirmation
  traffic, without replaying the power command.
- A model, firmware, date, command, response, timing, and unsupported-candidate
  validation record.

## Acceptance Criteria

- No control is displayed as available for an unknown or unvalidated model.
- A state-changing command is never automatically replayed after a timeout,
  disconnect, or reconnect.
- Successful socket delivery alone is not reported as confirmed success.
- The application API confirms a command with a matching authoritative query;
  failed confirmation produces an unconfirmed result.
- Application-API power-on prevents subsequent AVR traffic for at least one
  second and never resends the power command during confirmation.
- The CLI never replays a dispatched command and clearly separates dispatch
  from confirmation.
- Volume accepts only 0.0–100.0 values in 0.5 steps; GUI changes commit on
  release rather than sending every intermediate position.
- Every shipped input and surround choice is covered by the automated protocol
  release gate; live hardware validation remains required before broadening
  the allowlists.
