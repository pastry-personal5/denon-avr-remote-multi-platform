# Version 2 Future GUI User Guide

## Status

This is a future-facing guide and design boundary, not an implemented GUI
manual. The repository currently provides a CLI and library APIs only. No GUI
framework, desktop package, web application, JSON output contract, or GUI
control workflow is currently supported.

## Intended objective

A future GUI may present receiver discovery, saved identities, and partial
main-zone status for everyday use. It should make receiver identity,
connection state, data freshness, and unavailable fields visible rather than
implying a complete snapshot.

## Proposed first-screen scope

- Discoverable and saved receivers with explicit selection.
- Main-zone power, input, volume, mute, and surround mode.
- Connected, reconnecting, and disconnected states.
- Freshness and authoritative-refresh indicators.
- Clear errors with a manual-host fallback.

These are design targets, not current product capabilities. The GUI must not
claim receiver features without model and firmware evidence.

## Future interaction principles

- Keep AVR and HEOS protocol boundaries separate.
- Treat status as partial and preserve unknown or unsupported data visibly.
- Refresh authoritative state after connection or reconnect.
- Avoid mutating receiver state during startup or ordinary status discovery.
- Keep credentials out of the current configuration model unless a separately
  reviewed security design requires them.
- Make accessibility, keyboard navigation, and clear error recovery first-class
  requirements for whichever GUI platform is selected.

## Explicitly deferred

GUI implementation, receiver control commands, HEOS control, JSON output,
additional zones, broad model compatibility, packaging, auto-update behavior,
and remote/cloud access require separate design and validation.
