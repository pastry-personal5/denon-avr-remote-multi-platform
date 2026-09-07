# Version 2 - Phase 1 Overview

## Objective

Reorganize the v1 codebase into a clean layered architecture before adding the
Version 2 GUI. Improve cohesion, naming, dependency direction, error handling,
and testability without changing receiver behavior, CLI output, configuration
semantics, or supported capabilities.

## Scope

Phase 1 separates the crate into domain, application, protocol, infrastructure,
and presentation layers. Existing flat source files may be renamed or split
when they currently mix responsibilities. Internal code moves to typed domain
values and errors, application use cases depend on ports rather than concrete
filesystem or network implementations, and the CLI becomes a composition and
presentation boundary.

Legacy v1 façade modules are intentionally removed; callers use the layered
modules directly. Canonical implementation code must not depend on a
presentation or infrastructure API.

This is a behavior-preserving refactor. The following remain out of scope:

- GUI implementation and platform-native configuration migration;
- receiver control commands or execute-once command semantics;
- new discovery behavior, status fields, zones, HEOS features, or model claims;
- removal of canonical APIs or changes to the Kubernetes-style CLI contract;
- packaging, observability, and session lifecycle features planned for later
  phases.

## Key Deliverables

- A documented source tree with enforced inward dependency direction.
- Pure domain types for receiver identity, capabilities, main-zone values,
  partial snapshots, freshness, authority, and typed events.
- Application ports for configuration, discovery, and main-zone status access,
  plus focused receiver-selection and status-query use cases.
- Transport-independent AVR and HEOS protocol modules separated from Tokio,
  sockets, filesystem access, CLI rendering, and operating-system discovery.
- Infrastructure adapters for YAML configuration, SSDP discovery, persistent
  AVR sessions, and the bounded synchronous TCP path.
- A CLI composition root with parsing, rendering, and process exit behavior
  separated from application policy.
- One canonical main-zone field plan, parser set, and reducer shared by the
  asynchronous use case and synchronous TCP adapter.
- Structured internal errors with operation context; string conversion occurs
  only at the presentation boundary.

## Acceptance Criteria

- Domain and application modules do not import Tokio, sockets, filesystem APIs,
  serde/YAML, SSDP implementation details, or CLI code.
- Application use cases are constructible with deterministic fake ports and do
  not call concrete configuration, discovery, or transport functions directly.
- Protocol modules remain usable without a network connection or async runtime.
- Infrastructure implements application ports and contains all concrete I/O.
- Internal imports use the layered modules directly.
- The five-field definition, parsing, and reduction are implemented once and
  still preserve independent field failures and reconnect authority rules.
- Existing crate-root exports, documented module imports, CLI output, config
  shape/path, command framing, and timeout behavior continue to pass regression
  tests.
- `make check`, `make clippy`, and `git diff --check` pass after obsolete flat
  implementations are removed.

## Implementation Order

1. Introduce the target modules and pure typed domain model.
2. Move AVR/HEOS framing and parsing into the protocol layer.
3. Define application ports and move selection/status orchestration into use
   cases with fake-port tests.
4. Move YAML, SSDP, TCP, and Tokio implementations behind those ports.
5. Move CLI dispatch and rendering into the binary presentation boundary.
6. Remove the obsolete root modules and redirect all callers to
   callers to canonical code, and remove superseded implementations and duplicate
   tests.
7. Update `AGENTS.md`, `ARCHITECTURE.md`, and development documentation to the
   implemented source map and commands.
