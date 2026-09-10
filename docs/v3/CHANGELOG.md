# Version 3 changelog

## [Unreleased]

### Fixed

- Made the X3800H HTTP information probe retain partial batch evidence and
  retry only missing read-only AppCommand observations.

### Added

- Added a reproducible Apple Silicon macOS packaging workflow that produces
  an unsigned `.app` and versioned drag-and-drop `.dmg`.
- Added typed, read-only X3800H HTTP information cards and channel layouts to
  the desktop Dashboard, including automatic connected/on refreshes.
- Added the Version 3 Phase 3 clean-architecture refactor plan and automated
  workspace boundary checks.

### Changed

- Completed Version 3 Phase 4: Apple Silicon macOS distributable bundle
  packaging is available through `make package-macos`.
- Completed the workspace cutover: session contracts now live in application
  ports, the legacy root source tree is removed, and delivery packages do not
  encode receiver wire commands directly.
- Narrowed the application crate root to the coordinator facade, extracted
  supplemental receiver policies and GUI presentation modules, and unified
  CLI/controller control admission so listening-mode changes are not
  incorrectly suppressed as no-ops.
