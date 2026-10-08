# Version 4, Phase 3 — Policy, audit, and the Agent path architecture

This document fixes the types and rules that
[Planned architecture](../planned-architecture.md) leaves to phase 3. It does not
restate the design; where a rule is owned there it is linked. Text that is
implemented moves into [ARCHITECTURE.md](../../ARCHITECTURE.md) when the
milestone's exit criteria pass.

It was written on 2026-10-08 from the code at the head of `main` (`e95b5f9`) and
changes no code. Step 0 of the [overview](phase-3-policy-audit-gate-overview.md)
amended the design where the review found it wrong.

## Starting point

- The gate is Operator-only. `ControlService::handle` refuses an Agent with
  `ControlError::Forbidden`, and `admit` sets an operation straight to `Allowed`.
- Already present: `Precondition` and `FieldBaseline::capture` (`domain`),
  `RejectionCause::PreconditionMismatch`, `OperationStatus::Denied` and
  `ApprovalUnavailable`, `ControlError::RateLimited`, `AgentLabel`, and a
  `Principal` that every operation records as its owner.
- Absent: `policy`, `serde_json`, and `sha2` (not in `Cargo.lock`), any wall-clock
  type or `Clock` port, an audit port, and the port methods the policy, audit, and
  health views need. The repository's property tests are seeded loops, not
  `proptest`, and this phase follows them.
- Four types implement the port traits and must gain the new methods:
  `ServiceHandle`, the `Stub` in `control.rs`, the CLI's test `Fake`
  (`apps/cli/src/main.rs`), and `FastPort` in `crates/gui-lib/src/bridge.rs`.
- `main` already contains phase 2, and its two owner checks are open. They do not
  block this phase, which changes no GUI code.

## What the review changed

The roadmap's milestone 3 and the design had these faults. Each is fixed in the
design (step 0) or in this document.

| # | Fault | Fix |
| --- | --- | --- |
| R1 | `Decision::Allow` carried no baseline, yet `Allow` is the only decision that dispatches in 4.0.0 | `Allow { baseline }` |
| R2 | "Fields the matched rules consulted" omits the rule that did not match: an unmute is allowed because the loud-volume rule did not match | The baseline is every state field read by any *applicable* rule |
| R3 | The budget window must survive a restart, so it needs wall-clock time. `MonotonicMillis` does not, and tokio's paused clock does not move `SystemTime` | `WallTime` in `domain`, a `Clock` port, a fake in tests, and a rule for a clock that steps back |
| R4 | Two agents could each pass the pooled budget alone | Read the ledger, evaluate, and reserve in one step under one lock; reservations count |
| R5 | Audit order was unspecified, and the budget is rebuilt from audit | A `dispatching` record is synced to disk before the session is called; one with no `finished` counts as possibly dispatched |
| R6 | Operation ids restart at 1 on every run, so audit records from two runs collide | Every record carries the run (service start time) |
| R7 | Evaluation needs receiver state, but `admit` jumps to `Allowed` | An Agent submit returns `submitted`; the task leases, evaluates, then transitions |
| R8 | "Re-evaluation at dispatch" as a milestone 3 deliverable | Nothing waits between evaluation and dispatch until approval exists, so one evaluation under the lease is the one at dispatch |
| R9 | Classification was global, so a rule narrowed to one agent classified an intent for all agents | Classification is per agent; a rule naming no intent restricts but never classifies; `allow` rules only classify |
| R10 | "Pending caps" mean nothing without a broker | A cap on unfinished operations per label |
| R11 | An audit append for every refused request lets a flood rotate away the records the budget is rebuilt from | A cap-refused request creates no operation and no record |
| R12 | `ControlService::new` is synchronous, but the ledger rebuild reads files | `new` is unchanged and means no Agent path; an async `start` builds the full one |
| R13 | `dry_run` was a "submission flag" with no result type | Port methods that return a decision and create no operation |
| R14 | Policy, audit, and health views had no port methods, so milestone 4's `api-client` would have nothing to implement | Port additions below |
| R15 | The roadmap called the -20 dB hard limit "inert" | A target above it ends `denied`; only the band up to it waits on approval |
| R16 | A decrease with an unknown volume cannot be shown to be one | It needs approval too; documented as a consequence |
| R17 | `Minimum` has no arithmetic in the design | It is -80.0 dB in the rules |
| R18 | The boundary script sees direct edges only | The `policy` edges, a purity check, and a resolved-graph check |
| R19 | The exit criteria omitted audit rotation and reading, audit failure, policy loading, and digest | Added to the overview |

## `domain`: wall-clock time

`WallTime(u64)` in `crates/domain/src/wall_time.rs`: milliseconds since the Unix
epoch. It has `saturating_sub(Duration)` and `Ord`, and no clock read. The
domain tests need no runtime. `MonotonicMillis` stays for field validity.

## `policy`

New package `denon-avr-policy` at `crates/policy`, depending on `domain` only.

```text
IntentKind        SystemPower | MainZonePower | Zone2Power | Source | Volume
                  | Mute | SoundMode
                  IntentKind::of(&ReceiverIntent), an exhaustive match with no
                  wildcard arm, and IntentKind::ALL
Level             a volume in half dB steps; Minimum is -160 (-80.0 dB)
                  Level::of(MasterVolume), Level::from_db(f64) (the 0.5 dB grid,
                  -80.0 to +18.0), Level::from_half_steps, Level::half_steps()
Span              a distance in half dB steps (a step or a budget), 0 to 98 dB
PolicyConfig      PolicyConfig::new(rules: Vec<Rule>, approval_lifetime: Duration)
                    -> Result<PolicyConfig, PolicyError>; validates every rule
                  PolicyConfig::longest_window() -> Duration
Rule              id, scope, filter, effect
Scope             agents: Option<set of label text>, receivers: Option<set of id>
Filter            alternatives (IntentKind plus an optional on/off/standby
                  value; none means every intent) and conditions
Conditions        target_above, increase_over_baseline, budget (rise and window),
                  baseline_volume_above
Effect            Allow | RequireApproval | Deny

PolicyInput       agent: &str, receiver: &ReceiverId, intent: &ReceiverIntent,
                  state: &ReceiverState, recent: &[RecentChange], now: WallTime
RecentChange      at: WallTime, before: Option<Level>, target: Level
Decision          Allow { baseline }
                  | RequireApproval { reasons, rules, baseline }
                  | Deny { reasons, rules }
Baseline          the CoreFields the decision read; the gate adds the epoch
Reason            structured: TargetAbove, IncreaseOver, BudgetExceeded,
                  BaselineLoudOrUnknown, IntentRestricted, Unclassified, each
                  with the numbers that fired, and a Display for the text
evaluate(&PolicyInput, &PolicyConfig) -> Decision
```

`PolicyInput` takes the label as text because `AgentLabel` lives in
`application`.

### Evaluation

1. `applicable` = the rules whose scope matches the agent and receiver.
2. `classified` = some applicable rule has an alternative naming this intent: its
   kind, and its value when the alternative gives one (decision D21). A rule with
   no alternatives names nothing.
3. For each applicable rule with an alternative that matches the intent kind and
   value (a rule with no alternatives matches every intent): add the fields its
   state-dependent conditions read to the baseline, matched or not, then test its
   conditions. A rule **matches** when all of them hold.
4. The decision is the most restrictive matched effect: `Deny`, then
   `RequireApproval`. With none, an unclassified intent is `RequireApproval` with
   `Reason::Unclassified`, and a classified one is `Allow { baseline }`.

Conditions, with the owner's configuration as the reading (all limits strict):

| Condition | Matches when | State read |
| --- | --- | --- |
| `target_above_db: n` | the volume target is above `n` | none |
| `increase_over_baseline_db: n` | target minus the observed level is above `n`, or the level is unusable | Volume |
| `rise_over_window_db: n`, `window_minutes: w` | the target is above the observed level and target minus the floor is above `n`, or the level is unusable | Volume, ledger |
| `baseline_volume: { above_db: n, or_unknown: true }` | the observed level is above `n`, or unusable | Volume |

**Unusable** is `FieldBaseline::capture` returning `NoUsableValue`: the field is
stale, unknown, or unavailable, or has no last good value. A state-dependent
condition on an unusable field *matches*. That is the design's "unknown counts as
dangerous", and a property test pins it.

**Budget arithmetic.** The floor is the lowest of the observed level and, for
every `RecentChange` with `at` later than `now - w`, its `before` (when present)
and its `target`. An entry dated after `now` is inside the window. `Level::of`
maps `Minimum` to -160, so a rise from `Minimum` is overstated, which fails
closed.

**Validation in `PolicyConfig::new`.** An `allow` rule carries a scope, at least
one intent, and values only (decision D22). `target_above`, `increase_over_baseline`,
and `budget` need every alternative to be `Volume`, and a rule with no
alternatives cannot carry them. A value must belong to its intent: `on` and
`standby` for `system_power`; `on` and `off` for the zone powers and `mute`; none
for `source`, `volume`, and `sound_mode`. A rule id is unique and non-empty, and
an empty `agents` or `receivers` list is refused because it applies to nobody.
`window_minutes` is 1 to 1,440. Every limit is on the grid. These rules keep a
config author from writing something that reads like an exception and does
nothing.

## Policy file and loading

`policy.yaml` sits beside `denon-avr-remote.yaml`. The owner's configuration,
which becomes `docs/examples/policy.yaml` in step 4 and the table-test fixture:

```yaml
unclassified: require_approval
agent:
  approval_lifetime_minutes: 5
  rules:
    - id: volume-hard-limit
      when: { intent: volume, target_above_db: -20.0 }
      then: deny
    - id: volume-ceiling
      when: { intent: volume, target_above_db: -30.0 }
      then: require_approval
    - id: volume-step
      when: { intent: volume, increase_over_baseline_db: 6.0 }
      then: require_approval
    - id: volume-budget
      when: { intent: volume, rise_over_window_db: 10.0, window_minutes: 10 }
      then: require_approval
    - id: loud-or-unknown-baseline
      when:
        any:
          - { intent: main_zone_power, value: on }
          - { intent: mute, value: off }
        baseline_volume: { above_db: -30.0, or_unknown: true }
      then: require_approval
    - id: system-power
      when: { intent: system_power }
      then: require_approval
    - id: zone2-power
      when: { intent: zone2_power }
      then: require_approval
    - id: low-risk
      when:
        any:
          - { intent: main_zone_power, value: off }
          - { intent: mute, value: on }
          - { intent: source }
          - { intent: sound_mode }
      then: allow
```

The sample is the owner's configuration, not a shipped default: no code reads it.
The `claude-code-read-only` rule from the design is added to it as a commented
example. The expected decisions (each is a table case):

| Request, with a usable observed volume of -35 dB unless stated | Decision | Baseline fields |
| --- | --- | --- |
| Volume to -32 dB | `Allow` | Volume |
| Volume to -50 dB (a decrease) | `Allow` | Volume |
| Volume to -25 dB | `RequireApproval` (`volume-ceiling`; the rise of 10 also matches `volume-step`) | Volume |
| Volume to -15 dB | `Deny` (`volume-hard-limit`; `volume-ceiling`, `volume-step`, and `volume-budget` also match) | none: a `Deny` has no baseline |
| Volume to -31 dB with the observed volume -40 (a rise of 9) | `RequireApproval` (`volume-step` only) | Volume |
| From -50 dB, within 10 minutes: -44, -50, -44, then -38 | the last is `RequireApproval` (`volume-budget` only: the step is 6 and -38 is under the ceiling, but the rise is 12 above the floor -50) | Volume |
| The Operator raised -60 to -45 within the window, then the agent asks -50 | `Allow`: a decrease is never over the budget, though -50 is 10 above the floor -60 | Volume |
| Same history, the agent asks -44.5 | `RequireApproval` (`volume-budget` only: an increase of 0.5, but 15.5 above the floor -60) | Volume |
| Unmute at -30 dB exactly | `Allow` | Volume |
| Unmute at -29.5 dB, or with the volume stale | `RequireApproval` | Volume |
| Volume to -50 dB with the volume stale | `RequireApproval` | Volume |
| Main zone power off, mute on, source, sound mode | `Allow` | none |
| System or Zone 2 power | `RequireApproval` | none |

**Loader** (`crates/infrastructure/src/policy_yaml.rs`, `YamlPolicySource`): parses
with `serde_yaml` into a private shape with `deny_unknown_fields`, builds a
`PolicyConfig` through its constructors, and computes the SHA-256 of the file
bytes (`sha2`). It rejects `unclassified` other than `require_approval`,
`or_unknown` other than `true`, limits off the grid, an `allow` rule with extra
conditions, unknown keys (a time-of-day key among them), duplicate ids, and a
file over 256 KiB. Errors name the rule id or the key and never the file contents.
The port is `PolicySource::load() -> Result<LoadedPolicy, PolicyLoadError>` in
`application::policy_source`, where `LoadedPolicy` holds the `PolicyConfig`, its
`PolicyDigest`, and the text that was read (the Operator's `PolicyView` shows it).

**Strict loading** (decision D9). The service holds `Active(LoadedPolicy)` or
`Unavailable(error)`. A load that fails, at start or on `reload_policy`, replaces
`Active` with `Unavailable`; a later good load restores it. While `Unavailable`,
every Agent write ends `rejected`, "policy unavailable", with nothing dispatched.
Reads and Operator controls are unaffected.

## Clock and audit

```text
Clock             application::clock — now() -> WallTime; Send + Sync
SystemClock       infrastructure — reads std::time::SystemTime
AuditRecord       schema: u32 (1), run: WallTime, at: WallTime,
                  operation: Option<OperationId>, principal, receiver,
                  event: AuditEvent
AuditEvent        Decided { intent, decision, reasons, rules, baseline, policy }
                  | Dispatching { intent, before: Option<i16>, target: Option<i16>,
                                  precondition_fields }
                  | Finished { status, dispatch, confirmed, reason }
                  | PolicyLoaded { digest } | PolicyLoadFailed { error }
AuditLog          append(record, Durability) -> Result<(), AuditError>
                  since(WallTime) -> Result<Vec<AuditRecord>, AuditError>
                  query(AuditQuery) -> Result<AuditPage, AuditError>
Durability        Flushed | Synced
```

`before` and `target` are half steps, set for volume changes only. `intent` is a
bounded string (128 characters) in the same typed words the CLI prints, so agent
text such as a source id or sound-mode name is escaped by JSON and cannot run
long. `AuditEntry { seq, record }` is what `query` returns, newest first, in pages
of at most 500 with an opaque `AuditCursor`.

**Adapter** (`crates/infrastructure/src/audit_jsonl.rs`, `JsonlAuditLog`):

- One JSON object per line, written with `serde_json`. Records are appended under
  an async lock so lines never interleave.
- `Synced` appends call `sync_data` after the write; `Flushed` ones do not. Only
  `Dispatching` is `Synced`.
- The active file is `audit.jsonl`, with older files `audit.1.jsonl` to
  `audit.{n-1}.jsonl`. A record that would take the active file past the size limit
  first rotates the set, deleting the oldest beyond the count. Defaults: 20 MiB and
  10 files (decision D8).
- Directory mode 0700 and file mode 0600 on creation (`#[cfg(unix)]`). A file or
  directory that already exists with wider permissions is an error on first use,
  not a silent repair.
- `since` and `query` read across every retained file and skip a truncated last
  line, a line that is not JSON, and a record or field of a kind they do not know.
  `seq` continues from the newest record after a restart.
- The directory is `<support directory>/audit/`. A new `data_directory()`
  function in infrastructure returns the directory `default_config_path` already
  builds its path in, and both use it: `~/Library/Application Support/Denon AVR
  Remote` on macOS, and the relative `config/` fallback elsewhere, where the audit
  directory is then `config/audit/`.

**Sizing.** A write is three records of up to about 1.5 KB together, and the write
cap is 30 per minute per label, so one label at the cap writes about 65 MB in
24 hours. The 200 MiB default holds the ledger's whole 24-hour retention for about
three labels at the cap.

**Who writes.** Only a service built by `start` has an audit log. `ControlService::new`
has none and makes no audit call (decision D6), so the CLI and GUI, which still
compose their own service, are unchanged. The first real audit writer is
`api-server` in milestone 4.

## Ledger

`application::ledger::Ledger`, a plain structure the service guards with a
standard mutex. It is never held across an await.

```text
OpKey              (run: WallTime, OperationId)
Ledger::reserve(receiver, OpKey, RecentChange)
Ledger::settle(receiver, OpKey, dispatched: bool)    false removes the entry
Ledger::recent(receiver, now, longest_window) -> Vec<RecentChange>
Ledger::prune(now)                                   drops entries older than 24 h
Ledger::rebuild(records: &[AuditRecord], now) -> Ledger
```

- Retention is 24 hours and the loader caps `window_minutes` at 1,440. An entry
  dated after `now` is never pruned.
- **Counted:** an agent operation from `Allow` until it settles, kept if its
  `dispatch` was not `not_dispatched`; and an Operator volume write from the moment
  it dispatches (decision D7). Operator writes are recorded, never limited.
- **Rebuild.** From every `Dispatching` record in the 24 hours before `now`, minus
  those with a `Finished` record of the same `OpKey` and `dispatch: not_dispatched`.
  A `Dispatching` record with no `Finished` is kept. A `Finished` with no
  `Dispatching` is ignored. A rebuild that cannot read the log leaves the ledger
  not ready.
- A task that panics before settling leaves its reservation in place. That
  over-counts, which fails closed.
- An Operator write whose `Dispatching` append failed has no record and is not
  counted after a restart. Operator audit is best effort by design.

## Port additions

Defined first as signatures for review (step 2), with the service answering
`Unavailable` until the steps that fill them in:

```text
ReceiverReads::health()
  -> ServiceHealth { policy: NotConfigured | Active | Unavailable,
                     audit: NotConfigured | Ok | Failing,
                     ledger_ready: bool, approval: Unavailable }
OperationControl::dry_run(receiver, intent) -> DryRun
OperatorAdmin::dry_run_as(agent: AgentLabel, receiver, intent) -> DryRun
OperatorAdmin::policy() -> PolicyView { digest, loaded_at, text, error }
OperatorAdmin::reload_policy() -> PolicyView
OperatorAdmin::audit(AuditQuery) -> AuditPage
DryRun            Allow | RequireApproval { reasons, rules } | Deny { reasons, rules }
                  | Unavailable { reason }, with the policy digest
```

`health` is on `ReceiverReads` because the design serves `GET /v1/health` to both
endpoints; it carries coarse states and no paths or error text. `DryRun` for the
Operator's own principal is `Allow` with no rules, since the Operator skips policy.
`dry_run_as` lets the owner test a policy before an agent meets it; milestone 5
maps the CLI's `--dry-run` to it with the label named. A dry run leases the
receiver, evaluates, and creates no operation and no record, but it counts against
the write cap because it connects the receiver.

`OperationSnapshot` is unchanged. Its `reason` is text built from the `Reason`
values, which names the limit that fired; rule ids appear in the audit log, in
`DryRun`, and in Operator views only.

## The Agent path

`ControlService::start(connector, config, discovery, settings, agents)` is async
and infallible:

```text
AgentPath    policy: Arc<dyn PolicySource>, audit: Arc<dyn AuditLog>,
             clock: Arc<dyn Clock>, limits: AgentLimits
AgentLimits  writes_per_minute: 30, unfinished_operations: 8
```

It records the run time, loads the policy (a failure is service state, not an
error), appends `PolicyLoaded` or `PolicyLoadFailed`, and rebuilds the ledger from
`audit.since(now - 24 h)`. A rebuild that fails leaves `ledger_ready` false, and
every Agent write retries it first and ends `rejected` while it fails.
`handle(Principal::Agent(_))` succeeds only on a service built by `start`; on one
built by `new` it is `Forbidden`, as now.

### Submit

For an Agent, `submit` checks the caps, then admits:

- More than `writes_per_minute` new writes by the label in the last 60 seconds,
  or a label already holding `unfinished_operations` unfinished operations,
  returns `ControlError::RateLimited { retry_after }`. It creates no operation and
  no record. An existing operation returned for an idempotent or identical retry
  is not a new write and is not counted.
- Otherwise the operation is admitted as `Submitted` (an Operator's is `Allowed`,
  as now) and the first snapshot returns.

### The operation task

`run_operation` in `service.rs` stays the one caller of `operate`. The Agent
logic is in `service/agent.rs`, which returns either `Stop(resolution)` or
`Dispatch(precondition, settlement)`. In order:

1. Policy `Unavailable`, or the ledger not rebuilt: `Stop` as `rejected`.
2. Take a lease (connect and synchronize). A failure is `rejected`, as now.
3. **The atomic step**, under the ledger mutex and with no await: read
   `session.state().latest()`; `Ledger::recent`; `policy::evaluate`; for `Allow`,
   `Precondition::capture(&state, baseline.fields)` (no epoch is `rejected`); then
   `Ledger::reserve` for a volume change.
4. `Deny` is `denied`; `RequireApproval` is `approval_unavailable` (the broker joins
   in milestone 7). Each appends `Decided` and stops. A failed append here is
   logged and marks audit failing; the operation is already refused.
5. `Allow` moves `Submitted` to `Allowed` (cancellable), appends `Decided`, then
   appends `Dispatching` as `Synced`. Every append is bounded by 5 seconds, and a
   timeout is a failure, so a stalled disk cannot hold a lease or block shutdown.
   A failure of either releases the reservation and ends `rejected`, "audit log
   unavailable". A cancel that won meanwhile releases the reservation and ends
   `cancelled`.
6. `begin_session`, then `operate(OperationRequest::new(id, intent)
   .with_precondition(p))`, once. A mismatch comes back as `rejected` and is never
   retried.
7. `finish`; the ledger entry settles by `dispatch`; `Finished` appends as
   `Flushed`. A failed append marks audit failing and changes nothing the caller
   sees.

The Operator path keeps its steps, and on a service built by `start` adds the
`Decided`, `Dispatching`, and `Finished` records (each a warning on failure, not a
refusal) and the ledger entry for a volume write.

**Audit health** is the result of the last append. It recovers by itself: every
Agent write tries `Dispatching`, so a log that works again is noticed by the next
one.

### Statuses of the faults

| Condition | Status | `dispatch` |
| --- | --- | --- |
| Rule `Deny` | `denied` | `not_dispatched` |
| Rule `RequireApproval` | `approval_unavailable` | `not_dispatched` |
| Policy unavailable; ledger not ready; no epoch; audit append failed before dispatch; receiver unreachable | `rejected`, with the reason | `not_dispatched` |
| Session refuses (precondition mismatch, unsupported value) | `rejected` | `not_dispatched` |
| Cancelled before the session | `cancelled` | `not_dispatched` |
| Session outcome | as the [lifecycle table](../planned-architecture.md#operation-lifecycle) | as reported |

## Files

| File | Change |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | Member `crates/policy`; `serde_json` and `sha2` in infrastructure |
| `crates/domain/src/wall_time.rs` | New |
| `crates/policy/{Cargo.toml,src/{lib,intent,level,config,evaluate}.rs}` | New |
| `crates/policy/tests/{cases,properties}.rs` | Table cases and seeded properties |
| `crates/application/src/{clock,audit,policy_source,ledger}.rs` | New |
| `crates/application/src/control.rs` | Port additions |
| `crates/application/src/service.rs`, `service/agent.rs` | The Agent path; `service/agent_tests.rs` reuses the harness in `tests.rs` |
| `crates/infrastructure/src/{data_directory,audit_jsonl,policy_yaml,system_clock}.rs` | New |
| `crates/infrastructure/tests/live_x3800h_agent_gate.rs` | Armed live test |
| `apps/cli/src/main.rs`, `crates/gui-lib/src/bridge.rs` | One-line stubs for the new port methods in their test fakes |
| `tools/check-boundaries.sh`, `Makefile` | Edges, purity, resolved graph; `test-live-x3800h-agent` |
| `docs/examples/policy.yaml` | The owner's configuration as a sample |
| `ARCHITECTURE.md`, `AGENTS.md`, `docs/README.md` | Promotion at step 7 |

## Boundary rules added

- Edges `policy → domain`, `application → policy`, `infrastructure → policy`. No
  other edge involves `policy`.
- `crates/policy/src` contains no `async`, `.await`, `tokio`, `serde`, file or
  network type, `SystemTime`, or `Instant`.
- `cargo tree -p denon-avr-policy --edges normal` lists no package but itself and
  `denon-avr-domain`. This is the first use of the resolved graph, which milestone
  4's transitive rules reuse (roadmap finding 7).
- The `operate` allowlist is unchanged: the only call stays in `service.rs`.

## Decisions

**Answered by the owner on 2026-10-08.**

- **D6. Audit in the CLI and GUI.** A null sink: only a service built by `start`
  has an audit log, and only `api-server` will build one. Until milestone 5 the
  CLI and GUI operations are unaudited.
- **D7. The budget counts Operator volume changes.** Operator volume writes enter
  the ledger as observed levels, never as limited ones. Consequences, which the
  interview put only in part: after the owner lowers the volume, an agent's budget
  is measured from the lower floor; and after the owner *raises* it, the low level
  it rose from stays in the window, so with the owner's limits an agent cannot
  raise the volume even 0.5 dB for 10 minutes after the owner went from -60 to -45
  (the rise is 15.5 above the floor -60). An agent can still lower it. The Operator
  path of a service built by `start` writes ledger entries and `Dispatching`
  records. Reversing D7 means the floor ignores Operator entries.
- **D8. Caps and audit size.** 30 writes a minute and 8 unfinished operations per
  label; 20 MiB and 10 files of audit.
- **D9. Policy file.** `policy.yaml` beside the configuration. A missing or invalid
  file at start disables agent writes, and so does a failed reload (strict). No
  time-of-day dimension. The sample carries the owner's limits.

**Settled by the review.** Any can be reversed in a follow-up commit.

| # | Decision | Reason |
| --- | --- | --- |
| D10 | `dry_run` is a pair of port methods, not a submission flag, and is not audited | A dry run is not an operation, and the result is a decision, not a snapshot |
| D11 | `Allow` carries the baseline, and the baseline is every field an applicable rule read | R1, R2 |
| D12 | Classification is per agent; a rule naming no intent restricts without classifying; `allow` rules only classify | R9; the roadmap's "restricts that agent and leaves another unchanged" needs it |
| D13 | One evaluation under the lease; no separate re-evaluation until approval exists | R8 |
| D14 | `new` unchanged, async infallible `start` | R12; the CLI and GUI are untouched |
| D15 | Add `serde_json` and `sha2` to infrastructure only; no `proptest` | R10; the implementer needs crates.io access once |
| D16 | Faults end `rejected`, rule outcomes end `denied` and `approval_unavailable`; a cap is `RateLimited` | The design's fail-closed table says "rejected"; `denied` is a decision |
| D17 | `ReceiverReads::health` carries coarse states only | Agents may call it |
| D18 | `OperationSnapshot` is unchanged | Rule ids stay in audit and Operator views |
| D19 | A `Minimum` volume is -80.0 dB in rules | R17 |
| D20 | A decrease is exempt from the step and budget conditions only. `target_above_db` limits still apply to it | The design's rule order lists the target limits first. Consequence: with the volume at -15 dB, an agent can lower it to -30 or below but not to -18, which is above the hard limit and ends `denied` |
| D21 | Classification is by kind and, when a rule gives one, value: a rule naming `system_power` with value `standby` classifies standby only | Classifying by kind alone would turn a value-scoped `allow` into an `Allow` for the other value, so dropping the `system-power` rule would allow power-on. No table row changes |
| D22 | `PolicyConfig::new` also refuses an `allow` rule that names no intent, a value that does not belong to its intent, and an empty `agents` or `receivers` list | Each reads like a rule and does nothing, which the validation exists to prevent |

## Live checks

`crates/infrastructure/tests/live_x3800h_agent_gate.rs`, run by
`make test-live-x3800h-agent`, is `#[ignore]` and armed exactly like the controls
test: `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and
`DENON_X3800H_SAFE_VOLUME_HALF_STEPS`. It starts a service with the real
connector, a temporary policy file, and a temporary audit directory, and uses an
Agent handle.

The policy limits are built from the observed level `L` so that no refused target
can exceed the declared safe level `S` even if the gate were wrong: hard limit
`L + 3.0`, ceiling `L + 1.5`, step and budget as in the sample. The test asserts
`L + 3.5 <= S` before it writes anything, then:

1. Volume to `L + 1.0`: `completed`, receiver shows it.
2. Volume to `L + 2.0`: `approval_unavailable`, receiver unchanged.
3. Volume to `L + 3.5`: `denied`, receiver unchanged.
4. Restore `L` as the Operator.
5. The audit directory, whose path the test prints, holds the `Decided`,
   `Dispatching`, and `Finished` records of step 1 and a `Decided` and `Finished`
   for each refusal, with mode 0600.

The Operator side is covered by the phase 1 controls run, repeated.
