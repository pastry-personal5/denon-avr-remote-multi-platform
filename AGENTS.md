# Contributor Guide

## Repository map

- `crates/domain`: receiver concepts and state; no runtime, I/O, serialization,
  protocol, or presentation dependencies.
- `crates/protocol`: AVR, HEOS, and AppCommand wire shapes and parsers.
- `crates/policy`: the policy engine, a pure function over the domain; it imports
  domain only and does no I/O.
- `crates/application`: use-case policy, ports, the control-service port, and the
  in-process control service that owns every receiver session; it imports domain
  and policy, never protocol or infrastructure.
- `crates/api-contract`: the Control API's wire types, route table, and error
  mapping; serde only, with no HTTP stack, runtime, or filesystem.
- `crates/api-client`: the control-service port over the Control API server's Unix
  socket; HTTP/1.1 only, with no TLS and no server framework.
- `crates/infrastructure`: concrete TCP, HTTP, YAML, and SSDP adapters.
- `crates/gui-lib`: Iced presentation state, views, and the bridge to the
  control-service port; it must not import protocol or infrastructure.
- `apps/api-server`: the Control API server; it hosts the in-process control service
  behind the Operator and Agent endpoints and is, with diagnostics, where receiver
  sessions are constructed.
- `apps/cli`, `apps/desktop`, and `apps/diagnostics`: delivery and composition
  packages. Diagnostics are separate and read-only.

## Required reading

Start with the [documentation map](docs/README.md). The current architectural
source of truth is [ARCHITECTURE.md](ARCHITECTURE.md); engineering rules are in
[docs/contributing.md](docs/contributing.md), and commands are in
[docs/development.md](docs/development.md).

Keep dependencies directed inward: domain ← policy ← application ← infrastructure,
domain ← application, and domain ← protocol ← infrastructure; application ←
api-contract ← api-client and api-server. The canonical receiver-session contract is
defined once in `crates/application/src/session_v3.rs` and is owned by the
control service. Historical plans and validation records live in
`docs/archive/`; they are not active guidance.
