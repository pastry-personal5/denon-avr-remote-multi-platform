# Version 1 - Phase 1 Overview

## Objective

Discover a Denon AVR and display its main-zone status in a human-readable CLI,
with the Denon AVR-X3800H as the initial target.

## Scope

Phase 1 includes SSDP discovery, saved receiver identity, one-shot TCP 23
status queries, bounded operations, partial-status rendering, and actionable
errors. It does not include control commands, HEOS control, additional zones,
JSON output, or a broader model-compatibility claim.

Discovery uses UDP 1900 with a UDP 1800 compatibility probe and a bounded local
description scan fallback. The persisted shape is
`config/denon-avr-remote.yaml` with `receiver.host`, `model`, and
`friendly_name`; credentials are not stored.

## Key Deliverables

- Numbered receiver discovery and selection.
- Saved-identity preference with discovery fallback.
- Main-zone power, input, volume, mute, and surround status.
- Independent field failures so partial status remains useful.
- Unit, transport, CLI, and documented live-validation coverage.
