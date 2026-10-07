# CLI user guide

`denon-avr-remote` discovers local Denon and Marantz receivers, reads Main Zone
state, and submits Main Zone controls through the in-process control service.
It does not control HEOS. Receiver outcomes are evidence-based: a write
acknowledgement is not reported as confirmed state, and a result says
separately whether anything was dispatched and whether receiver evidence
confirms the requested value.

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

Each read connects, synchronizes through the control service, prints the
selected target and requested field or full state, reports readiness, then
closes the session so the receiver's single control connection is free for
other tools. A degraded or unavailable field remains explicitly represented;
it is not replaced with a guessed value.

## Main Zone controls

`set` accepts one operation per invocation. Power and mute values are `on` or
`off`. Input and surround values are sent as the supplied receiver identifiers.
Volume accepts `min` or an exact dB value from `-79.5` through `+18.0` in
`0.5` dB steps. `min` is a distinct receiver value, not an alias for `-79.5`.

A live operation is submitted to the control service, which allocates its id,
connects to the receiver and dispatches at most once. The CLI waits until the
operation is finished and prints its result:

```text
Outcome: completed
Dispatch: complete_write
Confirmed: true
Observation: post-dispatch receiver observation
```

`Outcome` is the operation's status (`completed`, `already_in_state`,
`rejected`, `cancelled`, `superseded`, or `indeterminate`). `Dispatch` says
whether a command was sent: `not_dispatched`, `possibly_dispatched`,
`complete_write` (the local write completed, which is not an acknowledgement
by the receiver), or `unknown`. `Confirmed` is `true` only when receiver
evidence shows the requested value. A `rejected` outcome always says
`not_dispatched` and gives a `Reason`; an `indeterminate` one says what is
known. The exit status is 0 whenever the operation finished, whatever its
outcome, so scripts should read `Confirmed`.

`--dry-run` validates the intent and prints it without connecting or
dispatching.

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
