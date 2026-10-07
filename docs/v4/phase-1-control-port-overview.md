# Version 4, Phase 1 — Control port and in-process service

**In progress — started 2026-10-07 on branch `v4/phase-1-control-port`.** This is
milestone 1 of the [roadmap](roadmap.md). It is a refactoring: it adds no
user-visible behavior except that the CLI reads and writes the receiver
configuration in the new multi-receiver schema.

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
| 4 | In-process control service and Operator gate | Not started |
| 5 | Inspection reads on the canonical contract | Not started |
| 6 | Session precondition | Not started |
| 7 | CLI on the port | Not started |
| 8 | Multi-receiver configuration file | Not started |

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

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac before merging.

1. **Loopback tests.** The test suite binds loopback sockets. Run
   `make check` outside a sandbox that blocks local ports, or allow local
   binding for it.
2. **Read-only CLI.** With a receiver reachable, `make run ARGS="get status"`
   prints the state and a readiness line, then exits. Repeat with
   `--host HOST`.
3. **Single control connection.** While the CLI runs, the receiver accepts no
   second Telnet client; after it exits, another client can connect at once.
4. **Configuration migration.** Copy a version 3 file to a scratch path, point
   the CLI at it, run any command, and check that the file is now version 2 and
   a `.v3.bak` copy holds the original.
5. **Armed controls.** With `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and a
   safe volume set, run the state-restoring live controls and record model,
   firmware, settings, commands, responses, and date as in
   `docs/archive/v3/live-validation-record.md`.
