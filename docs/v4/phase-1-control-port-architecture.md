# Version 4, Phase 1 — Control port architecture

This document fixes the types and rules that
[Planned architecture](../planned-architecture.md) leaves to phase 1. It does not
restate the design; where a rule is owned there it is linked. Text that is
implemented moves into [ARCHITECTURE.md](../../ARCHITECTURE.md) when the
milestone's exit criteria pass.

## Receiver id

Done in step 1. A saved receiver's id is its configuration entry name
(`ReceiverId::new`). A receiver chosen by address gets `ReceiverId::ad_hoc`,
which carries the reserved `adhoc:` prefix that saved names cannot use.
`application::receiver_selection::receiver_id` is the one derivation, and the
configuration adapter rejects names with the reserved prefix.

## Rejection cause

`OperationOutcome::RejectedBeforeDispatch` carries a typed cause as well as the
free text that is shown to users. The gate branches on the cause and never on
the text.

```text
RejectionCause
  UnsupportedIntent        the receiver profile does not support the value
  ObservationFailed        the receiver could not be observed before writing
  PreconditionMismatch(PreconditionMismatch)
  CommandRefused           the command could not be encoded, or the transport
                           refused it before any byte was written
  SessionStopped           the session ended before dispatch
```

A cause always means nothing was written. A write that began and then failed is
`Indeterminate`, never a rejection. The cause is `RejectionCause` in `domain`,
carried next to the display text as `RejectedBeforeDispatch { operation, cause,
reason }`.

## Precondition

The types live in `domain`, because the policy crate depends only on `domain`
and builds the baseline that becomes the precondition.

```text
Precondition      epoch + one FieldBaseline per consulted field
FieldBaseline     Value(FieldValue) | NoUsableValue(CoreField)
FieldValue        the typed value of one core field
                  (SystemPower, MainZonePower, Zone2Power, Source, Volume,
                   Mute, SoundMode id)
PreconditionMismatch
                  Epoch | Field(CoreField)
```

`OperationRequest` gains `precondition: Option<Precondition>` and is
`#[non_exhaustive]`, so it is built with `OperationRequest::new` and
`with_precondition`. Operator operations carry none.

**Capture.** `FieldBaseline::capture(state, field)` records the field's value
only when its validity is `Current` and it has a last good observation. Stale,
unknown, and unavailable fields capture as `NoUsableValue`, so a baseline never
presents unusable data as a value.

**Check order in `X3800hSession::operate`.** The order is part of the contract:

1. Cancelled-requester and intent admission checks, as before.
2. Re-observe the target field, as the preflight does today.
3. With a precondition, re-observe every other field it names. A failed
   observation is `RejectedBeforeDispatch(ObservationFailed)`.
4. Compare the epoch, then each field in `CoreField` order. A different epoch,
   a different value, or a `Value` baseline whose field no longer has a usable
   value is `RejectedBeforeDispatch(PreconditionMismatch)`. A `NoUsableValue`
   baseline matches only a field that still has no usable value, so a baseline
   that was unknown and has since become known is a mismatch.
5. Only then test whether the target is already in state
   (`AlreadyObserved`), and dispatch if it is not.

A mismatch takes precedence over `AlreadyObserved` because the precondition
guards the decision, not only the write: the caller learns that what it decided
on is no longer true and re-evaluates. Nothing is dispatched either way.

The change touches only the preflight. It adds no retry path and leaves the
at-most-once rule untouched.

## Receiver connector

`ReceiverConnector` in `application::ports` replaces the legacy
`SessionFactory` for the new path:

```text
connect(receiver: &ReceiverId, identity: &ReceiverIdentity)
    -> Result<SharedReceiverSession, OperationError>
```

It takes the receiver id explicitly, so the id is the configuration entry name
and not derived from the address. Infrastructure implements it as
`X3800hConnector`. The legacy `SessionFactory`, `CanonicalSessionFactory`, and
the controller are untouched; milestone 2 deletes them.

The name avoids the `ReceiverSession` and `SessionFactory` prefixes because the
boundary script's single-definition rule matches by prefix.

## Inspection reads

`CanonicalReceiverSession` gains three read-only methods with default
implementations that report `Unsupported`, so fakes need not implement them:
`source_catalog`, `quick_select_names`, and `http_information`. `X3800hSession`
implements them with the existing HTTP clients, which stay in infrastructure and
run on blocking tasks. The session records the receiver host and HTTP settings
at connect time for this purpose. Each read stamps its result with the session's
current connection generation, as the legacy path did.

## Control service

`application::service::ControlService` implements the three control-service
traits in process. It is built from four ports: `ReceiverConnector`,
`AsyncConfigRepository`, `AsyncReceiverDiscovery`, and a `ServiceConfig`. All
timing uses `tokio::time`, so tests drive it with a paused clock.

`ControlService::handle(principal)` returns a handle bound to that principal.
Only `Principal::Operator` is served in this milestone; an Agent principal is
refused with `ControlError::Forbidden` until milestone 3 adds the policy path.
Every administration method also checks the principal, so a handle can never
reach it by accident.

### Registry and connection lifetime

One slot exists per receiver id. A slot serializes connect, use, and release
with one async lock, which gives these properties:

- **Connect once.** Concurrent first requests wait on the slot lock, so exactly
  one connection is attempted. A connect also synchronizes, so the first state a
  caller sees is complete.
- **Release is atomic.** The idle check and the close happen under the same
  lock. A request that arrives during release waits for the close to finish and
  then reconnects, so it can never receive a closing session.
- **A lease is the unit of activity.** Reads and operations take a lease that
  holds the session. A state subscription holds a lease for as long as it lives.
  `StateSubscription` carries an optional guard for this.
- **Idle release.** When the last lease drops, a timer starts. When it fires,
  the slot is released only if no lease was taken since. The default is
  **60 seconds**, set in `ServiceConfig`. Any read or operation restarts the
  clock; only a held subscription or an operation in flight prevents release,
  so an agent that reads state and goes away does not hold the receiver's single
  control connection beyond the idle time.
- **Shutdown.** `ControlService::shutdown` is inherent, not on a port trait. It
  refuses new work, waits for operations in flight to finish, and closes every
  session. The CLI calls it before exiting.

### Operation gate (Operator principal)

The gate is the only caller of `CanonicalReceiverSession::operate` outside tests
and the session implementation.

- The gate allocates operation ids from a counter. The id it allocates is the
  `OperationRequest` id, so `SupersededBeforeDispatch { by }` names gate ids.
- `submit` returns promptly with an `Allowed` snapshot. The call into the
  session runs on a task the service owns, so a caller that goes away never
  abandons it. Shutdown waits for it.
- An operation moves `Allowed` to `InSession` and calls `operate` exactly once.
  The move and cancellation are decided under one lock, so a cancel either wins
  before the session is called or is told it is too late.
- A receiver that cannot be reached ends the operation as `rejected` with
  `not_dispatched`, carrying the connection error as its reason. The lifecycle
  table has no separate status for it, and nothing was written.
- If the task running an operation ends without resolving it, which only a panic
  can cause, the operation is resolved as `indeterminate` with `unknown`
  dispatch when it was in the session, so it is never left running.
- A request with an idempotency key that was already used returns the existing
  operation, and a key reused for a different request is an invalid request. An
  identical in-flight operation from the same principal and receiver also
  returns the existing operation.
- The session's outcome maps to `status`, `dispatch`, and `confirmed` exactly as
  the [lifecycle table](../planned-architecture.md#operation-lifecycle) specifies.
  A test covers every `OperationOutcome` variant.
- The operation table keeps the most recent 256 terminal operations, so memory
  is bounded. An operation that fell out of the table is not found.
- Operation events use a broadcast channel. A reader that falls behind receives
  `Missed` and reads the operation itself.
- `operation(id, wait)` waits for a terminal status for at most the requested
  time and never more than 30 seconds.

## Configuration file

The file moves to a multi-receiver schema with a version field. Entry names are
receiver ids.

```yaml
version: 2
current: living-room
receivers:
  living-room:
    host: 192.0.2.10
    model: AVR-X3800H
    friendly_name: Living Room
sound_mode_favorites:
  living-room:
    - DOLBY SURROUND
```

- **Reading.** A file with a top-level `receiver` is the single-receiver form.
  It is read as one entry named by its friendly name, or `default`, as before.
  Category-keyed favorites keep flattening as before.
- **Writing.** Only the new schema is written. Before a file that this release
  does not read as the new schema is replaced, its contents are copied to
  `<file>.v3.bak`. That covers the single-receiver file and any file this release
  cannot read, including one with the new schema's keys but another `version`, an
  unknown key, or an inconsistent entry. A file that reads cleanly is replaced
  without a backup, and a blank file has nothing to keep. An existing backup is
  never overwritten; a numbered suffix is used instead. If the copy fails,
  nothing is replaced. A save that fails validation writes no backup.
- **Empty.** An empty configuration is a valid state and can be saved. The
  single-receiver file could not represent it.
- **Rejections.** Duplicate entry names (a YAML parse error, never a silent
  merge), empty names, and names with the reserved ad hoc prefix are rejected on
  read and on write. A `current` entry or a favorites entry that names an
  unconfigured receiver is rejected, as is any version other than 2.
- **3.0.0 compatibility.** 3.0.0 cannot read the new file.
- **One code path.** The synchronous and asynchronous repositories share the
  decode, encode, and backup code.
- **No new way to add a second receiver.** Nothing in version 4 adds one through
  the GUI or CLI. A multi-receiver file comes from hand edits. The CLI therefore
  remembers its last-used receiver only when the file holds at most one entry,
  and never rewrites a file with several.

## Boundary rules added

- `application` gains no new workspace edge.
- Outside tests, only the control service and the session implementations call
  `operate`. `canonical_factory.rs` is allowed by name until milestone 2 deletes
  it.
- The session-contract single-definition rule is unchanged in this phase.
  `ReceiverConnector` is not one of the three protected names.
