# Architecture

The project uses layered architecture. Dependencies point downward, while domain and protocol code remain independent of the CLI, sockets, operating systems, and async runtimes.

```text
+--------------------------------------------------+
| Presentation: CLI (`src/main.rs`)                |
| Parses arguments and renders results/errors       |
+--------------------------------------------------+
| Application: device operations (planned)         |
| Coordinates commands, sessions, and state        |
+--------------------------------------------------+
| Domain / Protocol: `avr.rs`, `heos.rs`           |
| Commands, framing, parsing, volume codes         |
+--------------------------------------------------+
| Infrastructure: transports (planned)             |
| TCP sockets, buffering, reconnects, async I/O     |
+--------------------------------------------------+
| Operating system / network                        |
+--------------------------------------------------+
```

## Layer responsibilities

### Presentation

The CLI is a thin adapter. It converts command-line arguments into library calls and displays protocol results. It must not implement socket management or duplicate AVR/HEOS encoding rules.

### Application

This layer will provide user-facing operations such as selecting an input, changing volume, and querying state. It will coordinate protocol commands with a transport and handle request/event flow. It should depend on traits, not a concrete TCP or async implementation.

### Domain and protocol

The protocol layer is the stable core of the library:

- AVR commands use CR (`\\r`) framing.
- HEOS commands use CRLF (`\\r\\n`) framing.
- Wire formats are explicit and must not depend on host endianness.
- Model capabilities describe known facts and mark unverified behavior.

This layer should remain usable in tests without a receiver or network connection.

### Infrastructure

Transport implementations will provide TCP connections, buffered reads/writes, timeouts, reconnects, and unsolicited-event delivery. They translate I/O failures into library errors but do not decide device semantics.

## Dependency rules

1. Presentation may call application and domain APIs.
2. Application may use domain APIs and transport traits.
3. Domain/protocol code must not depend on CLI, sockets, or runtime-specific APIs.
4. Infrastructure may depend on domain-defined transport contracts.
5. Tests should prefer in-memory fakes and protocol fixtures over live hardware.

## Planned request flow

```text
CLI input
  -> application operation
  -> domain command construction
  -> transport write
  -> transport read/frame
  -> domain parsing
  -> application result
  -> CLI output
```

The current implementation contains the protocol layer and a small CLI; application and network-transport layers are intentionally still to be built.

