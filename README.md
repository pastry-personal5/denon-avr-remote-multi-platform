# Denon AVR Remote

Rust library and CLI for communicating with Denon and Marantz AV receivers
over the local IP protocols.
The validated target is the Denon AVR-X3800H.

Current capabilities:

- AVR ASCII commands and CR framing over TCP 23
- HEOS command construction with CRLF framing
- SSDP discovery with AIOS-description fallback
- saved receiver identity and one-shot main-zone status
- persistent asynchronous AVR sessions with events and bounded reconnects
- capability declarations that distinguish evidence from assumptions

Quick start:

```text
cargo run -- help
cargo run -- discover
cargo run -- status --host <receiver-ip>
make check
make clippy
```

Documentation:

- [Architecture](ARCHITECTURE.md)
- [Contributor guide](AGENTS.md)
- [Documentation index](docs/README.md)
- [v1.0.0 scope and plans](docs/v1/README.md)
- [Protocol research](docs/research/denon-avr-ip-protocol.md)
- [v2.0.0 follow-up areas](docs/v2/README.md)

Receiver-specific behavior is claimed only when supported by protocol evidence
or live validation.
