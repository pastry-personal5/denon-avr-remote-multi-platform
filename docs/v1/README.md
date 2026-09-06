# v1: Discovered AVR Status CLI

Status: milestone scope and implementation boundary. The one-shot discovery,
saved-identity, and status path is implemented; live X3800H compatibility is
still a validation requirement.

## Goal

Discover a Denon AVR and display the main-zone status in a human-readable CLI.
The first target is the Denon AVR-X3800H.

The milestone uses:

- SSDP discovery on standard multicast UDP 1900, with a UDP 1800
  compatibility probe;
- the Denon AVR ASCII protocol over TCP 23; and
- a persisted receiver identity in `config/denon-avr-remote.yaml`.

The initial status view contains power, input, volume, mute, and surround
mode. It should remain useful when only part of that status can be retrieved:
the CLI reports each field independently and identifies unavailable fields
instead of presenting an invented complete snapshot.

## Scope boundary

In scope for v1:

- numbered presentation and selection of discovered receivers;
- saved-identity lookup before falling back to discovery;
- one-shot main-zone status queries;
- bounded connection and read operations;
- human-readable output and actionable transport/protocol errors; and
- unit, mock-transport, CLI, and live-hardware acceptance coverage.

Deferred beyond v1:

- commands and other control actions;
- HEOS control;
- additional zones;
- JSON or other scriptable output; and
- long-running event sessions.

These deferred items must not be implied by a v1 capability or by an older
reference-protocol command table.

## Relationship to the project documents

The protocol evidence and the AVR-X3800H validation checklist remain in
[`../research/denon-avr-ip-protocol.md`](../research/denon-avr-ip-protocol.md).
The intended dependency boundaries are in
[`../../ARCHITECTURE.md`](../../ARCHITECTURE.md). This milestone defines the
first vertical slice within those boundaries; it does not duplicate the full
protocol specification.
