# Architecture

The project is a virtual Cargo workspace with independently compiled layered
packages. Presentation assembles application policy with concrete adapters;
network, filesystem, and runtime details stay at the edge.

```text
CLI / Iced GUI
      |
application ports and use cases
      |
domain types     protocol primitives
      ^                 ^
      \---- infrastructure adapters ----/
```

| Layer | Responsibility |
| --- | --- |
| `crates/domain` | Receiver identity, capabilities, Main Zone, and audio-context values. |
| `crates/protocol` | Transport-independent AVR, HEOS, and AppCommand framing/parsing. |
| `crates/application` | Ports, focused policies, and serialized controller lifecycle. |
| `crates/infrastructure` | SSDP, YAML, synchronous TCP/HTTP, and the Tokio AVR session. |
| `crates/gui-lib` | Iced state, reducer, views, and serialized controller bridge. |
| `apps/cli`, `apps/desktop`, `apps/diagnostics` | Delivery and concrete composition. |

## Invariants

- AVR commands end in one CR; HEOS commands end in CRLF.
- Domain and protocol code have no socket, filesystem, runtime, or presentation dependency.
- Operations are bounded and report context; sessions serialize writes.
- Uncorrelated lines remain events, and reconnect invalidates prior authority.
- Unsupported, malformed, unavailable, disconnected, and unknown data stay distinct.
- Capabilities are model- and evidence-bound; unvalidated models are read-only.
- State-changing controls execute once and require authoritative confirmation.
- Tests use deterministic fakes or local servers; live validation records model,
  firmware, settings, command, response, and date.

## Roadmap

Phases 1 and 3–7 established the layers, controls, controller, desktop GUI,
listening modes, and diagnostic observations. Phase 8 adds Quick Select and EQ
status. Phase 9 replaced the former package with an enforced Cargo workspace;
Phase 10 applies the desktop visual identity. Detailed scope and acceptance
criteria live in [the V2 plans](docs/v2/phase-8-quick-select-eq-overview.md),
[Phase 9](docs/v2/phase-9-workspace-refactor-overview.md), and
[Phase 10](docs/v2/phase-10-visual-identity-overview.md). The active Version
3 work includes [Phase 1: X3800H HTTP probe reliability](docs/v3/phase-1-x3800h-http-info-probe-overview.md)
and the completed [Phase 3 architecture refactor](docs/v3/phase-3-refactoring-overview.md).

Future HEOS, JSON contracts, additional zones, and broader-model support need
their own evidence and design.
