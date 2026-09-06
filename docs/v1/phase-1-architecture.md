# Version 1 - Phase 1 Architecture

The Phase 1 vertical slice separates AVR and HEOS wire formats, keeps protocol
parsing independent from sockets and runtimes, and places discovery,
configuration, and synchronous TCP at the infrastructure edge. The CLI is a
thin presentation and dispatch layer.

AVR commands are ASCII and CR-terminated over TCP 23. Status fields are
queried independently and correlated by command family; timeouts,
disconnects, malformed responses, and unsupported fields remain distinct from
valid values. The five initial fields are power, input, master volume, mute,
and surround mode.

The protocol reference is evidence for command shape, not a universal model
capability claim. X3800H behavior is validated separately, including volume
encoding, standby reachability, event delivery, and exposed services.

## Key interfaces

- `avr.rs`: transport-independent command framing and protocol primitives.
- `discovery.rs`: interface-aware SSDP and bounded description scanning.
- `config.rs`: persisted receiver identity without credentials.
- `status.rs`: one-shot status domain and compatibility TCP adapter.
- `main.rs`: argument parsing, exit codes, and human-readable presentation.

The design treats status as a partial snapshot and never invents unavailable
receiver data. HEOS remains a separate CRLF/JSON protocol boundary.

## Protocol evidence boundary

The Denon reference protocol documents half-duplex TCP 23, ASCII commands,
CR framing, a bounded message length, request responses, and unsolicited
events. It is an evidence source for command shape, not proof that every
receiver exposes every command. The project therefore records live model and
firmware validation before enabling capabilities.

Master volume uses receiver-specific numeric encoding: `80` represents 0 dB,
whole-dB values use two digits, and half-dB values append `5`. Input, mute,
surround, signal mode, channel volume, and additional zones remain separately
validated. HEOS uses a different CRLF/JSON protocol on its own boundary and
must not be folded into the AVR parser.
