# Phase 10 visual identity and UX review

## Review result

**Status: in progress; accessibility and visual-regression gates remain open.**

The redesign establishes a useful dark console shell, a persistent receiver
rail, reusable styling primitives, bounded session feedback, and a dedicated
feedback policy module. The active workspace compiles and the GUI-library tests
pass. The implementation does not yet satisfy all of the Phase 10
release-blocking accessibility and visual-regression criteria.

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

### High — semantic screen-reader bridge is not available as release evidence

The Settings view now provides session-only 100/125/150/175/200% scale,
normal/reduced motion, and normal/high-contrast controls. They are not
persisted. Reliable host preference discovery, reduced-motion visual behavior,
and the required native semantic accessibility bridge are not yet verified.
Iced issue #552 remains the upstream capability gate. Do not claim
VoiceOver/Narrator/Orca support until all three audits validate roles, values,
state, focus, and outcomes.

### Medium — visual regression evidence is absent

The GUI now selects a real custom Iced palette and shares component styles for
actions, navigation, panels, and setup fields. Native screenshot capture,
fixed fake-service scenarios, committed PNG baselines, automated comparison,
and per-platform review are not yet present.

### Medium — focus and screen-reader semantics remain unverified

The rail uses ordinary buttons and text, but there are no deterministic focus
order tests, explicit high-contrast focus-ring verification, accessible labels
for status groups, or platform screen-reader checks. The “Native keyboard
traversal” note in the rail is not evidence of release-blocking accessibility
verification and should remain qualified until those checks exist.

## Verification performed

- `design::tests::documented_text_and_focus_tokens_meet_their_contrast_floor`:
  passed. It asserts 7:1 for primary text and high-contrast focus border, and
  4.5:1 for muted text and accent against the documented canvas.
- `cargo check --workspace`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test -p denon-avr-gui-lib`: passed (5 tests).
- `git diff --check`: passed.
- Full workspace tests were attempted. Six infrastructure tests failed before
  exercising the assertions because the sandbox denied local TCP listener
  creation (`Operation not permitted`). This is an environment limitation and
  is not evidence that those tests pass.
- PNG baselines and hands-on macOS/Windows/Linux review were not performed.

## Acceptance recommendation

Keep the current shell and token direction, but do not mark Phase 10 complete.
Implement or explicitly block the host accessibility integration, then capture
and review the manifest cases on the supported platforms. Keep Phase 10 open
until those release gates are complete.
