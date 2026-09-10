# Development

## Prerequisites

- Rust stable and Cargo.
- `rg` and `jq` for the architecture boundary checks.
- A reachable Denon or Marantz receiver for optional live validation.
- Windows, macOS, or Linux with a working local network interface.

## Common commands

Run these from the repository root:

```text
make check          # format, boundaries, compile, and tests
make boundary      # verify workspace package graph and source boundaries
make clippy         # Clippy with warnings denied
make format         # format Rust sources
make test           # run all tests
make run ARGS="help"
make run-gui          # run the native Iced desktop GUI
make run-diagnostics ARGS="HOST TIMEOUT-MS MODEL FIRMWARE"
                      # run the read-only Telnet diagnostics probe
make run-diagnostics-http ARGS="HOST TIMEOUT-MS MODEL FIRMWARE [PORT]"
                      # run the read-only HTTP AppCommand evidence probe
cargo run -p denon-avr-diagnostics --bin source-catalog-probe -- \
  http://HOST:8080/goform/AppCommand.xml TIMEOUT-MS MODEL FIRMWARE SCENARIO
                      # diagnostic-only candidate source-catalog read; no writes
cargo run -p denon-avr-diagnostics --bin quick-select-name-probe -- \
  http://HOST:PORT TIMEOUT-MS MODEL FIRMWARE SCENARIO
                      # four fixed XML GETs plus receiver-advertised name read; no writes
make capture-visual-baselines CAPTURES=target/visual-captures
                      # captures every deterministic scenario at 100% and 200%, then exits
make visual-baselines PLATFORM=linux CAPTURES=target/visual-captures/linux
                      # compare all captured PNGs to committed Linux baselines
```

Equivalent Cargo commands are documented by the Makefile. Do not run live
receiver validation as part of ordinary unit-test work.

Contribution rules and engineering invariants are maintained in the
[contributing guide](contributing.md).

## Workspace architecture

The active implementation is entirely under `crates/` and `apps/`; a root
`src/` tree is intentionally absent. `domain` is runtime- and I/O-free.
`protocol` owns wire parsing, `application` owns ports and use-case policy,
and `infrastructure` owns concrete adapters. GUI, CLI, desktop composition,
and read-only diagnostics are delivery packages. Run `make boundary` after
moving code or changing a package dependency.

## Documentation

Active version documentation is under `docs/v1/`, `docs/v2/`, and `docs/v3/`. Put retired
material under `docs/archive/`. Phase directories are not used; phase numbers belong in filenames. Keep the root `README.md` concise and link to detailed guides from there.

Before submitting documentation changes, check links and run:

```text
git diff --check
```
