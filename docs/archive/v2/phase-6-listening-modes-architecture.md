# Version 2 - Phase 6 Architecture

**Status: Complete — 2026-09-08**

```text
GUI group selector
        |
  audio-context query
        |
typed capability registry
        |
group recall -> MainZoneControl::ListeningModeGroup
individual mode -> MainZoneControl::SurroundMode
        |
  execute once -> MSMOVIE/MSMUSIC/MSGAME or MS<MODE>
        -> authoritative MS? query
```

## Domain Model

Add a typed `ListeningModeGroup` with `Movie`, `Music`, and `Game` variants.
Grouped capability metadata maps each group to canonical `SurroundMode`
values. `SurroundMode` remains the protocol-facing value; group membership is
application/domain metadata and does not alter AVR framing.

Selecting a group is a state-changing control. The AVR recalls the last mode
associated with that group, so the controller does not choose a mode on the
user's behalf. The resulting `MS?` observation becomes the current mode.

Add an optional audio-input context sufficient to represent known or unknown
signal format and channel count. Speaker-layout and headphone restrictions
must be represented as explicit constraints when evidence is available; they
must not be inferred from the selected input name.

## Capability and Control Flow

The capability registry is keyed by normalized model identity and validation
evidence. It returns modes for a group only when the model, group, and current
audio context permit them. The X3800H mapping is initially evidence-gated;
other models do not inherit it.

The controller receives either a typed group control or surround-mode control
with the current resource version. It checks capability and conflicts, reports
a no-op when appropriate, dispatches the command once, and confirms with an
authoritative query. A receiver rejection, timeout, disconnect, or mismatching
response is reported using the existing structured control outcomes. No
fallback mode is sent by the application.

## Protocol and Evidence

The protocol adapter encodes modes as `MS<MODE>` and queries the current mode
with `MS?`. Candidate names come from the AVC-X3800H owner's manual, but each
shipped mode requires an automated protocol fixture and live validation record
covering model, firmware, settings, command, response, timing, and date.

## Verification

Tests cover exact group membership, shared modes, invalid combinations,
unknown model/context, speaker/headphone restrictions, command encoding,
resource-version conflicts, execute-once behavior, authoritative
confirmation, receiver events during an open menu, and all GUI control
outcomes. Live validation remains a release gate before broadening the
allowlist.
