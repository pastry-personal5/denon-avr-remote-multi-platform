# V3 Phase 7: Sound mode redesign completion record

## Purpose and status

Phase 7 is complete. This record preserves the pre-change behavior for
historical context and documents the delivered sound-mode design below. It is
not a replacement for the workspace architecture.

The canonical workspace and session rules remain in
[ARCHITECTURE.md](../../ARCHITECTURE.md). Historical V2 listening-mode material
is evidence only and does not supersede this record.

## Pre-Phase 7 implementation (historical baseline)

The following sections record the behavior that Phase 7 replaced.

### Pre-Phase 7 terms

- A **detailed sound mode** is the AVR `MS` value, represented in the domain as
  `SurroundMode`; examples include `DOLBY SURROUND`, `MCH STEREO`, `DIRECT`,
  and `PURE DIRECT`.
- A **sound-mode category** is the `SoundModeCategory` enum: `Movie`, `Music`,
  `Game`, or `Pure`. It groups rows in the desktop UI and is also currently
  used as a receiver-control request.
- A **favorite** is a user-owned `(category, detailed mode)` pair. It is scoped
  to the selected configured receiver, persisted in YAML, and is not receiver
  state.

`MS?` supplies only the detailed AVR value. The receiver state has no wire
field that independently reports Movie, Music, Game, or Pure as the active
category.

### Pre-Phase 7 cross-layer flow

```text
desktop sound-mode table
  category button -> RecallSoundModeCategory(category)
  detail radio    -> SelectSoundMode { category, mode }
                 |
application coordinator: capability check, one dispatch, MS? refresh
                 |
canonical infrastructure adapter: ReceiverIntent::SoundMode
                 |
AVR protocol: MSMOVIE / MSMUSIC / MSGAME / MSPURE DIRECT, or MS<mode>
                 |
AVR MS event or MS? response -> detailed SurroundMode snapshot
```

The CLI is separate from this category-aware route. `set surround VALUE`
constructs `SoundModeIntent::Select(VALUE)` directly, and `get surround`
prints the core session's detailed `SoundModeStatus`.

### Pre-Phase 7 domain and capability model

`crates/domain/src/main_zone.rs` defines `SurroundMode`,
`SoundModeCategory`, and three Main Zone controls:

- legacy exact `SurroundMode(mode)`;
- category-aware exact `SelectSoundMode { category, mode }`; and
- `RecallSoundModeCategory(category)`.

`MainZoneSnapshot` stores an authoritative `surround_mode` field plus an
optional `sound_mode_category`. The latter is populated only after a
category-aware control has been confirmed; it is cleared when an input changes
or a later detailed mode differs. A detailed mode appearing in more than one
category is therefore not reverse-mapped from `MS?`.

`crates/domain/src/capabilities.rs` contains a static X3800H allowlist and
four category lists. Movie, Music, Game, and Pure lists overlap. The profile
allows all listed exact modes and all category recalls only for the X3800H
model; unknown models have no category-mode controls. These lists do not come
from a runtime AVR query and do not encode every signal, speaker, headphone,
or setup restriction.

### Pre-Phase 7 protocol and infrastructure

The AVR protocol parses any non-empty `MS...` frame as a detailed
`SoundModeStatus`, preserving the receiver-provided value. Encoding in
`crates/protocol/src/avr/x3800h.rs` is currently:

| Current intent/control | AVR command |
| --- | --- |
| Exact detailed mode | `MS<mode>` |
| Movie category recall | `MSMOVIE` |
| Music category recall | `MSMUSIC` |
| Game category recall | `MSGAME` |
| Pure category recall | `MSPURE DIRECT` |

The canonical adapter in `crates/infrastructure/src/canonical_factory.rs`
maps Main Zone controls to those intents. Its `MS?` read converts only a
current core-session observation into a `MainZoneValue::SurroundMode`.

The older `x3800h_session` actor retains parallel sound-mode matching logic:
exact choices require an exact detailed observation, while Movie/Music/Game
recalls are treated as matched by any detailed mode in the corresponding
static category. It has no category-recall branch for Pure because the core
intent represents that path as `PureDirect`.

### Pre-Phase 7 application control and monitoring behavior

The application admits controls against the snapshot resource version and the
model capability profile, dispatches an admitted write once, then refreshes
Main Zone status. Exact detailed selections confirm only when the refreshed
`MS?` value equals the requested mode.

Category recalls intentionally bypass the normal no-op check. The application
requires the detailed mode returned after the refresh to differ from the
pre-dispatch detailed mode, then records the requested category alongside that
detailed result. This means a category action that validly leaves the AVR on
the same detailed mode is currently reported as unconfirmed.

An unsolicited or later receiver-mode change that produces a different
detailed mode clears the stored category. An unchanged external `MS` event
cannot be distinguished from the prior state by the snapshot reducer and does
not clear it. Refreshes and reconnects monitor detailed state through the
canonical session; they cannot observe a standalone category state.

### Pre-Phase 7 desktop UI and configuration behavior

The dashboard renders one combined SOUND MODE table in
`crates/gui-lib/src/views.rs`:

- each category label is a button that sends a category recall;
- each mode row has a radio control that sends its exact detailed mode with
  the row's category;
- duplicate detailed modes appear in each configured category, but one radio
  is selected using the snapshot category or the GUI's last-requested
  category preference;
- the heart button toggles the category-and-mode favorite and queues its YAML
  save.

While a control is pending, the GUI retains a local category preference so a
shared detailed mode can remain associated with the clicked row. It clears
that preference on receiver selection changes, disconnects, errors, and
unmatched control completion. The GUI does not own a receiver connection; it
uses the application bridge and snapshots.

`crates/infrastructure/src/config_yaml.rs` reads the legacy list-only favorite
format as Movie favorites and writes the current category-keyed format. The
CLI exposes detailed mode read/write only; it has no category-recall command
or favorite UI.

## Delivered Phase 7 design

This section records the implemented behavior, deliberately separate from the
historical baseline above. It does not change the workspace architecture.

The desktop panel shows four equal-width category buttons—MOVIE, MUSIC,
GAME, and PURE—followed by a three-column table: detailed sound mode, favorite,
and select. Its initial table contains every catalog mode once, in Movie →
Music → Game → Pure order. Selecting a category filters that local table; it
does not claim that a category is AVR-observed state.

MOVIE, MUSIC, and GAME filter the table and retain their receiver category
recalls. PURE filters to the Pure family only; it does not send `MSPURE DIRECT`.
Exact detailed-mode selection sends the exact `MS` target, using the mode's
first catalog category only to construct the existing category-aware control.

Favorites use one detailed-mode identity per receiver. A shared detailed mode
has one heart state in every table view. YAML reads prior category-keyed files
by flattening their lists to unique modes, then writes the canonical simple
per-receiver mode-list form.

The redesign is covered by GUI, domain, protocol, and persistence regression
tests. Workspace validation passed with `make format-check`, `make boundary`,
`make check`, `make clippy`, and `git diff --check`.

## Remaining validation limits

The official AVC/AVR-X3800H manuals show physical MOVIE, MUSIC, PURE, and
GAME buttons. MOVIE/MUSIC/GAME remember a prior selection and may choose a
compatible mode when that selection cannot apply; PURE selects explicit Direct,
Pure Direct, or Auto modes rather than documenting a remembered-category
recall. Available modes also depend on input signal and receiver setup.

Relevant manufacturer references:

- [Selecting a sound mode](https://manuals.denon.com/AVRX3800H/NA/EN/GFNFSYvhhpesdv.php)
- [Direct playback](https://manuals.denon.com/AVRX3800H/NA/EN/GFNFSYrhzdncbw.php)
- [Pure Direct playback](https://manuals.denon.com/AVCX3800H/EU/EN/GFNFSYuttunruh.php)
- [Types of input signals and corresponding sound modes](https://manuals.denon.com/AVCX3800H/EU/EN/GFNFSYwdnzswtk.php)

The static mode allowlist and receiver category recalls still lack full
state-restoring live validation across signal and speaker contexts. The
committed evidence covers read-only `MS?` synchronization, not category
writes, physical remote category presses, or Pure-button protocol equivalence.
These limits are documented profile policy; they do not leave the Phase 7 UI,
favorite identity, or YAML migration work incomplete.
