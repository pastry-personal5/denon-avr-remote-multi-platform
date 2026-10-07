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
| `crates/application` | Use-case policy, ports, status/control policy, the control-service port, and the in-process control service with its Operation Gate. |
| `crates/infrastructure` | SSDP discovery, YAML persistence, TCP/HTTP adapters, and concrete receiver sessions. |
| `crates/gui-lib` | Iced presentation state, reducers, views, the projection of receiver state into them, and the bridge to the control-service port. |
| `apps/cli` | Short-lived CLI composition over the in-process control service. |
| `apps/desktop` | Native GUI composition of the control service, and logging. |
| `apps/diagnostics` | Explicit, read-only evidence probes. |

```text
protocol       → domain
application    → domain
infrastructure → application, domain, protocol
gui-lib        → application, domain
cli            → application, domain, infrastructure
desktop        → gui-lib, application, domain, infrastructure
diagnostics    → domain, infrastructure, protocol
```

`domain` imports no workspace package. `protocol` and `application` depend only
on `domain`; infrastructure may compose all three core crates. `gui-lib` uses
application and domain only. Delivery packages compose the dependencies their
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

- `ReceiverReads`: the saved-receiver listing, a state subscription, and the
  source catalog.
- `OperationControl`: submitting an operation, reading its status, cancelling
  it, and the operation event stream, for the caller's own operations.
- `OperatorAdmin`: discovery, receiver configuration, ad hoc receivers, the
  inspection reads no agent tool uses (Quick Select names and HTTP information),
  and `refresh`, which reads every core field again. `refresh` is a read: it
  connects if the receiver is released, dispatches nothing, and does not use the
  gate.

`AgentControl` is the first two and `OperatorControl` all three. A handle is
bound to one `Principal` when it is created and the principal is never a request
parameter. `ControlService` (`crates/application/src/service.rs`) implements the
port in process. It serves the Operator principal; an Agent handle is refused
until the policy path exists.

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
The GUI keeps `MainZoneSnapshot` as its display model; replacing it is a separate
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
validates the resolved Cargo workspace edges, ensures the receiver-session
contract has exactly one definition and that the retired controller names do not
return, requires that only the Operation Gate and the session implementation
call a session's `operate`, and keeps the CLI from naming a session type. Run it whenever a package dependency or boundary
changes. Current engineering policy and verification gates are in
[docs/contributing.md](docs/contributing.md) and
[docs/development.md](docs/development.md).
