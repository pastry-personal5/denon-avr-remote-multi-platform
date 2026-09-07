# Version 2 - Phase 1 Architecture

Phase 1 establishes the dependency structure used by every later Version 2
phase. Clean architecture means policy is independent of delivery frameworks
and I/O mechanisms, not merely that files are stored in different directories.

```text
presentation / compatibility façades
                 |
infrastructure adapters -> application ports and use cases
                 |                    |
             protocol ------------> domain
```

Dependencies may point toward `domain`. Application code may depend on domain
types and its own port traits, but not on protocol encoders or infrastructure.
Infrastructure may depend on application ports, protocol code, domain types,
Tokio, sockets, serde, and operating-system APIs. Presentation assembles the
application with infrastructure and converts typed results into user-facing
text.

## Target Source Layout

```text
src/
  lib.rs                         public API and compatibility exports
  domain/
    mod.rs
    receiver.rs                  identity and normalized model
    capabilities.rs              evidence-bound model capabilities
    main_zone.rs                 typed values, partial state, events, authority
  application/
    mod.rs
    ports.rs                     configuration, discovery, and status contracts
    receiver_selection.rs        saved/manual/discovered selection policy
    main_zone_status.rs          five-field query and refresh use case
  protocol/
    mod.rs
    avr/
      mod.rs
      command.rs                 validation, CR framing, volume encoding
      response.rs                correlation and typed parsing
    heos.rs                      independent CRLF framing and line parsing
  infrastructure/
    mod.rs
    config_yaml.rs               serde DTO and filesystem repository
    discovery_ssdp.rs            SSDP and bounded description probing
    avr_session.rs               persistent Tokio actor and reconnect behavior
    tcp_avr.rs                   synchronous v1 compatibility adapter
  bin/
    denon-avr-remote/
      main.rs                    composition and process exit
      commands.rs                CLI argument dispatch
      render.rs                  human-readable presentation
  avr.rs, capabilities.rs, config.rs, discovery.rs, heos.rs,
  response.rs, session.rs, state.rs, status.rs, transport.rs
                                 thin v1 public compatibility façades
```

Do not create directories solely to hold one trivial type, and do not split
files by arbitrary line-count targets. A module is split when its parts have
different reasons to change or dependency rules.

## Responsibility Migration

| Current source | Canonical Phase 1 destination |
| --- | --- |
| `avr.rs`, `response.rs` | `protocol/avr/command.rs` and `protocol/avr/response.rs` |
| `heos.rs` | `protocol/heos.rs` |
| `capabilities.rs` | `domain/capabilities.rs` |
| identity in `config.rs` | `domain/receiver.rs` |
| state plus status value types | `domain/main_zone.rs` |
| policy in `application.rs` and query orchestration in `status.rs` | focused `application` use cases |
| traits in `transport.rs` | application ports; concrete session implementation stays outside |
| YAML and filesystem code in `config.rs` | `infrastructure/config_yaml.rs` |
| `discovery.rs` | `infrastructure/discovery_ssdp.rs` |
| `session.rs` | `infrastructure/avr_session.rs` |
| socket code in `status.rs` | `infrastructure/tcp_avr.rs` |
| rendering and dispatch in `main.rs` | the CLI binary directory |

The listed root module names remain as thin public façades for v1 source
compatibility. They re-export or delegate only and contain no business rules,
parser implementations, socket loops, or duplicated state. The
`application` module exposes the canonical use cases and re-exports the
compatibility `ApplicationService`; canonical application submodules never
depend on that façade.

## Domain Model

The canonical main-zone model uses typed power, input, volume, mute, and
surround values. A partial field represents either a value or a typed
unavailable reason; it does not use unrelated optional value and error members
internally. Snapshots retain field independence plus freshness and authority.
Validated unsolicited events use the same value types and reducer as query
responses.

Receiver identity and model normalization belong to the domain. Serde derives
and YAML field names belong to an infrastructure DTO that converts to and from
the domain identity. Capability declarations remain evidence-bound and do not
gain new claims during the move.

The existing string-based `StatusField` and `MainZoneStatus` shapes remain at
the v1 compatibility boundary. Conversions from the canonical typed snapshot
must preserve the established rendered values and errors.

## Application Ports and Use Cases

Application ports describe required behavior rather than technologies:

- `ReceiverConfigRepository`: load and save receiver identity;
- `ReceiverDiscovery`: return discovered receiver candidates within a supplied
  bound;
- `MainZoneStatusGateway`: query one semantic main-zone field and report the
  connection generation needed to establish authority.

Receiver selection is one use case with fixed precedence: explicit manual host,
usable saved identity when no discovery index is requested, then bounded
discovery and explicit selection when ambiguous. Status refresh is a separate
use case that queries the five semantic fields independently and repeats the
authoritative snapshot if its connection generation changes during the query.

Ports return structured application errors containing operation category and
context. Infrastructure-specific error types are mapped at adapter boundaries.
Presentation and old string-returning APIs perform the final human-readable
conversion.

## Infrastructure and Composition

YAML configuration retains the existing relative default path and serialized
shape in Phase 1. The platform-native path and import policy remain Phase 2
work. SSDP constants, interface enumeration, fallback scanning, worker threads,
and XML probing stay entirely inside the discovery adapter.

The Tokio session retains one reader, serialized writes, bounded CR framing,
unsolicited-event routing, finite reconnect, and connection generation. The
synchronous TCP adapter remains available only to support the established v1
surface. Both adapters reuse the same protocol parsing and correlation code.

The CLI binary is the composition root for concrete defaults. Argument parsing
produces presentation commands, application use cases return typed results, and
the renderer owns labels, usage text, and exit-facing error strings. No process
exit, printing, or argument parsing is allowed in library layers.

## Compatibility Policy

- Preserve existing crate-root exports and public module import paths through
  the root façade modules listed in the target layout.
- Preserve public v1 data shapes, deprecated annotations, CLI commands, output,
  exit codes, YAML shape, relative default path, and network timeouts.
- Add canonical layered APIs without requiring current callers to migrate in
  Phase 1.
- Test public façades as adapters; test behavior exhaustively at the canonical
  layer rather than duplicating identical test cases in both locations.
- Do not retain an old implementation after its façade delegates to the new
  implementation.

## Verification Strategy

Domain tests cover typed values, partial fields, event reduction, invalidation,
and authority. Protocol tests cover AVR/HEOS framing, volume encoding, response
correlation, typed parsing, malformed input, and unknown lines. Application
tests use fakes for selection precedence, ambiguity, partial status, reconnect
generation changes, and error mapping. Infrastructure tests retain deterministic
parser tests and local TCP servers. CLI integration tests pin current help,
status rendering, argument errors, and exit codes.

A source search is part of acceptance: domain modules and canonical application
submodules must contain no imports from infrastructure, Tokio, serde,
filesystem, networking, binary modules, or the `ApplicationService`
compatibility façade. The full format, build, unit, integration, Clippy, and
diff checks must pass with no live receiver dependency. Repository maps in
`AGENTS.md`, `ARCHITECTURE.md`, and development documentation must describe the
implemented layout rather than the former flat files.
