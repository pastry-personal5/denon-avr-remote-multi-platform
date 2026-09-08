# Denon AVR Remote

Rust library and CLI for read-only Denon and Marantz AVR status over local IP.
The validated target is the Denon AVR-X3800H.

Quick start:

```text
cargo run -- help
cargo run -- get receivers
cargo run -- get status --host <receiver-ip>
make check
make clippy
```

The CLI uses Kubernetes-style read-only resources: `get receivers` discovers
receivers, while `get status`, `get power`, `get input`, `get volume`, `get
mute`, and `get surround` query the main zone. Receiver-backed resources accept
`--host HOST` or a one-based `--receiver N` selector.

Documentation:

- [Development guide](docs/development.md)
- [Contributing guide](docs/contributing.md)
- [Architecture](ARCHITECTURE.md)
- [CLI user guide](docs/cli-user-guide.md)
- [Version 1 documentation](docs/v1/phase-1-overview.md)
- [Version 1.0.0 release notes](docs/v1/RELEASE_NOTES.md)
- [Version 2 Phase 1 architecture refactor](docs/v2/phase-1-foundation-overview.md)
- [Version 2 Phase 2 plan](docs/v2/phase-2-gui-overview.md)
