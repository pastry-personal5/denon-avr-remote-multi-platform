# Architecture

The project is a layered Rust library with a thin CLI. Dependencies point
toward stable protocol and domain code; network I/O, persistence, and runtime
details stay at the edges.

```text
CLI presentation                 src/main.rs
        |
Application operations           status.rs, session-facing orchestration
        |
Domain and protocol              avr.rs, heos.rs, capabilities.rs
        |
Infrastructure                  discovery.rs, config.rs, status transport,
                                  session.rs
        |
Operating system and network
```

## Current components

### Presentation

`src/main.rs` parses commands, selects a receiver, calls library operations,
and renders human-readable output. It owns no protocol framing or socket
loops.

### Application operations

The current application layer is intentionally small. `status.rs` coordinates
the five-field main-zone snapshot and preserves independent field failures.
The CLI coordinates saved-identity lookup, discovery fallback, and rendering.
Persistent session consumers use `query_main_zone_async` and receive raw
unsolicited events through `AvrSessionEvent`.

### Domain and protocol

`avr.rs` owns AVR command validation, CR framing, line primitives, and volume
encoding. `heos.rs` owns HEOS command validation and CRLF framing. These modules
must remain usable without a network connection or async runtime.

`capabilities.rs` records model facts and validation state. A command appearing
in reference documentation is not, by itself, an enabled capability.

### Infrastructure

- `discovery.rs` performs interface-aware SSDP discovery and bounded AIOS
  description probing.
- `config.rs` loads and saves the user’s receiver identity without credentials.
- `status.rs` contains the bounded one-shot TCP adapter.
- `session.rs` owns one Tokio TCP connection, serialized request writes,
  bounded CR framing, unsolicited-event routing, structured errors, reconnect
  backoff, and connection-generation tracking.

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

The one-shot CLI is the stable v1.0.0 user interface. The persistent session is a
library capability; CLI control operations are deliberately not implied by it.
HEOS remains a separate protocol and client boundary. JSON output, additional
zones, and broader model compatibility require their own evidence and design.

## Architecture TODO

The following work is intentionally deferred. Each item should preserve the
invariants above and add tests before becoming a user-facing capability.

### Remaining work

- Add cancellation and graceful shutdown to `AvrSession`, including clear
  behavior for queued requests when a session stops.
- Make discovery and description scanning independently injectable and expose
  scan limits/configuration at the application boundary.
- Add property/fuzz tests for CR framing, malformed UTF-8, oversized frames,
  response correlation, and configuration parsing.
- Define a capability registry keyed by model and firmware evidence instead of
  expanding a single placeholder record.

### Before broader product scope

- Design explicit control operations with safety rules, one-second power-on
  sequencing, idempotency, and confirmation queries.
- Implement HEOS as a separate async client with JSON envelope parsing,
  player identity, event registration, and reconnect refresh.
- Decide whether JSON output belongs in a versioned CLI contract.
- Add observability hooks for timeouts, reconnects, dropped events, and
  receiver-reported errors without leaking credentials or sensitive network
  data.
- Document compatibility and support policy for additional receiver models.

## Documentation structure

Project plans and milestone records live under `docs/`:

- `docs/v1/`: v1.0.0 scope, phase plans, and validation reports.
- `docs/v2/`: v2.0.0 follow-up product areas.
- `docs/research/`: external protocol evidence and live-validation notes.

Root-level documents are limited to repository orientation, contribution rules,
and architecture.
