# Contributor Guide

## Repository map

- `src/lib.rs`: public library API.
- `src/avr.rs`, `src/heos.rs`: transport-independent protocol primitives.
- `src/capabilities.rs`: model and evidence-bound capability declarations.
- `src/discovery.rs`, `src/config.rs`: discovery and saved identity.
- `src/application.rs`: application service and receiver-selection policy.
- `src/status.rs`: status domain types and synchronous compatibility transport.
- `src/session.rs`: persistent asynchronous AVR TCP session.
- `src/transport.rs`: runtime-neutral asynchronous transport contract.
- `src/response.rs`, `src/state.rs`: shared response handling and validated state reduction.
- `src/main.rs`: CLI presentation and command dispatch.
- `tests/`: end-to-end and CLI tests.
- `docs/archive/`: retired or superseded documentation.
- `docs/v1/`, `docs/v2/`: release-scoped plans, user guides, and architecture.
- `ARCHITECTURE.md`: current design, invariants, and architecture TODOs.

All project-plan documents belong under `docs/` and its scope-specific
subdirectories. Retired or superseded documentation may be preserved under
`docs/archive/`; archive files are not active phase documentation. Keep the
root README focused on orientation and usage.

See [`docs/development.md`](docs/development.md) for development commands and
[`docs/contributing.md`](docs/contributing.md) for contribution and engineering
rules.
