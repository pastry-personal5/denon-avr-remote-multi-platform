# Version 2 - Phase 6 Overview

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
| Movie | Stereo, Dolby Surround, Multi Ch Stereo, Mono Movie |
| Music | Stereo, Dolby Surround, Multi Ch Stereo, Rock Arena, Jazz Club, Matrix |
| Game | Stereo, Dolby Surround |

The receiver's exact spelling `Multi Ch Stereo` is used. Shared modes are
defined once and may appear in more than one group.

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
- A selected mode is encoded as a typed `MS<MODE>` command and is never
  automatically replayed or silently replaced by another mode.
- The GUI distinguishes confirmed, rejected, unsupported, transport-failure,
  and unconfirmed outcomes.
