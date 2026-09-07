# Denon AVR Remote

Rust library and CLI for read-only Denon and Marantz AVR status over local IP.
The validated target is the Denon AVR-X3800H.

Quick start:

```text
cargo run -- help
cargo run -- discover
cargo run -- status --host <receiver-ip>
make check
make clippy
```

Documentation:

- [Development guide](docs/development.md)
- [Contributing guide](docs/contributing.md)
- [Architecture](ARCHITECTURE.md)
- [CLI user guide](docs/v1/cli-user-guide.md)
- [Version 1 documentation](docs/v1/phase-1-overview.md)
- [Version 1.0.0 release notes](docs/v1/RELEASE_NOTES.md)
- [Future GUI guide](docs/v2/gui-user-guide.md)
