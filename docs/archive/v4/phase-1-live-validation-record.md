# Version 4, phase 1 live AVC-X3800H validation record

Live validation of the control port and in-process service through the CLI, the
last exit criterion of
[phase 1](../../v4/phase-1-control-port-overview.md). It follows the template in
[the version 3 record](../v3/live-validation-record.md). Do not commit receiver
addresses, account data, or other identifying values; the receiver is identified
by a redacted label.

**Status.** The owner reported that every step passed. This record was written
from that report, and the raw command output was not supplied to the author.
Fields marked **To fill** need the owner's run notes. Complete them before
treating this record as release evidence, as the version 3 record also required.

## Device and environment

- Date (UTC): 2026-10-08 (the date this record was written; confirm against the
  owner's notes)
- Redacted receiver label: physical AVC-X3800H (host withheld)
- Model: AVC-X3800H, as named in the saved configuration and not read from the
  receiver. **To fill:** confirm from receiver information.
- Firmware version: **To fill**
- Region: **To fill**
- Network Control setting: **To fill** (the run requires On)
- Active Main Zone / Zone 2 state: **To fill**
- Input and signal context: **To fill**
- Speaker/headphone context: **To fill**
- Safe volume ceiling (half-steps): **To fill** (the value passed as
  `DENON_X3800H_SAFE_VOLUME_HALF_STEPS`)
- Tool revision / commit: branch `v4/phase-1-control-port` at `17ea8ba`, with
  the review fixes applied as uncommitted working-tree changes when this record
  was written. **To fill:** the commit that was actually run, once the fixes are
  committed.

The review fixes in that working tree are: a backup is kept before a file this
release cannot read is replaced (a later `version`, a misspelt key, an
inconsistent entry); the receiver is no longer left reported as `Connecting`
when a caller gives up mid-connect; and a flaky config test no longer shares a
scratch directory with its neighbours.

## Steps run

Commands are those of the manual checklist in the phase overview. `HOST` stands
for the redacted receiver address. Each result below is the owner's report; the
responses are **To fill** from the run notes.

| # | Step | Command | Reported result |
| --- | --- | --- | --- |
| 1 | Deterministic gates, outside a sandbox | `make check && make clippy` | Passed |
| 2 | Read-only CLI, saved receiver and `--host` | `make run ARGS="get status"`, then `make run ARGS="get status --host HOST"` | Passed |
| 3 | Single control connection | `printf 'PW?\r' \| nc -w 2 HOST 23` after the CLI exits; the CLI run while `nc HOST 23` holds the connection | Passed |
| 4 | Configuration migration | `get status` on a copy of the version 3 file; a second command | Passed |
| 5 | Dry run | `make run ARGS="set mute on --dry-run"` | Passed |
| 6 | Armed mute round trip through the CLI | `get mute`, then `set mute` to the opposite value, then `set mute` back to the original | Passed |
| 7 | Live read-only session test | `DENON_X3800H_HOST=HOST make test-live-x3800h` | Passed |
| 8 | Live armed session test | `ALLOW_RECEIVER_WRITES=1 DENON_X3800H_HOST=HOST DENON_X3800H_SAFE_VOLUME_HALF_STEPS=<ceiling> make test-live-x3800h-controls` | Passed |

Optional parts of steps 4 (a `version: 3` file saved over) and 6 (setting a value
the receiver already has) are not confirmed by the report. **To fill:** whether
they were run.

## Read-only synchronization

- Readiness result: reported as `Readiness: ready` (step 2)
- Raw response frames (redacted): **To fill** (retain outside the repository)
- Connection epoch transitions: **To fill**
- Unknown/malformed diagnostics: **To fill**

## Armed control validation

- Arm command and environment gates: `ALLOW_RECEIVER_WRITES=1`, the host, and a
  declared safe volume ceiling, as in step 8
- Initial mute state: **To fill**
- Operations through the CLI: **To fill** the printed `Outcome`, `Dispatch`,
  `Confirmed`, and `Observation` lines for each `set`. The passing form is
  `completed`, `complete_write`, `true`, with an observation; setting a value the
  receiver already had is `already_in_state`, `not_dispatched`, `true`.
- Confirmation outcomes: receiver-confirmed, as reported for steps 6 and 8
- Final canonical values and validity: original mute state restored, as reported
- Restoration commands and outcomes: the second `set mute` of step 6 and the
  restoring write of step 8, both reported as passing
- Safe-volume invariant result: step 8 passed, which asserts the observed volume
  does not exceed the declared ceiling

## Review

- Reviewer: the repository owner
- Deviations / follow-up scenarios: none reported. **To fill:** anything seen
  during the run.
- Not covered by this record: a receiver whose configuration holds several
  entries, and the desktop app, which still reaches the receiver through the
  legacy controller until milestone 2.
