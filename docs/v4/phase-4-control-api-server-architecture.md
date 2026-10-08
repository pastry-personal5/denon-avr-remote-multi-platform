# Version 4, Phase 4 — Control API server, contract, and client architecture

This document fixes the types and rules that
[Planned architecture](../planned-architecture.md) leaves to phase 4. It does not
restate the design; where a rule is owned there it is linked. Text that is
implemented moves into [ARCHITECTURE.md](../../ARCHITECTURE.md) when the
milestone's exit criteria pass.

It was written on 2026-10-08 from the code at the head of `main` (`774c8f1`) and
changes no code. Step 0 of the [overview](phase-4-control-api-server-overview.md)
amended the design where the review found it wrong.

## Starting point

- The in-process service is complete for both principals. `ControlService::start`
  builds the Agent path; `handle(Principal)` returns the port; `shutdown` waits for
  operations in flight and closes every session. Nothing outside tests and the armed
  live test can name an Agent handle.
- The port types hold `&'static` data in three places, so a client cannot rebuild
  them from bytes: `ControlError::NotFound(&'static str)` (only `"receiver"`,
  `"operation"`, and, from this phase, `"token"` are produced), `OperationError.context: &'static str`, and
  `ReceiverCapabilities.inputs` and `surround_modes: &'static [&'static str]`.
- Absent: any HTTP code (`Cargo.lock` has no `hyper`, `http`, or `tower`), any token
  store or token method on `OperatorAdmin`, any audit event for a refused caller, a
  lock or socket module, and every package this phase adds. `application` has no
  `serde`; `serde` and `serde_json` are in `infrastructure` and `protocol`.
- Present and reused: `sha2` (infrastructure), `data_directory()`, `JsonlAuditLog`
  and its owner-only rules, `Principal`, `AgentLabel`, `IdempotencyKey`, the typed
  `ControlError`, the throttle pattern of commit `774c8f1` (`REFUSAL_LOG_EVERY` in
  `service/agent.rs`), and the CLI's `Fake` and the GUI's `FastPort`, which implement
  the port and must gain every method this phase adds.
- The Mac this was written on shows the facts S2 and step 5 turn on: `~` is
  `drwxr-x---+`, `~/Library` and `~/Library/Application Support` are `drwx------`,
  `~/Library/Application Support/Denon AVR Remote` is `drwxr-xr-x`, and
  `/Users/Shared` is `drwxrwxrwt`.
- `cli` and `desktop` still depend on `infrastructure` until phase 5.
- S2 has no research note yet, and the roadmap blocks this milestone on it.

## What the review changed

The roadmap's milestone 4 and the design had these faults. Each is fixed in the
design (step 0) or in this document.

| # | Fault | Fix |
| --- | --- | --- |
| R1 | S2, which milestone 4 is blocked on, has produced nothing | S2 is step 0b, a hand check on the Mac. The endpoint's code and tests take its directory and admitted uids as values, so only step 9, which reads them from a settings file and so first lets an agent connect, waits for it |
| R2 | The roadmap's step 6 says only `api-server` and `diagnostics` reach `infrastructure` and `protocol`. That is false until phase 5, because `cli` and `desktop` link `infrastructure` | The transitive rules apply to the packages this phase adds, with `cli` and `desktop` named as exceptions that phase 5 deletes |
| R3 | The Agent endpoint cannot live in the data directory: `~/Library` is 0700, so a dedicated account cannot reach it. `/Users/Shared` is 1777, so another user could create the directory first or plant a link | The location is S2's output. The server uses `lstat`, requires the directory to be a real directory owned by the server's uid, and refuses to create the endpoint otherwise. Access is gated by the directory and the peer's uid, never by the socket's mode alone |
| R4 | The data directory itself is 0755 here. The audit adapter's rule "wider existing permissions are an error", applied to it, would stop the server on its first run | The server creates and checks two subdirectories of its own, `run/` and `credentials/`, at 0700. The data directory's mode is not touched (D25) |
| R5 | `api-client` "implements the port", but three port types cannot be rebuilt from the wire (Starting point) | `NotFound` maps through a closed set, `OperationError.context` is a fixed string on the client, and `ReceiverCapabilities` holds owned lists (D30) |
| R6 | The Agent's `state` is the full `ReceiverState`. Its free text carries addresses and raw frames: `FieldIssue.message` is `error.to_string()` from the session (`x3800h_session.rs`), `avr_session.rs:233` formats `connecting to {address}`, `diagnostics` holds raw lines, `UnavailableStatus(raw)` is the receiver's raw frame, and `Suspended.reason` is free text that nothing sets today and a later change could. The source catalog the agent reads for `list_sources` has the same fault: `raw_response` is the receiver's raw reply and `catalog.error` is error text. The exit criterion "the Agent view carries no addresses" would fail | The Agent endpoint serves separate state and source views made of codes and the receiver's labels, with no field that can hold other text, and a test seeds an address into every free-text field of both (D26) |
| R7 | The design says state carries "validity and freshness", but `MonotonicMillis` is milliseconds since the session actor started and the state holds no "now", so a stamp means nothing to a client | The wire carries the validity class and the reason, not stamps. The client rebuilds the stamps as placeholders; a search finds no reader of them in the GUI, the CLI, or `FieldBaseline::capture` (D27) |
| R8 | `dry_run` is "a flag" on the submit resource. A server that ignores unknown fields turns a mistyped dry run into a real write | A dry run is its own route, and request bodies are strict (D28) |
| R9 | The roadmap's resource list omits `register_ad_hoc`, which the CLI's `--host` needs, and the design lists `GET /v1/approvals`, which has no port method | Add `POST /v1/receivers/ad-hoc`. Defer `/v1/approvals` to milestone 7 (D29) |
| R10 | "The Agent endpoint audits the attempt" has no audit event and no principal for an unauthenticated caller, and it reopens phase 3's R11: an agent that probes can rotate away the records the ledger is rebuilt from | A new `AccessRefused` event, `AuditRecord.principal` becomes optional, refusals are throttled as `774c8f1` does, and the rebuild ignores the event (D31) |
| R11 | Tokens have no port, `OperatorAdmin` has no token methods, and authentication is on no port. The phase 3 overview left a misspelt agent label unhandled "because tokens do not exist until milestone 4" | A `TokenStore` port, three `OperatorAdmin` methods, and a label rule: lowercase, one active token per label (D32) |
| R12 | "One server per user, exclusive bind" cannot rest on the socket: a killed server leaves a file that blocks the next bind, and unlinking it first lets two servers run | A lock on `run/server.lock` decides, and a stale socket is removed only while holding it (D34) |
| R13 | A held event stream holds a lease, which keeps the receiver connected. A dead or stalled client, or many streams, would defeat the idle release that frees the receiver's single control connection | A keep-alive, a send timeout that drops a stalled stream, a cap on streams per principal, and revocation that ends a token's streams (D35) |
| R14 | `PUT /v1/config` is last-writer-wins across the GUI and CLI, and the GUI's read-modify-write loses an edit made in between | `ETag` on `GET`, `If-Match` required on `PUT`, and the compare-and-swap done under a lock in the server (D36) |
| R15 | A receiver id may hold any text but control characters, so `/`, `?`, `%`, spaces and Unicode can appear in `/v1/receivers/{id}/...` | Percent-encoding, decoded after the path is split. Tests use each of those characters |
| R16 | The exit test "the Agent endpoint refuses every Operator resource" would be a hand-written list that a new resource escapes | The test is generated from the route table, which has an audience column |
| R17 | The roadmap checks `api-client` for TLS only. `mcp-stdio` inherits `api-client`, and the stdio build must have "no HTTP server framework" | `api-client` is checked now for TLS crates, server frameworks, and hyper's `server` feature, from the resolved graph (D24) |
| R18 | A long wait (`wait_ms` up to 30 s) is longer than a typical request timeout | The server's request timeout is the service's `max_operation_wait` plus five seconds, and the client's is longer still |
| R19 | The design makes `api-contract` the owner of OAuth claims and the registry export | Not in this phase. They arrive with milestone 9 (D41) |
| R20 | A server built for a non-Unix target would fail with a confusing error (roadmap risk) | `compile_error!` on `cfg(not(unix))` in `api-server` and `api-client` |
| R21 | The design has one event stream per receiver carrying state and operation events, but the port's `operation_events()` takes no receiver, so `api-client` could not implement it without filtering a stream for every receiver | State stays at `/v1/receivers/{id}/events` and operation events get `GET /v1/operations/events` |
| R22 | A policy rule can name an agent label that no token could ever hold (`agents: [Claude-Code]`), and loads without complaint, so its restriction applies to nobody | The label rule becomes a function in `policy`, used by token issue and by the policy loader, which refuses an `agents` entry outside it (D43) |

## Packages and edges

| Package | Path | Owns |
| --- | --- | --- |
| `denon-avr-api-contract` | `crates/api-contract` | The `/v1` wire types, the conversions to and from the port's types, the error envelope, the route table, the event encoding, `EndpointPaths`. `serde` and `serde_json` only: no HTTP, no runtime, no hash crate |
| `denon-avr-api-client` | `crates/api-client` | The port over a Unix socket: `ReceiverReads`, `OperationControl`, `OperatorAdmin` |
| `denon-avr-api-server` | `apps/api-server` | A library (`Server`, the endpoints, the request pipeline, the routes) and a binary (settings, composition, signals) |

```text
api-contract → application, domain
api-client   → api-contract, application, domain
api-server   → api-contract, application, domain, infrastructure
```

`api-server` is the composition root for infrastructure, with `diagnostics`. The
design lists a `policy` edge for it as a ceiling; nothing here needs one.
`cli` and `desktop` change in phase 5.

**HTTP stack (D24, the owner's choice).** `api-server` uses `axum` 0.8 for routing,
extractors, and the event-stream response, with `tower` for the layer that
authenticates and `hyper` 1 and `hyper-util` for the connection. It does not use
`axum::serve`. It runs its own accept loop, the pattern axum documents for a
connection the application must admit and configure itself: the loop reads the peer's
credentials before any byte is read, counts the connection against the endpoint's cap,
and serves it with hyper's HTTP/1 builder, whose head-read timeout, header size, and
keep-alive settings are the pipeline's limits. The router is a `tower` service that
the loop calls for each request, and the endpoint, the peer, and the principal reach
a handler as request extensions the loop and the authentication layer set.
`api-client` does **not** use axum, which has no client: it uses `hyper` with the
`http1` and `client` features, `hyper-util`, `http-body-util`, and `bytes`. That keeps
axum and `tower` out of the graph that `mcp-stdio` inherits through `api-client`, and
the boundary rule below keeps them out. Features are turned on one by one (`axum`
without its defaults: `http1`, `json`, `query`, `tokio`), and the exact names are
checked against the crates' documentation when the step that first uses each runs
(`getrandom` and `subtle` in step 4, the server's crates in step 5, the client's in
step 8). `cargo fetch` needs network access once, as phase 3 did for `serde_json` and
`sha2`.

## The wire contract

`api-contract` is the only place a wire shape is defined. Each type has a
conversion from the port's value and, where a client needs the value back, to it.
A `match` over `ReceiverIntent` and over `OperationStatus` has no wildcard arm, so a
new variant fails to compile in `api-contract` before it can reach a client.

### Conventions

- HTTP/1.1 over a Unix socket; no HTTP/2, no upgrade, no `Expect: 100-continue`.
  Bodies are `application/json; charset=utf-8`, `snake_case`, times in
  milliseconds since the Unix epoch, durations in milliseconds.
- **Requests are strict**: `deny_unknown_fields`, so a mistyped field is a `400`
  that names it. **Responses are lenient**: a client ignores fields it does not know,
  which is how the contract grows. A client given a value it cannot represent (an
  unknown status name) reports `Unavailable("the server speaks a newer contract")`
  and does not guess.
- A request body needs `Content-Length` and `Content-Type`; a chunked request body
  is refused (`411`).
- The credential is `Authorization: Bearer <token>`, exactly one such header. A
  token in the query string is a `400` and is never logged.
- Receiver ids are percent-encoded in paths by the client (`paths::encode_segment`)
  and decoded by axum's `Path` extractor, which decodes after the route has matched, so
  an encoded `/` stays inside its segment. A decoded id that is not valid UTF-8, or
  that `ReceiverId::new` refuses, is a `400`.

### Resources

| Resource | Port method | Served to |
| --- | --- | --- |
| `GET /v1/health` | `health`, and the server's own state | Both. Adds `contract: 1`. The Operator's response also carries a `server` section the service cannot know: whether the Agent endpoint is on and, if not, why, and whether the token store opened |
| `GET /v1/receivers` | `receivers` | Both |
| `GET /v1/receivers/{id}/state` | `state`, first snapshot | Both, as different views |
| `GET /v1/receivers/{id}/sources` | `source_catalog` | Both |
| `GET /v1/receivers/{id}/events` | `state`: the first snapshot, then each change | Both, as different views |
| `GET /v1/operations/events` | `operation_events` | Both; an agent's carries its own operations |
| `POST /v1/receivers/{id}/operations` | `submit` | Both |
| `POST /v1/receivers/{id}/operations/dry-run` | `dry_run`, or `dry_run_as` when the Operator names `as_agent` | Both; `as_agent` exists only in the Operator's request type |
| `GET /v1/operations/{id}?wait_ms=` | `operation` | Both; an agent sees its own |
| `POST /v1/operations/{id}/cancel` | `cancel` | Both; an agent cancels its own |
| `POST /v1/receivers/discover` | `discover` | Operator |
| `POST /v1/receivers/ad-hoc` | `register_ad_hoc` | Operator |
| `GET /v1/config`, `PUT /v1/config` | `configuration`, `save_configuration` | Operator; `ETag` and `If-Match` |
| `GET /v1/receivers/{id}/quick-select-names`, `GET /v1/receivers/{id}/http-information` | the two reads | Operator |
| `POST /v1/receivers/{id}/refresh` | `refresh` | Operator |
| `GET /v1/policy`, `POST /v1/policy/reload` | `policy`, `reload_policy` | Operator |
| `GET /v1/audit?limit=&cursor=` | `audit` | Operator |
| `POST /v1/tokens`, `GET /v1/tokens`, `DELETE /v1/tokens/{id}` | `issue_token`, `tokens`, `revoke_token` | Operator |

`GET /v1/approvals` waits for milestone 7, which has the first port method to
serve it. OAuth client registration waits for milestone 9. A resource that is not
in an endpoint's table does not exist on it.

### Route table

`api-contract::routes::TABLE` is the table of `(RouteId, method, path pattern,
audience)`, the pattern in axum's `{id}` syntax and the audience `Both` or `Operator`.
`api-server` builds each endpoint's `Router` from it by calling `handler_for(RouteId)`,
an exhaustive `match`, so a row without a handler does not compile and a handler
cannot exist outside the table. The Agent endpoint's router holds only the `Both`
rows. The tests walk the table. The contract also has the client's side: one function
per resource that builds its path, with `encode_segment` for ids.

### The state views

```text
StateView (Agent)    receiver, revision, epoch, and per field:
                       { value, validity: current | stale | unknown | unavailable,
                         reason: disconnected | query_failed | expired | receiver_changed }
                     no diagnostics, no issue text, no evidence text
OperatorStateView    StateView plus per field { issue: text, evidence: text }
                     and the bounded diagnostics
SourcesView (Agent)  entries { id, display_name?, visibility }, freshness, generation,
                     response (complete | partial | unsupported | malformed |
                     timeout | disconnected); no raw reply, no error text
OperatorSourcesView  SourcesView plus the raw reply and the error text
```

The Agent views are different types, so there is no field to leave a string in. The
display name is the receiver's own label for a source, which the agent needs to name
a source to the user. The
client rebuilds a `ReceiverState` from either: the stamps (`frame_seq`,
`observed_at`, `valid_until`) are placeholders (`0`, `0`, and `u64::MAX`), and a
field's `synchronization` is `NotStarted`. The only readers of those are the
reducer and `expire_fields`, which a client never runs; `FieldBaseline::capture`
and the GUI's projection read the validity class, `last_good.value`, and the
issue text, all of which the view carries. The server publishes a new snapshot when
a field expires, so a client sees `stale` without a clock.

### Operations and errors

- `SubmitRequest { intent, idempotency_key? }`. `intent` is one tagged variant per
  `ReceiverIntent`, with volume as `{"kind":"volume","level":{"half_steps":-71}}` or
  `{"kind":"volume","level":"minimum"}`, which cannot be both or neither. The response is the operation's snapshot:
  `id`, `receiver`, `intent`, `status`, `dispatch`, `confirmed`, `reason`,
  `observation`, with `by` on a superseded status.
- `DryRunRequest { intent }` for an agent; the Operator's adds `as_agent`. The
  response is `{ decision, policy_digest? }`; an agent's carries no rule ids.
- Operation ids are decimal strings in paths and numbers in bodies.

| `ControlError` | HTTP | `code` | Notes |
| --- | --- | --- | --- |
| `NotFound(what)` | 404 | `not_found` | `what` is `receiver`, `operation`, or `token`; the client maps through that closed set and any other word becomes `resource` |
| `Forbidden` | 403 | `forbidden` | |
| `InvalidRequest(m)` | 400 | `invalid_request` | |
| `Unavailable(m)` | 503 | `unavailable` | |
| `TooLate(snapshot)` | 409 | `too_late` | The body carries the snapshot |
| `RateLimited{retry_after}` | 429 | `rate_limited` | `Retry-After` in whole seconds, rounded up |
| `Receiver(error)` | 502 | `receiver_error` | `kind`, and `message` (an agent's is fixed text already) |

The server adds `401 unauthenticated`, `404` for an unknown route, `405`,
`408 request_timeout`, `411`, `412 precondition_failed`, `413 body_too_large`,
`415`, `428 precondition_required`, and `503 shutting_down`. Every error body is
`{"error":{"code","message",...}}`. `401` is the same for a missing, unknown, and
wrong-kind credential.

### Config, events, tokens

- **Config.** `GET` returns the configuration and `ETag` (a SHA-256 of the
  canonical JSON of what was read, computed by the server and opaque to a client).
  `PUT` requires `If-Match` and compares it with
  the file as it is now under one async lock in the server, then calls
  `save_configuration`. A missing header is `428`, a mismatch `412`.
- **Events.** `Content-Type: text/event-stream`. There are two streams, one for each
  port method that returns one. `GET /v1/receivers/{id}/events` is the state
  subscription: the first event is the full state (`event: state`), then one per
  change, coalesced like the subscription it comes from. `GET /v1/operations/events`
  is `operation_events`: `event: operation` carries a snapshot of one of the caller's
  operations (every operation, for the Operator) and `event: missed` says the reader
  fell behind. The port has no per-receiver operation stream, and the design's single
  stream per receiver would have left `api-client` to filter one. In both, `event:
  end` says why the stream is closing (`session_closed`, `revoked`, `shutdown`, and
  for an Agent's state stream `max_age`) and a comment line every 15 seconds keeps the
  connection written to. The server writes the stream with axum's `Sse` response and
  the contract holds the event names, their payload types, and the parser the client
  reads them with. There is no
  `Last-Event-ID` and no replay: a client that reconnects reads the state again and
  asks `GET /v1/operations/{id}` for any operation it was watching. An operation
  stream holds no lease on a receiver.
- **Tokens.** `POST` returns `{ id, label, token }` once. `GET` returns
  `{ id, label, created, revoked? }` and never a token or digest. `DELETE` is
  idempotent.

## Port and audit additions

All in `application`; the four implementors (`ServiceHandle`, the `Stub` in
`control.rs`, the CLI's `Fake`, the GUI's `FastPort`) gain the new methods.

```text
ReceiverCapabilities.inputs, surround_modes   Vec<String>   (was &'static [&'static str])

TokenId                       newtype; "t-" and 8 lowercase hex digits
label rule                    policy::label_is_well_formed(&str) -> bool, enforced when a
                              token is issued and when a policy file is loaded (D43), not
                              by AgentLabel: lowercase a-z 0-9 . _ -, starts with a letter
                              or digit, at most 64 characters, not "unauthenticated"
TokenRecord                   { id: TokenId, label: AgentLabel, created: WallTime,
                                revoked: Option<WallTime> }
IssuedToken                   { record: TokenRecord, secret: TokenSecret }
                              TokenSecret is shown once; Debug prints "<redacted>"
OperatorAdmin::issue_token(label: AgentLabel) -> IssuedToken
OperatorAdmin::tokens() -> Vec<TokenRecord>
OperatorAdmin::revoke_token(id: TokenId) -> TokenRecord

TokenStore (port)             issue(label, now) -> IssuedToken
                              list() -> Vec<TokenRecord>
                              revoke(id, now) -> TokenRecord
                              authenticate(presented: &str) -> Credential
                              is_active(id: &TokenId) -> bool
                              changes() -> watch::Receiver<u64>   (bumps on every revoke)
Credential                    Agent(AgentLabel, TokenId) | OperatorToken | Unknown
AgentPath::with_tokens(SharedTokenStore)

AccessRefusal                 { endpoint: EndpointKind (Operator | Agent),
                                reason: RefusalReason, principal: Option<Principal>,
                                peer_uid: Option<u32>, resource: Option<String> }
RefusalReason                 PeerNotAdmitted | NoCredential | UnknownCredential
                              | OperatorTokenOnAgentEndpoint | AgentTokenOnOperatorEndpoint
                              | ResourceNotServed
AuditEvent::AccessRefused     { endpoint, reason, peer_uid, resource: Option<String>,
                                suppressed: u32 }
                              resource is the route pattern an agent probed ("/v1/policy"),
                              never the raw path, which is the caller's text
AuditEvent::TokenIssued       { id, label }
AuditEvent::TokenRevoked      { id, label }
AuditRecord.principal         Option<Principal>   (None: no valid credential)
ControlService::record_refusal(AccessRefusal)    throttled, Flushed, bounded to one second
```

`authenticate` is called by `api-server` before it asks `ControlService::handle`
for a principal; the service itself never sees a credential. On a service built by
`new`, the three token methods answer `Unavailable`; an Operator-only check comes
first, so an agent is `Forbidden` whatever the service can answer.

**Refusal throttle.** `record_refusal` keeps the last write time per key
`(endpoint, reason, peer_uid or label)`. The first refusal of a key is written; a
later one inside 60 seconds only adds to a count; the next write after 60 seconds
carries the count as `suppressed`. At most 256 keys are tracked (the oldest
dropped), and at most 30 records a minute are written over all keys, so a flood of
distinct keys cannot rotate the log either. The append is bounded to one second, and
a failure is logged and does not change the audit health that `health` reports,
which stays the result of the gate's own appends; otherwise a caller could mark the
audit log failing, and so refuse every Agent write, by being refused. The ledger's
rebuild reads `Dispatching`
and `Finished` only, so the new events do not affect it, and a reader that does not
know them skips them as it does any unknown kind. `AUDIT_SCHEMA` stays 1: nothing
outside this repository reads the log, and the reader of the current build returns
nothing for a record whose principal is null (`decode_record` in `audit_jsonl.rs`
refuses any principal but `"operator"` or an agent object), so an older build skips
it as it skips an unknown kind. Token events are `Flushed`.

## Credentials (infrastructure)

`FileTokenStore` in `crates/infrastructure/src/token_store.rs`.

- `credentials/operator.token` (0600) holds the Operator token, `daro_` and 43
  base64url characters (32 random bytes from `getrandom`). It is created with
  `create_new` at first start and never rewritten while the server runs. To rotate
  it, delete the file and restart.
- `credentials/agent-tokens.json` (0600) holds an array of `TokenRecord` and the
  token's SHA-256 digest (hex), never the token. A change writes a temporary file in
  the same directory (created 0600), `sync_all`s it, and renames it over the
  original. A token is `dara_` and 43 base64url characters.
- `authenticate` hashes the presented string, compares the digest with every stored
  digest and the Operator's in constant time (`subtle`) without returning early, and
  never compares a revoked record. A string with neither prefix is `Unknown`
  without hashing.
- **Label rule (D32).** `issue` refuses a label that fails the rule above or that
  already has an active token. Revoke and issue rotates a token.
- A damaged `agent-tokens.json` (one that cannot be parsed, holds a label outside the
  rule or two active tokens for one label, or has wider permissions than 0600) is a
  fault, not an error: `FileTokenStore::open` still succeeds, because the Operator's
  token does not live in that file and the owner must not be locked out of their own
  server. The store holds no Agent tokens, refuses to issue or revoke, does not replace
  the file, and reports why through `TokenStore::fault`. The server then creates no
  Agent endpoint, reports the token store as unavailable in the Operator's health, and
  the three token methods answer `Unavailable`. A damaged `operator.token`, or a
  `credentials/` directory with wider permissions, is an error: nobody could be served.
- A directory or file that exists with wider permissions than 0700 or 0600 is an
  error, as in the audit adapter. The store creates `credentials/` at 0700.
- Revocation bumps `changes()`. An event stream or a wait compares its token against
  the store when it changes and ends if the token is revoked.

## The server

### Settings

`<data>/server.yaml`, read by `apps/api-server/src/settings.rs` with
`deny_unknown_fields`. Its absence is a valid file. The Operator endpoint needs no
setting. The Agent endpoint exists only when `agent_endpoint` is present, and its
keys are fixed by S2:

```yaml
agent_endpoint:
  directory: /Users/Shared/Denon AVR Remote    # S2 decides the default
  uids: [502]                                  # the accounts admitted
```

### The library

```text
ServerConfig     paths: EndpointPaths, agent: Option<AgentEndpointConfig>, limits: Limits
EndpointPaths    under(dir) -> run/operator.sock, run/server.lock,
                 credentials/operator.token, credentials/agent-tokens.json
Server::start(service: Arc<ControlService>, tokens: SharedTokenStore,
              config: ServerConfig) -> Result<RunningServer, StartError>
              (the store is required: the Operator's token comes from it)
RunningServer    operator_socket() -> &Path; agent_socket() -> Option<&Path>;
                 shutdown(self) -> impl Future
StartError       AlreadyRunning | SocketPathTooLong { len, max } | Directory { path, why }
                 | Bind { path, why } | Credentials(why)
ServerHealth     agent_endpoint: On | Off { reason }, token_store: Ok | Unavailable
                 (served in the Operator's health response only; the port's `health` is
                 the service's and has no such field, and `ApiClient::server_health()`
                 is an inherent method for the Operator audience, outside the traits)
```

The binary parses its arguments (`--data-dir DIR`, default `data_directory()`, and
`--exit-with-parent`), reads the settings, builds the infrastructure, calls
`ControlService::start`, and hands the result to `Server::start`.
Tests call `Server::start` over a fake session, so the binary has almost no logic.

### Start

1. Create `run/` and `credentials/` at 0700 if missing; a wider existing mode is an
   error. The data directory's own mode is not touched.
2. Take `File::try_lock` on `run/server.lock` (the lock file is created 0600). A
   failure is `AlreadyRunning`, and the binary exits with status 75. The server's own
   uid is the owner of the lock file it just created, which avoids a call to
   `geteuid`.
3. Check each socket path against the platform's `sun_path` limit (104 bytes on
   macOS), and fail with the path's length and the limit.
4. If `agent_endpoint` is configured and a token store opened, check its directory
   (below). Otherwise record why the Agent endpoint is off.
5. Only now remove a stale socket, and only a path that `lstat` reports as a socket
   owned by the server's uid, for `operator.sock` and for the Agent socket alike. Any
   other thing at the path (a file, a link, someone else's socket) is an error and is
   left alone, because `/Users/Shared` is writable by everyone and the unlink must not
   follow a link planted there.
6. Bind, and `chmod` the Operator socket 0600. Its directory is already 0700, so no
   other uid reaches it between the two. Check the Agent socket's directory and owner
   again, as the Agent endpoint's paragraph says.

The binary opens the token store and the audit log and runs `ControlService::start`
before it calls `Server::start`, so the steps above start from a running service.

A client that wants to know whether a server is running connects to
`operator.sock`; the lock is the server's, not the client's.

### Endpoints and the connection

An endpoint is `{ kind, socket path, admitted uids, credential kind, route table }`.
The Operator endpoint admits the server's own uid and accepts only the Operator
token. The Agent endpoint admits the configured uids and accepts only Agent tokens.

`UnixStream::peer_cred()` gives the connecting process's effective uid and gid, and
a pid that is optional; the pid is logged and decides nothing, because it can be
reused. A connection from an uid that is not admitted is closed before any byte is
read, and `record_refusal(PeerNotAdmitted)` is called.

**The Agent endpoint's directory** (S2 fixes the rest): an absolute path; `lstat`
says a directory, not a link; owned by the server's uid; not group- or world-
writable. If it does not exist, the server creates it at 0700 and the owner admits
the agent account (an ACL or a group, as S2 settles). Any other state is "the Agent
endpoint is not created", with the reason in the log and in the `server` section of
the Operator's health response.

### The request pipeline

For every connection and request, in this order, so a caller learns nothing before it
is authenticated. Steps 0 to 2 are the accept loop and hyper's HTTP/1 builder. Steps 3
and 5 are one `axum` middleware layer, added to the `Router` after its routes and its
fallback. A layer added that way wraps every one of them (axum documents it), a path that
matches nothing and a method that is not allowed included, so an unknown path is a
`401` before it can be a `404`; a test pins it. Steps 4 and 6 are the router and the
handler.

0. Admission: the peer's uid, as above. A connection that fails it is closed unread.
1. Connection caps: 16 on the Operator endpoint, 32 on the Agent endpoint. One more
   is closed.
2. Head: 5 seconds to read it, at most 16 KiB, HTTP/1.1. hyper applies the same
   timeout to the next request's head on a connection kept open, so an idle connection
   is closed after five seconds.
3. Authenticate (`401` for none, more than one, an unknown token, or the wrong
   kind). Every failure is `record_refusal` with its own reason; the response never
   says which. An Operator token on the Agent endpoint is its own reason.
4. Route in this endpoint's table: `404` or `405`. A valid Agent token on an
   Operator route is `404` and `record_refusal(ResourceNotServed)`.
5. Body: `Content-Length` at most 16 KiB (1 MiB for `PUT /v1/config`), read in 10
   seconds, strict JSON into the contract type, then into the port's type. The layer
   refuses a declared length over the cap before the body is read, since axum's
   `DefaultBodyLimit` acts only when an extractor reads the body, and sets that limit
   as well. A validation error names the field and never echoes the body.
6. `service.handle(principal)`, the port call, and the error mapping above. A
   request's total time is capped at `max_operation_wait` plus five seconds; an
   event stream is exempt.

### Streams and waits

`GET /v1/receivers/{id}/events` resolves `state(receiver)` first, so a failure is a
status code, then streams. The body of either stream is fed by a channel of one item:
the task that holds the subscription sends each event with a 10-second timeout and
ends the stream when it expires, which drops the subscription and releases the
receiver. A principal may hold four streams of either kind; one more is `429
too_many_streams`, and the connection cap bounds the endpoint as a whole.

Two limits of that are worth saying. A keep-alive comment does not detect a client that
is connected but not reading: it is small, and the socket's buffer takes hours of
comments to fill. A send timeout detects it only when an event is produced, and a quiet
receiver produces none. So an Agent's state stream also ends after ten minutes with
`event: end` (`max_age`), and the client resubscribes; a client that has stopped
reading cannot, and its lease is released. The Operator's streams have no maximum age,
because the GUI's subscription is meant to be held. A client that has *closed* its end
is found by the keep-alive write failing, within 15 seconds.

A wait is the port's own `operation(id, wait)` with `wait_ms` capped by the service.
`GET /v1/receivers/{id}/state` takes a subscription, reads its first snapshot, and
drops it.

### Shutdown

SIGINT, SIGTERM, and end of standard input in parent-pipe mode (`--exit-with-parent`)
start one shutdown: stop accepting, send `event: end` (`shutdown`) to every stream,
call `ControlService::shutdown`, remove both sockets, and release the lock. A
second signal exits at once. Killing the process leaves a stale socket and a
released lock, which the next start reclaims.

## The client

`api-client` implements `ReceiverReads`, `OperationControl`, and `OperatorAdmin`
over one endpoint and one token: `ApiClient::connect(Endpoint { socket, token,
audience }) -> Result<ApiClient, ConnectError>`. It opens one connection per
request (a Unix socket is cheap and a reused connection can be reset by a restarted
server), and one for each event stream. `audience` picks the state view to decode.

- A connection error is `ControlError::Unavailable("receiver service unavailable")`,
  the words the design gives an agent when the server is down. The Operator's text
  adds the socket path.
- `state(receiver)` opens the event stream and returns once the first `state`
  event arrives. It builds a `StateSubscription` over a `watch` channel that a task
  feeds from later events and ends with the "session closed" error on `event: end`
  or a closed stream, so a caller that resubscribes on that error behaves as it does
  against the in-process service.
- Timeouts: connect 2 seconds, a plain request 10, a wait `wait_ms` plus 10. An
  event stream has none; it ends with the connection.
- The credential is held in a type whose `Debug` prints `<redacted>` and is never
  put in an error.

## Boundary rules added

- The three edges above, and no other edge involving the new packages.
- `crates/api-contract/src` names no `axum`, `hyper`, `tower`, `tokio`, `http`, or
  `std::fs`.
- `cargo tree -p denon-avr-api-client --edges normal` lists no TLS crate
  (`rustls`, `native-tls`, `openssl`, `security-framework`, `ring`, `aws-lc-rs`,
  `webpki`) and no server framework (`axum`, `warp`, `actix-web`, `rocket`,
  `tower-http`, `poem`, `salvo`); `cargo tree -p denon-avr-api-client --edges
  normal,features` shows no `server` feature of `hyper` and none of `hyper-util`
  (`server`, `server-auto`, `server-graceful`). The kinds are `normal,features`
  because `--edges features` alone also walks dev-dependencies (checked on the
  application crate, where it shows the test-only `tokio` feature `test-util`), and
  a test-only fake server must not trip the rule. The lists live in the script, and
  milestone 6 reuses them for `mcp-stdio`.
- Receiver sessions are constructed in `api-server` and `diagnostics` only. The
  resolved graph of `api-contract` and `api-client` reaches neither `infrastructure`
  nor `protocol`. The general rule, that only `api-server` and `diagnostics` reach
  them, cannot be enforced while `cli` and `desktop` link `infrastructure`; those two
  edges stay in the script's allowlist, marked as temporary, and milestone 5 removes
  them and turns the general rule on.
- The library roots of `api-server` and `api-client` carry
  `#[cfg(not(unix))] compile_error!`.
- The `operate` allowlist is unchanged.

## Files

| File | Change |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | Three members; `axum`, `tower`, `hyper`, `hyper-util`, `http-body-util`, `bytes` (the client takes only the last four); `getrandom`, `subtle` in infrastructure |
| `crates/application/src/{control,audit,tokens,service}.rs` | Port additions, token and refusal types, `record_refusal` |
| `crates/policy/src/{lib,label,config}.rs`, `crates/policy/tests/cases.rs`, `crates/infrastructure/tests/policy_yaml.rs` | `label_is_well_formed`, the check in `PolicyConfig::new` (D43), and the one existing test that names an uppercase label |
| `crates/api-contract/src/{lib,error,routes,paths,receivers,state,operations,config,events,tokens,admin}.rs` | New |
| `crates/api-contract/tests/{golden,round_trip,leaks}.rs`, `tests/golden/*.json` | New |
| `crates/api-client/src/{lib,transport,sse,mapping}.rs` | New |
| `apps/api-server/src/{lib,main,settings,start,endpoint,pipeline,routes,events,shutdown}.rs` | New; `settings.rs` arrives in step 9 |
| `apps/api-server/tests/{support/mod,pipeline,contract,agent_endpoint,lifecycle,streams,client}.rs` | In-process server, fake session, real service |
| `docs/examples/server.yaml` | The settings sample, step 9 |
| `crates/infrastructure/src/token_store.rs`, `audit_jsonl.rs` | Token store; the new events and an optional principal |
| `apps/api-server/tests/live_x3800h_api.rs`, `Makefile` | Armed live test through the server |
| `apps/cli/src/main.rs`, `crates/gui-lib/src/bridge.rs` | One-line stubs for the new methods in their test fakes |
| `tools/check-boundaries.sh` | Edges, purity, the resolved-graph checks |
| `docs/research/agent-endpoint-access-macos.md` | S2's output |
| `ARCHITECTURE.md`, `AGENTS.md`, `docs/README.md`, `docs/development.md` | Promotion at step 11; `development.md` names `make test-live-x3800h-api` |

## S2: what the hand check decides

Run on the Mac with the dedicated agent account. Nothing below has been run. The
checks need a socket to connect to: after step 5 a throwaway server will do
(`denon-avr-api-server --data-dir <scratch>`, with the Agent endpoint given to a test
build), and before it a short script that binds a Unix socket and prints the peer's
credentials is enough for checks 1, 3, and 5.

1. **Directory and socket access.** Whether the agent account can `connect()` to a
   socket in a directory it reaches by (a) a directory ACL with an inherited entry,
   or (b) a shared group the owner and the agent account both belong to; and what
   mode the socket needs. On macOS, `connect()` is believed to need write
   permission on the socket file and search permission on each directory above it;
   the check confirms or corrects that.
2. **Location.** Whether `/Users/Shared/<name>` with `lstat` and owner checks is
   acceptable, or whether the path should be somewhere else the owner controls.
3. **Peer credentials.** That `peer_cred()` reports the agent account's uid, and
   what it reports for a process started with `sudo -u` and for one started by the
   agent host.
4. **What fails.** The agent account cannot open `credentials/`, `run/`, or the
   audit directory, and cannot connect to the Operator socket.
5. **Survival.** Whether the access still holds after the server restarts and the
   socket is recreated.

The result is a note in `docs/research/` that fixes `agent_endpoint`'s keys and
default directory. The code of step 9 takes the access rule as data, so either
outcome fits.

## Decisions

**Answered by the owner on 2026-10-08.**

- **Where the server's files live (D25).** `run/` and `credentials/` subdirectories at
  0700 under the data directory, leaving its 0755 alone. The alternative was to
  tighten the data directory itself to 0700 on first run, which would change a
  directory 3.0.0 created.
- **One active token per label (D32).** So two agents cannot share a label, a budget,
  and each other's operations by accident. The alternative allowed overlapping tokens
  for a rotation with no gap.
- **The server's HTTP stack (D24).** axum, over this plan's recommendation of hyper
  alone. The client stays on hyper, so the stdio build's graph is unaffected.

**Open, the owner's.** The Agent endpoint's directory and admission are decided by
S2's hand check, with `/Users/Shared/Denon AVR Remote` as the candidate; asking before
the check would be guessing.

**Settled by the review.** Any can be reversed in a follow-up commit.

| # | Decision | Reason |
| --- | --- | --- |
| D24 | `axum` on the server, in its own accept loop; `hyper` alone on the client; a route table with an audience column that builds each endpoint's router | The owner chose axum for the server on 2026-10-08 over the plan's recommendation of hyper alone. The accept loop keeps admission before the first read and the limits in hyper's builder. axum has no client, so `api-client` stays on hyper and `mcp-stdio`'s graph does not gain axum or `tower`. R17 |
| D25 | Endpoint and credential files are in `run/` and `credentials/` subdirectories at 0700 | R4. The data directory's mode is not this phase's to change. Answered by the owner |
| D26 | The Agent state view is a separate type made of codes | R6 |
| D27 | The wire carries the validity class and reason, not stamps; the client rebuilds placeholders | R7. Adding a "now" to `ReceiverState` would touch the domain, the session, and every reducer for a value nothing reads |
| D28 | A dry run is its own route; requests are strict, responses lenient | R8 |
| D29 | `POST /v1/receivers/ad-hoc` is added; `/v1/approvals` waits for milestone 7 | R9 |
| D30 | `ReceiverCapabilities` holds `Vec<String>`; `NotFound` maps through a closed set; the client's `OperationError.context` is a fixed string | R5. The only constructor of `ReceiverCapabilities` is `ServiceHandle::receivers` |
| D31 | `AuditRecord.principal` is optional; `AccessRefused` is throttled and ignored by the rebuild | R10 |
| D32 | `TokenStore` is a port the service holds through `AgentPath`; a label is lowercase `[a-z0-9._-]`, one active token per label, and `unauthenticated` is reserved | R11. The case rule is what keeps a `Claude-Code` token out of a `claude-code` rule |
| D33 | Tokens are `dara_` (Agent) and `daro_` (Operator) plus 43 base64url characters; SHA-256 digests only; constant-time compare | The design's token rule; the prefix lets a secret scanner find a leak |
| D34 | One server per user is a `File::try_lock` on `run/server.lock`; a stale socket is removed only under it | R12 |
| D35 | Streams: 15-second keep-alive, 10-second send timeout, four per principal, no replay; an Agent's state stream ends after ten minutes; state and operations are separate streams | R13, R21. The maximum age is what bounds a client that is connected and not reading, which neither the keep-alive nor the send timeout can see on a quiet receiver |
| D36 | Config writes use `ETag` and `If-Match` | R14 |
| D37 | Caps: 16 and 32 connections, 16 KiB head and body (1 MiB for config), 5 second head and idle, 10 second body | A caller is anonymous until the pipeline's authentication stage. The Operator endpoint is separate, so an agent that holds every connection of the Agent endpoint cannot starve the Operator |
| D38 | `server.yaml` configures the Agent endpoint; with no file there is none | Fail closed: nothing is exposed to an agent until the owner says so |
| D39 | `EndpointPaths::under(dir)` is in `api-contract` | Phase 5 removes the infrastructure edge from `cli` and `desktop`, and then only `data_directory()` needs to move |
| D40 | The transitive boundary rules cover the new packages; `cli` and `desktop` are named exceptions | R2 |
| D41 | OAuth claims, client registration, and the registry export are not in `api-contract` yet | R19 |
| D42 | `compile_error!` on `cfg(not(unix))` | R20 |
| D43 | `policy::label_is_well_formed(&str) -> bool` is the label rule. `application` uses it when a token is issued, and `PolicyConfig::new` refuses an `agents` entry that fails it, naming the rule | R22. No token existed before this phase, so no label can be in use under another spelling. A rule that names a well-formed label nobody holds (a misspelling) is still not caught, and `dry_run_as` is how the owner checks a tier |

## Live checks

`apps/api-server/tests/live_x3800h_api.rs`, run by `make test-live-x3800h-api`,
is `#[ignore]` and armed exactly like the other two: `ALLOW_RECEIVER_WRITES=1`,
`DENON_X3800H_HOST`, and `DENON_X3800H_SAFE_VOLUME_HALF_STEPS`. It starts a server
in the test process on a socket under a temporary directory with the real connector,
a temporary policy, and a temporary audit directory, issues an Agent token, and drives
the receiver through `ApiClient`:

1. Operator `get state` and `refresh` (read only).
2. As in the phase 3 test, limits built from the observed level `L`, then the Agent
   token: volume to `L + 1.0` is `completed`; to `L + 2.0` is `approval_unavailable`;
   to `L + 3.5` is `denied`. The test asserts `L + 3.5 <= S` before it writes.
3. Restore `L` as the Operator, through the Operator endpoint.
4. The audit directory the test prints holds the records of step 2, none holding a
   token, and `credentials/` shows `drwx------` and `-rw-------`.

The test lives in `api-server`, which already depends on `infrastructure`. Putting it
in `infrastructure` would make that package depend on the one that depends on it.
