# Contributor Guide

## Repository map

- `src/lib.rs`: public Rust library API.
- `src/domain`, `src/application`, `src/protocol`, `src/infrastructure`: the
  canonical layers; keep dependencies directed inward.
- `src/gui.rs` and `src/bin/`: desktop GUI and CLI presentation/composition.
- `tests/`: end-to-end and CLI tests.
- `docs/v1/` and `docs/v2/`: active phase plans; `docs/archive/`: retired material.

Keep plans under `docs/`, keep the root README to orientation and usage, and
do not treat archived material as active guidance. See
[`docs/development.md`](docs/development.md) for commands and
[`docs/contributing.md`](docs/contributing.md) for engineering rules.
