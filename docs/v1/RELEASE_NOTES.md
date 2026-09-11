# Version 1.0.0 Release Notes

**Release date:** 2026-09-07
**Package:** `denon-avr-remote` 1.0.0 (library and CLI)

## Summary

Version 1.0.0 freezes the Version 1 scope: read-only main-zone status for
Denon and Marantz AVR receivers over local IP, with the Denon AVC-X3800H as
the validated target. It ships the one-shot status CLI, the protocol and
application library, and a persistent asynchronous AVR session.

## What is included

### CLI

- `discover` — numbered receiver discovery using bounded SSDP searches on
  UDP 1900, a UDP 1800 compatibility probe, and a bounded local AIOS
  description-scan fallback.
- `status` — main-zone snapshot reporting power, input, master volume, mute,
  and surround mode independently; unavailable fields are reported as
  unavailable, never replaced with invented values.
- Receiver selection in the order saved identity
  (`config/denon-avr-remote.yaml`), numbered discovery (`--receiver`), or
  explicit host (`--host`); a successful query saves the selected identity.
- `avr`, `heos`, and `volume` development utilities for protocol framing and
  volume encoding. They are not receiver control workflows.
- Invalid arguments and failed operations exit non-zero and print usage
  guidance.

### Library

- `avr.rs` / `heos.rs` — transport-independent command validation, CR (AVR)
  and CRLF (HEOS) framing, line primitives, and receiver-specific volume
  encoding; usable without sockets or an async runtime.
- `capabilities.rs` — evidence-bound, model-specific capability records;
  reference documentation alone never enables a capability.
- One-shot synchronous status transport with bounded timeouts that keep
  timeouts, disconnects, malformed responses, and unsupported fields distinct
  from valid values.
- `AvrSession` — persistent Tokio TCP 23 session with one reader and one
  serialized writer, bounded CR framing, solicited-response correlation,
  unsolicited-event routing (`PW`, `SI`, `MV`, `MU`, and `MS` typed; unknown
  lines preserved raw), structured lifecycle errors, finite reconnect with
  configurable backoff, and connection-generation tracking that invalidates
  cached authority after reconnect.
- Typed Serde YAML configuration; saves write a sibling temporary file before
  replacing `config/denon-avr-remote.yaml`. Credentials are never stored.

### Validated on hardware

Denon AVC-X3800H at `192.168.0.8`, firmware `3.139.173` (firmware date
2026-07-14), tested over TCP 23 on 2026-09-07:

- The five main-zone queries (power, input, volume, mute, surround) passed
  with observed responses.
- Remote actions produced the expected `MUON`, `MUOFF`, `SI...`, `MV...`,
  and `MS...` unsolicited events.
- Power-on accepted the required delay before the next command, and
  standby/reconnect behavior passed the revised acceptance criteria.
- Read-only probing observed `SD?`, channel-volume `CV?`, and Zone 2 `Z2?`;
  `DC?`, `Z3?`, and an unsupported command produced no bounded response.
  Network Control and standby configuration were recorded as `not exposed`,
  not inferred.

The full record is in the [Phase 2 overview](phase-2-overview.md). These
observations are evidence for the documented v1 capabilities only; they do
not establish HEOS support, front-panel event equivalence, or broad
cross-model compatibility.

## What is not included

Version 1.0.0 is read-only. The following remain deferred and require their
own design and evidence:

- Receiver control operations (safety rules, power-on sequencing, idempotency)
- HEOS control (separate CRLF/JSON client)
- JSON output in a versioned CLI contract
- Additional zones and broader model compatibility
- Version 2 GUI work

See the Architecture TODO in [ARCHITECTURE.md](../../ARCHITECTURE.md) for the
deferred work list.

## Compatibility and maintenance notes

- Legacy direct transport APIs are deprecated in 1.0.0 and retained until a
  separately planned removal release.
- Configuration requires `host`; unknown fields and blank hosts are rejected.
- Capability claims remain evidence-bound to the X3800H validation record;
  adding a model requires recorded live validation.

## Documentation

- [CLI user guide](../cli-user-guide.md)
- [Phase 1 overview](phase-1-overview.md) and
  [Phase 1 architecture](phase-1-architecture.md)
- [Phase 2 overview](phase-2-overview.md) and
  [Phase 2 architecture](phase-2-architecture.md)
