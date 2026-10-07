# Version 4, Phase 2 — GUI on the port

**In progress, started 2026-10-08** on branch `v4/phase-2-gui-on-port`. Steps
1 to 3 are done. This is milestone 2 of the [roadmap](roadmap.md). It moves the
GUI from the legacy `ReceiverController` onto the control-service port and then
deletes the legacy path. It is a refactoring: the intended user-visible
differences are the ones the parity table lists as **Changed**. The owner asked
for the milestone to be completed without answering decisions D2 to D5 (in the
[architecture](phase-2-gui-on-port-architecture.md#owner-decisions)), so the
recommendations were taken as the decisions.

The rules the phase relies on are in
[Planned architecture](../planned-architecture.md). What the GUI keeps,
changes, and drops, the two port changes, and the deletion inventory are in the
paired [architecture](phase-2-gui-on-port-architecture.md).

## Steps

Work goes one numbered step at a time: `make check`, `make clippy`, and
`git diff --check` pass, the diff is reviewed, and the step is committed.

| # | Step | State |
| --- | --- | --- |
| 1 | Behavior parity table | Done |
| 2 | Port changes P1 `refresh` and P2 retiring a changed session, and the session checks they need | Done |
| 3 | Retarget `gui-lib` to the port; rewrite its tests | Done |
| 4 | Delete the legacy path; flip the guards; update the contract documents | Not started |
| 5 | Scope the domain and protocol cleanup by inventory | Not started |
| 6 | Live checks and exit | Not started |

The roadmap lists five steps; this phase adds step 2 because the table found
gaps in the port, and splits the roadmap's guard rewrite across steps 3 and 4.
That is forced by `make check` passing at every commit:

- **Step 2.** Make the two [port changes](phase-2-gui-on-port-architecture.md#port-changes),
  each with tests against the in-process service and a fake connector: P1 adds
  `OperatorAdmin::refresh`, and P2 retires a session whose saved entry changed,
  and makes a closed session end its subscriptions. Read `AvrSession::next_event`
  and `reconnect_indefinitely` to settle whether a session's actor can end other
  than by `close`; evict a stopped session with a test if it can. Decide whether
  the service's explicit `synchronize` on connect can go, because the session's
  actor already runs a startup pass (L3). Extend the transport's power-on quiet
  period to `ZMON` only if the live check C4 shows the receiver needs it.
  Amend the design as phase 1's step 0 did, so the roadmap does not fork it:
  record P1 and P2 in the Control service section of `ARCHITECTURE.md`, where the
  implemented port lives, and add the Operator `refresh` resource to the Control
  API table amendments that milestone 4 makes (the roadmap's milestone 4 step 1
  already names them).
- **Step 3.** Retargeting changes the API that
  `crates/gui-lib/tests/async_apis.rs` and the unit tests in the `gui-lib` root
  module (`lib.rs`) compile against, so they are rewritten in this step.
  The legacy types still exist and still compile; the GUI no longer uses them.
  `tools/check-phase5-ledger.sh` tests only that `async_apis.rs` exists, so it
  needs no change while the file keeps its path. Sound-mode category pairing,
  volume conversion, the supplemental-read triggers, and configuration
  read-modify-write each get their own tests here.
- **Step 4.** Delete the controller, the adapter, and the legacy traits, flip
  the single-definition rule in `tools/check-boundaries.sh` to
  `CanonicalReceiverSession`, and remove `canonical_factory.rs` from the
  `operate` allowlist, all in one commit: the script fails if either half lands
  alone. Update `ARCHITECTURE.md`, `AGENTS.md`, and the contributing guide to
  name the new contract, and correct `AGENTS.md`'s statement that the contract
  lives in `ports.rs`.
- **Step 5.** The legacy `MainZone*` types are still used by
  `protocol/src/avr/command.rs` and `response.rs`, and by the transport. Delete
  only what no remaining consumer needs, per the inventory.
- **Step 6.** Run the live checks and record them in the archive.

## Exit criteria

From the roadmap, with the Diagnostics exception that decision D4 proposes:

- No `ReceiverController`, legacy `ReceiverSession`, or `SessionFactory` symbol
  remains.
- `make visual-baselines` shows no pixel change on macOS. Proposed by D4: the
  twelve baselines other than `diagnostics-100pct.png` and
  `diagnostics-200pct.png` are byte-identical, and those two differ only in the
  Diagnostics rows D4 names. Until D4 is answered, the roadmap's wording stands.
- A live GUI session, including a forced receiver reconnect, behaves as in 3.0.0,
  except for the differences the owner accepted in D5.
- `make check` and `make clippy` pass.

## Known limits

- The first command that saves the configuration, from the CLI or the GUI,
  rewrites a version 3 file in the new schema. An installed 3.0.0 cannot read it
  until the `.v3.bak` copy is restored. This is unchanged from phase 1.
- The GUI no longer replaces the whole configuration when it saves a manually
  entered or discovered receiver: it reads the file, changes one entry, and keeps
  the rest (closed in step 3; the parity table's G2).

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac.

1. **Baseline capture, before step 3 changes any GUI code.** Capturing needs a
   window server, so it cannot run in a sandboxed shell; it opens a window for
   each of the fourteen scenarios. From the repository root, on a commit with no
   GUI change (this branch is such a commit until step 3):

   ```text
   mkdir -p target/visual-captures/before
   tools/capture-visual-baselines.sh target/visual-captures/before
   make visual-baselines PLATFORM=macos CAPTURES=target/visual-captures/before
   ```

   If the comparison fails on unchanged code, the committed baselines are stale.
   Refresh them in a commit of their own first, so a later pixel difference can
   be attributed to a step in this phase and not to drift. This was not run when
   step 1 was written, because the capture could not open a window in the
   session that wrote it.
2. **Loopback tests.** The test suite binds loopback sockets. Run `make check`
   outside a sandbox that blocks local ports, or allow local binding for it.
3. **Live checks.** The list under
   [Live checks](phase-2-gui-on-port-architecture.md#live-checks), at step 6.
