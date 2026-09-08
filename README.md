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

See the [CLI guide](docs/cli-user-guide.md), [development guide](docs/development.md),
[contributing guide](docs/contributing.md), [architecture](ARCHITECTURE.md),
and [V2 roadmap](docs/v2/phase-10-visual-identity-overview.md).
