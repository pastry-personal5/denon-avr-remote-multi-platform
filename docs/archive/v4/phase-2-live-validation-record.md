# Version 4, Phase 2 — GUI on the port: validation record

**Not run.** Every check below needs a window server, the receiver, or both, and
the session that implemented the phase had neither. The phase's code is on branch
`v4/phase-2-gui-on-port` and passes `make check` and `make clippy`. This record is
the owner's to fill; until its **Result** column is complete, the phase's exit
criteria for pixel parity and for a live GUI session are open, and the branch is
not to be merged.

The behaviors being checked are in the
[architecture](../../v4/phase-2-gui-on-port-architecture.md) under the IDs in the
first column; the exit criteria are in the
[overview](../../v4/phase-2-gui-on-port-overview.md#exit-criteria).

## Visual baselines

Run from the repository root on the Mac, outside a sandbox. Each capture opens a
window per scenario, fourteen in all.

1. **Before the phase, on main.** This tells whether the committed baselines are
   current, so a later difference is attributable.

   ```text
   git switch main
   tools/capture-visual-baselines.sh target/visual-captures/before
   make visual-baselines PLATFORM=macos CAPTURES=target/visual-captures/before/macos
   git switch v4/phase-2-gui-on-port
   ```

   The capture writes under a folder named for the platform (`…/before/macos`),
   and the comparison reads that folder, so the two directories differ by
   `macos`.

   If this fails, refresh the baselines in a commit of their own on main first.

2. **After the phase.**

   ```text
   tools/capture-visual-baselines.sh target/visual-captures/after
   make visual-baselines PLATFORM=macos CAPTURES=target/visual-captures/after/macos
   ```

   Expected: the comparison reports `diagnostics-100pct.png` and
   `diagnostics-200pct.png` as different and the other twelve as matching. The two
   differ only in the Diagnostics rows decision D4 removes: the EQ lines and
   "Snapshot authority". Open both captures and confirm that before replacing the
   baselines with them (copy them from `target/visual-captures/after/macos` to
   `tests/visual-baselines/macos`).

| Check | Expected | Result |
| --- | --- | --- |
| Before: all fourteen match the committed baselines | pass | To fill |
| After: twelve match byte for byte | pass | To fill |
| After: the two Diagnostics captures differ only in the removed rows | pass | To fill |
| Diagnostics baselines recaptured and committed | done | To fill |

## Transport and CLI

Step 4 changed `AvrSession`, which the CLI and the GUI both use: its unused
shadow snapshot and a retrying read helper are gone, and `close` is now a method
of its own. The version 4 milestone 1 gates were last run before that, so run
them again. Follow the repository's write-safety controls, as in the
[milestone 1 record](phase-1-live-validation-record.md).

| Check | How | Expected | Result |
| --- | --- | --- | --- |
| Read-only live suite | `DENON_X3800H_HOST=HOST make test-live-x3800h` | passes | To fill |
| Armed, state-restoring controls | `ALLOW_RECEIVER_WRITES=1 DENON_X3800H_HOST=HOST` and a safe-volume value, then `make test-live-x3800h-controls` | passes, and the receiver is left as it was | To fill |
| CLI read | `make run ARGS="get status"` | prints the state and a readiness line, then exits and frees the receiver's control connection | To fill |

## Live GUI session

With `DENON_X3800H_HOST` reachable, run `make run-gui` against the saved receiver.
Reads are safe. The checks that write use a safe volume and restore the state
they change; follow the repository's write-safety controls.

| ID | Check | Expected | Result |
| --- | --- | --- | --- |
| L3 | Time from launch to the dashboard, against 3.0.0 | A fraction of a second to a second or two later than 3.0.0 (two synchronization passes). Note both times | To fill |
| L4 | Forced reconnect: turn the Mac's Wi-Fi off for several seconds and on again, or power-cycle the receiver | The window shows "RECONNECTING", then the dashboard returns without a restart; note how long the recovery took against 3.0.0 | To fill |
| C4 | Power the main zone on from standby through the GUI | Confirmed without a timeout or a refused command. If the receiver needs a quiet period after `ZMON`, extend the transport's rule, not the GUI | To fill |
| C8 | Press each sound mode category from inside and from outside that category | From outside, the mode changes and is confirmed. From inside, the window says the receiver already has the requested value and nothing is sent | To fill |
| R1 | Change the input, the sound mode, and power; watch the audio, video, and Audyssey panels | Each refreshes within a second or two; they also refresh by themselves about every fifteen seconds | To fill |
| G2 | With a hand-edited two-receiver file, use "Save and connect" for a new receiver | Both original receivers and the sound mode favorites are still in the file afterwards | To fill |
| G7 | With the GUI connected, edit the saved host to a second address that reaches the same receiver, and save it through Receivers | The window moves to the new address without a restart, and the receiver never shows two control connections | To fill |
| Volume | Press "+" and "-" (half-dB steps) and drag the slider to the bottom | Each half step moves the volume by half a dB and is confirmed. At the bottom the receiver shows its minimum and the slider stays at -80.0 dB | To fill |
| Launch failure | Start the GUI with the receiver switched off at the network | "RECEIVER UNAVAILABLE" with "Retry Status", not an endless launch screen; retrying after switching it on connects | To fill |
| Stale click | Change the volume with the remote and click "+" at once | If the window had not yet shown the change, the window says "Receiver state changed before the command; retry" and sends nothing | To fill |

Record the receiver model, firmware, relevant settings, commands, responses, and
the date, as in the version 3
[validation record](../v3/live-validation-record.md).
