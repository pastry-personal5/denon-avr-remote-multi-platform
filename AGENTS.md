# Contributor Guide

## Repository map

- `crates/domain`: receiver concepts and state, independent of runtime, I/O,
  serialization, protocol, and presentation.
- `crates/protocol`: AVR, HEOS, and AppCommand wire shapes and parsers only.
- `crates/application`: use-case policy, synchronous/asynchronous ports, and
  the serialized receiver connection coordinator. It depends only on domain.
- `crates/infrastructure`: concrete TCP, HTTP, YAML, and SSDP adapters.
- `crates/gui-lib`: Iced presentation state, views, and the GUI-owned
  controller bridge; it must not import protocol or infrastructure.
- `apps/cli`, `apps/desktop`, and `apps/diagnostics`: delivery and composition
  packages. Diagnostics remain separate and read-only.
- `docs/v1/`, `docs/v2/`, and `docs/v3/`: active phase plans;
  `docs/archive/`: retired material.

Keep dependencies directed inward: domain ← application ← infrastructure and
domain ← protocol ← infrastructure. Keep plans under `docs/`, keep the root
README to orientation and usage, and do not treat archived material as active
guidance. See
[`docs/development.md`](docs/development.md) for commands and
[`docs/contributing.md`](docs/contributing.md) for engineering rules.
