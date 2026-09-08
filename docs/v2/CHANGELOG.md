# Changelog

All notable changes to Version 2 documentation will be documented in this
file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to Semantic Versioning.

## [Unreleased]

### Added

- Completed Version 2 Phase 5 with the Iced 0.14 desktop GUI, typed
  controller bridge, receiver setup/discovery flows, Main Zone dashboard,
  diagnostics routes, capability-gated controls, and stale-event protection.
- Completed the Version 2 Phase 1 clean layered architecture refactoring.
- Added the Version 2 Phase 5 GUI, Phase 6 listening-mode-group, and Phase 8
  Quick Select/EQ Status plans.
- Added the Phase 2 information architecture and GUI design specifications.
- Marked Phase 3 complete after implementing typed Main Zone controls, CLI
  dispatch, capability gating, and normalized volume-level handling.
- Added the Phase 4 application-owned receiver controller contract, typed
  session factory, lifecycle events, bounded close, and architecture records.

### Changed

- Refined the [Phase 2 IA](phase-2-gui-information-architecture.md) and
  [GUI design](phase-2-gui-design-specification.md) around user tasks, contextual feedback,
  consistent selectors, keyboard focus, accessible contrast, and explicit
  read-only/later-phase boundaries, with links to Apple design guidance.
- Added concrete setup, recovery, volume-confirmation, and usability acceptance
  requirements; defined Main Zone as the only supported zone across all v2 phases and removed additional-zone design targets.
- Refined the GUI specification for Rust Iced 0.14 with explicit boot/update/view
  responsibilities, Task and Subscription lifetimes, stale-result rejection,
  responsive widget composition, focus behavior, and accessibility capability
  gates.
- Marked Phase 1 complete after removing legacy façade modules, enforcing
  protocol boundaries, and passing the full verification suite.

- Positioned the GUI and control phases on top of the Phase 1 domain,
  application-port, protocol, infrastructure, and presentation boundaries.
- Defined Phase 2 as a buildable read-only Iced desktop GUI with shared
  platform-native configuration migration.
- Defined Phase 3 as evidence-gated X3800H main-zone control with execute-once
  delivery and authoritative confirmation.
- Recorded manual AVR verification for power controls and partial manual
  verification for normalized volume control.
- Replaced the generic future GUI boundary with the phased Version 2 user and
  API contract.
- Consolidated GUI delivery and controller integration in Phase 5, and added
  Phase 6 grouped listening-mode selection for the validated X3800H profile.
- Added Phase 8 planning for Main Zone Quick Select presets and independent
  Audyssey/Dirac EQ status.

### Fixed

## [1.0.0] - 2026-09-07

### Added

- Initial Version 2 documentation structure and baseline.
- Added a future GUI user guide defining the proposed scope and boundaries.

### Changed

- Updated Version 2 documentation to distinguish planned GUI work from current
  implementation.

### Fixed
