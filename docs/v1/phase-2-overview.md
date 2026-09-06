# Version 1 - Phase 2 Overview

## Objective

Validate the AVR-X3800H network and remote behavior required by the v1 status
contract, then add a persistent asynchronous AVR session without changing the
one-shot CLI contract.

## Scope

Phase 2 adds a Tokio session with serialized writes, bounded CR framing,
response correlation, unsolicited-event routing, structured failures, and
bounded reconnect with backoff. Reconnect requires an authoritative status
refresh. HEOS, control operations, JSON output, additional zones, and
unvalidated capabilities remain out of scope.

Live validation recorded TCP 23 reachability and the five main-zone queries,
power transitions, remote mute/input/volume/surround events, standby behavior,
and selected additional status families. Network Control and standby settings
that were not exposed are recorded as `not exposed`, not inferred.

## Key Deliverables

- Persistent `AvrSession` with structured lifecycle events and errors.
- Interleaved unsolicited-line handling and solicited-response matching.
- Automatic reconnect with bounded attempts and backoff.
- AVR-X3800H validation record dated 2026-09-07.
- Local tests for framing, correlation, reconnect, malformed frames, and CLI
  compatibility.

## Validation Record

The tested receiver was a Denon AVC-X3800H at `192.168.0.8`, firmware
`3.139.173` (firmware date 2026-07-14), tested on 2026-09-07 over TCP 23.
Network Control and standby configuration were not exposed by the tested
interfaces; TCP 23 was reachable in standby.

The five main-zone queries passed with observed responses for power, input,
volume, mute, and surround mode. Read-only probing also observed `SD?`,
channel-volume `CV?`, and Zone 2 `Z2?`; `DC?`, `Z3?`, and an unsupported
`FOO?` produced no bounded response in that configuration. Remote actions
produced `MUON`, `MUOFF`, `SI...`, `MV...`, and `MS...` events. Power-on
accepted the required delay before the next command, and standby/reconnect
behavior passed the revised acceptance criteria.

These observations are evidence for the documented v1 capabilities only.
They do not establish HEOS support, front-panel event equivalence, or broad
cross-model compatibility.
