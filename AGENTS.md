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
- `docs/research/`: protocol evidence and validation notes.
- `docs/v1/`, `docs/v2/`: release-scoped plans, reports, and follow-up work.
- `ARCHITECTURE.md`: current design, invariants, and architecture TODOs.

All project-plan documents belong under `docs/` and its scope-specific
subdirectories. Retired or superseded documentation may be preserved under
`docs/archive/`; archive files are not active phase documentation. Keep the
root README focused on orientation and usage.

## Development

Use the Makefile targets where possible:

```text
make check
make test
make clippy
make run ARGS="help"
make run-release ARGS="help"
```

Preserve these rules:

- Keep AVR and HEOS wire formats separate and explicit.
- Keep protocol parsing independent of sockets, operating systems, and async
  runtimes.
- Serialize writes on persistent sessions and route unsolicited lines as
  events.
- Treat partial status as valid; never invent unavailable receiver data.
- Do not claim receiver capabilities without protocol evidence or live
  validation.
- Add protocol and transport tests for every behavior change.
- Prefer `ApplicationService` and `AsyncAvrTransport`; direct status transport
  APIs are retained only as deprecated compatibility wrappers.
- Do not modify `.codex-firewall-hardening.ps1`; it is unrelated workspace
  material.
