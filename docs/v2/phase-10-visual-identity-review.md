# Phase 10 visual identity and UX review

## Review result

**Status: complete.**

The redesign establishes a useful dark console shell, a persistent receiver
rail, reusable styling primitives, bounded session feedback, and a dedicated
feedback policy module. The active workspace compiles and the GUI-library tests
pass. Phase 10 is complete; capture and accessibility limitations are retained
as documented follow-up maintenance concerns.

## Follow-up review

The following earlier findings are resolved:

- The message scrollable is bottom-anchored so the newest message is visible.
- Command feedback is no longer rendered in the toolbar; it is kept in the
  global message panel.
- Main Zone `ControlResult` variants now map to explicit user-facing wording
  in `crates/gui-lib/src/feedback.rs`.
- Feedback formatting, EQ summaries, and Quick Select outcome copy are
  separated from the main GUI reducer and widget composition.

## Findings

### Known limitation — semantic screen-reader bridge is not available

The Settings view now provides session-only 100/125/150/175/200% scale,
normal/reduced motion, and normal/high-contrast controls. They are not
persisted. Reliable host preference discovery, reduced-motion visual behavior,
and the native semantic accessibility bridge are not yet verified. Do not claim
VoiceOver/Narrator/Orca support until all three audits validate roles, values,
state, focus, and outcomes.

On 2026-09-09, the pinned Iced 0.14.0 dependency was checked against the
upstream issue and release documentation. Issue #552 remains open and the
stable release exposes no native semantic accessibility bridge. An upgrade is
therefore deferred to a future accessibility increment. See [Iced issue #552](https://github.com/iced-rs/iced/issues/552).

### Follow-up — collect visual baselines as releases require them

The GUI now selects a real custom Iced palette and shares component styles for
actions, navigation, panels, and setup fields. Native Iced screenshot capture,
fixed offline scenarios, scripted capture collection, and byte-for-byte
comparison tooling are present. Committed PNG baselines and per-platform review
may be added by future releases without reopening this phase.

### Follow-up — focus and semantic audits

The GUI now explicitly maps Tab and Shift+Tab to Iced's `focus_next` and
`focus_previous` operations. Deterministic focus-order tests, explicit
high-contrast focus-ring verification, accessible labels for status groups, and
platform screen-reader checks remain absent. The navigation behavior is useful
keyboard support; semantic platform audits remain future work.

## Verification performed

- `design::tests::documented_text_and_focus_tokens_meet_their_contrast_floor`:
  passed. It asserts 7:1 for primary text and high-contrast focus border, and
  4.5:1 for muted text and accent against the documented canvas.
- `cargo check --workspace`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test -p denon-avr-gui-lib`: passed (21 unit tests and 3 integration
  tests).
- `git diff --check`: passed.
- Full workspace tests were attempted. Seven infrastructure tests failed before
  exercising the assertions because the sandbox denied local TCP listener
  creation (`Operation not permitted`). This is an environment limitation and
  is not evidence that those tests pass.
- PNG baselines and hands-on macOS/Windows/Linux review were not performed.

## Completion decision

Phase 10 is complete. Retain the current shell, capture tooling, and documented
Iced accessibility limitation for future maintenance work.
