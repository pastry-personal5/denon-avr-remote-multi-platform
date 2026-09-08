# Version 2 - Phase 3 Architecture

Phase 3 adds semantic Main Zone control intents while keeping wire strings
inside the protocol adapter. The application invokes a control use case and
does not bypass the Phase 1 ports. Read-only queries may repeat after
reconnect; state-changing execution is always execute-once.

```text
Application or CLI control intent
      -> capability check
      -> typed command encoding
      -> execute once
      -> affected-field query
      -> confirmed state or structured uncertainty
```

## Public Control Model

The library adds a `MainZoneControl` enum for power, input, normalized volume
level, mute, and surround mode. `VolumeLevel` accepts 0.0–100.0 in 0.5 steps
and maps to the validated native receiver code range. Associated types represent
power and mute states plus validated input and surround identifiers.
Callers do not submit arbitrary AVR strings through this interface.

`ControlCapabilities` exposes whether each operation is enabled, the absolute
volume range, and the validated input and surround allowlists for a normalized
model identity. X3800H spelling variants such as AVR/AVC and manufacturer
prefixes normalize to one model; an unknown identity never inherits those
capabilities.

A structured application result distinguishes:

- confirmed: the follow-up query matches the requested state;
- rejected: the receiver reports an explicit error or a confirmed different
  value;
- unconfirmed: the action may have been delivered but no authoritative result
  can be obtained;
- unsupported: the model or requested value is outside its capability record;
- transport failure: delivery definitely did not begin or the session stopped.

## Transport Safety and CLI Contract

Add an execute-once transport contract alongside the existing asynchronous
query contract. The session implementation uses its single serialized writer
but bypasses `request_query` and its reconnect retry. Existing asynchronous queries and the bounded synchronous adapter remain
available.

Controls use one operation per request. A snapshot resource version advances
when an accepted observation changes state. A stale version is rejected before
dispatch, and a no-op target is reported without AVR traffic. The CLI performs
a fresh status preflight, uses that version automatically when no expected
version is supplied, and rechecks the supplied version immediately before dispatch,
and never sends if preflight fails. `--dry-run` validates the same inputs and
prints the planned action without dispatching.

The CLI uses `get` and `set` action verbs, supports `get capabilities`, and
returns dispatch-only success explicitly as unconfirmed. It uses saved receiver
configuration by default for writes, writes no configuration, and reports
results on stdout and errors on stderr.

Power-on is a special sequence:

1. Send `PWON` at most once.
2. Keep the command out of reconnect retry paths.
3. Wait at least one second before any confirmation traffic.
4. The application API confirms with a later power query; the CLI reports
   dispatch without waiting and requires a later `get status`.

Other application controls execute once and then query their own family. A
disconnect or timeout after dispatch does not prove failure, so the result is
unconfirmed unless a later authoritative query resolves it. The CLI's process
ends after dispatch and never claims confirmation.

## Capability Evidence

Before an input or surround value enters `ControlCapabilities`, cover its exact
setter and query response with an automated X3800H protocol fixture. Validate
power, mute, and every accepted normalized-volume boundary in the same suite.
Candidate lists may come from official Denon references, but failed, silent,
renamed, or ambiguous values are recorded and omitted. Live hardware
validation is required before broadening the allowlists beyond this gate.

## Verification Boundaries

Protocol tests cover typed encoding and rejection of raw or out-of-range
values. Fake transports cover capability gating and result classification.
Local TCP tests prove serialization, confirmation queries, no action replay,
power-on delay, reconnect behavior, and indeterminate delivery. GUI tests cover
pending/disabled controls and every result presentation. Live validation is a
release gate for every enabled X3800H operation.
