# Version 2 - Phase 8 Overview

**Status: Planned — complete before Phases 9 and 10.**

Phase 8 adds Main Zone Quick Select presets and EQ/room-correction status to
the Phase 5 GUI, building on the grouped listening modes from Phase 6.

## Objective

Let users quickly recall receiver playback setups and understand the active
equalization and room-correction state without confusing a preset with a
single source, sound mode, or generic “EQ on” flag.

## Scope

Quick Select exposes the receiver's four Main Zone preset slots. A preset may
include the receiver-supported combination of input source, volume, sound
mode, channel levels, Audyssey parameters, Restorer, Dialog Enhancer, HDMI
video output, speaker preset, Dirac Live, and related playback settings. The
GUI shows each slot's receiver name and registered-item summary.

The first delivery supports reading and recalling validated receiver presets.
Editing slot names, choosing registered items, and saving a preset are
separate operations and require explicit capability and protocol evidence.
No preset operation silently falls back to another source, mode, or volume.

EQ Status presents independent status for Audyssey MultEQ XT32, Dynamic EQ,
Dynamic EQ reference offset, Dynamic Volume, Audyssey LFC, and Dirac Live.
Each item can be on, off, configured, unavailable, not applicable, or unknown.
The GUI never reduces these states to a single EQ boolean.

Main Zone remains the only supported zone. Unknown or unvalidated receivers
remain read-only. Additional zones, HEOS, raw commands, automatic content
classification, installers, and cloud services remain out of scope.

## Key Deliverables

- Typed Quick Select slot and preset-summary models.
- Validated Main Zone recall flow with pending, confirmed, rejected,
  unsupported, transport-failure, and unconfirmed outcomes.
- Dashboard Quick Select controls and Settings management entry point.
- Dashboard EQ Status summary and detailed Diagnostics presentation.
- Typed, independent room-correction status fields and capability gates.
- Automated fixtures and live validation records for every supported command,
  query, and preset field.

## Acceptance Criteria

- Recalling a preset clearly identifies the target slot and Main Zone.
- Preset recall is execute-once and never automatically replayed after a
  timeout or reconnect.
- The UI distinguishes a preset's registered fields from fields it does not
  include.
- EQ Status reports each validated processing feature independently.
- Direct and Pure Direct modes, missing Audyssey calibration, unavailable
  Dirac state, and unknown receiver responses are represented honestly.
- Failed recall or status queries preserve existing observations and expose a
  clear Refresh or Retry action.
