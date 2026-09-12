# Phase 10 visual regression manifest

**Status: Complete — maintenance capture tooling.**

This manifest documents the deterministic review tooling for the desktop GUI. The
desktop capture mode uses Iced's native window screenshot API and encodes its
RGBA buffer directly to PNG. It never uses OS-level desktop capture. Set
`DENON_AVR_CAPTURE_DIR` plus a fixed `DENON_AVR_CAPTURE_SCENARIO` and optional
`DENON_AVR_CAPTURE_SCALE`; startup captures to
`$DENON_AVR_CAPTURE_DIR/$OS/$SCENARIO-$SCALEpct.png`.

The supported fixed scenarios are `connected`, `source-picker`, `unavailable`,
`settings`, `receivers`, `diagnostics`, and `messages`; they construct
presentation state without a receiver connection. This is deterministic capture
machinery. In
capture mode the application closes after the PNG is written, allowing
`make capture-visual-baselines CAPTURES=target/visual-captures` to collect the
complete scenario matrix without manual window management.

| Route/state | Window | Text scale | Required review |
| --- | --- | --- | --- |
| Dashboard / connected | 1400×880 | 100%, 200% | status groups, primary actions, mode availability |
| Dashboard / unavailable | 1400×880 | 100%, 200% | explicit unavailable and unknown wording |
| Receiver setup / saved, discovered, manual | 1400×880 | 100%, 200% | keyboard order and wrapped identities |
| Settings | 1400×880 | 100%, 200% | host accessibility preference copy |
| Diagnostics | 1400×880 | 100%, 200% | independent field evidence |
| Global messages / expanded, collapsed, full | 1400×880 | 100%, 200% | chronological scroll, clear, eviction |

The accepted layout invariants are a 240 px rail and a minimum window size of
1400×880. Capture each listed scenario at 100% and 200%, commit per-platform
PNG baselines under `tests/visual-baselines/$OS/`, and compare them
automatically when a future release elects to use baseline review. Record exact
baseline paths, contrast results, keyboard traversal, reduced-motion/high-
contrast state, failures, and macOS, Windows, and Linux sign-off with that
release's evidence.

After generating all fixed scenarios in a temporary capture directory, compare
them with `make visual-baselines PLATFORM=linux CAPTURES=/path/to/captures/linux`
(substitute the audited platform). The check fails for absent baselines, absent
captures, or byte differences.
