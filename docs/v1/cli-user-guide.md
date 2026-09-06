# Version 1 CLI User Guide

The `denon-avr-remote` CLI discovers Denon and Marantz receivers and displays
read-only main-zone status. The validated project target is the Denon
AVR-X3800H. The CLI does not currently change receiver state, control HEOS,
produce JSON, or promise compatibility with unvalidated models.

## Requirements

- Rust and Cargo, or a built `denon-avr-remote` executable.
- The computer and receiver on a reachable network.
- Network Control enabled when required by the receiver configuration.

Run commands from the project root during development with `cargo run --`.
Use `make run ARGS="..."` when preferred.

## Commands

```text
denon-avr-remote help
denon-avr-remote discover
denon-avr-remote status
denon-avr-remote status --receiver <number>
denon-avr-remote status --host <receiver-ip-or-host>
denon-avr-remote avr <command>
denon-avr-remote heos <command>
denon-avr-remote volume <db-tenths>
```

`help` prints the available commands. `avr`, `heos`, and `volume` are
development utilities for protocol framing or volume encoding; they are not
receiver control workflows.

## Discovering a receiver

Run `cargo run -- discover`. Discovery sends bounded SSDP searches on the
standard multicast endpoint and the Denon-documented compatibility port, then
may use a local AIOS description scan fallback. Results are numbered for the
current invocation and include available identity metadata. A zero-result
discovery does not prove that the receiver is absent; multicast filtering,
VLANs, firewalls, interface selection, and receiver settings can prevent
replies.

## Querying status

Run `cargo run -- status` for the normal path. The application first tries the
saved receiver identity in `config/denon-avr-remote.yaml`. If it is unavailable,
it falls back to discovery. With multiple discovered receivers, select one
explicitly using the displayed number:

```text
cargo run -- status --receiver 1
```

To bypass discovery and the saved identity, provide a host directly:

```text
cargo run -- status --host 192.0.2.10
```

Successful queries save the selected identity. Output reports power, input,
volume, mute, and surround mode independently. An unavailable field is
reported as unavailable rather than replaced with an invented value.

`--host` and `--receiver` cannot be combined. Receiver numbers are one-based
in the CLI. Invalid arguments or failed operations exit non-zero and print
usage guidance.

## Configuration

```yaml
receiver:
  host: 192.0.2.10
  model: AVR-X3800H
  friendly_name: Living room
```

`host` is required. Unknown fields and blank hosts are rejected. `model` and
`friendly_name` are descriptive metadata and do not enable capabilities.
Credentials are never stored. Saves use typed Serde YAML and a temporary-file
replacement flow.

## Troubleshooting

- If discovery returns no devices, check LAN reachability, multicast, and
  Network Control settings.
- Use `status --host <address>` when multicast is filtered or the subnet is
  unsuitable for the bounded discovery fallback.
- If only some fields are unavailable, the receiver may omit or delay those
  responses; other fields remain valid.
- See the [Phase 1 architecture](phase-1-architecture.md) and [Phase 2
  overview](phase-2-overview.md) for evidence and validation boundaries.
