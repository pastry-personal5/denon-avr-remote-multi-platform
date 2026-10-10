# Version 4, Phase 5 — Operator clients cut over

**Planned 2026-10-09. Nothing is implemented. The owner answered the questions the
plan could not settle itself, listed in the architecture's
[decisions](phase-5-operator-clients-architecture.md#decisions): the configuration
revision goes through the port, approvals are a placeholder, the new views live inside
Advanced, a lost server is not restarted without being asked, the receiver's 60-second
hold is accepted and a `release` command is added, and the Operations feed names who
acted. See [Exit status](#exit-status).**
This is milestone 5 of the [roadmap](roadmap.md), the first milestone that changes what
users run. The CLI and the GUI stop composing their own receiver service and become
clients of the Control API server that phase 4 built. One process then owns the receiver
connection, so the CLI and the GUI work at the same time, and everything the Operator does
is audited. The GUI gets four new views, the CLI gets the Agent-token commands and a real
dry run, and the macOS bundle carries the server.

The roadmap's milestone 5 left a client with no way to find the server or read its token, a
configuration save that can still overwrite another client's, a launcher with no readiness
signal or race rule, a packaging step that assumed a code-signing identity nobody had
checked, and a list of baselines to keep that never existed. The
[architecture](phase-5-operator-clients-architecture.md#what-the-review-changed) lists the
sixteen corrections and holds the types, the launcher's algorithm, and the decisions. The
rules the phase relies on are in [Planned architecture](../planned-architecture.md).

`main` (`79fd1dd`) contains phases 1 to 4 and the clean-architecture refactor. Phase 4's
live checks are still open, and they now apply to the refactored session actor; step 0c
makes them a gate.

## Steps

Work goes one numbered step at a time: `make check`, `make clippy`, and
`git diff --check` pass, the diff is reviewed, and the step is committed. A step that
changes `tools/check-boundaries.sh` lands with the code it checks, because the script fails
if either half lands alone. Steps 0b and 0c need the Mac, the receiver, and the owner.
The work is on branch `v4/phase-5-operator-clients`; this plan is on
`docs/v4-phase-5-plan`.

| # | Step | Gate | State |
| --- | --- | --- | --- |
| 0 | Amend the roadmap and the design | none | Done with this plan |
| 0b | S5: Local Network permission for the nested server, by hand | none | Done 2026-10-10: outcome B (D80) |
| 0c | Phase 4's armed live run and account-boundary run, on `main` | none | Planned |
| 1 | The configuration revision through the port | none | Planned |
| 1b | The principal on the operation snapshot | none | Planned |
| 1c | Receiver release: the port method and route | none | Planned |
| 2 | Client bootstrap: data directory, token file, "no server" message | none | Planned |
| 3 | CLI cutover | 0c | Planned |
| 4 | The launcher | 0c | Planned |
| 5 | GUI cutover | 0c | Planned |
| 6 | Advanced tabs and the Operations feed | none | Planned |
| 7 | The Audit tab | none | Planned |
| 8 | The Policy tab | none | Planned |
| 9 | The Approvals placeholder and the Diagnostics server panel | none | Planned |
| 10 | Packaging | 0b | Planned |
| 11 | Promote and exit | none | Planned |

The roadmap lists six steps; this phase splits them where a reviewer could reject one and
approve its neighbor, and adds the revision work, the two port additions the owner asked
for, and the client bootstrap that the roadmap assumed existed. Steps 1, 1b, 1c and 2 need
no receiver and can be built before 0c. Steps 1, 1b and 1c reopen phase 4's port, so each
is its own commit and its own review.

### Step 0 — Amend the roadmap and the design (done)

The roadmap's milestone 5 is rewritten to match this plan (the steps, the exit criteria,
the dropped Windows and Linux baselines, the placeholder). `planned-architecture.md` says
in its process model that a GUI whose server is lost does not start another, and in its
GUI and CLI section what `--dry-run`, `tokens`, and the Advanced tabs are. No code changes.

### Step 0b — S5: Local Network permission for the nested server, by hand

The owner's, on the Mac, with the receiver reachable. What it decides and the three possible
outcomes (A, B, C) are in the architecture's
[S5](phase-5-operator-clients-architecture.md#s5-what-the-hand-check-decides). The instructions
are the [runbook](../research/local-network-permission-server-macos.md), which uses throwaway
programs (archived on 2026-10-10 in `docs/archive/s5/tools/`): `build-bundles.sh`, the stand-in GUI
`app/` and its helper `host/`; the note's **Outcome** section is S5's output. It gates step 10 only; nothing
else in the phase waits for it. **Done 2026-10-10: the owner confirmed outcome B** (D80): the
nested server is signed `com.denonavr.remote.server` and the bundle `com.denonavr.remote`, inside
out. Outcome C was not needed.

### Step 0c — Phase 4's live runs

The owner's. On `main`, close the GUI and CLI and run what the phase 4 checklist lists:
the armed `make test-live-x3800h-api` (items 7 and 8) and the account-boundary run against
the real server (item 5), then record the results in
[the phase 4 record](../archive/v4/phase-4-live-validation-record.md). The runs gate step 3
onward (D73): the cutover removes the CLI's own service, and a server that has never driven
the real receiver is the wrong thing to remove it for.

### Step 1 — The configuration revision through the port

Files: `crates/application/src/{control,config_edit,lib}.rs`, `service/ports.rs`,
`crates/api-contract/src/{error,admin/config}.rs`, `crates/api-client/src/{lib,transport}.rs`,
`apps/api-server/src/{routes,endpoint,start}.rs`, and the fakes that implement
`OperatorAdmin`: `apps/cli/src/tests.rs`, `crates/gui-lib/src/bridge.rs` (`FastPort`),
`crates/gui-lib/tests/async_apis.rs`, `apps/api-server/tests/support/mod.rs`.
Produces `ConfigRevision`, `RevisedConfiguration`, the two changed methods,
`ControlError::ConfigurationChanged`, and `edit_configuration`. The CLI's `remember` and the
GUI's saves go through the helper in steps 3 and 5; this step changes the port, the service,
the route, the client, and the fakes, and the existing callers pass the revision they read.

Tests:

- `a_save_over_a_stale_revision_is_refused_and_writes_nothing`.
- `a_save_returns_the_revision_of_what_was_stored`.
- `two_saves_from_one_revision_let_exactly_one_win`, two tasks and the same base.
- `a_change_made_outside_the_service_changes_the_revision`, the stored configuration
  edited under the service between a read and a save.
- `the_revision_differs_for_each_part_of_the_configuration` (current receiver, a name, a
  host, a model, a friendly name, a favorite) and `the_same_configuration_has_the_same_revision`.
- `text_that_is_not_sixteen_lower_case_hex_digits_is_not_a_revision`.
- `an_edit_that_changes_nothing_writes_nothing`,
  `a_conflict_applies_the_edit_to_the_fresh_configuration`,
  `three_conflicts_in_a_row_return_the_error`.
- In `api-contract`: `configuration_changed_is_412_precondition_failed_and_back`.
- In `api-server`: `get_config_sends_the_revision_as_the_etag`,
  `put_with_a_stale_if_match_is_412_and_the_file_is_unchanged`,
  `put_without_if_match_is_428`, `put_with_text_that_is_not_a_revision_is_400`.
- The port conformance run, `the_port_behaves_the_same_in_process_and_over_the_socket`,
  gains a stale save and expects the same error both ways.

### Step 1b — The principal on the operation snapshot

Files: `crates/application/src/{control,service/operations}.rs`,
`crates/api-contract/src/operations.rs` and its goldens, `crates/api-client/src/{lib,sse}.rs`,
and the 18 places that build an `OperationSnapshot`: the service, the Agent tests'
`fixtures.rs`, `crates/gui-lib/src/{bridge,feedback}.rs`, and tests.
Produces `OperationSnapshot.principal` and `OperationDto.principal` (the audit records'
`PrincipalDto`). Design in the architecture's
[step 1b](phase-5-operator-clients-architecture.md#principal-on-the-operation-snapshot-step-1b).

Tests:

- `a_snapshot_names_who_submitted_the_operation`, for the Operator and for an agent, and
  `every_later_snapshot_of_an_operation_keeps_its_principal`.
- `an_agents_snapshots_and_events_carry_only_its_own_label`: it sees no other principal.
- `the_operation_dto_round_trips_its_principal` and the regenerated goldens.
- The Agent leak tests still find no address, path, raw frame, or error text in any
  Agent response.
- The port conformance run compares the principal in process and over the socket, for both
  audiences, with no exception.

### Step 1c — Receiver release: the port method and route

Files: `crates/application/src/{control,service/connections,service/ports}.rs`,
`crates/api-contract/src/{routes,paths,admin/inspection}.rs` and goldens (the result type sits
beside `ReadinessDto`, which `refresh` returns),
the root module of `crates/api-client`, `apps/api-server/src/routes.rs`, and the fakes that implement
`OperatorAdmin` (the same list as step 1, and the `Stub` in `control.rs`).
Produces `OperatorAdmin::release`, `ReleaseOutcome`, and `POST /v1/receivers/{id}/release` on
the Operator endpoint. The CLI command is in step 3. Design in the architecture's
[step 1c](phase-5-operator-clients-architecture.md#receiver-release-step-1c).

Tests:

- `release_closes_an_idle_session_and_the_next_lease_connects_again`.
- `release_does_not_close_under_a_lease`: a held subscription, a read, and an operation in
  flight are each `InUse`, with the counts, and the session is still open and the operation
  still completes.
- `release_reports_not_connected_with_no_slot_and_with_an_empty_slot`, and
  `release_of_an_unknown_receiver_is_not_found`.
- `two_releases_at_once_close_the_session_once`.
- `release_writes_nothing_to_the_audit_log`.
- In `api-contract`: `each_release_result_round_trips` and its goldens.
- In `api-server`: `release_answers_each_result_as_documented`, and, from the generated refusal
  matrix, `an_agent_endpoint_does_not_serve_release` with an Agent token, the Operator token, and
  none.
- The port conformance run releases a connected receiver both ways and expects the same
  outcome.

### Step 2 — Client bootstrap

Files: `crates/api-contract/src/paths.rs`, `crates/api-client/src/{lib,bootstrap}.rs`,
`crates/api-client/Cargo.toml` (`rustix`), `crates/application/src/ports.rs` and
`crates/infrastructure/src/{discovery_ssdp,data_directory,config_yaml}.rs` (the discovery
constant moves), `apps/api-server/src/{main,routes}.rs`. Produces
`default_data_directory`, `Endpoint::operator`, `read_operator_token`, `BootstrapError`, and
`TokenFileError`. The path functions in `infrastructure` go once steps 3 and 5 have removed
their last callers; until then they stay, and this step does not delete them.

Tests:

- `the_default_data_directory_is_under_application_support` and
  `an_unset_or_empty_home_has_no_default`.
- `a_token_file_that_is_group_readable_is_refused`, `…that_is_world_readable_is_refused`,
  `a_symlinked_token_file_is_refused`, `an_empty_or_oversized_token_file_is_refused`,
  `a_token_with_a_space_in_it_is_refused` and `a_trailing_newline_is_trimmed`.
- `a_token_file_of_another_owner_is_refused`, on the pure check with a made-up uid, since
  a test cannot create a file owned by someone else.
- `a_missing_token_file_names_the_file`.
- `a_socket_path_over_the_limit_is_reported_with_its_length_by_both_clients`: the check is
  `Endpoint::operator`'s, so the CLI and the launcher show the same text.
- `the_message_for_a_stopped_server_names_the_socket_and_how_to_start_one`.
- `the_token_never_appears_in_an_error_or_in_debug`.
- `a_server_with_no_home_and_no_data_dir_is_a_usage_error_that_names_home`.

### Step 3 — CLI cutover

Gate: step 0c. Files: `apps/cli/src/{main,args,commands,target,render}.rs`,
`apps/cli/Cargo.toml`, `apps/cli/tests/serve.rs`, `apps/api-server/src/testing.rs`,
`apps/api-server/Cargo.toml` (the `testing` feature), `tools/check-boundaries.sh` (the CLI
edges and the source rule).
Consumes: `ApiClient`, `Endpoint::operator`, `edit_configuration`. Produces the new
grammar (`--data-dir`, `--dry-run [--as-agent LABEL]`, `tokens issue|list|revoke`), the
"outcome unknown" path, and the test support: the fake connector and session move from
`apps/api-server/tests/support/mod.rs` to `testing.rs`, behind the feature.

Tests in `apps/cli/src` (parsing, rendering) and `apps/cli/tests/serve.rs` (a real server in
process, a real `ApiClient`):

- `data_dir_is_accepted_before_the_command`; `tokens_issue_list_and_revoke_parse`;
  `as_agent_requires_dry_run`; `as_agent_with_a_host_or_a_receiver_number_is_refused_at_parse_time`;
  `an_agent_label_outside_the_label_rule_is_refused_before_any_request`.
- `get_status_reads_through_the_server` and `set_volume_waits_for_the_outcome`.
- `set_dry_run_as_agent_reports_the_agent_decision`, `set_dry_run_without_a_label_says_the_operator_is_not_subject_to_policy`,
  `a_dry_run_dispatches_nothing`, and `a_dry_run_does_not_save_the_configuration` (the fake
  records no save, with or without `--as-agent`).
- `a_get_with_no_server_says_how_to_start_one`.
- `a_server_that_stops_while_the_cli_waits_is_outcome_unknown_and_nothing_is_submitted_again`.
- `tokens_issue_prints_the_token_once_and_list_never_prints_it` and `tokens_revoke_by_id`.
- `release_frees_an_idle_receiver`, `release_reports_not_connected`,
  `release_reports_in_use_after_waiting_and_exits_2` (a held subscription; the wait is short
  in the test), `release_right_after_a_get_succeeds_once_the_closed_stream_is_seen`,
  `release_never_saves_the_configuration`, and `a_get_after_a_release_connects_again`.
- `release_accepts_the_same_selectors_as_get`: a saved receiver, `--host`, and `--receiver N`.
- `remember_writes_nothing_when_the_current_receiver_is_unchanged` and
  `remember_survives_a_save_by_another_client` (a second client saves between the read and the
  write; the edit is applied again).
- `a_usage_error_prints_the_usage_and_a_runtime_error_does_not`.
- The existing fake-based tests in `tests.rs` keep running against the same port.

### Step 4 — The launcher

Gate: step 0c. Files: `apps/desktop/src/launcher.rs`, `apps/desktop/tests/launcher.rs`,
`Makefile` (`test-launcher`), `tools/check-boundaries.sh` (only the launcher spawns a
process). Produces `Launcher::ensure`, `Attachment`, `Owner`, `LaunchError`, and the
release. It is not wired into `main` until step 5.

Tests, against the real `denon-avr-api-server` binary and scratch data directories under
`/tmp` (socket paths stay under 104 bytes):

- `no_server_means_spawn_and_the_launcher_owns_the_child`.
- `a_running_server_means_attach_and_nothing_is_spawned`.
- `a_socket_left_by_a_killed_server_is_reclaimed_by_a_spawn`.
- `two_launchers_at_once_end_with_one_server_and_one_owner`: the loser sees status 75 and
  attaches.
- `a_path_that_is_not_a_socket_is_failed_and_nothing_is_spawned` and
  `a_listener_that_never_answers_is_failed_and_nothing_is_spawned`.
- `a_missing_executable_is_reported_with_its_path`.
- `a_child_that_exits_early_is_failed_with_its_status_and_the_log_path`, with a script that
  exits 3.
- `a_noisy_child_does_not_stall_because_its_error_output_is_a_file`: a script writes 2 MiB
  to standard error and then runs the real server; the log grows and the launcher still
  attaches.
- `dropping_the_launcher_ends_the_child`, and `killing_the_parent_ends_the_child`, in which
  the test re-runs itself as the parent, kills it with `SIGKILL`, and waits for the child's
  process to go.
- `release_of_a_spawned_child_waits_for_it_and_returns_within_the_grace` and
  `release_of_an_attached_server_stops_nothing`.
- `a_contract_version_mismatch_is_incompatible_and_nothing_is_spawned`, against a fake
  responder on the socket.
- `a_group_readable_token_is_failed_and_not_attached`.
- `reopening_while_the_old_server_still_holds_the_lock_spawns_again_when_it_lets_go`: the
  test re-runs itself as a stand-in that holds `run/server.lock` for two seconds and then
  lets go, and the launcher ends up with one real server it owns.
- `the_log_is_0600_and_is_truncated_at_spawn_when_over_one_mebibyte`.
- `the_server_is_looked_for_beside_the_current_executable_and_nowhere_else`.

### Step 5 — GUI cutover

Gate: step 0c. Files: `apps/desktop/src/{main,link}.rs`, `apps/desktop/Cargo.toml`,
`crates/gui-lib/src/{bridge,lib,messages,state,shell,session,setup,sound_mode,server_status}.rs`,
`crates/gui-lib/tests/wire_round_trip.rs`, `crates/gui-lib/Cargo.toml` (the dev-dependency),
`crates/gui-lib/src/capture_scenario.rs`, `tools/capture-visual-baselines.sh`,
`tests/visual-baselines/macos/server-unavailable-*.png`, `tools/check-boundaries.sh` (the
desktop edges, the general rule, the `gui-lib` source rule).
Consumes the launcher, `ApiClient`, `edit_configuration`. Produces `OperatorLink`,
`ServerLink`, `ServerStatus`, the Dashboard banner with **Start server**, and the removal of
the last `infrastructure` edge from a delivery package. This step also deletes the path
functions in `crates/infrastructure/src/data_directory.rs` and `YamlConfigRepository::default()`
(with its test), whose last callers were the two compositions removed in steps 3 and 5.

Tests:

- `a_call_before_attach_is_unavailable_with_the_server_not_running_text` and
  `after_attach_calls_reach_the_client`.
- `unavailable_with_a_live_socket_is_not_a_server_problem` and
  `unavailable_with_a_dead_socket_sets_the_status`; `the_link_reports_the_server_back_when_the_socket_returns`.
- `a_server_from_another_build_that_appears_after_a_loss_is_incompatible_and_not_used` and
  `a_server_that_first_appears_after_a_failed_launch_gives_the_link_its_client`: only a
  completed attach sets `Running` (D75).
- `the_banner_shows_while_the_server_is_not_running`, `start_server_calls_the_link_once`,
  `the_gui_never_starts_a_server_unprompted_after_the_first_launch`, and
  `the_server_coming_back_selects_the_receiver_again`.
- `closing_runs_release_once_inside_the_grace` and `closing_with_an_attached_server_stops_nothing`
  (the existing closing tests, adapted).
- `capture_mode_never_calls_the_launcher`.
- `crates/gui-lib/tests/wire_round_trip.rs`: the projection and `FieldBaseline::capture` are
  equal for a state and for the same state rebuilt from the Operator view, for every
  validity class (`projection_of_a_rebuilt_state_equals_the_original`,
  `a_rebuilt_states_field_baselines_equal_the_originals`).
- The baseline `server-unavailable` at 100% and 200%.

### Step 6 — Advanced tabs and the Operations feed

Files: `crates/gui-lib/src/{advanced,operations}.rs` (with `settings_diagnostics.rs` and
`bridge.rs`), `messages.rs`, the capture scenarios. Produces `AdvancedTab`,
`Message::AdvancedTab`, the feed, and the bridge task for `operation_events()`.

Tests:

- `advanced_has_five_tabs_and_status_is_first` and
  `selecting_a_tab_changes_only_the_tab`.
- `the_feed_shows_the_latest_status_of_each_operation_newest_first`,
  `the_feed_keeps_the_newest_200_operations`, `a_missed_event_adds_its_line_once`, and
  `the_feed_opens_again_when_the_server_comes_back`.
- `every_intent_has_words` over each `ReceiverIntent`.
- `each_row_shows_the_operator_or_the_agents_label` (the principal column).
- Baselines `advanced-status` and `advanced-operations` at 100% and 200%.

### Step 7 — The Audit tab

Files: `crates/gui-lib/src/audit_view.rs`, the capture scenarios. Tests:

- `opening_the_tab_reads_the_newest_page`, `older_continues_from_the_cursor`,
  `the_tab_stops_at_500_entries_and_says_so`, `refresh_replaces_the_list`.
- `a_failed_read_shows_the_error_and_no_table`.
- `each_audit_event_has_a_one_line_summary`, over every `AuditEvent` variant, and
  `a_refused_caller_shows_no_principal`.
- Baseline `advanced-audit` at 100% and 200%.

### Step 8 — The Policy tab

Files: `crates/gui-lib/src/policy_view.rs`, the capture scenarios. Tests:

- `opening_the_tab_reads_the_policy` and `no_policy_file_says_so`.
- `reload_replaces_the_view`, and `a_failed_reload_shows_its_error_and_keeps_the_policy_shown`.
- `long_policy_text_is_capped_at_300_lines_with_a_note`.
- Baseline `advanced-policy` at 100% and 200%.

### Step 9 — The Approvals placeholder and the Diagnostics server panel

Files: `crates/gui-lib/src/{advanced,settings_diagnostics}.rs`, the capture scenarios. Tests:

- `the_approvals_tab_calls_nothing`: a port that records every call sees none.
- `the_approvals_tab_says_where_refused_requests_show`.
- `diagnostics_shows_the_server_panel_for_each_status`.
- Baselines `advanced-approvals` at 100% and 200%, and `diagnostics-100pct` and
  `diagnostics-200pct` captured again.

### Step 10 — Packaging

Gate: step 0b. Files: `tools/package-macos.sh`, `Makefile` (`run-gui` builds the server),
`docs/development.md`. Produces the bundle with `Contents/MacOS/denon-avr-api-server`,
signed inside out with the identity S5 chose (B), and the script's smoke step.

Checks (the script's own, since there is no test harness for it): the packaged server starts
under `--data-dir` in the work directory, its Operator socket appears within 5 seconds,
`SIGTERM` stops it with status 0, and the socket and lock are gone; `codesign --verify
--strict` passes for the server and the bundle; the script prints both identifiers. The
owner's checks are in the manual checklist.

### Step 11 — Promote and exit

Move the implemented design into `ARCHITECTURE.md` (the process model, the CLI and GUI as
clients, the launcher, the revision, the bootstrap), update the repository map in
`AGENTS.md`, leave pointers in `planned-architecture.md`, and remove the text the
implementation has made false, including every "unaudited until milestone 5" (phase 3's D6
and `planned-architecture.md`). Update the CLI and desktop user guides: no server means the
CLI fails; the receiver is released after 60 seconds; the GUI-owned server ends with the GUI;
the new tabs; `--data-dir`, `--dry-run --as-agent`, and `tokens`; macOS only. Write the live
record in `docs/archive/v4/phase-5-live-validation-record.md` with every result **To fill**,
update the exit status below, the documentation map, the roadmap, and `docs/development.md`.

## Exit criteria

- `make check`, `make clippy`, and `git diff --check` pass.
- `make boundary` shows, from the resolved graph, that `cli`, `desktop`, `gui-lib`,
  `api-client`, and `api-contract` reach neither `infrastructure` nor `protocol`; that the
  graphs of `cli` and `desktop` contain no `denon-avr-api-server` and no normal build enables
  its `testing` feature; and that only the launcher spawns a process. The temporary exceptions
  phase 4 wrote are gone.
- A stale configuration save is refused with the same error in process and over the socket,
  two saves from one revision let one win, and the CLI writes nothing when the current
  receiver is unchanged.
- The CLI's `set --dry-run --as-agent LABEL` returns the server's decision for that label,
  `tokens issue`, `list`, and `revoke` work against a real server, and a CLI with no server
  says how to start one.
- `denon-avr-remote release` closes an idle receiver connection at once, refuses to close one
  that something holds and says so, and an Agent token cannot reach the route.
- Every operation the Operator can read names the principal that submitted it, and the
  Operations feed shows it; an agent sees only its own label.
- A CLI that loses the server while it waits reports an unknown outcome and submits nothing
  again.
- The launcher attaches to a running server, spawns one when none is running, ends up with
  one server when two start together, and reports a missing executable, an early exit, a
  version mismatch, and a token it cannot trust, each with a message a person can act on.
- A killed GUI ends its child. A GUI attached to a standalone server leaves it running when
  it closes. A GUI that loses its server says so, keeps trying to re-attach, and starts one
  only when asked.
- The projection and the field baselines of a state rebuilt from the wire equal those of the
  original.
- Advanced shows the Operations feed, the Audit tab, the Policy tab with a working reload,
  and the Approvals placeholder; Diagnostics shows the server panel. The 12 baselines of
  unchanged screens are byte-identical; the new and re-captured ones are reviewed by the
  owner.
- The CLI and the GUI operate the receiver at the same time over one receiver connection,
  recorded as a live validation.
- S5 is settled and the packaged app, launched from Finder, reaches the receiver through its
  nested server.
- Phase 4's live runs are done and recorded.
- The user guides and `ARCHITECTURE.md` say what changed, and the guides say that macOS is
  the only supported platform. The README's statement is milestone 10's.

## Inputs the specification is silent on

The design says what the software must do, not everything it will meet. These are the
inputs most likely to bite, most likely first. Each has a named test above.

1. **A server that stops under a client.** Under the CLI, mid-wait, it leaves the outcome
   unknown, and the CLI must not submit again
   (`a_server_that_stops_while_the_cli_waits_is_outcome_unknown_and_nothing_is_submitted_again`).
   Under the GUI it is the banner and the re-attach, not a respawn
   (`the_gui_never_starts_a_server_unprompted_after_the_first_launch`).
2. **Quitting the app and opening it again at once.** The old server stops accepting
   first and keeps the lock until its connections have ended, its sessions are closed, and
   its operations in flight are done, which can take its 5-second grace and more. The new
   GUI finds a refused socket, spawns, and its child exits 75 because the lock is still held.
   The launcher then waits and spawns again, within one deadline, instead of waiting for a
   "winner" that is leaving
   (`reopening_while_the_old_server_still_holds_the_lock_spawns_again_when_it_lets_go`).
3. **Two GUIs, or a GUI and a terminal, starting a server at once.** The lock lets one win;
   the loser's child exits 75 and the loser attaches
   (`two_launchers_at_once_end_with_one_server_and_one_owner`).
4. **A server killed with `kill -9`.** Its socket is still there and refuses connections; the
   launcher spawns, and the new server reclaims the socket under the lock
   (`a_socket_left_by_a_killed_server_is_reclaimed_by_a_spawn`).
5. **A child that writes more than a pipe holds.** An undrained standard error would stall it
   (`a_noisy_child_does_not_stall_because_its_error_output_is_a_file`).
6. **A token file that is wrong.** Missing (no server has ever run), readable by others, a
   link, empty, or too long: each is refused with the file's name and the reason, and none
   is attached with.
7. **A server from another build.** A standalone server from an older or newer checkout
   speaks another contract version (`a_contract_version_mismatch_is_incompatible_and_nothing_is_spawned`).
8. **`HOME` unset.** The old fallback made a directory called `~`
   (`an_unset_or_empty_home_has_no_default`).
9. **A long user name.** The Operator socket's path is 75 bytes for a user named `user1`,
   and the client refuses a path of 104 bytes or more, so a user name of 34 characters or
   more fails. `ApiClient::connect` refuses it with the length and the limit, and both
   clients show that text (`a_socket_path_over_the_limit_is_reported_with_its_length_by_both_clients`,
   in step 2).
10. **A configuration edited by hand while the server runs.** The revision changes with the
   content, so the next save from a client that read the old one is refused and retried
   (`a_change_made_outside_the_service_changes_the_revision`).
11. **A GUI with no server executable beside it** (`cargo run` without building the server,
    or a copied binary): `ExecutableMissing` with the path, and no search.
12. **`Message::Shutdown` on an attached GUI.** It releases the GUI's leases and stops
    nothing (`closing_with_an_attached_server_stops_nothing`).

Also covered: a socket that is not ours, a listener that never answers, a mistyped agent
label in `--as-agent` (refused by the label rule before any request), and a policy that
fails to reload.

## Exit status

**Planned.** Nothing is implemented. Steps 0b and 0c are the owner's and can start now;
steps 1, 1b, 1c and 2 need neither. The owner's answers are recorded as decisions D44 to
D47, D77 and D78, and the review's as D48 to D76 and D79.

## Known limits

- **The CLI no longer frees the receiver when it exits.** The server keeps a receiver
  connected for 60 seconds after the last lease (and the lease of a CLI command may take up
  to 15 seconds to be seen as closed), so another tool that needs the receiver's single
  control connection must wait that long, or run `denon-avr-remote release`, which frees it
  at once when nothing else is using it. `release` never closes a receiver the app's window is
  holding.
- **A GUI-owned server ends with the GUI.** The CLI and any agent lose service when the
  window closes, which is the design: agent access exists only while a server runs. A
  server that should outlive the window is started by hand.
- **The CLI works only while a server runs,** and does not start one. After this phase
  `get` and `set` need the app open or `denon-avr-api-server` running.
- **The feed names the Operator, not the program.** The CLI and the GUI are both the Operator,
  so the Operations feed cannot tell a CLI command from a click. The Audit tab has the same
  limit.
- **The Operator is audited and counts toward agent budgets** from this phase (phase 3's D6
  and D7 become true of the CLI and GUI).
- **A hand edit of the configuration is detected at the next save,** not as it happens, and
  a client that has the old one open is told to read again.
- **Approvals are a placeholder.** The tab has no data; a request that needs approval ends
  `approval_unavailable` and shows in the audit log.
- **Any process that can read `credentials/operator.token` acts as the Operator,** as the
  design accepts. The reader refuses a file that is not owner-only, which keeps the file
  honest and does not make it secret from the owner's own processes.
- **A server that never becomes ready is reported and left to its pipe,** not killed (D72).
- **macOS only.** `api-client` refuses to build elsewhere, so `cli` and `desktop` do too, with
  that message. Windows and Linux users stay on 3.0.0. The visual baselines are macOS only
  and always were.
- **Local Network permission was settled by S5 (outcome B).** The first connection after an
  install or an update is refused once with `os error 65` and the retry about 5 s later passes,
  so the launcher and the server status show "waiting for permission" and retry (D80). A
  Finder-launched packaged app is still checked by hand at the end.
- **Fetching `rustix` as a direct dependency of `api-client`** changes `Cargo.toml`; it is
  already in `Cargo.lock`, so no new crate is downloaded.

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac.

1. **S5**, before step 10: done 2026-10-10, outcome B. See the architecture.
2. **Phase 4's runs**, before step 3: item 5 (the account boundary) and items 7 and 8 (the
   armed live run with no competing client) of the phase 4 checklist.
3. **Two clients at once.** With the GUI open on a receiver (and so owning the server), run
   `denon-avr-remote get volume` and `set volume <a level you accept>` from a terminal. The
   GUI shows the change and the Operations tab shows the CLI's operation. `lsof -i` shows one
   connection to the receiver, held by the server's process. The Audit tab shows the
   operation with the Operator as principal.
4. **A killed GUI.** `kill -9` the GUI; within a few seconds the server process is gone and
   both sockets are removed or reclaimable (`ls ~/Library/Application\ Support/Denon\ AVR\
   Remote/run`).
5. **An attached GUI.** Start `denon-avr-api-server` from a terminal, open the GUI (it
   attaches; Diagnostics says it was not started by the app), close the GUI, and see the
   server still running.
6. **A lost server.** With the GUI attached, stop the standalone server. The GUI shows the
   banner and does not start a server. Start one from the terminal and see the GUI recover
   by itself; stop it again and press **Start server**.
7. **A CLI with no server.** Stop everything and run `denon-avr-remote get volume`; the
   message names the socket and says how to start a server.
8. **Dry run as an agent.** With a policy that has an agent rule, `denon-avr-remote set
   volume <level> --dry-run --as-agent <label>` prints that agent's decision and nothing is
   written to the receiver.
9. **Tokens.** `tokens issue claude-code`, `tokens list`, `tokens revoke <id>`; the token
   shows once and never again.
10. **The bundle.** `make package-macos`, open the app from Finder, accept the Local Network
    prompt if it appears, and confirm the receiver connects. Look at `codesign -dvvv` on the
    app and the server.
11. **The baselines.** `make capture-visual-baselines CAPTURES=<dir>`, compare, and review
    the new and re-captured images by eye.
12. **No competing client.** Close other Denon tools before the live runs.
13. **Release.** Run `denon-avr-remote get volume`, then `denon-avr-remote release`; it prints
    that the receiver is free, and `lsof -nP -iTCP:23` shows no connection from the server (or
    another Telnet client can connect). With the GUI open on the receiver, `release` waits about
    20 seconds and then says the receiver is in use, and changes nothing.
