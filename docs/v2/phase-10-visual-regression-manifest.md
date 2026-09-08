# Phase 10 visual regression manifest

This manifest is the deterministic review checklist for the desktop GUI. The
Iced runtime currently has no screenshot harness in this workspace, so PNG
capture remains an explicit release blocker rather than an unverified claim.

| Route/state | Window | Text scale | Required review |
| --- | --- | --- | --- |
| Dashboard / connected | 1400×880 | 100%, 200% | status groups, primary actions, mode availability |
| Dashboard / unavailable | 1400×880 | 100%, 200% | explicit unavailable and unknown wording |
| Receiver setup / saved, discovered, manual | 1400×880 | 100%, 200% | keyboard order and wrapped identities |
| Settings | 1400×880 | 100%, 200% | host accessibility preference copy |
| Diagnostics | 1400×880 | 100%, 200% | independent field evidence |
| Global messages / expanded, collapsed, full | 1400×880 | 100%, 200% | chronological scroll, clear, eviction |

The accepted layout invariants are a 240 px rail and a minimum window size of
1400×880. Baselines must be captured and reviewed on macOS, Windows, and Linux
before release; this repository does not claim those platform reviews are
complete.
