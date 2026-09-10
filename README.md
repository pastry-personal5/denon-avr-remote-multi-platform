# Denon AVR Remote

Rust library, CLI, and desktop client for local Denon and Marantz AVR control.
Main Zone behavior is validated on the AVR-X3800H; unvalidated models remain
read-only.

```text
make run ARGS="get status --host <receiver-ip>"
make run-gui
make check
```

The CLI reads receivers, capabilities, and Main Zone status; validated models
also support `set power|input|volume|mute|surround`. Receiver-backed commands
accept `--host HOST` or `--receiver N`.

See the [CLI guide](docs/cli-user-guide.md), [desktop guide](docs/desktop-user-guide.md), [development guide](docs/development.md),
[contributing guide](docs/contributing.md), [architecture](ARCHITECTURE.md),
the [V3 Phase 1 probe-reliability plan](docs/v3/phase-1-x3800h-http-info-probe-overview.md),
and the [V3 Phase 3 architecture notes](docs/v3/phase-3-refactoring-overview.md).
