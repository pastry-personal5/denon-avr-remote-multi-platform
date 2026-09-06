# v1 Phase 1: Discovered AVR Status CLI

Status: implemented one-shot path with live-hardware validation pending. The
CLI, SSDP discovery, saved identity, bounded TCP status query, and partial
status rendering are implemented. Receiver-specific compatibility remains
subject to live validation.

## User-facing commands

The intended one-shot commands are:

```text
denon-avr-remote discover
denon-avr-remote status
denon-avr-remote status --receiver <number>
denon-avr-remote status --host <ip>
```

`discover` sends an SSDP M-SEARCH on UDP 1900 (plus a UDP 1800 compatibility
probe), presents deduplicated results
with a stable number for the invocation, and prints enough identity data to
choose a receiver. `status` first tries the saved identity. If there is no
usable saved identity, or the saved receiver cannot be reached, it performs
discovery and uses the selected result. The framing commands documented by the
bootstrap CLI (`avr`, `heos`, `volume`, and `help`) remain available as
development tools. `status --host <ip>` is an explicit manual-IP path for
networks where SSDP multicast is filtered; it uses TCP 23 directly and stores
the successful identity for subsequent `status` commands.

The status output is human-readable and stable for people, but is not a v1
machine-readable contract. JSON output is deferred.

## Discovery and selection

Discovery uses the standard SSDP multicast endpoint on UDP 1900 and also probes
the UDP 1800 endpoint listed by Denon's exposed-services reference. Both probes
use the Denon/HEOS search target documented in the research file. The
implementation must:

1. bind an appropriate local interface or interfaces;
2. send the search with a bounded discovery timeout;
3. parse response headers without assuming their order or casing;
4. deduplicate equivalent address/identity results; and
5. show a numbered list when more than one receiver is found.

A zero-result discovery is not proof that the receiver is absent: multicast
filtering, VLAN boundaries, host firewalls, and the receiver's Network Control
setting can all suppress replies.

On multi-homed systems, including Windows hosts with WSL or Hyper-V adapters,
the implementation binds and probes each private IPv4 interface. Relying on the
operating system's single default multicast route can send M-SEARCH on a
virtual network that cannot reach the receiver.

If SSDP produces no Denon-family responses, Phase 1 performs a bounded scan of
the local IPv4 `/24` for the Denon/Marantz AIOS description endpoint on TCP
60006. Candidates are accepted only when the XML description identifies Denon
or Marantz. This fallback is intended for ordinary home `/24` LANs; manual IP
remains the deterministic path for other subnet layouts.

The selected number is local to the command invocation. A saved identity is
preferred on later runs, so discovery is a fallback rather than a requirement
for every status query. Discovery proves that a device answered SSDP; it does
not prove that TCP 23 is enabled or that every status field is supported.

## Identity configuration

The planned file is `config/denon-avr-remote.yaml`. It is user configuration,
not a protocol fixture or a capability claim. The initial shape is expected to
contain one receiver identity, for example:

```yaml
receiver:
  host: 192.0.2.10
  model: AVR-X3800H
  friendly_name: Living room
```

`host` is the address used for TCP 23. `model` and `friendly_name` are
identification metadata and must not enable commands by themselves. The
implementation may add a persisted discovery identifier when needed, but must
keep the file forward-compatible and avoid storing credentials. Invalid or
unreadable configuration should produce a clear error and allow the documented
discovery fallback where safe.

## Transport boundaries and timeouts

The AVR protocol module remains transport-independent: it constructs CR-
terminated commands and parses CR-delimited lines. Phase 1 uses a synchronous
transport adapter for its bounded one-shot operation; the `AvrTransport` trait
keeps the application boundary replaceable by a Tokio adapter later without
putting Tokio types into `avr.rs` or other domain modules.

The Phase 1 defaults are a 2-second TCP connection timeout and a
1-second read timeout for each status query. They should be configurable at the
application boundary, applied independently to each operation, and included in
transport errors. These are implementation defaults, not receiver guarantees.

## Status query and partial results

After connecting to TCP 23, the status operation queries the main-zone fields
in this order:

1. power (`PW?` or `ZM?`, once the X3800H choice is validated);
2. input (`SI?`);
3. master volume (`MV?`);
4. mute (`MU?`); and
5. surround mode (`MS?`).

The exact power command and response forms require live X3800H validation. The
reference command names are evidence for the shape of the query, not a claim
that every model exposes identical behavior.

Each response is framed at CR and associated with its request. A timeout,
disconnect, malformed response, or unsupported field marks that field
unavailable and preserves successfully parsed fields. A total connection
failure remains an operation error. The CLI must distinguish unavailable data
from values such as standby, muted, or an unknown input.

This phase performs a bounded snapshot and then closes the connection. It does
not subscribe to unsolicited events or promise live updates.

## Acceptance tests

The implementation is complete for Phase 1 only when the following coverage
exists:

- Unit tests cover SSDP header parsing, deduplication, identity selection,
  AVR query construction, response parsing, volume representation, and
  partial-status merging.
- Mock-transport tests cover CR framing, query order, connection/read
  timeouts, malformed lines, unsupported fields, and a disconnected receiver.
- CLI tests cover discovery numbering, saved-identity preference, discovery
  fallback, human-readable complete and partial output, and non-zero errors.
- A live X3800H test records firmware and network-control/standby settings,
  verifies TCP 23 reachability and all five status queries, and records the
  actual response names and volume encoding.

Live results must be recorded as `pass`, `fail`, or `not exposed` in the
research validation checklist. No live result may be inferred from the
AVR-3313 reference protocol.

## Known validation requirements and non-goals

The X3800H must be tested for the selected power query, input names, surround
mode names, mute and volume response formats, standby reachability, and socket
behavior. Network-control settings, firmware, multicast filtering, and DHCP or
manual addressing can affect results.

This phase does not assume action commands, HEOS TCP 1255/1256 behavior,
additional zones, event completeness, JSON output, or compatibility with other
receiver models. Those topics remain outside the v1 status contract.

## References

- [`v1 README`](README.md) — milestone goal and scope boundary.
- [`Protocol research`](../research/denon-avr-ip-protocol.md) — evidence and
  live-validation checklist.
- [`Architecture`](../../ARCHITECTURE.md) — layer and dependency rules.
