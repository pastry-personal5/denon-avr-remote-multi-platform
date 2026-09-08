# CLI User Guide

The `denon-avr-remote` CLI discovers Denon and Marantz receivers, displays
Main Zone status, and provides capability-gated controls for the validated
Denon AVR-X3800H. State-changing commands dispatch once and report that they
are not confirmed; run a later status command to verify the receiver state.
The CLI does not control HEOS or promise compatibility with unvalidated models.

## Requirements

- Rust and Cargo, or a built `denon-avr-remote` executable.
- The computer and receiver on a reachable network.
- Network Control enabled when required by the receiver configuration.

Run commands from the project root during development with `cargo run --`.
Use `make run ARGS="..."` when preferred.

## Commands

```text
denon-avr-remote help
denon-avr-remote --version
denon-avr-remote get receivers
denon-avr-remote get capabilities [--host HOST | --receiver N]
denon-avr-remote get status
denon-avr-remote get status --receiver <number>
denon-avr-remote get status --host <receiver-ip-or-host>
denon-avr-remote get power [--host HOST | --receiver N]
denon-avr-remote get input [--host HOST | --receiver N]
denon-avr-remote get volume [--host HOST | --receiver N]
denon-avr-remote get mute [--host HOST | --receiver N]
denon-avr-remote get surround [--host HOST | --receiver N]
denon-avr-remote get level [--host HOST | --receiver N]
denon-avr-remote set power on [--resource-version N] [--host HOST | --receiver N]
denon-avr-remote set input CD [--resource-version N] [--host HOST | --receiver N]
denon-avr-remote set volume 50.0 [--resource-version N] [--host HOST | --receiver N]
denon-avr-remote set level 50.0 [--resource-version N] [--host HOST | --receiver N]
denon-avr-remote set mute off [--resource-version N] [--host HOST | --receiver N]
denon-avr-remote set surround STEREO [--resource-version N] [--host HOST | --receiver N]
```

`level` is an alias for `volume`, and `surround-mode` is an alias for
`surround`. Power accepts `off` as an alias for `standby`. `get receivers` does
not accept receiver selectors. Read resources use the saved receiver when
possible and fall back to discovery; writes use the saved receiver only unless
`--host` or `--receiver N` is supplied. The two selectors cannot be combined.

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
volume level, mute, and surround mode independently. The individual resources
print the corresponding line using the same labels and value formatting. An
unavailable field is reported as unavailable rather than replaced with an
invented value. Every successful read prints the receiver target and snapshot
resource version; use that version with `set`.

`get capabilities` lists the validated writable operations and exact input and
surround values. An unknown model returns a successful read-only report with
no writable capabilities.

## Controlling the Main Zone

The `set` command accepts one operation per invocation. Power values are `on`
and `off` (`standby` is also accepted); mute values are `on` and `off`.
Volume level accepts 0.0 through 100.0 in 0.5 steps, and `level` is an alias
for the volume operation. Input and surround values must exactly match the
allowlist shown by `get capabilities`.

Before dispatch, `set` performs a fresh status preflight. If
`--resource-version` is supplied, it compares it with the live version; when
omitted, the live preflight version is used automatically. A mismatch,
unavailable preflight,
unsupported model, or invalid value prevents all state-changing traffic. A
matching value is reported as a no-op. `--dry-run` validates and prints the
planned receiver action without dispatching it. A live command sends exactly
once and prints that it is unconfirmed;
run `get status` to check the resulting state.

Results are printed on stdout and diagnostics on stderr. Exit status `0` means
success; parse, validation, receiver, stale-version, transport, and unsupported
outcomes return exit status `2` and print usage guidance.

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
- See the [Phase 1 architecture](v1/phase-1-architecture.md), [Phase 2
overview](v2/phase-2-gui-overview.md), and [Phase 3 overview](v2/phase-3-main-zone-controls-overview.md)
for evidence and validation boundaries.
# Controller-backed operation model

The CLI uses the application controller’s typed admission policy for receiver
selection, status reads, and Main Zone controls. A `set` command performs one
validated dispatch only; it does not claim that the receiver has accepted the
new value. Run the corresponding `get` command to obtain authoritative
confirmation. Resource versions are selected automatically from the preflight
snapshot unless `--resource-version` is supplied.
