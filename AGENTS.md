# Contributor Guide

## Repository map

The canonical public API is organized under `src/domain`, `src/application`, `src/protocol`, and `src/infrastructure`. Legacy v1 façades have been removed.

- `src/lib.rs`: public library API.
- `src/bin/denon-avr-remote/main.rs`: CLI composition, presentation, and command dispatch.
- `tests/`: end-to-end and CLI tests.
- `docs/archive/`: retired or superseded documentation.
- `docs/`: user guides; `docs/v1/` and `docs/v2/`: release-scoped plans and architecture.
- `ARCHITECTURE.md`: current design, invariants, and architecture TODOs.

All project-plan documents belong under `docs/` and its scope-specific
subdirectories. Retired or superseded documentation may be preserved under
`docs/archive/`; archive files are not active phase documentation. Keep the
root README focused on orientation and usage.

See [`docs/development.md`](docs/development.md) for development commands and
[`docs/contributing.md`](docs/contributing.md) for contribution and engineering
rules.
