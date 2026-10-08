# Version 4, Phase 3 — Policy, audit, and the Agent path

**Implemented, 2026-10-08, on the branch `v4/phase-3-policy-audit-gate`; the owner's
live checks are open.** Every step is done and `make check`, `make clippy`, and
`git diff --check` pass, but nothing has been run against a receiver: see the
[exit status](#exit-status). This is milestone 3 of the [roadmap](roadmap.md). It adds the `policy`
crate, the audit log, and the Agent path through the Operation Gate, all in
process: nothing is exposed until milestone 4, and no Operator-visible behavior
changes. It is the first code that lets anything but the Operator write to the
receiver, so it is built to fail closed.

The first draft of this milestone, in the roadmap, had faults that would have
shipped a gate with no baseline on `Allow`, a budget that two agents could split,
and an audit log written after the dispatch it had to record. The
[architecture](phase-3-policy-audit-gate-architecture.md#what-the-review-changed)
lists the nineteen corrections and holds the types and decisions. The rules the
phase relies on are in [Planned architecture](../planned-architecture.md).

`main` contains phase 2's code, and its two owner checks are still open. They do
not block this phase: it touches no GUI code, only a test fake in
`crates/gui-lib/src/bridge.rs`.

## Steps

Work goes one numbered step at a time: `make check`, `make clippy`, and
`git diff --check` pass, the diff is reviewed, and the step is committed. A step
that changes `tools/check-boundaries.sh` lands with the code it checks, because
the script fails if either half lands alone.

| # | Step | State |
| --- | --- | --- |
| 0 | Amend the design | Done |
| 1 | `policy` crate, `WallTime`, boundary rules | Done |
| 2 | Port additions: dry run, health, policy and audit views | Done |
| 3 | `Clock` and audit ports; JSON Lines adapter | Done |
| 4 | Policy loading, digest, sample file | Done |
| 5 | Ledger | Done |
| 6a | Gate: Agent path, evaluation, and fail-closed outcomes | Done |
| 6b | Gate: dispatch with the precondition, audit order, live test | Done |
| 7 | Promote to `ARCHITECTURE.md`; exit | Done, live checks open |

The roadmap's milestone 3 summarizes the same steps; this phase splits step 6
into 6a and 6b, where a reviewer could reject one and approve its neighbor.

### Step 0 — Amend the design (done)

`planned-architecture.md` now says: the baseline is every state field an
applicable rule read and comes with `Allow`; classification is per agent, a rule
naming no intent restricts without classifying, and an `allow` rule only
classifies; the budget's exact definition over a ledger of agent and Operator
volume changes, with the clock rule; the Audit section and its write order;
strict policy loading; no time-of-day dimension; `dry_run` semantics; and the
evaluation-under-a-lease note. The roadmap's milestone 3, finding 5, consequences,
risks, and gaps table are corrected to match.

### Step 1 — `policy` crate

Files: `crates/domain/src/wall_time.rs`; `crates/policy/{Cargo.toml,src/{lib,intent,level,config,evaluate}.rs}`;
`crates/policy/tests/{cases,properties}.rs`; `Cargo.toml`; `tools/check-boundaries.sh`.
Signatures are in the architecture's [policy](phase-3-policy-audit-gate-architecture.md#policy)
section. The crate has no dependency but `denon-avr-domain`.

Tests in `cases.rs`:

- `the_owners_configuration_decides_each_table_case`: every row of the
  architecture's table, as a built `PolicyConfig`.
- `an_unmute_that_no_rule_matched_still_carries_the_volume_baseline`.
- `a_rule_naming_only_an_agent_denies_every_intent_for_that_agent`.
- `a_rule_narrowed_to_one_agent_does_not_classify_for_another`: agent B's volume
  change is `RequireApproval` with `Unclassified` when only agent A's rule names
  volume.
- `a_rule_naming_only_an_agent_restricts_without_classifying`: a rule that names
  no intent and does not match leaves the intent unclassified.
- `a_value_scoped_allow_does_not_classify_the_other_value` (decision D21).
- `every_limit_is_strict_so_a_value_exactly_at_it_is_not_over_it`: the ceiling,
  the hard limit, the step, the budget, and a change exactly as old as the window.
- `an_allow_rule_with_a_condition_is_rejected`, and the same for a volume-only
  condition on a non-volume alternative; `an_allow_rule_naming_no_intent_is_rejected`,
  `a_value_that_does_not_belong_to_the_intent_is_rejected`, and
  `rule_ids_are_unique_and_non_empty_and_lists_are_not_empty` (decision D22).
- `minimum_is_a_decrease_as_a_target_and_overstates_a_rise_as_a_baseline`.
- `every_intent_kind_is_mapped`: an exhaustive `match` over `ReceiverIntent` in the
  test, so a new variant fails to compile here as well.
- `the_extremes_of_the_scale_do_not_overflow`: targets and baselines at
  `Minimum`, -79.5, and +18.0 in every pairing.
- `an_empty_policy_requires_approval_for_everything`.

Seeded properties in `properties.rs` (64 seeds, as `receiver_properties.rs` does):
`raising_the_target_never_lowers_severity`,
`a_decrease_from_a_usable_level_is_never_held_by_the_step_or_the_budget`,
`an_unusable_volume_never_allows_a_state_dependent_rule`,
`an_allow_rule_never_weakens_a_deny`,
`alternating_steps_never_reset_the_budget`,
`a_future_dated_entry_still_counts`, `evaluation_is_deterministic`.

Boundary script: the three edges, the purity check, and the resolved-graph check
(see the architecture's [boundary rules](phase-3-policy-audit-gate-architecture.md#boundary-rules-added)).

### Step 2 — Port additions

`crates/application/src/control.rs`: the methods and types in the architecture's
[port additions](phase-3-policy-audit-gate-architecture.md#port-additions).
`ServiceHandle` answers `ControlError::Unavailable` for each until steps 3 to 6
fill them in, which fails closed. Add a one-line stub to `Stub` in `control.rs`, to
`Fake` in `apps/cli/src/main.rs`, and to `FastPort` in `crates/gui-lib/src/bridge.rs`.
Extend `the_stub_fails_every_call_with_a_typed_error` to the new methods; the
object-safety test (`one_handle_narrows_to_each_surface_without_new_objects`)
must still compile. The types the views name come with them: the audit data
types in `application::audit` and `PolicyDigest` in `application::policy_source`
(tests `a_page_is_at_least_one_record_and_never_more_than_the_cap`,
`a_digest_prints_as_64_lowercase_hex_digits_and_reads_back`, and
`the_health_dry_run_policy_and_audit_views_fail_closed_until_the_agent_path_exists`,
which also pins that the Operator-only views are `Forbidden` to an agent,
whatever the service can answer).

### Step 3 — Clock and audit

`application::{clock,audit}`; `infrastructure::{data_directory,audit_jsonl,system_clock}`;
`serde_json` in infrastructure. Tests in `crates/infrastructure/tests/audit_jsonl.rs`:
`records_round_trip_in_order`, `rotation_keeps_the_newest_files_within_the_size_limit`,
`since_and_query_read_across_the_retained_files`,
`a_truncated_last_line_and_unknown_kinds_are_skipped`,
`agent_text_is_bounded_and_escaped_in_the_line`: a source id or sound-mode name
with a newline, a quote, and 10,000 characters stays one valid line of at most
128 characters of intent text,
`files_are_0600_in_a_0700_directory`, `wider_existing_permissions_are_an_error`,
`seq_continues_after_reopening`, `a_synced_append_returns_after_writing`,
`a_log_that_does_not_exist_yet_reads_as_empty_and_is_not_created`, and
`concurrent_appends_never_interleave_lines`. `SystemClock` and `data_directory`
have unit tests. What a test cannot observe is that `sync_data` runs; that is
read from the code.

### Step 4 — Policy loading

`application::policy_source`; `infrastructure::policy_yaml`; `sha2` in
infrastructure; `docs/examples/policy.yaml`. Tests in
`crates/infrastructure/tests/policy_yaml.rs`:
`the_sample_loads_and_has_the_expected_digest` (the digest is a constant computed
independently), `the_sample_classifies_every_intent_kind_for_any_agent`,
`unclassified_allow_is_rejected`, `or_unknown_false_is_rejected`,
`a_limit_off_the_half_decibel_grid_is_rejected`,
`an_allow_rule_with_a_condition_is_rejected`, `a_time_of_day_key_is_rejected`,
`a_duplicate_rule_id_is_rejected`, `an_oversize_file_is_rejected`,
`a_missing_file_is_reported_as_missing`, `one_changed_byte_changes_the_digest`,
and those for what the loader fixes (see the architecture's
[loader](phase-3-policy-audit-gate-architecture.md#policy-file-and-loading)):
`an_empty_file_is_an_empty_policy_that_requires_approval_for_everything`,
`a_budget_needs_both_its_limit_and_its_window`,
`unknown_keys_are_rejected_by_name_wherever_they_are`,
`what_a_rule_names_must_make_sense`,
`a_rule_can_be_narrowed_to_agents_and_receivers`,
`the_approval_lifetime_is_in_minutes_and_has_a_default`,
`a_file_that_cannot_be_read_is_unreadable_not_missing`,
`an_edit_is_seen_by_the_next_load`, and
`errors_name_the_rule_or_key_and_never_quote_the_file`. If you edit
`docs/examples/policy.yaml`, compute its digest again with `shasum -a 256` and
change the constant in the test.

### Step 5 — Ledger

`application::ledger`. Unit tests: `a_settled_dispatch_keeps_its_entry`,
`a_settled_non_dispatch_removes_it`, `rebuild_keeps_dispatching_without_finished`,
`rebuild_drops_a_finished_not_dispatched`, `rebuild_ignores_finished_without_dispatching`,
`rebuild_counts_operator_volume_writes`,
`two_runs_with_the_same_operation_id_do_not_collide`,
`a_future_dated_entry_still_counts_and_is_never_pruned`,
`entries_older_than_24_hours_are_pruned` (an entry exactly 24 hours old stays),
and: `recent_returns_what_is_inside_the_window_oldest_first`,
`rebuild_keeps_every_finish_that_may_have_written`,
`rebuild_does_not_pair_a_finish_from_another_run`,
`rebuild_counts_only_volume_writes_inside_the_day`,
`rebuild_of_a_volume_it_cannot_read_assumes_the_lowest_level` (a level off the
scale reads as the lowest, which makes the budget harder to stay inside), and
`a_rebuilt_ledger_answers_as_the_live_one_did`.

### Step 6a — Agent path: evaluation and refusals

`service.rs`, `service/agent.rs`, `service/agent_tests.rs`. `ControlService::start`,
`AgentPath`, `AgentLimits`, the caps, `dry_run`, `dry_run_as`, `health`, `policy`,
`reload_policy`, `audit`. An `Allow` ends `rejected` with the reason "agent
dispatch is not enabled" until 6b, which keeps the intermediate commit fail
closed. Tests:

- `an_agent_handle_needs_a_service_built_by_start`.
- `a_deny_ends_denied_and_a_require_approval_ends_approval_unavailable`, each
  asserting the fake session's `operate` was never called.
- `an_unavailable_policy_rejects_every_agent_write_and_leaves_reads_and_operator`.
- `a_failed_reload_disables_agent_writes_until_a_good_one`.
- `an_unreadable_audit_log_keeps_the_ledger_unready_and_writes_rejected`.
- `a_dry_run_returns_the_decision_and_creates_no_operation_or_record`, and
  `dry_run_as_applies_the_named_agents_rules`.
- `the_write_cap_refuses_with_rate_limited_and_creates_nothing`, and the same for
  the unfinished-operation cap.
- `a_dry_run_counts_against_the_write_cap`.
- `an_idempotent_retry_is_not_a_new_write`.
- `a_service_built_by_new_audits_nothing_and_serves_no_agent`.
- Also: `an_agents_request_starts_submitted_and_an_operators_starts_allowed`,
  `a_decision_records_what_it_read`,
  `an_unreachable_receiver_is_reported_to_an_agent_without_detail`,
  `an_audit_log_that_cannot_be_read_is_reported_without_its_text`,
  `the_operator_reads_the_audit_log_through_the_port`,
  `a_healthy_start_reports_every_part_working`, and, in `tests.rs`,
  `the_administration_views_are_the_operators_alone_whatever_the_service_can_answer`,
  which replaced the step 2 test of the same subject now that `health` and the
  Operator's `dry_run` have answers on a service built by `new`.

### Step 6b — Agent path: dispatch

The atomic step, the precondition, the `Dispatching` record before `operate`,
settlement, and the Operator path's records and ledger entries. Add
`crates/infrastructure/tests/live_x3800h_agent_gate.rs` and `make test-live-x3800h-agent`.
Tests in `agent_tests.rs`, with a fake session, fake clock, and fake audit log
that records the order of its calls against the session's:

- `an_allow_dispatches_once_with_a_precondition_naming_its_baseline`.
- `an_unmute_precondition_carries_the_volume_though_the_loud_rule_did_not_match`:
  the fake session reports `PreconditionMismatch` when the volume changed after
  evaluation; the status is `rejected`, `operate` was called once, and nothing
  retried.
- `no_established_epoch_rejects_without_dispatch`.
- `dispatching_is_appended_before_operate_and_a_failed_append_rejects`, and
  `an_audit_append_that_never_returns_is_a_failure_after_five_seconds` on the
  paused clock, with `shutdown` still completing.
- `an_operator_continues_when_audit_fails_and_health_says_failing`.
- `two_agents_each_within_the_budget_cannot_together_exceed_it`: the first
  agent's rise is held inside the session and the receiver has applied it; the
  second agent's rise, within every limit alone, needs approval because of the
  first agent's reservation and only that. A test that built the two requests from
  absolute targets against one unchanged level would pass with no reservation at
  all.
- `a_cancel_before_the_session_releases_the_reservation`, and the two other
  places a withdrawal can win: `a_cancel_while_the_receiver_connects_dispatches_nothing`
  and `a_cancel_while_the_dispatching_record_is_written_sends_nothing`.
- `a_restart_rebuilds_the_ledger_from_audit`: a second `start` over the same
  audit log; includes a `Dispatching` with no `Finished`.
- `every_session_outcome_settles_the_ledger_by_dispatch`: all six outcomes.
- `an_operator_volume_write_enters_the_ledger`.
- `a_superseded_agent_operation_releases_its_reservation`.
- `an_idempotent_retry_dispatches_once`.
- `an_allow_that_read_nothing_still_carries_the_epoch`,
  `only_the_record_made_before_a_write_is_synced_to_disk`,
  `the_budget_forgets_changes_older_than_its_window`, and
  `what_the_operator_did_while_the_log_was_unreadable_survives_the_late_rebuild`.

`a_restart_rebuilds_the_ledger_from_audit` covers a log of a finished run, a
`Dispatching` with no `Finished`, and an Operator write. Each test that moves the
volume uses a fake session that applies the write to its own state and refuses a
write whose precondition no longer holds, so the harness cannot pass for want of
an established epoch or a usable volume.

The live test is `#[ignore]` and refuses to run unless armed. It was compiled and
its Makefile target checked to refuse when unarmed; it was **not run**, because
running it writes to a receiver.

### Step 7 — Promote and exit

Move the implemented design into `ARCHITECTURE.md` (a "Policy and audit" section,
the Agent path in "Control service", and the `policy` package in the workspace
table and graph), add `crates/policy` to the repository map in `AGENTS.md`, and
leave pointers in `planned-architecture.md` as phases 1 and 2 did. Write the live
record in `docs/archive/v4/phase-3-live-validation-record.md` with every result
**To fill**, update the exit status below, and update the documentation map.

## Exit criteria

- `make check`, `make clippy`, and `git diff --check` pass.
- Policy tests: a table case per rule of the owner's configuration, and the
  seeded properties of step 1.
- An `Allow` carries its baseline, including a field read only by a rule that did
  not match, and a change in that field before the write ends the operation
  `rejected` with `operate` called once.
- Gate tests cover allow, deny, `approval_unavailable`, policy unavailable, a
  failed audit append, a failed precondition, supersession, and an idempotent
  retry. Each asserts no dispatch on every non-allowed path and at most one on an
  allowed one.
- Two agents each within the budget alone cannot together exceed it.
- A rule narrowed to one agent label restricts that agent and leaves another
  agent's decisions unchanged, including a read-only label.
- A restart rebuilds the ledger from audit, counting a `Dispatching` record with
  no `Finished` record and the Operator's volume writes.
- The audit adapter rotates by size and file count, reads across the retained
  files, and creates owner-only files. An audit failure rejects Agent writes and
  lets Operator controls continue, and `health` says so.
- The policy loader rejects what the architecture lists, the digest changes with
  the file, and a failed reload disables Agent writes until a good one.
- `make boundary` checks the `policy` edges, its purity, and that its resolved
  graph reaches only `domain`.
- Live, armed (`make test-live-x3800h-agent` and the phase 1 controls run): the
  Operator controls still pass, and the in-process Agent run completes a volume
  change within the limits, is refused above the ceiling and above the hard limit
  without a change at the receiver, and restores the volume.

## Inputs the specification is silent on

The design says what the software must do, not everything it will meet. These are
the inputs most likely to bite, most likely first. Each has a named test above.

1. **A misspelt agent label in a rule.** Labels match exactly and case-sensitively,
   so `agents: [Claude-Code]` silently leaves `claude-code` under the general
   rules, and a read-only tier is not read-only. Nothing in this phase can catch
   it, because tokens do not exist until milestone 4. `dry_run_as` is how the owner
   checks a tier, and milestone 6's exit already requires Claude Code's writes to
   be refused by policy.
2. **Agent text in an audit line.** A source id or sound-mode name with a newline,
   a quote, or 10,000 characters (`agent_text_is_bounded_and_escaped_in_the_line`).
3. **A stalled audit disk.** An append that never returns must fail, not hold a
   lease or block shutdown
   (`an_audit_append_that_never_returns_is_a_failure_after_five_seconds`).
4. **The ends of the volume scale.** `Minimum` and +18.0 as target and baseline
   (`the_extremes_of_the_scale_do_not_overflow`).
5. **No rules at all.** An empty file is valid and fails closed
   (`an_empty_policy_requires_approval_for_everything`).

## Exit status

**Implemented; the live exit criteria are open.** Steps 0 to 7 are done. Every exit
criterion below that a test can show is shown by a test in this branch, and
`make check`, `make clippy`, and `git diff --check` pass. The last criterion, the
armed live run, has **not** been run: it writes to a receiver, and the session that
built the phase had none. The same goes for the Operator regression and the audit
permissions on the Mac. The [validation record](../archive/v4/phase-3-live-validation-record.md)
lists each with its result **To fill**, and the branch is not to be merged until
the owner has filled it.

Two things were checked in place of the live run: the live test compiles under
`--all-targets`, and `make test-live-x3800h-agent` refuses when `ALLOW_RECEIVER_WRITES`
is not `1`.

## Known limits

- Nothing exposes the Agent path before milestone 4. Only tests and the armed
  live test reach it, with an in-process handle.
- The CLI and GUI are unaudited until milestone 5, because only a service built
  by `start` has an audit log (decision D6).
- While the observed volume is unknown or stale, every agent volume change needs
  approval and so ends `approval_unavailable` in 4.0.0, a decrease included. The
  same holds for a main-zone power-on or unmute.
- A target above the -20 dB hard limit ends `denied`; one between -30 and -20 dB
  ends `approval_unavailable`. The second outcome becomes approvable in
  milestone 7.
- The cumulative budget measures from the Operator's own lowest level too
  (decision D7). Lowering the volume yourself lowers the floor an agent's next
  rise is measured from, and after you raise it (say -60 to -45) the level you rose
  from stays in the window, so with the owner's limits an agent cannot raise it
  even 0.5 dB for 10 minutes. It can still lower it.
- A decrease is exempt from the step and the budget but not from the target
  limits (decision D20): with the volume at -15 dB an agent can lower it to -30 or
  below, not to -18.
- A forward step of the system clock can expire ledger entries early. A backward
  step cannot shrink the budget.
- The audit log is not tamper-proof against a process running as the same user,
  as the design states.
- Adding `serde_json` and `sha2` changes `Cargo.lock`, and fetching them needs
  network access once.

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac.

1. **Loopback tests.** The test suite binds loopback sockets. Run `make check`
   outside a sandbox that blocks local ports, or allow local binding for it.
2. **Crates.** Step 3 and step 4 add `serde_json` and `sha2`. Run `cargo fetch`
   once with network access before the sandboxed `make check`.
3. **Operator regression.** The phase 1 live checks through the CLI: read-only
   `get status`, then the armed, state-restoring controls run, with the model,
   firmware, settings, commands, responses, and date recorded.
4. **Agent live run.** With `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and
   `DENON_X3800H_SAFE_VOLUME_HALF_STEPS` set to a level you accept, run
   `make test-live-x3800h-agent`. The test asserts that the highest target it
   will name stays at or below the declared safe level before it writes. Record
   the result in the validation record.
5. **Audit permissions.** After the run, `ls -ld` and `ls -l` on the audit
   directory the test printed show `drwx------` and `-rw-------`.
