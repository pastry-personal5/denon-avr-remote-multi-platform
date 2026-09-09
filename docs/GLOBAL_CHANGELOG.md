# Changelog

All notable changes to the overall project documentation architecture and
documentation across all versions will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to Semantic Versioning.

## [Unreleased]

### Fixed

- Added the active [Version 3 Phase 1 plan](v3/phase-1-x3800h-http-info-probe-overview.md)
  for reliable, read-only X3800H HTTP information-probe evidence capture.
- Fixed AppCommand request serialization for AVR-X3800H firmware that returns
  an empty response for compact XML.

## [2.0.0] - 2026-09-09

### Added

- Released Version 2.0.0, including the workspace-based Rust packages and the
  native desktop application. See the [Version 2 release notes](v2/RELEASE_NOTES.md).

### Changed

- Advanced the workspace package version from 1.0.0 to 2.0.0.

### Added

- Added the Version 2 layered-architecture, GUI, control, and lifecycle
  stabilization roadmap.

### Changed

- Updated the root architecture and documentation navigation for the active
  Version 2 phase plan.

### Fixed

## [1.0.0] - 2026-09-07

### Added

- Initial documentation structure and baseline.
- Added the development guide.
- Added the standalone contribution guide.
- Added the Version 1 CLI user guide and Version 2 future GUI user guide.
- Added the Version 1.0.0 release notes.

### Changed

- Updated active documentation links and architecture guidance to match the
  canonical version and archive hierarchy.
- Renamed the 0.1.0 documentation baseline release to 1.0.0 and recorded it as
  the Version 1.0.0 freeze.

### Fixed

## Documentation hierarchy exception

Retired or superseded documentation may be moved into `docs/archive/`.
Active version documentation remains directly under `docs/v1/` or `docs/v2/`
using the phase-in-filename convention; archive content must not introduce
phase subdirectories.
