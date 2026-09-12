# Version 2 - Phase 10 Overview

**Status: Complete.**

Phase 10 gives the desktop application a cohesive visual identity while
preserving its established receiver behavior, routes, and safety guarantees.

## Objective

Deliver a refined, dark-only, premium restrained interface for the completed
Main Zone experience. The result should make everyday status, controls,
Quick Select, EQ status, setup, and diagnostics feel coherent without
misrepresenting receiver state.

## Scope

Apply the design system to the shell, receiver setup, Dashboard, listening
modes, Quick Select, EQ status, Settings, Diagnostics, forms, banners, and
control outcomes. Require a minimum application window size of 1400 × 880 and
use the wide desktop composition, visible focus, semantic feedback, and
motion/contrast accommodations supported by Iced.

Phase 10 adds no speculative receiver capability, light mode, theme picker,
appearance persistence, vendor branding, third-party visual assets, HEOS,
additional zones, web/mobile client, or cloud service. The
[source-catalog increment](phase-11-source-catalog-overview.md) is independent
and separately evidence-gated; it is not a dependency of visual-identity
completion.

## Deliverables

- Code-native design tokens and reusable Iced components.
- Consistent styles for normal, focused, disabled, pending, confirmed,
  rejected, unconfirmed, unavailable, and error states.
- Wide-layout rules for every established route at the minimum window size.
- A native Iced screenshot harness, deterministic capture scenarios, and
  optional baseline-comparison tooling for future visual maintenance.

## Acceptance Criteria

- Visual styling never changes an observed receiver value or implies that an
  unconfirmed command succeeded.
- The wide composition remains usable at 1400 × 880 and 200% text scale.
- Color is never the only state indicator; contrast and semantics remain
  usable across interactive states.
- Keyboard traversal, focus, contrast, and session text-scaling controls are
  implemented. Native screen-reader semantics remain dependent on Iced support
  and do not block this completed phase.
- Reduced motion removes cinematic transitions while retaining static feedback.
- macOS, Windows, and Linux reviews are encouraged for future releases.
