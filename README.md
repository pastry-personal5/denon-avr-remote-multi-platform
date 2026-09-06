# Denon AVR Remote

Rust library for controlling Denon and Marantz AV receivers over their local IP protocols.

The initial target is the Denon AVR-X3800H. The project keeps the low-latency AVR ASCII protocol (TCP 23) separate from the HEOS CLI (TCP 1255). Receiver-specific capabilities remain explicitly unverified until tested against live hardware.

## Current implementation

- AVR command construction with CR framing
- HEOS command construction with CRLF framing
- Basic line classification primitives
- Reference volume-code handling
- AVR-X3800H capability placeholder
- SSDP receiver discovery on UDP 1900, plus a validated AIOS-description scan
  fallback for receivers that do not answer M-SEARCH
- Saved receiver identity in `config/denon-avr-remote.yaml`
- One-shot main-zone status over TCP 23 with partial-field output
- Unit tests for protocol invariants

The next layer is the persistent async TCP session: serialized writes, bounded
reads, unsolicited event routing, reconnects, and structured transport errors.

## Development

```text
cargo test
```

See [`docs/research/denon-avr-ip-protocol.md`](docs/research/denon-avr-ip-protocol.md) for protocol evidence and the AVR-X3800H validation checklist.
