# Architecture

This is the authoritative description of the current implementation. Protocol
research and archived plans provide evidence and history; neither changes the
rules here.

## Workspace and dependencies

The Cargo workspace contains independently compiled packages. Its permitted
workspace edges are enforced by `make boundary`.

| Package | Owns |
| --- | --- |
| `crates/domain` | Receiver identity, configuration, capabilities, state, intents, and evidence/validity types. |
| `crates/protocol` | AVR, HEOS, and AppCommand framing, wire values, and parsing. |
| `crates/application` | Use-case policy, ports, status/control policy, and the serialized receiver coordinator. |
| `crates/infrastructure` | SSDP discovery, YAML persistence, TCP/HTTP adapters, and concrete receiver sessions. |
| `crates/gui-lib` | Iced presentation state, reducers, views, and the GUI controller bridge. |
| `apps/cli` | Short-lived CLI composition over the canonical receiver service. |
| `apps/desktop` | Native GUI composition, logging, and concrete adapter wiring. |
| `apps/diagnostics` | Explicit, read-only evidence probes. |

```text
protocol       → domain
application    → domain
infrastructure → application, domain, protocol
gui-lib        → application, domain
cli            → application, domain, infrastructure
desktop        → gui-lib, application, domain, infrastructure
diagnostics    → domain, infrastructure, protocol
```

`domain` imports no workspace package. `protocol` and `application` depend only
on `domain`; infrastructure may compose all three core crates. `gui-lib` uses
application and domain only. Delivery packages compose the dependencies their
delivery role requires, but do not create competing architectural contracts.

## Session boundary

`crates/application/src/ports.rs` is the one definition site for
`SessionEvent`, `ReceiverSession`, and `SessionFactory`. A `ReceiverSession`
combines reads, one-shot writes, lifecycle events, and shutdown because one
owner is necessary to preserve ordering. `SessionFactory` creates a session;
the application coordinator owns it for its complete lifetime. GUI bridges and
adapters forward through that boundary rather than owning or duplicating a
session.

The canonical async receiver service serializes connection lifecycle and
operations. A reconnect or receiver change invalidates authority from the old
connection. Uncorrelated receiver lines remain events rather than being
assigned to a command opportunistically.

## Receiver-correctness invariants

- AVR commands end in one CR; HEOS commands end in CRLF.
- Domain and protocol code do not depend on sockets, filesystems, runtimes, or
  presentation.
- I/O is bounded and failures retain useful context.
- State distinguishes unsupported, malformed, unavailable, disconnected,
  stale, and unknown observations; unavailable data is never invented.
- Capability claims are model- and evidence-bound. Unvalidated behavior stays
  read-only or reports unsupported.
- A state-changing operation dispatches at most once once dispatch begins, and
  its result is only authoritative when receiver evidence confirms it.
- Deterministic fakes or local servers cover ordinary tests. Live validation
  records the model, firmware, relevant settings, command, response, and date.

## Delivery and enforcement

The CLI and desktop executable are composition roots. The CLI selects a
receiver, connects a canonical async service, synchronizes before reads or
writes, and closes the service after its short-lived command. Diagnostics are
deliberately independent of normal delivery behavior and never issue writes.

`make boundary` checks source imports, prohibits the retired root `src/` tree,
validates the resolved Cargo workspace edges, and ensures the session contracts
have exactly one definition. Run it whenever a package dependency or boundary
changes. Current engineering policy and verification gates are in
[docs/contributing.md](docs/contributing.md) and
[docs/development.md](docs/development.md).
