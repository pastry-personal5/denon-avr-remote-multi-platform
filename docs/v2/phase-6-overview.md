# Version 2 - Phase 6 Overview

## Objective

Add safe, confirmed main-zone controls to the Phase 2 GUI for the validated
AVR/AVC-X3800H target.

## Scope

Phase 6 controls power, input, absolute master volume, mute, and surround mode.
Every operation uses a typed application command, is serialized, is sent at
most once, and is followed by a query of the affected field. The GUI presents
the operation as pending until that query confirms the resulting receiver
state.

Controls are enabled only when the connected receiver maps to the validated
X3800H capability record. Unknown models and other Denon or Marantz receivers
remain read-only. Input and surround selectors contain only values whose set
command and confirmation query pass live X3800H validation. Reference protocol
documentation supplies candidates, not capability claims.

HEOS, additional zones, raw-command entry, relative volume commands, macros,
automation, scheduling, remote/cloud access, and controls for unvalidated
models remain out of scope.

## Key Deliverables

- Typed power, input, absolute-volume, mute, and surround-mode operations.
- A distinct execute-once transport path that cannot inherit reconnect retry
  behavior intended for read-only queries.
- Structured confirmed, rejected, unconfirmed, unsupported, and transport
  failure outcomes.
- Per-control pending states and conflict prevention while an operation is in
  flight.
- A one-second quiet period after power-on before confirmation or any other
  receiver command.
- Evidence-backed X3800H volume bounds plus input and surround-mode allowlists.
- A model, firmware, date, command, response, timing, and unsupported-candidate
  validation record.

## Acceptance Criteria

- No control is displayed as available for an unknown or unvalidated model.
- A state-changing command is never automatically replayed after a timeout,
  disconnect, or reconnect.
- Successful socket delivery alone is not reported as confirmed success.
- A matching confirmation query updates the affected field authoritatively;
  failed confirmation produces an unconfirmed result and refresh option.
- Power-on prevents all subsequent AVR traffic for at least one second and is
  not resent when confirmation requires reconnecting.
- Volume accepts only validated 0.5 dB values and commits a slider change on
  release rather than sending every intermediate position.
- Every shipped input and surround choice has recorded live setter/query
  evidence for the X3800H.
