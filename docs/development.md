# Development

## Prerequisites

- Rust stable and Cargo.
- A reachable Denon or Marantz receiver for optional live validation.
- Windows, macOS, or Linux with a working local network interface.

## Common commands

Run these from the repository root:

```text
make check          # format, boundaries, compile, and tests
make boundary      # verify domain/application/protocol dependency boundaries
make clippy         # Clippy with warnings denied
make format         # format Rust sources
make test           # run all tests
make run ARGS="help"
make run-gui          # run the native Iced desktop GUI
make run-diagnostics ARGS="HOST TIMEOUT-MS MODEL FIRMWARE"
                      # run the read-only Telnet diagnostics probe
```

Equivalent Cargo commands are documented by the Makefile. Do not run live
receiver validation as part of ordinary unit-test work.

Contribution rules and engineering invariants are maintained in the
[contributing guide](contributing.md).

## Documentation

Active version documentation is under `docs/v1/` and `docs/v2/`. Put retired
material under `docs/archive/`. Phase directories are not used; phase numbers belong in filenames. Keep the root `README.md` concise and link to detailed guides from there.

Before submitting documentation changes, check links and run:

```text
git diff --check
```
