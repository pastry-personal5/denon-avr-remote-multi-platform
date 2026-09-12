# Development

## Prerequisites

- Rust stable and Cargo.
- `rg` and `jq` for architecture-boundary checks.
- A reachable Denon or Marantz receiver only for optional live validation.
- Windows, macOS, or Linux with a usable local network interface.

## Common commands

Run these from the repository root:

```text
make check          # formatting, boundaries, deterministic checks, compile, tests
make format-check   # verify Rust formatting without changing files
make boundary       # verify workspace package graph and source boundaries
make clippy         # Clippy with warnings denied
make format         # format Rust sources
make test           # run all tests
make run ARGS="help"
make run-gui        # run the native Iced desktop GUI
make run-diagnostics ARGS="HOST TIMEOUT-MS MODEL FIRMWARE"
make run-diagnostics-http ARGS="HOST TIMEOUT-MS MODEL FIRMWARE [PORT]"
make package-macos  # build the unsigned Apple Silicon .app and .dmg
```

Additional read-only diagnostics and visual-regression commands are documented
in the [Makefile](../Makefile). Do not run live validation in ordinary unit-test
work. `make test-live-x3800h` is opt-in; state-restoring live controls also
require `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and a safe-volume value.

On an Apple Silicon Mac, `make package-macos` writes the unsigned app and DMG
under `target/release/macos/`. Gatekeeper may require right-clicking the app
and choosing Open on its first launch.

## Verification gates

Run `make format-check`, `make boundary`, and `git diff --check` for every
change. Run `make check` for implementation changes; run `make clippy` before
submission when Rust code changes. `make boundary` must also be run after moving
code or changing a package dependency.

## Documentation locations

Use the [documentation map](README.md) to find current guidance.
`ARCHITECTURE.md` is the current architectural reference; `contributing.md`
contains engineering policy; this document contains commands and gates.
Research in `docs/research/` is supporting evidence. Completed material is in
`docs/archive/` and is not active guidance.
