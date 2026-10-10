# Spike S5: the programs, archived

Spike S5 asked how macOS Local Network permission treats the Control API server when it is nested
in the signed app. It ran on 2026-10-10 and its outcome is **B**: sign the server
`com.denonavr.remote.server` and the bundle `com.denonavr.remote`, inside out. The result and the
reasoning are in [the research note](../../research/local-network-permission-server-macos.md); the
decision is D80 in [the phase 5 architecture](../../v4/phase-5-operator-clients-architecture.md).

`tools/` here holds the throwaway programs the spike used, **for reference only**:

| Path | What it is |
| --- | --- |
| `tools/build-bundles.sh` | Builds the test bundles (`own`, `same`, `deep`), signs them, retags their programs' UUIDs, and prints the lines to run |
| `tools/host/` | `s5-host`, a standard-library Rust program: starts the nested server, drives it, writes a report; also reads and retags Mach-O UUIDs |
| `tools/app/main.swift` | `s5-app`, a small AppKit window that stands in for the GUI, with buttons that record dialogs |

They were written to run from `tools/s5/` (the build script finds the repository root two directories
up), so move this `tools/` directory back to `tools/s5/` to run them. They need macOS, Xcode's command
line tools and the `aarch64-apple-darwin` Rust target. The receiver's address was removed from them
and from the notes; they ask for it (`S5_RECEIVER_HOST`).

The twelve screenshots that were here (the permission dialogs and System Settings → Privacy &
Security → Local Network) were deleted on 2026-10-10. The research note's fifth and sixth runs record
what they showed.
