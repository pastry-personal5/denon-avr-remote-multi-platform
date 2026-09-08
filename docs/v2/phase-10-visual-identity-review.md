# Phase 10 visual identity and UX review

## Review result

**Status: substantially improved; accessibility and visual-regression gates remain open.**

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

### High — host accessibility preferences are not implemented

The Settings view says that reduced motion, contrast, and 200% text scaling are
supported host preferences, but the GUI has no preference subscription, text
scale application, reduced-motion behavior, or higher-contrast handling. The
review manifest correctly leaves screenshot and platform review open; these
features must not be described as supported until the selected Iced runtime
integration is verified.

### Medium — visual tokens are only partially applied

The token and component modules exist, and receiver actions use the new button
styles. Several route controls, text inputs, and dashboard status groups still
use default Iced styling, and the custom theme remains `Theme::Dark` rather
than a palette applying the documented tokens globally. The resulting visual
hierarchy is therefore inconsistent across routes.

### Medium — focus and screen-reader semantics remain unverified

The rail uses ordinary buttons and text, but there are no deterministic focus
order tests, explicit high-contrast focus-ring verification, accessible labels
for status groups, or platform screen-reader checks. The “Native keyboard
traversal” note in the rail is not evidence of release-blocking accessibility
verification and should remain qualified until those checks exist.

## Verification performed

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
