# Architecture

This is the authoritative description of the current implementation. Protocol
research and archived plans provide evidence and history; neither changes the
rules here.

## Workspace and dependencies

The Cargo workspace contains independently compiled packages. Its permitted
workspace edges are enforced by `make boundary`.

| Package | Owns |
| --- | --- |
| `crates/domain` | Receiver identity, configuration, capabilities, state, intents, and evidence/validity types. |
| `crates/protocol` | AVR, HEOS, and AppCommand framing, wire values, and parsing. |
| `crates/policy` | The policy engine: a pure function from a request, the receiver's state, and the recent volume changes to a decision. Depends on `domain` alone. |
| `crates/application` | Use-case policy, ports, status/control policy, the control-service port, and the in-process control service with its Operation Gate, Agent path, audit records, and budget ledger. |
| `crates/api-contract` | The Control API's wire types, version 1: the views of state and sources (separate types for an agent and for the Operator), the error mapping, intents and operations, the route table, and the paths a client builds. Depends on `application` and `domain` and on `serde`; it names no HTTP stack, runtime, or filesystem. |
| `crates/api-client` | The control-service port over the server's Unix socket: it implements `ReceiverReads`, `OperationControl`, and `OperatorAdmin` by asking the Control API server. HTTP/1.1 over a Unix socket only: no TLS crate, no server framework, no `server` feature of `hyper`. Depends on `api-contract`, `application`, and `domain`. |
| `apps/api-server` | The Control API server: it hosts the in-process control service behind two Unix-socket endpoints, the Operator's and, when `server.yaml` names a directory and the uids to admit, the Agent's, with the request pipeline that admits, authenticates, and routes. Nothing in the CLI or the GUI uses it yet. |
| `crates/infrastructure` | SSDP discovery, YAML persistence, TCP/HTTP adapters, concrete receiver sessions, the JSON Lines audit log, the policy file loader, the file-backed token store, and the system clock. |
| `crates/gui-lib` | Iced presentation state, reducers, views, the projection of receiver state into them, and the bridge to the control-service port. |
| `apps/cli` | Short-lived CLI composition over the in-process control service. |
| `apps/desktop` | Native GUI composition of the control service, and logging. |
| `apps/diagnostics` | Explicit, read-only evidence probes. |

```text
protocol       → domain
policy         → domain
application    → domain, policy
api-contract   → application, domain
api-client     → api-contract, application, domain
api-server     → api-contract, application, domain, infrastructure
infrastructure → application, domain, policy, protocol
gui-lib        → application, domain
cli            → application, domain, infrastructure
desktop        → gui-lib, application, domain, infrastructure
diagnostics    → domain, infrastructure, protocol
```

`domain` imports no workspace package. `protocol` and `policy` depend only on
`domain`, and `application` on `domain` and `policy`; infrastructure may compose
all four core crates. `gui-lib` uses application and domain only. Delivery packages compose the dependencies their
delivery role requires, but do not create competing architectural contracts.

## Session boundary

`CanonicalReceiverSession` (`crates/application/src/session_v3.rs`) is the one
definition of a receiver session, and the control service owns every session
through it. A session serializes one receiver connection: it publishes complete
state with per-field validity, answers reads, and runs one operation at a time
with the outcome and dispatch certainty the operation lifecycle needs.
`ReceiverConnector` (in `ports.rs`) opens one. The service connects on demand,
holds the session while anything uses it, and releases it after an idle time;
nothing else, including the GUI and CLI, constructs or names a session.

A reconnect or receiver change starts a new epoch, and authority from the old
connection ends with the old epoch. When the transport reconnects, the session
reads every core field again at once, which only queries, rather than leaving the
fields stale until its next five-second sweep; a reconnect that happens in the
middle of an operation's confirmation is picked up by the sweep. Uncorrelated receiver lines remain events
rather than being assigned to a command opportunistically. The 3.0.0 controller,
its compatibility adapter, and the `ReceiverSession` and `SessionFactory`
contracts they served were retired in version 4; `make boundary` fails if their
names return.

## Control service

The control-service port in `crates/application/src/control.rs` is how a
surface observes and controls receivers. It is three traits split by
authority, and a surface takes only the traits it needs:

- `ReceiverReads`: the saved-receiver listing, a state subscription, the
  source catalog, and `health`, which reports coarse states (policy, audit log,
  ledger, approval) with no paths or error text.
- `OperationControl`: submitting an operation, reading its status, cancelling
  it, and the operation event stream, for the caller's own operations, and
  `dry_run`, which reports what the policy would decide without making an
  operation or a record.
- `OperatorAdmin`: discovery, receiver configuration, ad hoc receivers, the
  inspection reads no agent tool uses (Quick Select names and HTTP information),
  and `refresh`, which reads every core field again. `refresh` is a read: it
  connects if the receiver is released, dispatches nothing, and does not use the
  gate. It also holds `dry_run_as` (a dry run as a named agent label, which does
  not spend that agent's allowance), `policy` and `reload_policy`, `audit`, and the
  token methods `issue_token`, `tokens`, and `revoke_token`. A token belongs to one
  agent label, is shown once when issued, and is kept only as a digest. A label is
  lowercase letters, digits, `.`, `_`, and `-` (`policy::label_is_well_formed`), and
  a label holds at most one active token. The methods answer `Unavailable` on a
  service with no token store, and `Forbidden` to an agent whatever it has.

`AgentControl` is the first two and `OperatorControl` all three. A handle is
bound to one `Principal` when it is created and the principal is never a request
parameter. `ControlService` (`crates/application/src/service.rs`) implements the
port in process. `ControlService::new` serves the Operator alone, makes no audit
call, and refuses an Agent handle; the CLI and the GUI use it. `ControlService::start`
also builds the Agent path described under [Policy and audit](#policy-and-audit),
and serves an Agent handle. The [Control API](#control-api) server is what exposes an
Agent handle to anything outside the process.

**Receiver identity.** A receiver's id is the name of its saved configuration
entry and does not change when its address does. A receiver chosen only by
address gets an id with the reserved `adhoc:` prefix, which saved names cannot
use. It is for Operator use and is never listed to an Agent, and the Agent view
carries no network address. `receiver_id` in `receiver_selection` is the one
derivation.

**Receiver connection.** The service holds at most one session per receiver,
opened through `ReceiverConnector` on the first read, subscription, or
operation, and synchronized before the first state is returned. A read,
subscription, or operation in flight holds a lease on the session. When the last
lease drops, the session is closed after an idle time (60 seconds by default)
unless something used it since, which frees the receiver's single control
connection for other tools. Connect, use, and release are serialized by one lock
per receiver, so concurrent first requests connect once and a request that
arrives during release waits for the close and reconnects.

Saving the configuration retires the session of every receiver whose entry was
removed or now holds a different host, so the next lease connects to the address
in the file. A held subscription would otherwise keep the old session, which
reconnects for ever to an address that no longer answers, and idle release would
never come. Retirement takes the same per-receiver lock, so a new connection
never opens beside the old one, and it waits for operations in flight because a
close that outlasts its grace period aborts the session and would lose their
outcome. It does not wait for subscribers. The service ends the subscriptions of
every session it closes (retirement, idle release, shutdown), and
`StateSubscription::changed` then returns the "session closed" error, because a
session keeps the sender of its own state channel for as long as it exists.
A subscriber that sees the error subscribes again. A change of model or name
alone retires nothing.

**Operation Gate.** Every state-changing request becomes an operation owned by
the gate. The gate allocates the operation id, records the owning principal,
honors an idempotency key, returns the existing operation for an identical
request already running, and calls the session's `operate` exactly once on a
task the service owns, so a caller that goes away never abandons the call. The
gate is the only caller of `operate` outside tests and the session
implementations. A cancel is decided against the move into the session under one
lock: it either wins before the session is called or is told it is too late.
The session's outcome is reported without reinterpretation:

| Session outcome | Status | `dispatch` | `confirmed` |
| --- | --- | --- | --- |
| `ObservedRequestedValue` | `completed` | as reported | true |
| `AlreadyObserved` | `already_in_state` | `not_dispatched` | true |
| `RejectedBeforeDispatch` | `rejected`, with the session's reason | `not_dispatched` | false |
| `Cancelled` | `cancelled` | `not_dispatched` | false |
| `SupersededBeforeDispatch` | `superseded` | `not_dispatched` | false |
| `Indeterminate` | `indeterminate` | as reported | false |

A receiver that cannot be reached ends the operation as `rejected` with nothing
dispatched. `dispatch` is the session's `DispatchCertainty` and nothing more:
`complete_write` means the local write completed, not that the receiver
acknowledged it. `confirmed` is true only when receiver evidence shows the
requested value.

**Session contract additions.** `CanonicalReceiverSession` (in `session_v3.rs`)
carries three things beyond core state and operations:

- *Precondition.* `OperationRequest` has an optional `Precondition`: the receiver
  epoch and, for every field a decision consulted, the value it saw or the fact
  that none was usable. Before writing, the session re-observes the target and
  every named field, compares the epoch and each field, and only then tests
  whether the target is already in state, so a mismatch takes precedence over
  `AlreadyObserved`. A mismatch returns `RejectedBeforeDispatch` and nothing is
  written. It adds no retry path. Operator operations carry none.
- *Typed rejection cause.* `RejectedBeforeDispatch` carries a `RejectionCause`
  (`UnsupportedIntent`, `ObservationFailed`, `PreconditionMismatch`,
  `CommandRefused`, `SessionStopped`) next to the display text. Callers branch on
  the cause and never on the text.
- *Inspection reads.* `source_catalog`, `quick_select_names`, and
  `http_information` are read-only and report `Unsupported` by default. The HTTP
  clients that serve them stay in infrastructure.

**Configuration file.** The YAML file holds several receivers by name, under a
`version: 2` key, and the entry names are the receiver ids. The single-receiver
file written by 3.0.0 is still read. A file this release does not read as the new
schema (the 3.0.0 file, another version, or a file with an error such as an
unknown key) is copied to `<file>.v3.bak` before it is replaced, never over an
existing backup; a file that reads cleanly is replaced without one. Duplicate,
empty, and reserved names and unknown versions are refused.
3.0.0 cannot read the new file. Nothing adds a second receiver through the GUI
or CLI.

## Policy and audit

An Agent's request is decided by the policy engine and recorded in the audit log
before it reaches a receiver. The path is built by `ControlService::start` from a
`PolicySource`, an `AuditLog`, a `Clock`, and caps (`AgentPath`). It is built to
fail closed: whatever it cannot judge, record, or count, it does not send.

**Policy engine.** `denon_avr_policy::evaluate` is a pure function of a
`PolicyInput` (the agent label, the receiver id, the intent, the receiver's
`ReceiverState`, the recent volume changes, and the time) and a validated
`PolicyConfig`. It reads no clock and does no I/O; `make boundary` checks that
`crates/policy/src` has no async, serialization, file, network, or clock type and
that its resolved dependency graph reaches `domain` and nothing else.

- A rule *applies* when its agent and receiver lists match exactly and case
  sensitively. The decision is the most restrictive matched effect, `Deny` then
  `RequireApproval`. With none, an intent that some applicable rule names (its
  kind, and its value when the rule gives one) is `Allow`, and any other is
  `RequireApproval` as unclassified. An `allow` rule carries no condition and only
  classifies; a rule that names no intent restricts but never classifies.
- Limits are strict and on the 0.5 dB grid, and `Minimum` is -80.0 dB. A volume
  that is stale, unknown, or unavailable matches every condition that needs it,
  so what cannot be checked counts as dangerous. A decrease is exempt from the
  step and budget conditions but not from the target limits.
- The budget is the rise above the lowest level in a window: the observed level
  and the `before` and `target` of every counted change inside it. An entry dated
  after now is inside the window.
- `Allow` and `RequireApproval` carry a *baseline*: every state field read by
  any applicable rule that covers the intent, whether or not it matched. The gate
  turns it into the write's precondition, so an unmute allowed because the
  loud-volume rule did not match is still refused if the volume changed.

**Policy file.** `policy.yaml` sits beside the configuration and is read by
`YamlPolicySource`: unknown keys (a time-of-day key among them) are refused, so is an
`agents` entry that no token label could equal (labels are lowercase letters, digits,
`.`, `_`, and `-`, per `policy::label_is_well_formed`), the file is at most 256 KiB, its digest is the SHA-256 of the bytes read, and errors
name the rule or key and never quote the file. An empty file is a policy with no
rules, which requires approval for everything. `docs/examples/policy.yaml` is the
owner's configuration, not a default. The service holds the policy in force or
none: a load that fails, at start or on `reload_policy`, replaces the policy in
force with none, and while there is none every Agent write ends `rejected`.
Reads and the Operator's controls are unaffected.

**Budget ledger.** `application::ledger` holds the volume changes per receiver,
keyed by the run (the service's start time) and the operation id, because ids
restart at 1. An agent's change counts from the moment it is allowed, so it is
reserved under the same lock as the evaluation, and stays counted unless the
session says nothing was dispatched. The Operator's volume writes count too and
are never limited. Entries last 24 hours. After a restart the ledger is rebuilt
from the audit log: every volume `Dispatching` record of the last day counts
except those whose `Finished` record says `not_dispatched`, so a write the process
died during still counts. Until the log has been read, the ledger is not ready and
Agent writes are refused.

**Audit log.** `AuditLog` appends records and reads them back; `JsonlAuditLog`
keeps one JSON object per line in `audit.jsonl` and rotated files by size (20 MiB)
and count (10), in a directory created 0700 with files 0600. A directory or file
that already exists with wider permissions is an error, not a repair. Reading skips
a line cut short, one that is not JSON, and a record of a kind or schema it does
not know, and numbering continues past them. A record is one of `Decided`,
`Dispatching`, `Finished`, `PolicyLoaded`, `PolicyLoadFailed`, `AccessRefused` (a
caller refused at an endpoint), `TokenIssued`, and `TokenRevoked`, and the text in
it is bounded whatever it is handed. A record's principal is absent when no valid
credential was presented. A refusal is written at most once a minute for each
caller and reason, with the count of those not written; at most 256 callers are
tracked and 30 refusals a minute are written over all of them; and a refusal's
failing append never changes the audit health, so being refused cannot mark the log
failing. The ledger's rebuild reads `Dispatching` and `Finished` only. Only
`Dispatching` is synced to disk. An
append runs on a task of its own, so a caller that stops waiting (the service
bounds every append) does not leave a line cut short or a sequence number used
twice. Reading the last day skips a file last written before the cutoff, and
reads the rest line by line. The log is not tamper-proof against a process
running as the same user.

**The Agent path.** `submit` counts a new write against the label's caps (30 a
minute and 8 unfinished; a retry of an operation that exists is not a new write,
and a dry run counts because it opens the receiver) and returns `RateLimited`
without creating an operation or a record. Otherwise the operation starts
`submitted` and its task:

1. Refuses as `rejected` when there is no policy or the ledger cannot be rebuilt.
2. Takes a lease on the receiver.
3. In one step under the ledger lock, with nothing awaited: reads the policy in
   force at that moment (not the one in force before the lease, which can take
   seconds to get, so a reload made meanwhile decides), the state, and the ledger,
   evaluates, captures the precondition from the baseline, and reserves a volume
   change.
4. Ends `denied` or `approval_unavailable` for a refusal, with `Decided` recorded.
   There is no approval path yet, so what needs approval is not sent.
5. For an allow: moves to `allowed` (cancellable), appends `Decided`, then appends
   `Dispatching` synced. Each append is bounded to five seconds. If either fails
   or times out, or a cancel won, the reservation is released and nothing is sent.
6. Calls `operate` once with the precondition. A mismatch comes back `rejected` and
   is never retried.
7. Settles the ledger by the reported `dispatch`, ends the operation, and appends
   `Finished`.

Every agent operation ends with a `Finished` record, except that a refusal made
before any decision (no policy, or no readable history) is written once, then at
most once a minute with the count of those not written, so a client that keeps
asking cannot fill the log. On a service built by `start`
the Operator's writes get `Decided`, `Dispatching`, and `Finished` records and a
ledger entry for a volume change; a failing log does not stop them, and `health`
reports it as failing. An Operator's appends are bounded to one second, not five,
and are skipped while the log is known to be failing; the `Finished` record that
follows the write is what notices the log working again. Audit health is the result
of the last append, and every Agent write tries again, so it recovers by itself.
Policy reloads run one at a time, from the read to the record, so the policy in
force and the log agree on which file loaded last. The ledger drops entries older
than a day as it reserves new ones.

| Condition | Status | `dispatch` |
| --- | --- | --- |
| A rule denies | `denied` | `not_dispatched` |
| A rule requires approval | `approval_unavailable` | `not_dispatched` |
| No policy, ledger not rebuilt, receiver unreachable, no established state, audit append failed or timed out | `rejected`, with fixed text | `not_dispatched` |
| The receiver changed since the decision | `rejected`, the session's reason | `not_dispatched` |
| Cancelled before the session | `cancelled` | `not_dispatched` |
| Any other session outcome | as the table above | as reported |

An agent is told the sentences of the limits that fired and fixed text for
everything else: for the faults above, for what a session reports (one sentence
per rejection cause, and "the outcome could not be established" for an
indeterminate result, because the session's own text formats the I/O error
underneath and can name an address), and for a failed connection or read (an
agent's `state`, `source_catalog`, `submit`, and `dry_run` never carry the
connector's or the repository's message). Rule ids, file paths, error text, and
the receiver's address appear in the audit log and the Operator's views, and the
Operator's `dry_run_as` shows the rule ids; an agent's own `dry_run` shows the
limits and no ids.

## Control API

`apps/api-server` hosts the in-process control service and serves the
control-service port over HTTP/1.1 on Unix sockets; `crates/api-contract` defines
the wire (`/v1`, contract version 1) and `crates/api-client` implements the port
over it. The server has no network listener. Nothing in the CLI or the GUI uses it
yet: they still compose their own service, so **while the server holds a receiver, a
CLI or GUI cannot connect to it**, because the receiver accepts one control
connection. The server releases a receiver after the service's idle time.

**Files and endpoints.** Under the data directory, `run/` and `credentials/` are
created 0700 (the data directory's own mode is left alone; a wider existing mode is an
error). `run/operator.sock` is 0600, `run/server.lock` holds the lock, and
`credentials/operator.token` and `credentials/agent-tokens.json` are 0600. The
**Operator endpoint** admits the server's own uid and accepts only the Operator
token. The **Agent endpoint** (`agent.sock`) admits the uids it is configured with
and accepts only Agent tokens. It exists only when `server.yaml` in the data directory
has an `agent_endpoint` (`directory`, which defaults to `/Users/Shared/Denon AVR
Remote`, and `uids`, which is required; the socket's mode is not a setting and is
`0600`). The file may be absent, a key it does not know is refused by name, and a file
that is wrong stops the executable with status 2 before it opens anything. Before it
binds, `Server::start` checks the directory: a real directory (not a link), owned by the
server's uid, and not writable by its group or by anyone else, made at `0700` (with any
missing parent) when it is missing; after the bind it reads the directory and the socket
again and removes a socket that is not its own. A stale socket there is removed only if
it is the server's own. With no configuration, no admitted uid, a token store that cannot
be used, or a directory that fails a check, the endpoint is not created, the log and the
`server` section of the Operator's health response say why, and the Operator endpoint
serves regardless. The account is admitted by a directory ACL that the owner sets once
(`docs/examples/server.yaml` has the commands, and
[the S2 note](docs/research/agent-endpoint-access-macos.md) what was checked on macOS):
an entry for `search`, and an inheritable `write` entry that every socket the server makes
there carries, so the socket stays `0600`.

**One server per data directory.** The executable takes `File::try_lock` on
`run/server.lock` (`InstanceLock::acquire`) before it opens the token files or the audit
log, and so before it touches a socket; the audit log's sequence numbers are counted in
memory, so a second writer would reuse them. A second server finds the lock held,
writes nothing, and the executable exits with status 75. A socket left by a killed server is removed only
when `lstat` says it is a socket owned by the server's uid; a file, a link, or someone
else's socket at the path is an error and is left alone. A socket path over the
platform's `sun_path` limit (104 bytes on macOS) is refused with its length and the limit.

**The request pipeline.** Every connection and request passes through these in
order, so a caller learns nothing before it has proved who it is:

1. *Admission.* The peer's uid, read from the connection (`peer_cred`), must be
   admitted by the endpoint; any other connection is closed before a byte is read and
   audited as `PeerNotAdmitted`.
2. *Caps.* 16 connections on the Operator endpoint and 32 on the Agent endpoint, so
   an agent that holds every connection of its endpoint cannot starve the Operator;
   the head is read in 5 seconds and is at most 16 KiB, and an idle connection is
   closed after the same five seconds.
3. *Authentication.* One layer added to the router after its routes and its fallback,
   so it wraps every path, a path that matches nothing included: an unknown path is a
   `401` before it can be a `404`. Only `Authorization: Bearer` is accepted; a
   credential in a query string is refused. A missing, repeated, unknown, revoked, or
   wrong-kind credential gets the same `401`, and each is audited with its own reason.
4. *Routing.* Each endpoint's router is built from the rows of
   `api_contract::routes::TABLE` that are served to it, so a resource that is not in an
   endpoint's table does not exist on it. An Operator resource asked of the Agent
   endpoint is a `404`, audited as `ResourceNotServed`.
5. *Body.* `Content-Length` is required for `POST` and `PUT` (chunked bodies are
   refused), at most 16 KiB (1 MiB for the configuration), read in 10 seconds, and
   parsed strictly: a field the contract does not know is a `400` that names the field
   and never repeats the body.
6. *Handler.* It asks the service for a handle bound to the authenticated
   `Principal` and calls the port; it never decides who may do what. A request's
   total time is capped at `max_operation_wait` plus five seconds; an event stream's
   cap covers the time until it begins.

**What the wire carries.** Requests are strict and responses are lenient: a client
ignores fields it does not know, so the contract can grow. An Agent is given its own
view types for state, sources, and dry runs, made of codes and with no text field, so a
receiver's address, a raw frame, a path, or error text cannot reach an agent however
the Operator's views grow. The wire carries a field's validity class and reason, not
its age. A dry run is its own resource, so a request that mistypes it can never become
a write. `PUT /v1/config` requires `If-Match` with the `ETag` of the configuration
read (`428` if missing, `412` if the file has changed). Errors are
`{"error": {"code", "message", ...}}` with the codes the port's `ControlError` maps to;
`api-client` maps them back.

**Event streams and waits.** `GET /v1/receivers/{id}/events` is the state subscription
(`event: state`: the whole state first, then the newest after each change) and
`GET /v1/operations/events` is the operation stream (`event: operation`, and
`event: missed` when the reader fell behind, after which the operation is read with
`GET /v1/operations/{id}`). There is no replay: a client that reconnects reads the
state again. A held state stream keeps its receiver connected, so each stream is fed
through a channel of one item and dropped if its client does not take an event within
10 seconds; a comment line every 15 seconds finds a client that has closed its end;
a principal may hold four streams (a fifth is `429 too_many_streams`); and an Agent's
state stream also ends after ten minutes, because neither of the others can see a
client that is connected and not reading when the receiver is quiet. The Operator's
streams have no maximum age. A stream ends with `event: end` and a reason:
`session_closed`, `revoked`, `shutdown`, or `max_age`. An operation stream holds no
lease on a receiver. A wait (`GET /v1/operations/{id}?wait_ms=`) is the port's own and
ends at the service's cap.

**Tokens.** Agent tokens are `dara_` and the Operator's is `daro_`, each followed by 43
base64url characters; only SHA-256 digests are kept, compared in constant time. A label
is lowercase `[a-z0-9._-]` (the rule is `policy::label_is_well_formed`, which a
policy file's `agents` entries must also satisfy), and a label holds at most one active
token. Revoking a token ends its streams and waits within a second and refuses its next
request, while the operations it started continue to their end. A damaged Agent token
file does not lock the Operator out: the store opens without Agent tokens, refuses to
issue or revoke, and the server creates no Agent endpoint; a damaged Operator token or
a `credentials/` directory with wider permissions is a startup error.

**Refused callers are audited.** A caller refused at an endpoint is an `AccessRefused`
record with its reason, peer uid, and resource, and no principal when none was
established. They are throttled (at most one record for an endpoint, reason, and caller in 60
seconds, 256 such keys, and 30 records a minute), they do not change the audit
health, and the ledger's rebuild ignores them, so a flood of refused requests can
neither fill the log nor push an Operator's record out of the window.

**Process model.** `denon-avr-api-server [--data-dir DIR] [--exit-with-parent]` reads
`server.yaml`, opens the token store and the audit log, starts the control service, and
serves. SIGTERM and
SIGINT start one shutdown: stop accepting, send `event: end` (`shutdown`) to every
stream, wait for operations in flight, close the sessions, remove the sockets, and
release the lock. With `--exit-with-parent` the end of standard input starts it too, so
a GUI that started the server and died takes it down; a standalone server ignores
standard input. A second signal while the shutdown runs exits at once with 128 plus
its number. Killing the process leaves a stale socket and a released lock, which the
next start reclaims.

**The client.** `ApiClient::connect(Endpoint { socket, token, audience })` opens no
connection; each request opens its own (a Unix socket is cheap and a reused
connection can be reset by a restarted server), and so does each event stream. A
server that cannot be reached is `Unavailable("receiver service unavailable")`, with
the socket's path added for the Operator; an answer that is not the contract's error
body is `Unavailable` and never a guess; a client made for an agent refuses the
Operator's methods without sending a request. The credential prints as `<redacted>`
and is in no error. `state` returns once the first `state` event arrives, and a
subscription ends with the service's "session closed" error on an `end` event or a
closed stream, so a caller that resubscribes behaves as it does against the in-process
service. `tests/client.rs` in `api-server` runs one function over the port twice, against
the service directly and through the socket, and compares what it saw.

### Not yet built

- **Approvals and OAuth** (`/v1/approvals`, client registration) wait for later
  milestones.
- **A user of the server.** The CLI and the GUI become clients in milestone 5.

## Desktop GUI

The GUI is a client of the operator port. `PortBridge` is the one task that owns
the port handle and the state subscription for the selected receiver; commands
reach it in order, and a control, a refresh, or a supplemental read runs on a task
of its own so a slow receiver never holds back state. A spawned task hands its
result to the bridge's loop, which sends any newer state to the window before it
sends the result, because a window told that a control finished may accept the
next click. Every event carries the `Select` request it belongs to, and the window
drops those of a receiver it has left.

`projection::project` turns a `ReceiverState` into the display model the views
read (`MainZoneSnapshot`, `Zone2Snapshot`). A field is usable only when its
validity is `Current`; a stale value is shown as unavailable and never acted on.
A disconnect and a higher epoch are both a reconnect, and a reconnect clears what
was read over HTTP and reads it again; the core fields are re-read by the session
itself. The audio, video, and Audyssey information has no push, so the GUI reads
it on a timer and after a confirmed control that can change it, one read at a
time.

A control carries the value its target field showed, and the bridge refuses it
if the subscription now shows another, except a power control, which states an
absolute target. Saving a receiver reads the stored configuration, changes one
entry, and writes it back, so other receivers and sound mode favorites survive.
Closing the window closes the receiver connection first, through a hook the
composition root supplies, within a short grace period so a receiver that never
answers cannot keep the window open; a second request closes at once. The desktop
turns off the automatic exit on a close request to allow this. What the closing
connection reports meanwhile is ignored. The GUI keeps `MainZoneSnapshot` as its
display model; replacing it is a separate
refactor with its own visual risk.

## Receiver-correctness invariants

- AVR commands end in one CR; HEOS commands end in CRLF.
- Domain and protocol code do not depend on sockets, filesystems, runtimes, or
  presentation.
- I/O is bounded and failures retain useful context.
- State distinguishes unsupported, malformed, unavailable, disconnected,
  stale, and unknown observations; unavailable data is never invented.
- Capability claims are model- and evidence-bound. Unvalidated behavior stays
  read-only or reports unsupported.
- A state-changing operation dispatches at most once once dispatch begins, and
  its result is only authoritative when receiver evidence confirms it.
- Deterministic fakes or local servers cover ordinary tests. Live validation
  records the model, firmware, relevant settings, command, response, and date.

## Delivery and enforcement

The CLI and desktop executable are composition roots. The CLI composes the
control service, selects a receiver through the port, submits operations and
waits for them to finish, and shuts the service down after its short-lived
command, which closes the receiver connection. It constructs no session and
names no session type. The desktop composes the same service and hands the GUI
only the operator port and a hook that closes it, so the GUI never names the
service and a later release can replace the supplier without touching the GUI.
Diagnostics are
deliberately independent of normal delivery behavior and never issue writes.

`make boundary` checks source imports, prohibits the retired root `src/` tree,
validates the resolved Cargo workspace edges, keeps `crates/policy` pure and its
resolved graph to `domain`, ensures the receiver-session
contract has exactly one definition and that the retired controller names do not
return, requires that only the Operation Gate and the session implementation
call a session's `operate`, and keeps the CLI from naming a session type. It keeps
`api-contract` free of an HTTP stack, an async runtime, and the filesystem; keeps
the resolved graph of `api-contract` and `api-client` away from `infrastructure` and
`protocol`; keeps `api-client` free of any TLS crate, any server framework, and the
`server` features of `hyper` and `hyper-util`; and requires that `api-server` and
`api-client` refuse to build off Unix. Run it whenever a package dependency or boundary
changes. Current engineering policy and verification gates are in
[docs/contributing.md](docs/contributing.md) and
[docs/development.md](docs/development.md).
