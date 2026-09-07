# CLI User Guide

The `denon-avr-remote` CLI discovers Denon and Marantz receivers and displays
read-only main-zone status. The validated project target is the Denon
AVR-X3800H. The CLI does not change receiver state, control HEOS, or promise
compatibility with unvalidated models.

## Requirements

- Rust and Cargo, or a built `denon-avr-remote` executable.
- The computer and receiver on a reachable network.
- Network Control enabled when required by the receiver configuration.

Run commands from the project root during development with `cargo run --`.
Use `make run ARGS="..."` when preferred.

## Commands

```text
denon-avr-remote help
denon-avr-remote get receivers
denon-avr-remote get status
denon-avr-remote get status --receiver <number>
denon-avr-remote get status --host <receiver-ip-or-host>
denon-avr-remote get power [--host HOST | --receiver N]
denon-avr-remote get input [--host HOST | --receiver N]
denon-avr-remote get volume [--host HOST | --receiver N]
denon-avr-remote get mute [--host HOST | --receiver N]
denon-avr-remote get surround [--host HOST | --receiver N]
```

`surround-mode` is accepted as an alias for `surround`. `get receivers` does
not accept receiver selectors. All other resources use the saved receiver when
possible, fall back to discovery when necessary, and accept either `--host`
or `--receiver N`; the two selectors cannot be combined.

## Discovering receivers

Run `cargo run -- get receivers`. Discovery sends bounded SSDP searches on the
standard multicast endpoint and the Denon-documented compatibility port, then
may use a local AIOS description scan fallback. Results are numbered for the
current invocation and include available identity metadata. A zero-result
discovery does not prove that a receiver is absent; multicast filtering,
VLANs, firewalls, interface selection, and receiver settings can prevent
replies.

## Querying status and fields

Run `cargo run -- get status` for the normal path. With multiple discovered
receivers, select one explicitly using the displayed one-based number:

```text
cargo run -- get status --receiver 1
```

To bypass discovery and the saved identity, provide a host directly:

```text
cargo run -- get status --host 192.0.2.10
cargo run -- get power --host 192.0.2.10
```

`get status` preserves the complete status output, reporting power, input,
volume, mute, and surround mode independently. The individual resources print
the corresponding line using the same labels and value formatting. An
unavailable field is reported as unavailable rather than replaced with an
invented value.

Invalid arguments or failed operations exit non-zero and print usage guidance.

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
- Use `get status --host <address>` when multicast is filtered or the subnet is
  unsuitable for the bounded discovery fallback.
- If only some fields are unavailable, the receiver may omit or delay those
  responses; other fields remain valid.
- See the [Phase 1 architecture](v1/phase-1-architecture.md) and [Phase 2
overview](v2/phase-2-overview.md) for evidence and validation boundaries.
