# Version 3, Phase 4 — macOS distributable bundle

**Completed — 2026-09-10.** This phase adds a reproducible Apple Silicon
packaging workflow. It builds the desktop GUI, assembles a native
`Denon AVR Remote.app`, and creates a drag-and-drop disk image.

## Scope and artifacts

Run `make package-macos` on an Apple Silicon Mac. The workflow requires Rust
with the `aarch64-apple-darwin` target, the Xcode Command Line Tools, and the
macOS `hdiutil`, `iconutil`, and `sips` utilities. It produces:

- `target/release/macos/Denon AVR Remote.app`
- `target/release/macos/Denon-AVR-Remote-<version>-arm64.dmg`

The bundle identifier is `com.denonavr.remote`, the minimum macOS version is
11.0, and the executable is built for `arm64`. The version in both artifact
names and bundle metadata comes from the Cargo workspace.

This release is unsigned and unnotarized. Gatekeeper can warn on first launch;
use Finder’s Open action after verifying that the artifact came from a trusted
source. Developer ID signing and notarization are future work.

## Configuration

When launched from Finder, the desktop client uses:

`~/Library/Application Support/Denon AVR Remote/denon-avr-remote.yaml`

The directory is created when settings are saved. A missing file starts with
an empty receiver set. Explicit paths supplied to `YamlConfigRepository` are
unchanged, as is the repository-relative `config/denon-avr-remote.yaml` path
on development and non-macOS runs.

## Future phases

Developer ID signing, notarization, Intel (`x86_64`) support, PKG installers,
and automated release publishing are not part of this phase.
