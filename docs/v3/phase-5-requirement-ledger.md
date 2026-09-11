# Phase 5 requirement-to-evidence ledger

This ledger is intentionally checked in beside the Phase 5 test plan. A row is
considered implemented only when the linked executable test or reviewed live
record proves the stated invariant; documentation alone is not evidence.

The deterministic evidence set also includes the reviewed raw-frame replay
fixture [`x3800h_trace_replay.rs`](../../crates/infrastructure/tests/x3800h_trace_replay.rs),
which exercises production parsing and reduction outside the live socket.

| ID | Requirement | Executable evidence | Status |
|---|---|---|---|
| SMK-01 | Connect, synchronize, and perform a targeted control | [`core_smoke.rs`](../../crates/infrastructure/tests/core_smoke.rs) | Green |
| SMK-02 | Physical-remote burst merges one activity debt cycle | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-03 | Several controls preserve typed wire intent and evidence | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-04 | Interleaved control surfaces retain independent evidence | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-05 | Disconnect stales values and reconnect recovers | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-06 | Slow subscribers converge to the latest complete snapshot | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-07 | Lost event is repaired by targeted observation/sweep | [`core_smoke.rs`](../../crates/infrastructure/tests/core_smoke.rs) | Green (loopback) |
| SMK-08 | Supplemental HTTP failure cannot block core session work | [`async_apis.rs`](../../crates/gui-lib/tests/async_apis.rs) | Green (adapter) |
| SMK-09 | Ambiguous write is not replayed and remains truthful | [`receiver_scenarios.rs`](../../crates/infrastructure/tests/receiver_scenarios.rs) | Green (model) |
| SMK-10 | Explicit close stops retries and closes without task leakage | [`core_smoke.rs`](../../crates/infrastructure/tests/core_smoke.rs) | Green (loopback) |

## Follow-up (outside the Phase 5 release gate)

- The comprehensive connection, framing, byte-fault, scheduler-fairness, and
  mixed-controller catalog remains broader than the current deterministic model
  tests and should continue as follow-up coverage.
- Quick Select/EQ write validation is explicitly deferred; those capabilities
  remain gated and are not promoted by Phase 5.
- Both opt-in live read-only synchronization and the armed mute round-trip have
  passed, and the live record contains the available device metadata, evidence,
  and anonymous human review.
