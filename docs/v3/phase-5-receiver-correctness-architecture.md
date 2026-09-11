# Version 3, Phase 5 architecture — canonical receiver state and operations

**Implemented — 2026-09-11.** This is the normative design for Phase 5. The
[overview](phase-5-receiver-correctness-overview.md) defines scope and delivery
gates. The [supplement](phase-5-receiver-correctness-supplement.md) records the
current behavior and research that this design replaces. The
[headless test plan](phase-5-receiver-correctness-test-plan.md) is normative for
verification coverage and harness behavior.

## Architectural decision

One asynchronous session actor will own the AVR TCP stream, transmission
schedule, connection epoch, frame sequence, and canonical core-state reducer.
There will be no second mutable core snapshot in application or presentation.
Every consumer observes full state from that actor; every control transaction
returns evidence from that same ordered frame stream.

This is a replacement architecture. Compatibility adapters that recreate
aggregate freshness, fake receiver versions, normalized volume, or `PW`-based
Main Zone power are prohibited because they would preserve the correctness
defects under new names.

## Invariants

1. Exactly one task reads or writes a receiver's AVR TCP stream.
2. Every transmitted command is valid for the selected X3800H profile,
   CR-terminated, no longer than the protocol maximum, and globally paced.
3. Every parsed receiver frame is assigned one monotonically increasing frame
   sequence and reduced before any waiter or subscriber is notified.
4. Every observation names its receiver and established connection epoch.
   Results from another receiver or epoch are never merged.
5. Notification pressure may coalesce complete states, but cannot lose the
   latest canonical state or block socket reads.
6. A query yields a post-query receiver observation, not a protocol
   acknowledgement. A control result never claims more than its evidence.
7. A failed read or disconnected socket preserves last-good evidence and marks
   it stale. It does not convert an old value into a current value or erase it.
8. Controls are never replayed after uncertain delivery.
9. Operation identity, state publication order, connection identity, and
   receiver identity are separate concepts.
10. Supplemental HTTP work cannot delay core TCP observation or control.
11. Core correctness is testable through public application/infrastructure APIs
    without importing or launching GUI, CLI, or another delivery package.
12. Desktop presentation never promotes a draft, dispatched intent, or stale
    value into a receiver observation.
13. Operation progress/outcomes never mutate receiver state. Only parsed
    receiver evidence enters the canonical reducer.

## Package responsibilities

```text
domain <- application <- infrastructure <- desktop / CLI composition
   ^            ^              ^
   |            |              +--- protocol
   +------------+------------------- protocol

application + domain <- gui-lib <- desktop composition
domain + protocol + infrastructure <- read-only diagnostics
```

The existing inward dependency direction remains unchanged.

- `domain` owns identities, exact receiver values, observations, field state,
  operation intents/outcomes, capability evidence, and pure reducers.
- `protocol` owns X3800H wire commands and parsers. It has no sockets, timers,
  queues, or retry policy.
- `application` owns the session port, semantic admission, operation use cases,
  supplemental resource policy, and consumer-facing service.
- `infrastructure` implements the session actor with TCP and protocol, and
  implements independent HTTP/YAML/SSDP adapters.
- `gui-lib` renders application/domain state and submits intents. It does not
  tag receiver events with UI request IDs.
- desktop and CLI compose the same application service. Diagnostics stay
  separate and read-only.

Cross-package session contracts continue to have one definition in
`crates/application/src/ports.rs`. The concrete actor remains in infrastructure
because it composes runtime I/O and protocol parsing. Its reducer calls pure
domain transitions; it does not invent presentation policy.

Test support remains in each crate's test targets. The headless full-stack
harness belongs under `crates/infrastructure/tests` because infrastructure is
the outermost reusable layer that can legally compose domain, application, and
protocol. No production package depends on the virtual receiver, scripted I/O,
fixtures, or deterministic clock.

## Domain model

Names below are design-level Rust and may be refined mechanically during
implementation. Their distinctions are required.

```rust
struct ReceiverId(/* stable selected receiver identity */);
struct Epoch(u64);          // increments only after a socket is established
struct FrameSeq(u64);       // increments for every accepted frame in an epoch
struct StateRevision(u64);  // local publication order; never a CAS token
struct OperationId(u64);    // identifies one submitted operation only
struct SyncCycleId(u64);    // identifies one convergence cycle

struct Observation<T> {
    receiver: ReceiverId,
    epoch: Epoch,
    frame_seq: FrameSeq,
    observed_at: MonotonicInstant,
    origin: ObservationOrigin,
    value: T,
}

enum ObservationOrigin {
    ReceiverFrame,
    ReceiverFrameInQueryWindow { query: QueryId },
}

struct FieldState<T> {
    last_good: Option<Observation<T>>,
    validity: FieldValidity,
    synchronization: FieldSynchronization,
    last_issue: Option<FieldIssue>,
}

enum FieldValidity {
    Unknown,
    Current { valid_until: MonotonicInstant },
    Stale { reason: StaleReason },
    Unavailable { evidence: ReceiverEvidence },
}

enum FieldSynchronization {
    NotStarted,
    Converging {
        cycle: SyncCycleId,
        cause: SyncCause,
        reconcile_by: MonotonicInstant,
    },
    Settled {
        cycle: SyncCycleId,
        observed_at: MonotonicInstant,
    },
    Suspended { reason: SyncSuspension },
}

struct ReceiverState {
    receiver: ReceiverId,
    revision: StateRevision,
    connection: ConnectionState,
    system_power: FieldState<SystemPower>,
    main_zone: MainZoneState,
    zone2: ZoneState,
}

struct MainZoneState {
    power: FieldState<ZonePower>,
    source: FieldState<SourceId>,
    volume: FieldState<MasterVolume>,
    mute: FieldState<Mute>,
    sound_mode: FieldState<SoundModeStatus>,
}
```

`Current` means a value was observed in the active epoch and has not exceeded
the configured field-validity window or encountered a later query/connection
failure. `Converging` means that value is the latest receiver evidence but a
known local/remote activity burst or dependency change still has reconciliation
debt. `Settled` means that debt was reconciled after a quiet window; it does not
mean the receiver value is immutable or causally owned by this client.

A recognized receiver unavailable status, such as `MV---`, has receiver
evidence and is different from a local timeout; it does not erase a previous
volume. Validity and synchronization are separate so the desktop can show a
recent value immediately while also saying that related receiver state is
still converging.

`last_good` survives `Stale` and `Unavailable` transitions for display and
diagnostics. Switching to a different `ReceiverId` starts a new state with no
borrowed values. Wall-clock time may be included for logs, but validity uses a
monotonic clock.

### Exact power scopes

```text
SystemPower  <-> PW? / PWON / PWSTANDBY
MainZonePower <-> ZM? / ZMON / ZMOFF
Zone2Power    <-> Z2? / Z2ON / Z2OFF
```

`ZonePower` uses `On` and `Off`; it does not call zone-off “standby.” System
standby is exposed only by an explicitly named system-power intent. Main Zone
screens and controls never issue `PW` implicitly.

### Exact volume

```rust
enum MasterVolume {
    Minimum,
    DbHalfSteps(i16), // -159 through +36 => -79.5 dB through +18.0 dB
}
```

The wire encoder maps `Minimum` to the model-validated minimum code, maps exact
half-steps from `MV005` (-79.5 dB) through `MV98` (+18.0 dB), and rejects
anything above `MV98` for the X3800H profile. The minimum code and the `MV---`
unavailable status remain distinct. A receiver-configured lower maximum is
capability or observed rejection evidence; it is not silently clamped.
Presentation may derive Denon's alternate 0–98 scale, but that scale is not the
domain value.

### Sources

```rust
struct SourceDefinition {
    id: SourceId,                  // stable AVR protocol identifier
    display_label: Option<String>, // receiver/user supplied
    visibility: Visibility,
    evidence: CapabilityEvidence,
}
```

The model profile defines legal identifiers and aliases. Runtime catalog data
adds labels and enabled/hidden state. When reliable runtime evidence exists,
effective choices are the intersection of profile support and receiver
availability. If catalog evidence is unavailable, the UI may offer only the
region-specific, chart-backed profile choices and must describe runtime
availability as unknown. Arbitrary strings never reach the encoder.

The parser normalizes a documented response alias to a canonical `SourceId`
without changing the display label. Exact receiver identity replaces fuzzy
model-name `contains` checks.

### Sound mode

```rust
enum SoundModeIntent {
    Auto,
    Direct,
    PureDirect,
    Stereo,
    RecallMovie,
    RecallMusic,
    RecallGame,
    Select(VerifiedSoundModeId),
}

struct SoundModeStatus {
    id: SoundModeStatusId,
    raw: String,
}
```

An intent describes what the user asked the receiver to do. Status describes
the detailed value observed from `MS`. MOVIE/MUSIC/GAME recall does not write a
category into receiver state; the resulting detailed status is whatever the
receiver reports for the current signal and configuration. Unsupported raw
status remains diagnosable without becoming an encodable command.

## Session API

The application-facing service is cloneable, but cloning it never creates
another socket or state owner.

```rust
trait ReceiverSession {
    fn state(&self) -> StateSubscription<ReceiverState>;
    async fn synchronize(&self) -> Result<Readiness, SessionError>;
    async fn operate(&self, request: OperationRequest) -> OperationOutcome;
    async fn close(&self) -> Result<(), SessionError>;
}
```

`StateSubscription` behaves like a watch: a slow consumer can skip intermediate
revisions and immediately read the latest complete `ReceiverState`. Publication
does not await the consumer. Operation completion uses a dedicated one-shot
reply. Optional progress and diagnostics use a separate lossy telemetry channel;
losing telemetry cannot lose state or change an operation result.

The application facade may map or authorize requests but stores no mutable copy
of core receiver state. It reads the session subscription when it needs current
evidence.

## Session actor

### Owned state

The actor exclusively owns:

- socket reader and writer;
- selected `ReceiverId`, connection state, and active `Epoch`;
- frame decoder and next `FrameSeq`;
- canonical `ReceiverState` and next `StateRevision`;
- last-transmission time and post-`PWON` quiet deadline;
- one pending query matcher;
- one active mutating operation;
- queued user intents and scheduled reconciliation reads;
- merged synchronization debt, dependency closure, activity sequence, and
  quick/final reconciliation deadlines;
- reconnect attempt and monotonic deadlines.

User operations have priority over routine reconciliation, but never bypass wire
pacing. Incoming frames are always read and reduced while an operation waits.

### Transmission scheduler

Before every write the actor validates these gates in order:

1. selected receiver and established epoch still match the request;
2. encoded command belongs to the exact device profile;
3. command plus CR is within the documented length and ASCII grammar;
4. at least 50 ms has elapsed since the previous transmission;
5. the one-second quiet deadline after `PWON`, if any, has elapsed.

The actor records a write sequence before attempting the write and never runs
two writes concurrently. A user cancellation can remove work that has not begun;
once writing starts, cancellation only detaches the waiter because delivery may
already have happened.

### Reading and reducing

Every complete frame is processed in this order:

1. validate CR framing, length, and text encoding;
2. assign `(ReceiverId, Epoch, FrameSeq)`;
3. parse into a typed observation or retained unknown-frame diagnostic;
4. apply the pure domain reducer;
5. publish a new full state revision if state changed;
6. offer the typed observation to pending query and operation matchers.

Malformed or unknown frames cannot mutate typed state. An oversized or
unterminated frame makes stream alignment unsafe, so the actor marks fields
stale, closes the socket, and reconnects. An unknown but well-framed command
family is retained as bounded diagnostic evidence and reading continues.

### Query semantics

Only one query is in flight. A query matcher specifies a typed field, epoch,
write sequence, and deadline. The first valid observation for that field with a
later frame sequence completes the query. Other frames still update state.

Because Denon does not mark a line as “response to query” versus “unsolicited
event,” completion means **post-query receiver observation**. The origin records
that the frame arrived in a query window but does not imply acknowledgement.
The default response deadline is based on the documented 200 ms period plus a
small scheduler/network margin and is finalized by live validation.

A timeout makes that field stale. Because stream correlation is now uncertain,
the actor reconnects before another query. Read-only queries may be retried on
the new epoch; controls never are.

## Monitoring lifecycle

```text
Selected
   |
   v
Connecting --failure--> Reconnecting(backoff + jitter) --+
   |                                                   ^  |
   | socket established: new Epoch                    |  |
   v                                                   |  |
Synchronizing -> Ready <-> Degraded --stream fault-----+  |
   |              |                                      |
   |              +-- events + periodic reconciliation --+
   +-- per-field observations published as they arrive

explicit close -> Selected/Stopped (no automatic reconnect)
```

### Initial synchronization

After each established socket, the actor queries at least:

1. system power `PW?`;
2. Main Zone power `ZM?`;
3. source `SI?`;
4. master volume `MV?`;
5. mute `MU?`;
6. sound mode `MS?`;
7. Zone 2 power `Z2?` when supported and enabled.

Each field publishes independently. Synchronization finishes after every
required field has an outcome. `Ready` requires current or explicitly
receiver-unavailable evidence for all required fields; any local timeout,
transport, or parse failure yields `Degraded`. Socket establishment alone is
`Synchronizing`, and no path fabricates a complete snapshot.

Queries are power-aware. When a zone is off and a field is documented as
unavailable, the state records that condition and preserves its previous value
as last-good. It does not repeatedly hammer a known unavailable family.

### Events, validity, and reconciliation

Recognized receiver frames update and publish the mirror before any debounce,
query, or UI work. Reconciliation is a safety net for delayed/lost receiver
events, related fields changed implicitly by the AVR, idle-socket detection,
and physical-remote activity. It is not one aggregate snapshot transaction.
Queries are paced, and every field retains its own observation time.

Phase 5 uses adaptive reconciliation:

- a connected Main Zone receives a paced core sweep every five seconds with
  ±10% jitter; power-aware profiles may reduce inactive-field queries but must
  continue `PW`/`ZM` liveness reads;
- a recognized changed event opens a targeted activity cycle and schedules a
  quick reconciliation after 250 ms without relevant activity;
- a local write opens a control cycle immediately, while operation matching
  watches receiver frames without waiting for the background cycle;
- several controls or events merge affected fields into one debt set rather
  than starting competing full refreshes; and
- validity expires after two missed successful sweep opportunities, while a
  failed targeted query marks the affected field stale immediately.

The concrete five-second and 250 ms defaults are initial correctness SLOs and
must be confirmed by load and live-receiver tests. Configuration may tune them
within validated bounds; it cannot disable pacing, epoch checks, or the
event-loss fallback.

### Synchronization debt and dependency closure

```rust
struct SyncDebt {
    cycle: SyncCycleId,
    receiver: ReceiverId,
    epoch: Epoch,
    fields: FieldSet,
    causes: CauseSet,
    activity_seq: u64,
    quick_reconcile_at: MonotonicInstant,
    final_reconcile_at: Option<MonotonicInstant>,
}
```

Debt is idempotently unioned by field and active epoch. It is never represented
as a second snapshot. Opening debt marks affected fields `Converging` while
retaining and publishing their latest validity/value.

The minimum dependency closure is:

| Cause or changed field | Core fields reconciled | Supplemental effect |
|---|---|---|
| System power | `PW`, `ZM`, all enabled zones, all active-zone core fields | Invalidate/schedule every power-dependent resource |
| Main Zone power | `ZM`, `SI`, `MV`, `MU`, `MS` | Invalidate/schedule Main Zone resources |
| Source | `SI`, `MS` | New source/context revision; refresh signal-dependent information |
| Volume | `MV` | None |
| Mute | `MU` | None |
| Sound mode | `MS` | New sound-context revision; refresh mode-dependent information |
| Zone 2 power | `Z2` | Zone 2 resources only |
| Quick Select recall | `ZM`, `SI`, `MV`, `MU`, `MS` | Refresh all Main Zone dependent resources |
| Reconnect or unknown prior state | Every required core field | Refresh resources after core settles |

Live evidence may expand a closure but may not remove a documented dependency.
Supplemental refresh is triggered only after the core context observation it
depends on and never blocks debt reconciliation.

For each debt cycle the actor:

1. captures `activity_seq` and the active epoch;
2. waits for the quick quiet deadline while still reducing every frame;
3. issues only the dependency-closure queries, one at a time through the
   transmission scheduler;
4. merges newly observed dependencies or activity back into the debt;
5. clears a field only after a successful post-query observation and a 250 ms
   relevant-frame quiet period with unchanged epoch/activity token; and
6. publishes `Settled` only when the entire debt set is clear.

A query-window frame can satisfy the current field and still expand dependency
debt if its value changed. It does not recursively create a duplicate cycle.
Any relevant receiver frame after a field was queried reopens that field before
the cycle can settle.

Local writes additionally retain a final reconciliation deadline at least five
seconds after the most recent related write, matching Denon's documented event
allowance. A quick cycle can update the UI and complete an operation earlier,
but fields touched by a local control remain `Converging` until the final
post-horizon targeted query succeeds. Repeated local controls extend one final
deadline and merge their dependency closures, so a burst creates one final
settlement pass rather than one full refresh per control.

Continuous hardware-remote activity keeps affected fields `Converging` while
every received value is still published. The actor runs at most one quick
activity reconciliation per second during a sustained burst, then settles once
the stream is quiet. Routine sweeps and control writes remain starvation-free
through a bounded scheduler budget.

The scheduler has one queue and these priorities/limits:

1. inbound frames are always reduced before another queued action;
2. an already-started write/read and the `PWON` quiet rule retain wire safety;
3. an active operation receives its next required preflight/dispatch/
   confirmation action;
4. due final and quick synchronization debt receives the next eligible query
   after at most eight user transmissions or 500 ms of continuously ready user
   work, whichever comes first; and
5. periodic sweep work is lowest priority and is merged with existing debt
   instead of duplicated.

The initial eight/500 ms fairness limits are configurable only within tested
bounds. A control burst can delay a reconciliation slot, but it cannot cancel
debt, mark it settled, or starve it indefinitely.

### Convergence service levels

These SLOs apply when the TCP connection and receiver remain responsive:

| Trigger | Required behavior |
|---|---|
| Complete receiver frame | Reduce and publish before servicing the next queued application request; desktop target is within 100 ms under the tested load envelope. |
| Local control sequence | Show every observed value immediately; finish final dependency reconciliation within 5.5 seconds plus one bounded query sequence after the last related write. |
| Hardware-remote event burst | Show each latest received value immediately; begin targeted reconciliation after 250 ms quiet and settle within one bounded query sequence if no later activity arrives. |
| Lost hardware-remote event | Detect through the five-second core sweep and converge within that sweep's bounded query duration. |
| Re-established socket | Publish `Synchronizing` immediately and finish a responsive core synchronization within two seconds. |

If an SLO cannot be met, state remains `Converging`, becomes `Stale`, or the
connection becomes `Degraded`; it is never silently called settled. The failure
artifact records scheduler delay, debt age, oldest affected field, and last
successful reconciliation.

### Reconnect

On disconnect, framing loss, or query timeout, the actor:

- marks current fields stale while preserving last-good observations;
- resolves the active operation as indeterminate when dispatch may have begun;
- closes the old socket and rejects any delayed old-epoch work;
- retries with bounded exponential backoff and jitter, for example 250 ms,
  500 ms, 1 s, 2 s, 5 s, 10 s, then at most 30 s;
- continues until connection succeeds, the selected receiver changes, or the
  user explicitly closes the service.

An epoch increments only after a new socket is established. “Reconnecting” does
not manufacture an epoch. Initial connection refusal should include guidance
about Network Control in standby and another Telnet client, while clearly
stating that either is only a possible cause.

## Control transaction

Only one mutating AVR operation is active at a time. This makes conflicting
user operations deterministic while the reader continues processing external
changes.

```text
validate intent and capability
          |
targeted preflight observation
          |-- target current --> AlreadyObserved
          |
wait for scheduler -> write once -> dispatch certainty
          |                         |
          |                         +-- definitely not started
          |                              -> RejectedBeforeDispatch
          |
observe events + paced targeted queries through horizon
          |-- target observed after write --> ObservedRequestedValue
          +-- disconnect/expiry/cancel ----> Indeterminate
```

### Admission and preflight

Application policy validates the intent against the exact model profile and
effective capability evidence. The actor then obtains a targeted observation of
the affected field. Cached state may replace that query only when it is current
and `Settled` in the active epoch and younger than the strict control-preflight
limit. A current-but-`Converging` target still requires a query. If that
post-query observation matches, the operation may return `AlreadyObserved`, but
the broader dependency debt remains until its own settlement rule passes.

There is no `expected_version`. The receiver offers no atomic compare-and-set,
and an aggregate local version cannot prevent changes between preflight and
write. `StateRevision` exists solely to order local publications.

### Dispatch certainty

```rust
enum DispatchCertainty {
    NotStarted,
    CompleteWrite,
    Unknown, // write began but completion cannot be established
}

enum OperationOutcome {
    AlreadyObserved { observation: ReceiverEvidence },
    ObservedRequestedValue {
        operation: OperationId,
        dispatch: DispatchCertainty,
        observation: ReceiverEvidence,
    },
    RejectedBeforeDispatch { reason: OperationRejection },
    SupersededBeforeDispatch {
        operation: OperationId,
        by: OperationId,
    },
    Indeterminate {
        operation: OperationId,
        dispatch: DispatchCertainty,
        last_observation: Option<ReceiverEvidence>,
        reason: IndeterminateReason,
    },
}
```

Validation, unsupported capability, or disconnection before any write begins is
`RejectedBeforeDispatch`. Once the socket write begins, an error is
`Indeterminate { dispatch: Unknown }`; the actor must not retry. A complete
local write is evidence that bytes were handed to the socket, not that the AVR
applied them.

### Observation window

After a complete or ambiguous dispatch, every same-field frame is reduced into
state. A matching value with a frame sequence after write start completes as
`ObservedRequestedValue`. This wording is intentional: another remote could
have caused the same value.

Nonmatching observations are retained but do not fail the operation early.
This is required for `MS`, where Denon documents an intermediate present mode.
The actor schedules targeted reads at paced points in the window; a candidate
schedule is 250 ms, 1 s, 2.5 s, and 5 s. `PWON` suppresses every transmission
for its required first second. The final horizon is at least the documented
five-second event allowance plus response and scheduling margin, initially
5.5 seconds.

Expiry with a different value is still `Indeterminate`: the target may have
briefly applied and then changed externally. The final result carries the last
observation. Multi-field operations such as Quick Select define an explicit
observation predicate and affected-field set rather than running a generic full
refresh.

### Cancellation

Before dispatch, cancellation removes the queued operation and returns
`RejectedBeforeDispatch`. During or after a write, cancellation cannot erase
delivery uncertainty; the operation reaches `Indeterminate` for the requesting
consumer while the core state reader continues. Dropping a GUI future never
stops the session actor or rolls back receiver state.

### Repeated controls and hardware-remote contention

Every intent declares its directly controlled field and dependency/conflict
scope. Writes remain globally ordered, and synchronization debt is merged
independently from operation completion.

- An identical intent for an already pending operation joins that operation;
  it does not produce a duplicate write.
- Multiple queued, not-yet-started intents with overlapping scope retain only
  the newest target. Older requests finish as `SupersededBeforeDispatch`.
- If a newer overlapping intent arrives after the old write began but before
  its target was observed, the old operation becomes
  `Indeterminate(SupersededByNewerIntent)` and is never replayed. The newer
  intent is dispatched only after normal pacing.
- Nonoverlapping intents remain FIFO behind the active operation. An operation
  normally releases the queue as soon as its target is observed; final
  synchronization continues through merged debt and does not block the next
  independent user intent.
- Power and Quick Select use their full dependency closure as conflict scope;
  source and sound-mode scopes overlap because source selection can change the
  receiver's detailed mode.

A physical remote has no application `OperationId` and never enters this
coalescing policy. Every one of its receiver frames updates canonical state.
A nonmatching external value during local confirmation does not fail early; it
is retained as the latest observation and contributes synchronization debt. If
the local target is never observed, the operation ends `Indeterminate` with
that evidence. If the local target is observed and a later remote action changes
it, the historical operation result remains true only for its recorded time;
the dashboard always renders the newer canonical state.

The domain stores no “desired receiver state.” A queued or dispatched target is
operation data, so it cannot overwrite hardware-remote observations or be
mistaken for synchronization.

## Supplemental resources

Core state is never nested with HTTP or derived evidence. Each supplemental
resource has its own lifecycle:

```rust
struct ResourceState<T> {
    receiver: ReceiverId,
    epoch: Option<Epoch>,
    context_revision: Option<StateRevision>,
    last_good: Option<T>,
    condition: ResourceCondition,
    observed_at: Option<MonotonicInstant>,
    last_error: Option<ResourceError>,
}
```

The application supplemental coordinator watches core state and starts
single-flight jobs through HTTP adapters. Completion is accepted only when
receiver and epoch still match. Signal-dependent information also checks the
source/sound-mode context revision that launched it. A late result is discarded,
not merged and relabeled.

HTTP information, source catalog, Quick Select names, and EQ evidence publish
separate subscriptions. Their failure can make only that resource stale.
Neither completion nor invalidation changes core `StateRevision` unless core
state itself changes. No supplemental job runs on the TCP actor or blocks its
channels.

Quick Select recall is a multi-field intent. A command-family echo is not
per-slot registration evidence. Recall outcome is based on the explicitly
chosen post-dispatch state predicate; saved slot contents remain unavailable
unless a documented and validated read supplies them.

## Presentation and CLI

### One-way desktop state flow

```text
core state watch --------------------+
supplemental resource watches -------+--> DesktopProjection --> Iced view
operation progress/final outcome ----+

Iced user action --> typed intent --> ReceiverSession operation
```

`DesktopProjection` is a deterministic presentation reducer in `gui-lib`. It
does not own receiver truth, infer protocol authority, or feed presentation
state back into core state. The desktop executable creates dependencies and
runs Iced; it contains no monitoring/control policy.

The GUI subscribes once to latest core state and independently to supplemental
resources. It tracks an `OperationId` only for the control that owns a spinner
or message; it never uses that ID to reject state. State acceptance compares
receiver identity, epoch, and revision supplied by the state owner. A receiver
switch first changes the selected `ReceiverId`, clears receiver-specific drafts
and feedback, detaches old subscriptions, and then binds the new service. Late
old-receiver state or supplemental results cannot flash in the new dashboard.

### Presentation state

Receiver evidence and user interaction are separate:

```rust
struct ControlPresentation<T> {
    observed: PresentedField<T>,
    draft: Option<T>,
    pending: Option<PendingOperation>,
    last_outcome: Option<PresentedOutcome>,
}
```

`observed` is derived only from `FieldState`. `draft` is local editing state and
is labeled as a requested value when shown. `pending` records an `OperationId`
and intent, never a predicted receiver value. `last_outcome` renders the
evidence-based operation result and can coexist with a later external state
change.

For volume, beginning a drag creates a draft while the observed label continues
to show the receiver value and validity. External volume events continue to
update observed state during the drag. Release submits the draft as an intent;
after a terminal outcome the draft is cleared and the control rebases to the
latest observed state. NaN, infinity, non-half-step values, values below
-79.5 dB unless `Minimum` was explicitly selected, and values above +18.0 dB
never become an intent.

Repeated activation of the same pending control is suppressed in presentation,
and application admission remains authoritative. Other controls may queue only
through the application's serialized operation service. Disconnect, receiver
switch, or task cancellation never paints a requested value as applied.

Presentation distinguishes:

- current-and-settled value from current-but-converging value;
- stale last-known value and reason;
- receiver-reported unavailable;
- unknown/not yet observed;
- synchronizing, ready, degraded, reconnecting, and explicitly disconnected;
- requested value observed versus indeterminate control.

The dashboard derives a `SyncSummary` from core fields and shows the oldest
outstanding convergence debt, last successful settlement, and reconnect state.
A manual Refresh action adds every required core field to the actor's debt set;
it does not create a presentation-owned snapshot or parallel query loop. The
summary clears only from a canonical `Settled` publication.

It also distinguishes Main Zone, system, and Zone 2 power in control labels and
feedback. Main Zone surfaces never use “standby” for zone-off. Unsupported or
insufficiently evidenced controls are disabled with a reason; stale state alone
cannot justify a no-op or success message.

Operation feedback uses historical wording such as “requested value observed at
14:03:12” and never “receiver is now …” after the canonical state has moved on.
When several desktop controls are queued, each field shows its own draft/pending
state while one shared sync summary reports the merged hardware convergence.

Supplemental cards render their own loading/current/stale/unavailable/error
condition. They cannot move the core dashboard from synchronizing to ready,
disable unrelated Main Zone controls, or replace current core state with an
older context result.

Subscription and operation tasks are owned by one desktop lifecycle scope.
Closing the app or changing receiver cancels presentation waiters, requests
service shutdown where appropriate, and joins tasks without waiting on a
blocked HTTP job. Task failures become persistent diagnostic feedback rather
than disappearing or panicking the update loop.

The CLI creates the same async service in a short-lived runtime, synchronizes,
submits one operation, prints its evidence-based outcome, and closes. It does
not calculate or accept a receiver resource version. If another process owns
the receiver's Telnet connection, it fails with actionable guidance rather than
using a second protocol path or undocumented HTTP write.

The standalone diagnostics application remains read-only. Documentation must
require the desktop/other Telnet client to disconnect before a live diagnostic
run because Phase 5 does not add a cross-process broker.

## Test architecture

The detailed suite, harness, scenario catalog, commands, and completion gates
are defined in the
[headless correctness test plan](phase-5-receiver-correctness-test-plan.md).
Tests are an architectural seam, not delivery automation:

- core smoke/scenario/property tests compose only domain, application,
  protocol, and infrastructure;
- a strict loopback X3800H tests real framing and socket integration;
- scripted byte I/O deterministically tests partial writes and exact fault
  boundaries;
- a manually advanced clock drives all protocol and lifecycle deadlines;
- a checked coverage ledger maps each invariant and acceptance criterion to
  named executable evidence; and
- desktop projection tests replay canonical traces without opening a native
  window.

### Pure tests

- exhaustive power-family, volume-boundary, source-ID, alias, unavailable, and
  sound-status parser/encoder tests;
- reducer properties: frame sequence never regresses, old epochs cannot mutate
  state, failures preserve last-good values, and revision changes only when
  published state changes;
- operation state-machine tests for every dispatch certainty and terminal
  outcome;
- capability tests separating model evidence, region evidence, runtime catalog,
  and current signal context.

### Deterministic virtual AVR

A local scripted server uses a controllable monotonic clock and can:

- reject writes less than 50 ms apart;
- send the old `MS` value before the requested value;
- delay an event to the edge of the five-second allowance;
- send a same-family unsolicited frame before a query's later status frame;
- emit more changes than every notification capacity;
- disconnect before write, during a partial write, after write, and during
  synchronization;
- reconnect while old HTTP and query work remains outstanding;
- refuse a second client;
- simulate Network Control unavailable in standby;
- distinguish `PW`, `ZM`, and `Z2` while zones differ;
- accept `MV98` and reject `MV985`;
- expose renamed, hidden, aliased, region-specific, and missing sources.

Assertions inspect both the wire transcript and consumer-visible state. Tests
must prove no control replay, no cross-epoch merge, no socket-reader
backpressure, and desktop/CLI semantic parity. Scenario failures include the
wire transcript, logical time, epochs, state revisions, operation transitions,
and reproducible random seed when applicable.

### Headless smoke gate

The fast smoke gate uses only public application/session APIs against the real
loopback actor. It proves connect and synchronization, unsolicited observation,
one observed control, stale/reconnect recovery, slow-subscriber convergence,
supplemental isolation, ambiguous-write no-replay, and clean shutdown. It must
finish in seconds, use no external network, and import no GUI or CLI package.

### Scenario and generated gates

The deterministic scenario gate covers every state edge, protocol family,
timing boundary, control outcome, disconnect/write boundary, cancellation,
receiver switch, pressure case, supplemental transition, and shutdown path in
the checked ledger. Property-based reference-model tests then generate valid
and adversarial interleavings and minimize failures to replayable traces.

Arbitrary parser bytes, targeted mutation tests, and reviewed live-trace replay
complement the named scenarios. Line coverage is diagnostic only; it does not
replace behavioral and invariant coverage.

### Headless desktop gate

Canonical core traces feed `DesktopProjection` directly. Tests cover every
field validity and lifecycle representation, receiver/epoch rejection,
draft-versus-observed interaction, operation feedback, power scope, source and
sound-mode semantics, supplemental isolation, external events during pending
controls, receiver switching, task failure, and shutdown. These tests do not
run Iced or a delivery executable.

### Live validation

Live tests are opt-in and never part of `make check`. Each run records receiver
model, firmware, region, Network Control setting, active zones, input signal,
speaker/headphone context, commands, raw frames, monotonic timings, and date.
State-changing tests require an explicit hardware-test command and safe volume
limits; diagnostics remain read-only.

Required scenarios cover initial synchronization, external remote changes,
rapid volume events, each Main Zone control, system-versus-zone power, sound
mode transition, standby/reconnect, another active TCP client, and supplemental
HTTP failure. An explicitly armed headless sequence performs several service
controls, several physical-remote controls, and same-/different-field
interleavings while recording convergence latency and final settlement. Redacted
approved traces become regression fixtures.

## Migration and removal order

1. Add the deterministic clock, strict virtual receiver, scripted byte I/O,
   trace recorder, and requirement ledger as test-only infrastructure.
2. Add new domain types and reducers beside old types with conversion allowed
   only in tests and temporary composition code.
3. Add the exact X3800H protocol profile and scripted-server cases.
4. Implement the canonical actor and application session contract without
   exposing it to production delivery paths.
5. Implement operation transactions and supplemental subscriptions against the
   new actor.
6. Implement and exhaustively test `DesktopProjection`, then cut desktop and GUI
   state/control to the new service in one change.
7. Cut CLI to that service and update its grammar/output.
8. Remove `SyncAvrClient`, old controller snapshot ownership, synthetic bridge
   correlation, `resource_version` admission, normalized volume, and obsolete
   protocol mappings.
9. Run smoke, scenario, property, trace, mutation, desktop-model, and live
   gates, then update user guides and mark the phase
   implemented.

Temporary parallel code may exist while compiling the migration, but only one
state machine may be reachable in any production executable after cutover.

## Explicitly rejected alternatives

- **Increase queue capacity.** This delays but does not fix divergent snapshots
  or loss under an unbounded receiver burst.
- **Poll faster.** This increases protocol pressure, still violates pacing if
  unscheduled, and does not establish operation causality.
- **Keep aggregate resource versions.** A local counter cannot make the AVR
  perform atomic compare-and-set and unrelated fields create false conflicts.
- **Treat command echo as acknowledgement.** Responses and events share wire
  shapes and no transaction ID exists.
- **Use HTTP writes when Telnet is busy.** The endpoint is undocumented for the
  target and field reports show compatibility failures.
- **Keep desktop and CLI transports for convenience.** Duplicate semantics are
  already a source of drift and conflict with one-client receiver behavior.
- **Preserve old APIs with adapters.** Adapting `PW` to look like Main Zone
  power or exact dB to look like generic 0–100 values would preserve false
  meaning, not compatibility.
- **Count unit tests as full-stack evidence.** Isolated tests cannot expose
  scheduler, socket, reducer, operation, and subscriber interactions. The
  headless composed smoke and scenario suites are mandatory.
- **Use wall-clock sleeps to provoke races.** Timing luck creates flaky tests
  and misses exact boundaries. Protocol time is injected and manually advanced.
- **Rely on rendered GUI automation for desktop correctness.** Rendering tests
  are useful for appearance, but canonical-state projection and operation
  semantics need deterministic headless reducer tests.
