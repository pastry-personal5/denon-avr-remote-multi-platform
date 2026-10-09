# Version 4, Phase 4 — Control API server, contract, and client

**Planned 2026-10-08 and implemented on the branch `v4/phase-4-control-api-server`
(2026-10-08 and 2026-10-09). The owner answered the questions the plan could not settle
itself, listed in the architecture's
[decisions](phase-4-control-api-server-architecture.md#decisions); the last, the Agent
endpoint's directory and admission, was settled by the S2 hand check on 2026-10-09. See
[Exit status](#exit-status).**
This is milestone 4 of the [roadmap](roadmap.md). It adds three packages, the
`/v1` wire contract (`api-contract`), a client that implements the control-service
port over a Unix socket (`api-client`), and the server that hosts the in-process
service behind two endpoints (`apps/api-server`). It is additive: no shipped
delivery package uses it until milestone 5, and no Operator-visible behavior
changes. It is the first code that lets a process other than the one that owns the
receiver act on it, so the Agent endpoint is built to fail closed.

The roadmap's milestone 4 and the design had faults that would have shipped an Agent
view that leaks addresses, a mistyped `dry_run` that writes, an exclusive bind that
two servers can win, an event stream a dead client could hold open for ever, and
boundary rules that fail on the day they land. The
[architecture](phase-4-control-api-server-architecture.md#what-the-review-changed)
lists the twenty-two corrections and holds the types and decisions. The rules the phase
relies on are in [Planned architecture](../planned-architecture.md).

`main` contains phase 3's code, and its live checks are still open. They do not block
this phase, which changes no gate logic; it adds a way to reach it.

## Steps

Work goes one numbered step at a time: `make check`, `make clippy`, and
`git diff --check` pass, the diff is reviewed, and the step is committed. A step that
changes `tools/check-boundaries.sh` lands with the code it checks, because the script
fails if either half lands alone. Step 0b needs the Mac and the dedicated account and
is the owner's; it gates step 9 only.

| # | Step | State |
| --- | --- | --- |
| 0 | Amend the design | Done |
| 0b | S2: the Agent endpoint's directory and admission, by hand | Done, 2026-10-09 |
| 1a | The label rule in `policy` and the policy loader | Done |
| 1b | Port additions, token and refusal types, the audit adapter | Done |
| 2 | `api-contract`: conventions, reads, state views, errors, routes | Done |
| 3 | `api-contract`: requests, configuration, events, Operator resources | Done |
| 4 | Token store | Done |
| 5 | `api-server`: start, lock, both endpoints, request pipeline | Done |
| 6 | `api-server`: the routes and the refusal matrix | Done |
| 7 | Event streams, waits, and revocation | Done |
| 8 | `api-client` and the port conformance run | Done |
| 9 | Enable the Agent endpoint from settings (needs 0b) | Done |
| 10 | Process model, live test | Done; the live test is compiled, not run |
| 11 | Promote and exit | Done |

The roadmap lists six steps; this phase splits them where a reviewer could reject one
and approve its neighbor, and adds the port work and the token store that the
roadmap's step 5 assumed already existed.

### Step 0 — Amend the design (done)

`planned-architecture.md` now says:

- the API table with the resources of the architecture's table, and the decisions D28,
  D29, D36, and D43: a dry run is its own route and `as_agent` is the Operator's
  alone, `register_ad_hoc` and the inspection reads have routes, `/v1/approvals`
  waits for milestone 7, configuration writes use `ETag` and `If-Match`, operation
  events have a stream of their own, and the policy loader refuses an agent label no
  token could hold;
- the Agent state and source views, and why they are separate types;
- the `run/` and `credentials/` directories, and that the endpoint's admission is by
  its directory and the peer's uid, with the socket's mode as no part of it;
- the `access_refused` and token audit events, the throttle, and that they do not
  affect the ledger;
- the one-server lock, and that the OAuth schemas arrive with milestone 9.

The roadmap's header, milestone 4, and gaps table are corrected to match, and the
documentation map names this phase.

### Step 0b — S2: the Agent endpoint's directory and admission (done)

Done by the owner on 2026-10-09; the result is
[agent-endpoint-access-macos.md](../research/agent-endpoint-access-macos.md): a
directory ACL admits the account, the socket stays `0600`, and the keys are `directory`
and `uids`. Output: `docs/research/agent-endpoint-access-macos.md`. The procedure is in the
architecture's [S2](phase-4-control-api-server-architecture.md#s2-what-the-hand-check-decides)
section. It needs the dedicated account and a second terminal session as that
account. The note fixes `agent_endpoint`'s keys and its default directory, and says
whether an ACL or a group admits the account. Step 9 reads it. Steps 1a to 8 do not:
the endpoint's code and tests take its directory and admitted uids as plain values,
and what S2 gates is the binary reading them from a settings file, which is the
moment an agent can first connect.

### Step 1a — The label rule in `policy` and the policy loader

Files: `crates/policy/src/{lib,label,config}.rs`, `crates/policy/tests/cases.rs`,
`crates/infrastructure/tests/policy_yaml.rs`, and the "Policy file" paragraph of
`ARCHITECTURE.md`, which lists what the loader refuses and changes in this step, not
at step 11, because that file describes the code as it is.
Produces `policy::label_is_well_formed(label: &str) -> bool`, and a check in
`PolicyConfig::new` that refuses a rule whose `agents` list holds a label for which it
is false, naming the rule. This is the only step that changes phase 3's code. It is
separate so that it can be rejected without losing the tokens.

Tests in `crates/policy/tests/cases.rs`:

- `the_label_rule_accepts_lowercase_and_refuses_the_rest`: `claude-code` and `a.b_c-1`
  pass; `Claude-Code`, `a b`, `-x`, an empty label, 65 characters, and
  `unauthenticated` are refused.
- `a_rule_naming_a_label_no_token_could_hold_is_refused_by_rule_id`.

Tests in `crates/infrastructure/tests/policy_yaml.rs`:

- `an_uppercase_label_in_a_policy_file_is_refused`.
- `the_existing_cases_and_the_sample_policy_still_load`. The labels the existing tests
  use (`a`, `agent`, `claude-code`) are well formed. One is not:
  `a_rule_can_be_narrowed_to_agents_and_receivers` lists `Other` and asserts the scope
  matches `Other` and not `other`. It changes to list `other` and to assert that
  `Other` does not match, which keeps the case-sensitive point.

### Step 1b — Port additions, token and refusal types, the audit adapter

Files: `crates/application/src/{control,audit,tokens,service,lib}.rs`;
`crates/infrastructure/src/audit_jsonl.rs`; the test fakes in `apps/cli/src/tests.rs`
and `crates/gui-lib/src/bridge.rs`; and the "Control service" and "Audit log"
paragraphs of `ARCHITECTURE.md` (the token methods, the owned capability lists, the
record kinds, and the optional principal), updated here for the reason step 1a gives.
Signatures are in the architecture's
[port and audit additions](phase-4-control-api-server-architecture.md#port-and-audit-additions).
The adapter changes with the types because `AuditRecord.principal` becomes optional
and the build would not compile otherwise.

Consumes: the phase 3 port, `AuditLog`, `Principal`, `AgentLabel`, and
`label_is_well_formed` from step 1a. Produces: the `TokenStore` port, `TokenId`,
`TokenRecord`, `IssuedToken`, `Credential`, `AccessRefusal`,
`ControlService::record_refusal`, the three `OperatorAdmin` token methods, and
`AgentPath::with_tokens`.

Tests in `crates/application`:

- `receiver_capabilities_own_their_lists`.
- `issuing_a_token_applies_the_label_rule`.
- `a_token_id_prints_and_reads_back`, and `a_token_secret_never_prints`: `Debug` and
  `Display` of `IssuedToken` and `TokenSecret` hold no secret.
- `the_token_methods_are_the_operators_alone_whatever_the_store_can_answer`: an agent
  is `Forbidden`.
- `a_service_with_no_token_store_answers_the_operator_unavailable`.
- `issuing_and_revoking_are_audited_and_the_record_holds_no_secret`.
- `the_first_refusal_of_a_key_is_written_and_the_rest_inside_a_minute_are_counted`,
  `the_next_record_after_a_minute_carries_the_suppressed_count`,
  `at_most_256_keys_and_30_records_a_minute_are_kept_whatever_the_keys`.
- `a_refusal_record_is_flushed_and_bounded_to_a_second_on_a_stalled_disk` (paused
  clock), and `a_failed_refusal_append_does_not_change_the_audit_health`.
- `the_ledger_rebuild_ignores_refusals_and_token_events`.
- `the_stub_fails_every_call_with_a_typed_error` is extended to the new methods, and
  `one_handle_narrows_to_each_surface_without_new_objects` still compiles.

Tests in `crates/infrastructure/tests/audit_jsonl.rs`:
`access_refused_and_token_events_round_trip`, `a_record_with_no_principal_round_trips`,
`since_and_query_return_the_new_events`, and `text_in_the_new_events_is_bounded` (the
`resource` and the labels included).

### Step 2 — `api-contract`: conventions, reads, state views, errors, routes

Files: `Cargo.toml`; `crates/api-contract/{Cargo.toml,src/{lib,error,routes,paths,receivers,state}.rs}`;
`crates/api-contract/tests/{golden,round_trip,leaks}.rs` and `tests/golden/*.json`;
`tools/check-boundaries.sh` (the three edges, the purity check).
The package depends on `denon-avr-application`, `denon-avr-domain`, `serde`, and
`serde_json` (the event encoding writes JSON). No HTTP, no runtime, no hash crate.

Produces: `ApiError` and the `ControlError` mapping in both directions;
`routes::TABLE: &[Route]` with `Route { id: RouteId, method, pattern, audience }`, the
pattern in axum's `{id}` syntax; `paths::encode_segment(&str) -> String` and one path
builder per resource for the client; `EndpointPaths::under(dir)`;
`ReceiverSummaryDto`, `HealthDto` (the service's health and `contract`, with an
optional `server: ServerHealthDto` that only the Operator's response fills),
`StateView`, `OperatorStateView`, `SourcesView`, `OperatorSourcesView`, each with
`From` in the direction a server needs and `TryFrom` in the direction a client needs.

Tests:

- `every_control_error_maps_to_its_status_and_code_and_back`, with `Retry-After`
  rounded up to whole seconds, `TooLate` carrying its snapshot, and each of the eleven
  `OperationErrorKind`s surviving in a `Receiver` error.
- `a_not_found_word_outside_the_closed_set_becomes_resource`.
- `the_route_table_lists_exactly_the_documented_resources` (a golden listing), and
  `no_two_rows_share_a_method_and_pattern`.
- `encode_segment_leaves_unreserved_characters_and_encodes_the_rest`: slash, space,
  percent, question mark, hash, and Unicode in an id become `%XX` sequences that
  decode to the original, and every path builder uses it. The server side of the
  round trip is in step 6.
- `an_operator_state_view_rebuilds_an_equal_state_except_the_stamps`: seeded states,
  64 seeds as `receiver_properties.rs` does; value, validity class, reason, issue
  text, evidence, epoch, and revision survive, and the stamps are the placeholders.
- `field_baseline_capture_agrees_before_and_after_the_round_trip`.
- `an_address_seeded_into_every_free_text_field_never_reaches_an_agent_view`:
  `FieldIssue.message`, `diagnostics`, `Suspended.reason`, `UnavailableStatus`,
  `SourceCatalog.error`, and `raw_response`, each holding `connecting to
  192.168.1.20:23` and a raw frame; the serialized Agent views contain neither string.
- `the_agent_views_have_no_text_field` (the golden keys), and
  `the_operator_view_keeps_the_issue_text_the_gui_shows`.
- `a_response_with_an_extra_field_is_read_and_a_request_with_one_is_refused_by_name`.
- `a_response_naming_an_unknown_status_is_not_guessed`.
- `a_receiver_summary_has_no_address_field`.
- `golden_json_for_each_type_is_unchanged`.

### Step 3 — `api-contract`: operations, configuration, events, Operator resources

Files: `crates/api-contract/src/{requests,config,events,admin}.rs` and tests. The
intent and operation types (`IntentDto`, `OperationDto`) and their tests landed in
step 2, because the `too_late` error carries an operation.
Produces: `SubmitRequest`, `DryRunRequest` (agent) and `OperatorDryRunRequest`
(with `as_agent`), `DryRunDto` (agent, no rule ids) and
`OperatorDryRunDto`, `ConfigDto`, `DiscoveredDto`, `AdHocRequest`,
`PolicyDto`, `AuditPageDto`, `TokenIssueRequest`, `IssuedTokenDto`, `TokenDto`,
`QuickSelectNamesDto`, `HttpInformationDto`, and `ReadinessDto` (the Operator's
inspection reads, which the plan had not listed), and `events::{EventName, EndReason,
Parser}`. The server
writes the stream with axum's `Sse`, so the contract holds the names, the payloads, and
the parser the client reads them with, and no encoder.

Tests:

- `a_dry_run_for_an_agent_has_no_as_agent_and_no_rule_ids`.
- `config_round_trips_with_favorites_and_current`.
- `audit_pages_round_trip_with_cursors_and_every_event_kind`, and
  `policy_and_discovery_views_round_trip`.
- `a_token_listing_has_no_secret_or_digest_field` and
  `only_the_issue_response_holds_a_secret`.
- `the_sse_parser_reassembles_events_split_at_every_byte`: comments, multi-line data,
  CRLF, an unknown event name (ignored), and an event over 1 MiB (refused).
- `the_parser_reads_a_hand_written_frame_for_every_event`: `state`, `operation`,
  `missed`, and `end` with each of its reasons (`session_closed`, `revoked`,
  `shutdown`, `max_age`). Step 7 adds `the_parser_reads_what_the_server_writes`.

### Step 4 — Token store

Files: `crates/infrastructure/src/token_store.rs`, `Cargo.toml` (`getrandom`,
`subtle`), `crates/infrastructure/tests/token_store.rs`.
Consumes: the `TokenStore` port (step 1b). Produces `FileTokenStore::open(dir) ->
Result<FileTokenStore, TokenStoreError>`.

Tests:

- `an_issued_token_authenticates_as_its_label_and_a_revoked_one_never_does`, and
  `a_revoked_record_stays_in_the_listing`.
- `the_file_holds_a_digest_and_never_the_token`.
- `the_operator_token_is_created_once_and_read_back`, and
  `an_operator_token_is_not_an_agent_credential_and_the_reverse`.
- `an_unknown_or_unprefixed_string_is_unknown_and_is_not_hashed`.
- `a_second_active_token_for_a_label_is_refused_and_a_revoked_one_does_not_count`.
- `a_damaged_agent_file_degrades_the_store_and_is_not_replaced`: the store opens with a
  fault, the Operator's token still works, and issuing and revoking are refused.
- `credentials_are_0600_in_a_0700_directory` and `wider_existing_permissions_are_an_error`.
- `a_crash_between_the_temporary_file_and_the_rename_keeps_the_old_file`.
- `changes_bump_on_revoke_and_on_nothing_else`.
- `concurrent_issues_never_lose_a_record`.
- `issued_tokens_are_distinct_and_have_the_documented_shape` (40 issues, each an fsync),
  and, on the generators with no I/O, `a_thousand_secrets_and_ids_are_distinct`.

### Step 5 — `api-server`: start, lock, both endpoints, request pipeline

Files: `apps/api-server/{Cargo.toml,src/{lib,main,start,endpoint,pipeline,shutdown}.rs}`,
`apps/api-server/tests/{support/mod,lifecycle,pipeline}.rs`,
`tools/check-boundaries.sh` (the `api-server` edges). The route table is empty
except `GET /v1/health`.
Consumes: `ControlService`, `SharedTokenStore`, `EndpointPaths`, `routes::match_request`.
Produces: `Server::start`, `RunningServer`, `StartError`, `Limits`,
`AgentEndpointConfig { directory, uids }`, and the test support: a fake
`ReceiverConnector` and session, a real `ControlService::start` over a temporary
audit directory and policy file, and a raw Unix-socket HTTP helper.

The server runs its own accept loop over each `UnixListener` (peer uid, connection cap,
then hyper's HTTP/1 builder with the head timeout, which also bounds the idle time
between requests, and the header size), serving an axum `Router` with a middleware
layer, added after its routes and its fallback, that authenticates and checks the
declared body length. `axum::serve` is not used (see the architecture's [HTTP stack](phase-4-control-api-server-architecture.md#packages-and-edges)).

An endpoint is a value (kind, socket, admitted uids, credential kind), so the Agent
endpoint is the same code as the Operator's. `Server::start` creates it when it is
handed an `AgentEndpointConfig`, which only tests do until step 9 teaches the binary
to read one from the settings and check its directory. The binary takes
`--data-dir DIR` (default `data_directory()`), which the lifecycle tests use to run
several servers side by side without touching `HOME`, and which the S2 hand check
uses to run a throwaway server.

Tests in `lifecycle.rs` (socket paths kept short; they use a directory made under
`/tmp` and not `$TMPDIR`, whose path is long on macOS):

- `a_second_server_refuses_with_already_running`.
- `a_socket_left_by_a_killed_server_is_reclaimed`: the real binary, `kill -9`, then a
  second start.
- `a_stale_socket_is_not_removed_while_another_server_holds_the_lock`.
- `a_stale_path_that_is_not_our_socket_is_not_removed`: a regular file and a symlink
  at the socket path are each a `StartError` and are still there afterwards.
- `a_path_over_the_limit_fails_with_its_length_and_the_limit` and
  `a_path_exactly_at_the_limit_binds`.
- `the_directories_are_created_0700_and_the_data_directory_is_left_alone` (a 0755
  data directory stays 0755), and `a_wider_run_or_credentials_directory_is_an_error`.
- `the_operator_socket_is_0600`.
- `admission_compares_the_peer_uid_with_the_endpoints_list`, on made-up credentials.

Tests in `pipeline.rs`, over the raw helper, on both endpoints:

- `a_request_without_a_credential_is_401_whatever_the_path`: the same body for a path
  that exists and one that does not.
- `two_authorization_headers_are_refused`, `a_token_in_the_query_string_is_400`,
  `a_scheme_other_than_bearer_is_401`.
- `an_agent_token_is_accepted_only_on_the_agent_endpoint_and_the_operator_token_only_on_the_operator_endpoint`.
- `a_head_over_16_kib_is_refused`, `a_slow_head_is_closed_when_the_head_timeout_passes`
  (a 300 ms timeout in the test, with `default_limits_are_the_documented_ones` pinning
  the five seconds), `a_body_over_the_cap_is_413_before_it_is_read`, and
  `a_chunked_body_is_411`. The slow-body case, `a_slow_body_is_408`, needs a handler that
  reads a body, so it is in step 6.
- `the_connection_cap_closes_the_extra_connection`.
- `health_answers_with_the_contract_version`,
  `health_on_the_operator_endpoint_reports_the_agent_endpoint_and_the_token_store`, and
  `health_on_the_agent_endpoint_has_no_server_section`.

### Step 6 — `api-server`: the routes and the refusal matrix

Files: `apps/api-server/src/routes.rs`, `apps/api-server/tests/{contract,agent_endpoint}.rs`.
Every route of the table except `events`, whose handler is step 7. The Operator
endpoint serves the whole table and the Agent endpoint its `Both` rows.

Tests in `contract.rs`, each with the fake session and a real service:

- `each_both_resource_returns_what_the_port_returns` and
  `each_operator_resource_returns_what_the_port_returns`: a table over the routes.
- `every_control_error_reaches_the_wire_as_documented`.
- `receiver_ids_with_slash_space_percent_question_mark_and_unicode_reach_the_port_intact`,
  through the client's `encode_segment`, and
  `an_id_that_is_not_utf8_or_that_receiver_id_refuses_is_a_bad_request`.
- `routing_matches_what_it_should_and_nothing_else`: a trailing slash, `..`, a query
  string, an unknown method, and an empty id segment, which reaches its handler and is
  refused there with a `400`. (`/v1/operations/events` as the stream, and never an
  operation with the id `events`, is step 7's.)
- `every_row_of_the_table_has_a_handler_and_no_route_exists_outside_it`: the router is
  walked against the table, on both endpoints.
- `a_mistyped_field_in_a_submit_is_400_and_nothing_is_submitted`, and
  `a_body_naming_dry_run_on_the_submit_route_is_400_and_does_not_write`.
- `the_dry_run_route_never_submits`: `operate` is never called, and no operation or
  record is made.
- `put_config_needs_if_match_and_a_stale_one_is_412`, `two_puts_with_one_etag_apply_once`,
  and `the_etag_changes_with_any_edit_and_not_with_key_order`. The server computes the
  `ETag` (a SHA-256 of the canonical JSON), so `api-contract` needs no hash crate, and
  a client treats the value as opaque.
- `policy_reload_and_audit_paging_work_through_the_server`.
- `a_token_is_issued_listed_and_revoked_and_its_secret_is_shown_once`.
- `register_ad_hoc_returns_an_ad_hoc_id_the_state_route_can_use`.
- `a_discovery_timeout_over_the_cap_is_clamped_and_not_refused`.
- `a_validation_error_names_the_field_and_never_echoes_the_body`.

Tests in `agent_endpoint.rs`, with the Agent endpoint admitting the test process's own
uid:

- `every_operator_row_of_the_route_table_is_404_on_the_agent_endpoint_for_an_agent_token_and_is_audited`:
  generated from `routes::TABLE`, so a new resource cannot escape it.
- `with_the_operator_token_the_same_rows_are_401_and_audited_as_the_operator_token_on_the_agent_endpoint`
  and `with_no_token_they_are_401`.
- `an_agent_token_on_the_operator_endpoint_is_401_and_audited`.
- `every_both_row_works_on_the_agent_endpoint`.
- `no_address_reaches_an_agent_in_any_response_or_error`: the fake session's state,
  source catalog, and errors hold addresses and raw frames; every byte an agent reads
  over every `Both` route and every error body is scanned. Step 7 adds the streams.
- `the_receivers_list_for_an_agent_has_no_host_and_no_ad_hoc_id`, and
  `an_ad_hoc_receiver_is_404_to_an_agent`.
- `an_agent_cannot_read_or_cancel_another_agents_operation`.
- `an_agent_cannot_name_as_agent_in_a_dry_run`, and `an_agents_dry_run_shows_no_rule_ids`.
- `a_flood_of_refused_requests_writes_a_bounded_number_of_records_and_the_ledger_still_rebuilds`:
  10,000 refusals, a count of records at or under the throttle's bound, then a
  restart whose rebuilt ledger equals the one before.
- `an_agent_write_through_the_server_is_decided_by_policy`: allow, deny, and
  `approval_unavailable` over the socket, `operate` called once or never.
- `a_label_typo_is_refused_at_issue` (`Claude-Code`).
- `an_agent_that_holds_every_agent_connection_does_not_starve_the_operator`.

### Step 7 — Event streams, waits, and revocation

Files: `apps/api-server/src/events.rs`, `apps/api-server/tests/streams.rs`.
Tests, on both endpoints:

- `the_first_event_is_the_full_state_then_one_per_change`.
- `operation_events_reach_only_their_owner`, on `GET /v1/operations/events`: an agent
  sees its own and the Operator sees every operation's; and
  `an_operation_stream_holds_no_lease_on_a_receiver`.
- `no_address_reaches_an_agent_in_a_state_stream`.
- `a_slow_client_sees_the_newest_state_and_never_a_backlog`.
- `a_missed_operation_event_is_reported_and_the_operation_can_be_read`.
- `a_keep_alive_is_written_every_fifteen_seconds`, which pins the defaults (15 s
  keep-alive, 10 s send timeout, 10 min maximum age, four streams), and
  `a_quiet_stream_is_written_to_at_the_keep_alive_interval`, which watches a shortened
  interval. The limits are fields of `Limits`, so no test waits out a real one: a
  paused clock does not hold still while a test also uses real sockets.
- `a_stalled_client_is_dropped_after_ten_seconds_and_the_receiver_is_released`: the
  fake connector counts open sessions; with state changing, the stream is dropped when
  a send times out and, after the idle time, the count is zero (with the timeout
  shortened).
- `a_stalled_client_on_a_quiet_receiver_is_released_when_the_stream_reaches_its_maximum_age`
  (with the maximum age shortened): no event is produced, the send timeout never
  fires, and an Agent's
  state stream ends with `max_age` at ten minutes. The Operator's does not, and
  `an_operator_state_stream_has_no_maximum_age` says so.
- `the_parser_reads_what_the_server_writes`: the bytes of each event kind, captured
  from the real server, go through `events::Parser`.
- `a_client_that_disconnects_releases_its_lease_at_once`.
- `the_fifth_stream_of_one_principal_is_429`.
- `a_session_closed_by_the_service_ends_the_stream_and_a_new_stream_connects_again`.
- `shutdown_ends_every_stream_with_an_end_event`.
- `a_wait_returns_when_the_operation_finishes_and_otherwise_at_the_cap`, and
  `the_request_timeout_exceeds_the_services_wait_cap`.
- `a_revoked_token_ends_its_streams_and_waits_within_a_second_and_refuses_its_next_request`,
  with an Operator stream open meanwhile that continues.
- `get_state_takes_one_snapshot_and_drops_its_lease`.

### Step 8 — `api-client` and the port conformance run

Files: `crates/api-client/{Cargo.toml,src/{lib,transport,sse,mapping}.rs}`,
`apps/api-server/tests/client.rs`, `tools/check-boundaries.sh`.
Produces `ApiClient`, `Endpoint { socket, token, audience }`, `ConnectError`, the
three port traits, and the inherent `ApiClient::server_health()` for the Operator
audience. The boundary step adds the `api-client` edges and the resolved-graph checks.

Tests in `api-client`: `a_connection_refused_is_receiver_service_unavailable`,
`the_credential_never_appears_in_debug_or_errors`,
`an_unknown_status_is_unavailable_and_not_a_guess`, and
`state_ends_with_session_closed_on_an_end_event_or_a_closed_stream`.

The conformance run in `client.rs` is one function over a harness trait, run twice,
against `ControlService::operator()` directly and against `ApiClient` through the
server. It asserts the same results both ways: receivers, the first state snapshot and
a later one, submit then completed, an idempotent retry returning the same id,
`TooLate` carrying its snapshot, `RateLimited` with a `retry_after`, `NotFound` for
both words, `Forbidden` for an agent calling an Operator method (the agent client
refuses without sending a request), a dry run, health, and `state` ending with "session
closed" at shutdown. Milestone 6's MCP conformance suite follows the same shape.
`the_port_behaves_the_same_in_process_and_over_the_socket` is its name.

A state read over the socket never equals the one read in process: the stamps are
placeholders, `synchronization` is `NotStarted`, and an agent's copy has no issue text
or diagnostics. So the run compares a normalization, written once in
`tests/support`, and not the states: for each of the seven fields, its
`FieldBaseline::capture`, its validity class, and its stale reason; the epoch and the
revision; and, for the Operator only, each field's issue text and the diagnostics.
`the_normalization_of_a_state_ignores_the_stamps_and_nothing_else` pins it: a change
to any compared part changes the normalization and a change to a stamp does not.

### Step 9 — Enable the Agent endpoint from settings

Needed step 0b, since this is the step that lets an agent connect to a real server.
Files: `apps/api-server/src/{settings,agent_directory,start,main}.rs`,
`apps/api-server/tests/{settings,agent_directory,lifecycle}.rs`,
`docs/examples/server.yaml`. Produces the `agent_endpoint` settings key (its shape from
S2's note: `directory`, defaulting to `/Users/Shared/Denon AVR Remote`, and `uids`; the
socket's mode is not a key) and the checks of the endpoint's directory. The checks are in
`Server::start`, not only in the settings path, so every caller gets them: an Agent
endpoint that fails one is not created, and the Operator endpoint serves regardless.

Tests:

- `the_settings_file_may_be_absent`, `unknown_settings_keys_are_refused`, and
  `the_agent_endpoint_is_not_created_without_settings`.
- `the_agent_endpoint_refuses_an_unsafe_directory`: a link, a directory not owned by
  the server's uid, and one that is group- or world-writable are each "not created",
  with the reason in the log, and the Operator's health `server` section says the
  Agent endpoint is off and why.
- `a_missing_directory_is_created_0700`, and `a_socket_that_is_not_ours_after_bind_is_removed_and_refused`
  (the directory and the socket are checked again after the bind).
- `an_agent_endpoint_with_no_uids_is_refused`, and `the_sample_settings_file_loads`.
- `the_operator_endpoint_serves_when_the_agent_endpoint_cannot_be_created`.

### Step 10 — Process model and live test

Files: `apps/api-server/src/{main,shutdown}.rs`,
`apps/api-server/tests/{lifecycle,live_x3800h_api}.rs`, `Makefile`.
Tests in `lifecycle.rs`, with the real binary:

- `end_of_input_in_parent_pipe_mode_shuts_down_removes_both_sockets_and_frees_the_lock`.
- `a_standalone_server_ignores_standard_input`.
- `sigterm_and_sigint_shut_down_the_same_way` and `a_second_signal_exits_at_once`.
- `shutdown_waits_for_operations_in_flight_before_closing_sessions`.
- `a_shutdown_while_a_client_streams_ends_the_stream_with_an_end_event`.

The live test and `make test-live-x3800h-api` are described in the architecture's
[live checks](phase-4-control-api-server-architecture.md#live-checks). It is
`#[ignore]`, refuses to run unless armed, and was compiled and its target checked to
refuse when unarmed; it is **not run** by the session that builds the phase.

### Step 11 — Promote and exit

Move the implemented design into `ARCHITECTURE.md` (a "Control API" section; the
three packages in the workspace table and graph; the process model and the endpoint
rules), add the three packages to the repository map in `AGENTS.md`, leave pointers in
`planned-architecture.md` as the earlier phases did, and remove the sections the
implementation has made true. Write the live record in
`docs/archive/v4/phase-4-live-validation-record.md` with every result **To fill**,
update the exit status below, and update the documentation map, the roadmap, and
`docs/development.md`, which names the live targets.

## Exit criteria

- `make check`, `make clippy`, and `git diff --check` pass.
- The port conformance run passes against the service directly and through the server.
- The Agent endpoint refuses every Operator resource of the route table, with an Agent
  token, with the Operator token, and with none, and audits each attempt, at a bounded
  rate that leaves the ledger's rebuild unchanged.
- No receiver address, path, raw frame, or error text reaches an agent on any route,
  stream, or error body.
- State snapshots coalesce for a slow client, a stalled or departed client releases
  its lease, and operation events reach only their owner.
- A mistyped request field is a `400` that submits nothing; a dry run never calls
  `operate`.
- A token label or a policy `agents` entry outside the label rule is refused, and
  the existing policy tests and sample still pass.
- One server per data directory: a second refuses, and the socket left by a killed
  one is reclaimed.
- `make boundary` checks the three edges, `api-contract`'s purity, and the resolved
  graph of `api-client` (no TLS crate, no server framework, and no `server` feature
  of `hyper` or `hyper-util`, read with `--edges normal,features` so dev-dependencies
  do not count), with `cli` and `desktop` named as the exceptions to the
  `infrastructure` rule.
- Endpoint permissions, checked by hand on the Mac (the checklist below): the
  dedicated agent account reaches the Agent endpoint and cannot reach the Operator
  endpoint, `run/`, `credentials/`, the audit directory, or the data directory's
  files.
- Live, armed (`make test-live-x3800h-api`): the Operator and the agent token operate
  the receiver through the server within the limits, the refusals leave the receiver
  unchanged, and the volume is restored.

## Inputs the specification is silent on

The design says what the software must do, not everything it will meet. These are the
inputs most likely to bite, most likely first. Each has a named test above.

1. **A server killed, or started twice by hand.** The first leaves a socket that
   blocks the next bind; the second would unlink the first's
   (`a_socket_left_by_a_killed_server_is_reclaimed`,
   `a_second_server_refuses_with_already_running`).
2. **A receiver named with `/`, a space, `%`, `?`, or Unicode.** Nothing but control
   characters and the `adhoc:` prefix is forbidden in an id
   (`receiver_ids_with_slash_space_percent_question_mark_and_unicode_reach_the_port_intact`).
3. **An event client that stalls or disappears.** A held subscription keeps the
   receiver connected, which is the thing the idle release exists to end. A client
   that closes is found by the next keep-alive write. One that stays connected and
   stops reading is found by a send timeout only while events flow, so an agent's
   stream also has a maximum age
   (`a_stalled_client_is_dropped_after_ten_seconds_and_the_receiver_is_released`,
   `a_stalled_client_on_a_quiet_receiver_is_released_when_the_stream_reaches_its_maximum_age`).
4. **An agent label typed with the wrong case.** Phase 3 listed this as the input
   most likely to bite and could not catch it. A `Claude-Code` token would sit under
   the general rules and leave a `claude-code` read-only rule unapplied, and so would
   a rule written `Claude-Code`. Both are now refused: the token at issue
   (`a_label_typo_is_refused_at_issue`) and the rule when the policy loads
   (`an_uppercase_label_in_a_policy_file_is_refused`). A rule that names a
   well-formed label nobody holds, a misspelling, is still not caught;
   `dry_run_as` is how the owner checks a tier.
5. **A mistyped field on a write.** `dry_run` on the submit route, or `dryrun` on
   the dry-run route (`a_body_naming_dry_run_on_the_submit_route_is_400_and_does_not_write`).

Also covered: a socket path over the platform limit, a head or body that never ends,
10,000 refused requests, two saves of the configuration at once, and a token revoked
while its stream is open.

## Exit status

**Implemented, and awaiting the owner's live checks.** All steps are done on branch
`v4/phase-4-control-api-server`: the contract, the token store, the server with both
endpoints, the routes and the refusal matrix, the event streams, the client and the port
conformance run, the settings and the Agent endpoint's directory checks, the process
model, and the armed live test, which is compiled and has not been run. S2 (step 0b) was
run by the owner on 2026-10-09 and is recorded in
[agent-endpoint-access-macos.md](../research/agent-endpoint-access-macos.md). The exit
criteria that need the receiver, and the account boundary against the real server, are
open: they are in the
[validation record](../archive/v4/phase-4-live-validation-record.md). The design is
promoted into [ARCHITECTURE.md](../../ARCHITECTURE.md#control-api).

## Known limits

- Nothing uses the server until milestone 5. The CLI and GUI still compose their own
  service, so **while the server holds a receiver, a CLI or GUI cannot connect to it**:
  the receiver accepts one control connection. The server releases a receiver 60
  seconds after the last lease. Do not run them together before phase 5.
- The CLI and GUI remain unaudited until milestone 5 (decision D6).
- Any process that can read `credentials/operator.token` acts as the Operator, as
  the design accepts. The Agent endpoint's protection is the account boundary and
  the directory, not the token alone.
- A hostile agent can hold the Agent endpoint's connections for up to the head
  timeout each. The Operator endpoint is separate and unaffected.
- Revoking a token ends its streams and waits; an operation it started continues to
  its end, because a call into the session is never abandoned.
- The wire carries a field's validity class, not its age. A client cannot tell how
  recent a `current` value is.
- Whether the GUI's projection treats a rebuilt state as it treats the original is
  checked in phase 5, because `api-contract` cannot depend on `gui-lib`. This phase
  checks `FieldBaseline::capture` across the round trip, and the GUI reads the same
  things (the validity class, the last good value, the issue text, and the epoch).
- An event stream has no replay. A client that reconnects reads the state again.
- The Agent state view has no diagnostics, so an agent cannot see raw frames.
- No `/v1/approvals` and no OAuth until milestones 7 and 9.
- `cli` and `desktop` keep their `infrastructure` edge until milestone 5, so the
  roadmap's rule about it holds only for the packages this phase adds.
- The server builds only on Unix. Linux shares the code and is not supported.
- The Operator's event streams have no maximum age, because the GUI's subscription is
  meant to be held, so an Operator client that stalls holds its receiver until it
  closes. An Agent's ends after ten minutes.
- Adding `axum`, `tower`, `hyper`, `hyper-util`, `http-body-util`, `bytes`,
  `getrandom`, and `subtle` changes `Cargo.lock`, and fetching them needs network
  access once.

## Manual macOS checklist

`make check` does not cover these. Run them on the Mac.

1. **Crates.** Run `cargo fetch` once with network access before the sandboxed
   `make check`.
2. **Sockets.** The tests bind Unix sockets and loopback ports. Run `make check`
   outside a sandbox that blocks them. The lifecycle tests make their socket
   directories under `/tmp` so the paths stay under 104 bytes.
3. **S2.** Done 2026-10-09; see `docs/research/agent-endpoint-access-macos.md`. Re-run
   its connect check from the agent account after a major macOS update.
4. **Permissions after a run.** `ls -ld` on `run/` and `credentials/` shows
   `drwx------`, and `ls -l` shows the socket, lock, and token files owner-only.
5. **The account boundary.** As the dedicated account, a request to the Agent socket
   with an Agent token succeeds, and each of these fails: connecting to
   `operator.sock`, listing `run/`, `credentials/`, and the audit directory, and
   reading `operator.token`.
6. **A killed server.** `kill -9` the server, start it again, and see it bind.
7. **Live run.** With `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and
   `DENON_X3800H_SAFE_VOLUME_HALF_STEPS` set to a level you accept, run
   `make test-live-x3800h-api`. Record the result in the validation record.
8. **No competing client.** Close the GUI and CLI before the live run.
