# Architecture

The project is a layered Rust library with a thin CLI. Dependencies point
toward stable protocol and domain code; network I/O, persistence, and runtime
details stay at the edges.

```text
CLI presentation                 src/bin/denon-avr-remote/main.rs
        |
Application operations           src/application/, with ports and use cases
        |
Domain and protocol              src/domain/, src/protocol/
        ^
Infrastructure adapters          src/infrastructure/
        |
Operating system and network
```

## Current components

### Presentation

`src/bin/denon-avr-remote/main.rs` parses commands, selects a receiver, calls
library operations, and renders human-readable output. It owns no protocol
framing or socket loops.

### Application operations

`src/application/` contains the canonical ports and use cases. Receiver
selection and main-zone status live behind those ports rather than in the
presentation layer.

### Domain and protocol

`src/domain/` owns receiver identity, capability, and main-zone value types.
`src/protocol/` owns transport-independent AVR and HEOS protocol primitives.
These modules remain usable without a network connection or async runtime.

`src/domain/capabilities.rs` records model facts and validation state. A command
appearing in reference documentation is not, by itself, an enabled capability.

### Infrastructure

- `src/infrastructure/discovery_ssdp.rs` performs interface-aware SSDP
  discovery and bounded AIOS description probing.
- `src/infrastructure/config_yaml.rs` loads and saves the user’s receiver
  identity without credentials.
- `src/infrastructure/tcp_avr.rs` contains the bounded synchronous TCP adapter.
- `src/infrastructure/avr_session.rs` owns one Tokio TCP connection,
  serialized request writes, bounded CR framing, unsolicited-event routing,
  structured errors, reconnect backoff, and connection-generation tracking.

The canonical infrastructure adapters are the only supported concrete I/O boundary.

## Invariants

1. AVR commands end in exactly one CR; HEOS commands end in CRLF.
2. Protocol modules do not depend on sockets, Tokio, CLI code, or platform APIs.
3. Every bounded operation has an explicit timeout and reports its context.
4. A persistent session has one writer and one reader; requests are serialized.
5. Lines that do not correlate to a solicited request are preserved as events.
6. A reconnect invalidates cached authority; status is queried again.
7. Unsupported, malformed, disconnected, and unavailable data remain distinct.
8. Capabilities are evidence-bound and model-specific.
9. Tests use deterministic fakes or local servers; live hardware tests document
   model, firmware, settings, commands, responses, and date.

## Data flows

### One-shot status

```text
CLI -> saved identity or discovery -> TCP adapter
    -> AvrCommand -> CR-framed queries -> parsed fields -> human output
```

### Persistent status

```text
application -> AvrSession request queue -> session actor -> TCP 23
            <- correlated response / event channel
            <- reconnect -> generation change -> fresh snapshot
```

## Boundaries

The one-shot CLI is the supported Kubernetes-style user interface. The persistent session is a
library capability; CLI control operations are deliberately not implied by it.
HEOS remains a separate protocol and client boundary. JSON output, additional
zones, and broader model compatibility require their own evidence and design.

## Version 2 Planned Architecture

Version 2 has four defined milestones. These are planned boundaries, not
current v1 capabilities.

Phase 1 reorganizes the flat crate into domain, application, protocol,
infrastructure, and presentation layers. Application policy depends on ports
instead of concrete YAML, SSDP, or TCP implementations; the layered modules are exposed directly without legacy façade modules.

Phase 2 adds a read-only GUI and a background receiver worker that owns the
persistent session. Iced sends connect, disconnect, and refresh intents and
receives immutable lifecycle and partial-state updates. Both CLI and GUI use a
platform-native configuration file after a non-destructive one-time import of
the legacy relative YAML file.

Phase 6 adds typed main-zone controls and an execute-once transport path.
Read-only queries may repeat after reconnect; state-changing commands never do.
Each command is capability-gated, serialized, and followed by an authoritative
query. Only live-validated X3800H controls and choice values are exposed.

Phase 7 extracts the temporary GUI worker into an application-owned
`ReceiverController`, extends the Phase 1 ports for long-lived lifecycle
coordination, and completes graceful shutdown and observability. The existing
CLI and canonical layered APIs remain supported.

```text
CLI presentation             Iced presentation
          \                     /
             ReceiverController
            /    |       |     \
      discovery config transport observability
                         |
                     AVR session
```

## Architecture TODO

The following work is intentionally deferred. Each item should preserve the
invariants above and add tests before becoming a user-facing capability.

### Remaining work

- Keep the Phase 1 layered source reorganization and canonical APIs
  covered by boundary and API tests.
- Implement the Version 2 read-only Iced GUI and configuration migration
  defined by the Phase 2 plans.
- Add the evidence-gated, execute-once control workflow defined by the Phase 6
  plans.
- Extract the reusable controller, injectable edges, cancellation, graceful
  shutdown, and observability defined by the Phase 7 plans.
- Add property/fuzz tests for CR framing, malformed UTF-8, oversized frames,
  response correlation, and configuration parsing.
- Define a capability registry keyed by model and firmware evidence instead of
  expanding a single placeholder record.

### After the Version 2 roadmap

- Implement HEOS as a separate async client with JSON envelope parsing,
  player identity, event registration, and reconnect refresh.
- Decide whether JSON output belongs in a versioned CLI contract.
- Add observability hooks for timeouts, reconnects, dropped events, and
  receiver-reported errors without leaking credentials or sensitive network
  data.
- Document compatibility and support policy for additional receiver models.

## Documentation structure

Project plans and milestone records live under `docs/`:

- `docs/`: user guides; `docs/v1/`: Version 1 phase plans and architecture notes,
  and release notes.
- `docs/v2/`: Version 2 layered-architecture, GUI, control, and lifecycle
  stabilization plans plus GUI guidance.
- `docs/archive/`: retired or superseded documentation; it is not active scope.

Active phase files use `phase-<number>-<topic>.md`; phase directories are not
used. Root-level documents are limited to repository orientation, contribution
rules, and architecture.
