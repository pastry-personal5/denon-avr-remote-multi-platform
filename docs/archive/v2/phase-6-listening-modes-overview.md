# Version 2 - Phase 6 Overview

**Status: Complete — 2026-09-08**

Phase 6 adds grouped listening-mode selection to the Phase 5 GUI using the
typed Main Zone control and capability boundaries established in Phases 3-5.

## Objective

Allow users to select a Denon listening-mode group and then choose a validated
sound mode appropriate to that group and the receiver's current audio context.

Listening-mode groups are not input sources. `BD`, `CD`, and `TV AUDIO` remain
source selections; `Movie`, `Music`, and `Game` are mode-group selections.

## Validated X3800H Groups

The initial AVC/AVR-X3800H candidate mapping is:

| Group | Modes |
| --- | --- |
| Movie | Stereo, Dolby Surround, MCH Stereo, Mono Movie |
| Music | Stereo, Dolby Surround, MCH Stereo, Rock Arena, Jazz Club, Matrix |
| Game | Stereo, Dolby Surround, Video Game |

The AVR Telnet protocol's exact spelling `MCH STEREO` is used. (The HTTP
Information view may label the same mode `Multi Ch Stereo`.) Shared modes are
defined once and may appear in more than one group. Selecting Movie, Music,
or Game is itself a receiver control: the receiver recalls the last mode
saved for that group. The GUI must preserve and display that recalled mode;
it must not synthesize a replacement mode.

Denon documents that available modes vary with input signal format, channel
count, speaker configuration, and headphone use. The GUI must therefore not
assume that every group entry is always selectable. Unknown context is shown
as unavailable or pending rather than being guessed.

## Scope

Phase 6 adds grouped capability metadata, current-context filtering, checked
selection, and controller-backed mode changes. Every state-changing request
uses resource-version validation, execute-once dispatch, and authoritative
follow-up confirmation.

Unknown or unvalidated receivers remain read-only. Receiver-reported modes
outside the setter allowlist remain readable but are not selectable. Automatic
fallback, raw-mode entry, per-input mode profiles, content classification,
HEOS modes, and additional zones are out of scope.

## Acceptance Criteria

- Movie, Music, and Game expose exactly the validated choices for the current
  model and context.
- Shared modes do not create duplicate protocol definitions.
- Unsupported context, stale state, disconnected sessions, and pending
  operations disable selection with an explanatory status.
- Selecting a group uses the typed group control and recalls the receiver's
  remembered mode; selecting an individual mode uses a typed `MS<MODE>`
  command. Neither operation is automatically replayed or silently replaced.
- The GUI distinguishes confirmed, rejected, unsupported, transport-failure,
  and unconfirmed outcomes.
