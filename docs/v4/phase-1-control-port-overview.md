# Version 4, Phase 1 — Control port and in-process service

**Steps 0–8 are implemented on branch `v4/phase-1-control-port` (started
2026-10-07). The owner reports that its last exit criterion, live receiver
validation through the CLI, passed on 2026-10-08. The
[record](../archive/v4/phase-1-live-validation-record.md) still needs its device
fields completed before it is release evidence.** This is milestone 1 of the
[roadmap](roadmap.md). It is a refactoring: it adds no user-visible behavior
except that the CLI reads and writes the receiver configuration in the new
multi-receiver schema, prints `set` results as `Outcome`, `Dispatch`, and
`Confirmed`, and stops discarding sound mode favorites when it remembers the
last receiver.

The rules the phase relies on are in
[Planned architecture](../planned-architecture.md). The types and decisions it
fixes are in the paired [architecture](phase-1-control-port-architecture.md).

## Steps

Work goes one numbered step at a time: `make check`, `make clippy`, and
`git diff --check` pass, the diff is reviewed, and the step is committed.

| # | Step | State |
| --- | --- | --- |
| 0 | Amend the design | Done |
| 1 | Stable receiver id | Done |
| 2 | Control-service port | Done |
| 3 | Receiver connector port | Done |
| 4 | In-process control service and Operator gate | Done |
| 5 | Inspection reads on the canonical contract | Done |
| 6 | Session precondition | Done |
| 7 | CLI on the port | Done |
| 8 | Multi-receiver configuration file | Done |

## Exit criteria

- `make check` and `make clippy` pass.
- The loopback tests in `x3800h_session.rs` cover the precondition: matching,
  mismatched target, mismatched non-target field, and epoch change.
- Every `OperationOutcome` variant is tested against the status mapping.
- Gate tests with a fake session assert at most one dispatch, including a retry
  with the same idempotency key.
- With a fake connector and clock, an idle receiver is released and the next
  request reconnects. A receiver with a subscriber or an operation in flight is
  not released.
- The configuration adapter round-trips the new schema, reads the old file,
  backs it up once, and rejects duplicate or reserved names.
- Live read-only validation and the armed, state-restoring controls run pass on
  the X3800H through the CLI, with the result recorded as in version 3.

## Exit status

All exit criteria except the last are met by deterministic tests that
`make check` runs. The last, live read-only validation and the armed,
state-restoring controls, needs the receiver and the owner's explicit safety
controls (`ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, a safe volume). It was
run on 2026-10-08 and reported as passing; it is recorded in the
[archive](../archive/v4/phase-1-live-validation-record.md), whose fields marked
**To fill** come from the owner's run notes. The phase is complete when the
record is complete. Merge to main waits for that.

## Known limits until milestone 2

- The desktop app still reaches the receiver through the legacy controller. Its
  receiver selection replaces the whole configuration with the selected
  receiver, so it overwrites a hand-edited multi-receiver file without a backup
  (the backup applies only to a file this release does not read as the new
  schema). Milestone 2 moves the GUI onto the port.
- The first command that saves the configuration, from the CLI or the GUI,
  rewrites a version 3 file in the new schema. An installed 3.0.0 cannot read it
  until the `.v3.bak` copy is restored.

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac before merging.

1. **Loopback tests.** The test suite binds loopback sockets. Run
   `make check` outside a sandbox that blocks local ports, or allow local
   binding for it.
2. **Read-only CLI.** With a receiver reachable, `make run ARGS="get status"`
   prints the state and a readiness line, then exits. Repeat with
   `--host HOST`.
   The CLI no longer prints `Outcome: {debug}`: a `set` prints `Outcome`,
   `Dispatch`, and `Confirmed` lines, and only `--dry-run` skips the receiver.
3. **Single control connection.** While the CLI runs, the receiver accepts no
   second Telnet client; after it exits, another client can connect at once.
4. **Configuration migration.** Copy your version 3 file aside. Run a `get`
   with the real file and check that it now starts with `version: 2`, that a
   `.v3.bak` copy holds the original, and that the receiver and any sound mode
   favorites are unchanged. Run another command and check that no second
   backup appears.
5. **Armed controls.** With `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and a
   safe volume set, run the state-restoring live controls and record model,
   firmware, settings, commands, responses, and date as in
   `docs/archive/v3/live-validation-record.md`.
