# Version 2.0.0 Release Notes

**Release date:** 2026-09-09  
**Packages:** `denon-avr-domain`, `denon-avr-protocol`,
`denon-avr-application`, `denon-avr-infrastructure`, `denon-avr-gui-lib`,
`denon-avr-cli`, `denon-avr-desktop`, and `denon-avr-diagnostics` 2.0.0

## Summary

Version 2.0.0 introduces the native desktop application and a workspace-based
Rust package layout. The desktop experience is a Main Zone (Zone 1) console:
it supports receiver selection, discovery, authoritative status refresh, and
capability-gated controls without extending control to additional zones.

## Highlights

### Desktop application

- Added the Iced desktop application with receiver setup and discovery,
  dashboard, diagnostics, accessibility preferences, and deterministic visual
  capture support.
- Added Main Zone controls for power, input, master volume, mute, and supported
  listening modes. The dashboard power button toggles only Main Zone (Zone 1)
  power; Zone 2 and Zone 3 controls are not exposed.
- Displayed volume on the receiver's native decibel range (-80.0 dB through
  +18.5 dB), including half-decibel steps.
- Kept the volume control visible while a volume command is awaiting receiver
  confirmation, instead of replacing it with a transient status label.

### Reliable receiver lifecycle and status

- Added the application-owned controller and typed session boundary for
  serialized receiver commands, lifecycle state, reconnect handling, and
  authoritative Main Zone snapshots.
- Prevented a continuous stream of unsolicited receiver notifications from
  starving queued commands such as Refresh Status.
- Made rediscovery resilient to a failure while closing an old receiver
  session, so the replacement receiver can still be selected and refreshed.

### Optional receiver features and diagnostics

- Added typed Quick Select recall and EQ-status surfaces behind explicit
  validation gates.
- Added a read-only, evidence-preserving source-catalog reader behind an
  explicit validation gate. It does not enable source editing or infer support
  when a receiver does not provide a usable response.
- Added bounded Telnet and AppCommand diagnostic probes for recording receiver
  observations without enabling unvalidated controls.

## Compatibility notes

- Version 2.0.0 reorganizes the project as a virtual Cargo workspace. Consumers
  should depend on the role-specific `denon-avr-*` packages rather than a
  monolithic package.
- Capability APIs were renamed to describe their feature area rather than a
  development phase. Code using earlier phase-numbered names must migrate to
  the corresponding Quick Select/EQ or source-catalog APIs.
- Main Zone remains the only supported control scope. Additional zones, HEOS
  control, and source-catalog writes are outside this release.

## Validation boundary

Capability gates remain evidence-bound. A known receiver model alone does not
enable optional Quick Select, EQ, or source-catalog behavior; the relevant
validation configuration is required. See the [Version 2 changelog](CHANGELOG.md)
and the active phase documents for implementation and validation records.
