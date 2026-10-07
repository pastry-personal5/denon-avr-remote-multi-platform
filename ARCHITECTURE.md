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
| `crates/gui-lib` | Iced presentation state, reducers, views, and the GUI controller bridge. |
| `apps/cli` | Short-lived CLI composition over the in-process control service. |
| `apps/desktop` | Native GUI composition, logging, and concrete adapter wiring. |
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

`crates/application/src/ports.rs` is the one definition site for
`SessionEvent`, `ReceiverSession`, and `SessionFactory`. A `ReceiverSession`
combines reads, one-shot writes, lifecycle events, and shutdown because one
owner is necessary to preserve ordering. `SessionFactory` creates a session;
the application coordinator owns it for its complete lifetime. GUI bridges and
adapters forward through that boundary rather than owning or duplicating a
session.

The canonical async receiver service serializes connection lifecycle and
operations. A reconnect or receiver change invalidates authority from the old
connection. Uncorrelated receiver lines remain events rather than being
assigned to a command opportunistically.

## Control service

The control-service port in `crates/application/src/control.rs` is how a
surface observes and controls receivers. It is three traits split by
authority, and a surface takes only the traits it needs:

- `ReceiverReads`: the saved-receiver listing, a state subscription, and the
  source catalog.
- `OperationControl`: submitting an operation, reading its status, cancelling
  it, and the operation event stream, for the caller's own operations.
- `OperatorAdmin`: discovery, receiver configuration, ad hoc receivers, and the
  inspection reads no agent tool uses (Quick Select names and HTTP
  information).

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
file written by 3.0.0 is still read. A file that is not already in the new schema
is copied once to `<file>.v3.bak` before it is replaced, never over an existing
backup. Duplicate, empty, and reserved names and unknown versions are refused.
3.0.0 cannot read the new file. Nothing adds a second receiver through the GUI
or CLI.

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
names no session type. The desktop executable still reaches the receiver through
the legacy controller until the GUI moves onto the port. Diagnostics are
deliberately independent of normal delivery behavior and never issue writes.

`make boundary` checks source imports, prohibits the retired root `src/` tree,
validates the resolved Cargo workspace edges, ensures the session contracts
have exactly one definition, requires that only the Operation Gate and the
session implementations call a session's `operate` (the legacy compatibility
adapter is the one remaining exception), and keeps the CLI from naming a
session type. Run it whenever a package dependency or boundary
changes. Current engineering policy and verification gates are in
[docs/contributing.md](docs/contributing.md) and
[docs/development.md](docs/development.md).
