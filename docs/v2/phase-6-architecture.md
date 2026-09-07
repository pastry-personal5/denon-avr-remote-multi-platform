# Version 2 - Phase 6 Architecture

Phase 6 extends the receiver worker with semantic control intents while keeping
wire strings inside the protocol adapter. The worker invokes an application
control use case and does not bypass the Phase 1 ports. Read-only queries and
state-changing execution remain separate because only queries are safe to
repeat after reconnect.

```text
Iced control intent
      -> capability check
      -> typed command encoding
      -> execute once
      -> affected-field query
      -> confirmed state or structured uncertainty
```

## Public Control Model

The library adds a `MainZoneControl` enum for power, input, absolute volume,
mute, and surround mode. Associated types represent power and mute states,
volume in validated 0.5 dB steps, and validated input and surround identifiers.
Callers do not submit arbitrary AVR strings through this interface.

`ControlCapabilities` exposes whether each operation is enabled, the absolute
volume range, and the validated input and surround allowlists for a normalized
model identity. X3800H spelling variants such as AVR/AVC and manufacturer
prefixes normalize to one model; an unknown identity never inherits those
capabilities.

A structured result distinguishes:

- confirmed: the follow-up query matches the requested state;
- rejected: the receiver reports an explicit error or a confirmed different
  value;
- unconfirmed: the action may have been delivered but no authoritative result
  can be obtained;
- unsupported: the model or requested value is outside its capability record;
- transport failure: delivery definitely did not begin or the session stopped.

## Transport Safety

Add an execute-once transport contract alongside the existing asynchronous
query contract. The session implementation uses its single serialized writer
but bypasses `request_query` and its reconnect retry. Existing public query and
synchronous compatibility APIs remain available.

Controls for a receiver are serialized. While one is pending, conflicting
controls are disabled rather than queued from repeated UI input. Absolute
volume changes are emitted only when the user commits the selected 0.5 dB
value.

Power-on is a special sequence:

1. Send `PWON` at most once.
2. Prevent all AVR writes and confirmation queries for at least one second.
3. Reconnect if required without resending `PWON`.
4. Query power and return confirmed or unconfirmed state.

Other controls execute once and then query their own family. A disconnect or
timeout after dispatch does not prove failure, so the result is unconfirmed
unless a later authoritative query resolves it.

## Capability Evidence

Before an input or surround value enters `ControlCapabilities`, test its exact
setter and query response on the target X3800H. Validate power, mute, and every
accepted absolute-volume boundary in the same record. Candidate lists may come
from official Denon references, but failed, silent, renamed, or ambiguous
values are recorded and omitted from the GUI.

## Verification Boundaries

Protocol tests cover typed encoding and rejection of raw or out-of-range
values. Fake transports cover capability gating and result classification.
Local TCP tests prove serialization, confirmation queries, no action replay,
power-on delay, reconnect behavior, and indeterminate delivery. GUI tests cover
pending/disabled controls and every result presentation. Live validation is a
release gate for every enabled X3800H operation.
