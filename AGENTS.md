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
- `crates/infrastructure`: concrete TCP, HTTP, YAML, and SSDP adapters.
- `crates/gui-lib`: Iced presentation state, views, and the bridge to the
  control-service port; it must not import protocol or infrastructure.
- `apps/cli`, `apps/desktop`, and `apps/diagnostics`: delivery and composition
  packages. Diagnostics are separate and read-only.

## Required reading

Start with the [documentation map](docs/README.md). The current architectural
source of truth is [ARCHITECTURE.md](ARCHITECTURE.md); engineering rules are in
[docs/contributing.md](docs/contributing.md), and commands are in
[docs/development.md](docs/development.md).

Keep dependencies directed inward: domain ← policy ← application ← infrastructure,
domain ← application, and domain ← protocol ← infrastructure. The canonical receiver-session contract is
defined once in `crates/application/src/session_v3.rs` and is owned by the
control service. Historical plans and validation records live in
`docs/archive/`; they are not active guidance.
