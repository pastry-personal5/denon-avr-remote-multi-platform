# Version 2 - Phase 10 Visual Identity

**Status: In progress — tokens and shell are implemented; evidence gates remain open.**

This specification is authoritative for Phase 10 appearance. It complements
the Phase 2 information architecture and interaction rules; it does not alter
receiver policy or control semantics.

## Direction

The application is dark-only and premium restrained: calm charcoal surfaces,
warm precise emphasis, and dense-but-legible receiver information. Use native
system fonts, text-first labels, and code-native Iced drawing only. Do not use
vendor marks, receiver imagery, gradients, external art assets, or an icon
library dependency.

## Tokens

| Purpose | Value |
| --- | --- |
| Canvas | `#111315` |
| Raised surface | `#181C20` |
| Active surface | `#22272D` |
| Border | `#343B43` |
| Primary text | `#F4F1EA` |
| Secondary text | `#B5BBC3` |
| Accent and focus | `#D99A3A` |
| Success | `#58B88D` |
| Warning / pending | `#D99A3A` |
| Error | `#E16E67` |

These values are the starting token set, not immutable values. Implementation
may tune the palette, spacing, radii, and type scale within this direction; the
final values must be recorded here before Phase 10 acceptance. Use an 8 px
spacing grid, 8 px control radius, 12 px panel radius, restrained surface
elevation, and system UI type at 12, 14, 16, 20, 28, and 36 px. Labels use
secondary text; values and page titles use primary text. Status labels always
include text or structure in addition to color.

## Shell and components

Enforce a 1400 × 880 minimum application window. At that size and above, show a
240 px left rail with receiver context and vertical navigation. Use the wide
content composition for every supported window; wrap long receiver, source,
mode, and preset names rather than truncate essential meaning. Preserve all
controls and status text at 200% text scale.

Use raised status groups with one heading, related values, local exceptions,
and an optional details action. Primary receiver actions use the warm accent;
destructive or uncertain outcomes use explicit wording plus their semantic
color. Focus is a high-contrast 2 px accent ring outside the component edge.
Disabled controls show their reason adjacent to the control rather than relying
on faded opacity alone.

Pending controls retain the observed value and show the requested target.
Confirmed, rejected, and unconfirmed outcomes use a persistent local result
panel until resolved or dismissed. Last-known, unavailable, unknown, and
unsupported observations remain visually and verbally distinct.

## Motion and verification

Use bounded cinematic motion only for the application shell: staged navigation,
panel, and background-surface transitions may run for 300–600 ms. Receiver
values, pending operations, confirmations, errors, and status text update
immediately and are never animated in a way that implies confirmation. With
reduced motion, remove shell transitions and spinners while retaining static
feedback. Use the host higher-contrast preference where Iced reliably exposes
it. The current implementation also provides session-only manual text-scale
(100–200%), motion, and contrast overrides; manual choices take precedence and
are never persisted. Host preference detection remains evidence-gated.
Required keyboard, focus, contrast, text-scaling, and screen-reader semantics
are release-blocking; do not claim support where the selected Iced runtime
cannot provide it.

Capture deterministic fake-service cases through Iced's native window
screenshots at 1400 × 880 and 100%/200% scale, encode per-platform PNG
baselines, and compare them in automated checks. Verify keyboard-only
traversal, readable outcome copy, contrast, and platform accessibility
semantics on macOS, Windows, and Linux before release. Iced must first provide
a native semantic accessibility bridge and it must pass VoiceOver, Narrator,
and Orca validation of roles, values, state, focus, and outcomes. Otherwise
the phase remains open; a parallel custom accessibility tree is out of scope.
