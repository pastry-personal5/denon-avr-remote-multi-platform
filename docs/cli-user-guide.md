# CLI user guide

`denon-avr-remote` discovers local Denon and Marantz receivers, reads Main Zone
state, and submits Main Zone controls through the canonical async receiver
service. It does not control HEOS. Receiver outcomes are evidence-based: a
write acknowledgement is not reported as confirmed state; run a later `get`
command for authoritative receiver evidence.

## Requirements

- Rust and Cargo during development, or a built `denon-avr-remote` executable.
- A reachable receiver on the local network.
- Network Control enabled when the receiver configuration requires it.

From the repository root, run `make run ARGS="..."` or
`cargo run -p denon-avr-cli --bin denon-avr-remote -- ...`.

## Commands

```text
denon-avr-remote help
denon-avr-remote --version
denon-avr-remote get receivers
denon-avr-remote get status [--host HOST | --receiver N]
denon-avr-remote get power [--host HOST | --receiver N]
denon-avr-remote get input [--host HOST | --receiver N]
denon-avr-remote get volume [--host HOST | --receiver N]
denon-avr-remote get mute [--host HOST | --receiver N]
denon-avr-remote get surround [--host HOST | --receiver N]
denon-avr-remote set power on [--dry-run] [--host HOST | --receiver N]
denon-avr-remote set input CD [--dry-run] [--host HOST | --receiver N]
denon-avr-remote set volume -20.0 [--dry-run] [--host HOST | --receiver N]
denon-avr-remote set mute off [--dry-run] [--host HOST | --receiver N]
denon-avr-remote set surround STEREO [--dry-run] [--host HOST | --receiver N]
```

`level` aliases `volume`; `surround-mode` aliases `surround`. `get receivers`
does not accept a selector. `--host HOST` and one-based `--receiver N` are
mutually exclusive. Without either selector, a status or control command uses
the saved receiver; if none is saved, it fails rather than guessing.

The former `--resource-version` option has been removed: a local snapshot
counter cannot provide receiver-side compare-and-set semantics.

## Discovery and reads

`get receivers` performs bounded SSDP discovery and prints numbered results for
that invocation. A zero-result discovery does not prove the receiver is absent:
multicast filtering, VLANs, firewalls, interfaces, and receiver settings can
prevent replies.

Use a selector when reading a receiver:

```text
make run ARGS="get status --receiver 1"
make run ARGS="get power --host 192.0.2.10"
```

Each read connects, synchronizes through the canonical service, prints the
selected target and requested field or full state, reports readiness, then
closes the session. A degraded or unavailable field remains explicitly
represented; it is not replaced with a guessed value.

## Main Zone controls

`set` accepts one operation per invocation. Power and mute values are `on` or
`off`. Input and surround values are sent as the supplied receiver identifiers.
Volume accepts `min` or an exact dB value from `-79.5` through `+18.0` in
`0.5` dB steps. `min` is a distinct receiver value, not an alias for `-79.5`.

Before a live operation, the CLI synchronizes the selected receiver and submits
one operation through the canonical service. `--dry-run` validates the intent
and prints it without connecting or dispatching. A live result is printed as
the service outcome; only a subsequent receiver observation can confirm the
new state.

## Configuration and troubleshooting

Successful selected reads and writes save the selected identity as the current
receiver using the YAML configuration adapter. The CLI creates an ad-hoc
identity for `--host`; configured metadata is descriptive and does not itself
grant capabilities.

- Check LAN reachability, multicast, and Network Control if discovery finds no
  device.
- Use `--host <address>` when discovery is filtered or unsuitable.
- If a read is degraded or a field is unavailable, retry later and preserve the
  reported outcome when diagnosing the receiver.
- Consult the [architecture](../ARCHITECTURE.md) for the session and evidence
  model, and [research](research/) for supporting protocol evidence.
