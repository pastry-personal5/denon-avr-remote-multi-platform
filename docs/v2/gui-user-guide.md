# Version 2 GUI User Guide

## Status

The GUI is not part of the current v1 release. Phase 1 first establishes the
layered application architecture, Phase 2 delivers a buildable read-only Iced
desktop application, Phase 6 adds validated X3800H controls, and Phase 7
stabilizes the completed application lifecycle. This guide defines the intended
user-visible contract for those phases; it must be updated as each phase becomes
implemented.

## Supported Product Surface

Version 2 targets a native local-network application for Windows, macOS, and
Linux. The source will be buildable with Cargo, but v2 Phase 2 does not promise
installers, signed or notarized bundles, auto-update, or a cross-platform CI
artifact matrix.

The existing CLI remains supported. Both interfaces use the same
platform-native receiver configuration after importing a valid legacy
`config/denon-avr-remote.yaml` when necessary.

## Phase 2: Read-Only GUI

When a saved receiver exists, the application attempts to connect at startup.
If no receiver is saved or connection fails, the user can discover receivers,
select one explicitly, or enter a manual host. Errors remain visible with the
available recovery actions.

The main screen presents:

- receiver identity;
- main-zone power, input, volume, mute, and surround mode;
- connected, reconnecting, and disconnected state;
- field-level unavailable or malformed results;
- whether displayed state is unknown, invalidated, event-derived, or confirmed
  by an authoritative refresh;
- manual refresh, disconnect, and receiver-selection actions.

Initial connection and reconnect invalidate previous authority and trigger a
new five-field status query. Validated receiver events may update individual
fields between refreshes, but the application does not imply that an event is a
complete authoritative snapshot.

## Phase 6: Main-Zone Controls

Controls are available only for a receiver recognized as the validated
AVR/AVC-X3800H target. Other and unknown models keep the Phase 2 read-only
experience.

The planned controls are:

- power on and standby;
- input selection from a live-validated allowlist;
- absolute master volume in validated 0.5 dB steps;
- mute on and off;
- surround-mode selection from a live-validated allowlist.

The application shows an operation as pending, sends it once, and queries the
affected field before calling it confirmed. It never blindly repeats a control
after disconnect or timeout. When delivery may have happened but confirmation
cannot be obtained, the result is shown as unconfirmed and the user can refresh.
Power-on enforces the receiver's required one-second quiet period before any
following command.

## Interaction and Accessibility

- Every operation and recovery action must be reachable by keyboard.
- Focus must be visible and controls must have meaningful accessible labels.
- Connection, availability, pending, error, and authority states must use text
  or labels and not color alone.
- Controls must be disabled when disconnected, unsupported, or conflicting with
  an operation already in flight.
- Startup, discovery, status refresh, and ordinary status observation must not
  mutate receiver state.
- Partial status remains useful; one unavailable field must not erase valid
  values from other fields.

## Compatibility and Security Boundaries

- AVR and HEOS remain separate protocol and client boundaries.
- Capabilities are enabled from recorded model and firmware evidence, not from
  family resemblance or an older reference command list.
- Configuration contains receiver identity only. Credentials are not added
  without a separately reviewed security design.
- The application operates on the local network and does not expose an HTTP,
  cloud, or remote-control service.

## Explicitly Deferred

HEOS control, additional zones, raw protocol entry, JSON output, browser and
mobile clients, macros, scheduling, remote/cloud access, broader receiver
support, installers, signing, notarization, and auto-update are outside this
roadmap.

## Related Plans

- [Phase 1 overview](phase-1-overview.md) and
  [architecture](phase-1-architecture.md)
- [Phase 2 overview](phase-2-overview.md) and
  [architecture](phase-2-architecture.md)
- [Phase 6 overview](phase-6-overview.md) and
  [architecture](phase-6-architecture.md)
- [Phase 7 overview](phase-7-overview.md) and
  [architecture](phase-7-architecture.md)
