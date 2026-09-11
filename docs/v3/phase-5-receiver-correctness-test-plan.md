# Version 3, Phase 5 — Headless correctness test plan

**Complete — 2026-09-11.** Core smoke, protocol/domain boundary, headless
projection tests, deterministic scenarios, properties, replay coverage, and
live validation are implemented. Broader catalog expansion remains follow-up.
This
plan defines the delivery-independent verification required by
[Phase 5](phase-5-receiver-correctness-overview.md). The
[architecture](phase-5-receiver-correctness-architecture.md) defines the system
under test; this document defines how its claims become executable evidence.

“Comprehensive” and “complete” mean complete against the Phase 5 requirements,
state transitions, protocol families, operation outcomes, and enumerated fault
boundaries below. They do not mean proof against every future firmware behavior.
Any newly discovered behavior must first become a fixture and scenario, then a
supported contract.

## Test objectives

- Exercise domain, application, protocol, and infrastructure together without
  launching or importing CLI, desktop, or GUI packages.
- Provide a fast smoke answer to “does the core stack connect, synchronize,
  observe, control, recover, and close?”
- Test complete monitoring and control scenarios against a strict virtual
  X3800H with deterministic time and byte-level fault injection.
- Prove convergence after several desktop controls, several physical-remote
  controls, and adversarial interleavings of both control surfaces.
- Explore valid and adversarial event interleavings with property-based state
  machines while preserving reproducible seeds and minimized failures.
- Replay reviewed real-receiver captures through the same parser and reducer.
- Test desktop presentation correctness headlessly, without opening an Iced
  window, as a separate projection suite.
- Make every failure explain itself with the wire transcript, logical time,
  state revisions, connection epochs, operation trace, and random seed.

## Test topology and boundaries

```text
                        no delivery dependencies
                                  |
                                  v
domain unit/property <- application policy/contract tests
        ^                         |
        |                         v
protocol corpus/property -> infrastructure session actor
                                      |
                                      v
                           headless core integration
                             /                 \
                    strict virtual AVR     scripted transport

canonical state/outcome traces -> headless desktop projection tests
                                  (gui-lib model only; no window)
```

The core smoke and scenario suites may depend on `domain`, `protocol`,
`application`, and `infrastructure`. They must not depend on or import
`gui-lib`, `apps/cli`, `apps/desktop`, or `apps/diagnostics`. The boundary check
will enforce this from both Cargo metadata and source imports.

Desktop projection tests are separate. They may import `gui-lib`, application,
and domain, but may not launch an Iced runtime, create a native window, open a
real socket, or invoke a delivery executable. CLI parser/output tests remain
ordinary delivery tests and are not evidence for core correctness.

## Planned test layout

```text
crates/domain/
  src/...                                  # reducer and value unit tests
  tests/receiver_state_properties.rs       # generated state transitions

crates/protocol/
  tests/x3800h_protocol_matrix.rs          # chart-backed table tests
  tests/x3800h_trace_replay.rs             # reviewed raw frame corpus

crates/application/
  tests/receiver_operation_contract.rs     # fake-session use-case tests
  tests/supplemental_resource_contract.rs

crates/infrastructure/tests/
  core_smoke.rs                            # small public-API smoke suite
  receiver_scenarios.rs                    # full scenario catalog
  receiver_properties.rs                   # generated interleavings
  support/
    clock.rs                               # manually advanced monotonic clock
    scripted_io.rs                         # deterministic byte/fault seam
    virtual_avr.rs                         # strict loopback X3800H
    scenario.rs                            # typed Given/When/Then DSL
    trace.rs                               # wire/state/operation recorder
    assertions.rs                          # invariant-aware diagnostics
  fixtures/x3800h/
    protocol/                              # reviewed synthetic frames
    captures/                              # redacted live traces + metadata

crates/gui-lib/tests/
  desktop_projection.rs                   # no renderer or native window
  desktop_operation_scenarios.rs           # canonical trace replay
```

Shared helpers remain test-only. No production crate may depend on test support,
fixtures, a fake clock implementation, or the virtual AVR.

## Developer commands and required gates

Phase 5 will add these stable commands:

```text
make test-core-smoke       # deterministic, delivery-free, target under 5 s
make test-core-scenarios   # complete deterministic scenario catalog
make test-core-properties  # seeded state-machine/property tests
make test-desktop-model    # headless presentation projection tests
make test-live-x3800h      # opt-in read-only AVC-X3800H hardware smoke
make test-live-x3800h-controls
                           # explicitly armed app/physical-remote sequences
```

`make check` must run core smoke, deterministic scenarios, property tests with a
fixed CI case count, and desktop-model tests. No required test may access the
LAN, depend on a physical receiver, use an arbitrary sleep, or start CLI/GUI.
The live commands are excluded from ordinary checks. The ordinary live smoke is
read-only. State-changing hardware validation remains a separate, explicitly
armed workflow requiring `ALLOW_RECEIVER_WRITES`, a safe volume ceiling, and
recorded restoration values.

## Harness architecture

### Deterministic clock

All production deadlines and scheduling relevant to correctness use an injected
monotonic clock/sleeper contract. Tests advance logical time explicitly. This
covers:

- 50 ms minimum transmission spacing;
- query response deadlines;
- the one-second post-`PWON` quiet period;
- the five-second event allowance and control horizon;
- per-field validity expiry;
- reconciliation cadence and jitter;
- reconnect backoff; and
- cancellation deadlines.

The harness may yield to let tasks progress, but it may not use wall-clock sleeps
to make races “usually pass.” Every test has a short real-time watchdog solely
to fail deadlocks.

### Two transport levels

The suite uses two complementary adapters:

1. `StrictVirtualAvr` is a real loopback TCP server on an ephemeral port. It
   verifies framing, command order, parser integration, socket lifecycle, and
   public infrastructure composition.
2. `ScriptedIo` implements an internal byte-I/O seam. It deterministically
   fails before the first byte, after any selected byte, after a complete
   write, between reads, or during shutdown. It is required because an
   operating-system TCP socket cannot reliably reproduce every partial-write
   boundary.

The same actor, reducer, operation machine, and scheduler run above both. Only
the lowest byte transport changes.

### Strict virtual X3800H

The virtual receiver maintains independent system, Main Zone, and Zone 2 state
and rejects behavior outside the target profile. It will:

- accept exactly CR-terminated, length-bounded ASCII commands;
- reject or record transmissions closer than 50 ms;
- enforce the post-`PWON` quiet period;
- answer supported queries with configured latency;
- emit immediate, delayed, duplicate, omitted, reordered, and burst events;
- model `PW`, `ZM`, and `Z2` independently;
- model Min, -79.5 through +18.0 dB, `MV---`, and configured volume limits;
- model protocol source IDs, aliases, labels, hidden sources, and region data;
- emit detailed sound status independently from MOVIE/MUSIC/GAME intent;
- execute physical-remote actions that change one or several receiver fields
  without creating an application operation ID;
- allow scripted socket refusal, half-close, malformed frames, and reconnect;
- refuse a second client when that scenario is enabled; and
- record every command, frame, connection, and logical timestamp.

It must be strict enough to fail an invalid client. A permissive fake that
returns the expected answer regardless of command cannot support correctness
claims.

### Typed scenario DSL

Scenarios are compile-checked Rust descriptions, with raw frame fixtures used
where exact bytes matter. A scenario declares initial receiver state, actor
configuration, receiver script, client actions, and required evidence:

```rust
Scenario::named("sound_mode_old_then_new")
    .given(system_on().main_zone_on().sound_mode("STEREO"))
    .when(control(SoundModeIntent::RecallMovie))
    .receiver(emits_after(100.ms(), ms("STEREO")))
    .receiver(emits_after(4_900.ms(), ms("DOLBY SURROUND")))
    .expect_outcome(observed_requested_value("DOLBY SURROUND"))
    .expect_invariant(no_write_spacing_below(50.ms()));
```

The exact API may differ, but every scenario must have a stable name and use
Given/When/Then data rather than bespoke timing code.

### Failure artifact

Every failed scenario prints or stores one deterministic artifact containing:

- scenario name and source requirement IDs;
- random seed and minimized action list, when generated;
- logical clock transitions;
- socket connect/disconnect and epoch transitions;
- byte-level writes and complete received frames;
- parsed observations;
- canonical state revisions and field conditions;
- synchronization cycle IDs, merged debt, activity tokens, quick/final
  deadlines, and settlement transitions;
- operation states, dispatch certainty, and terminal outcome; and
- unmet expectations.

Receiver addresses, user labels, and other capture identifiers are redacted
before fixtures are committed.

## Fast core smoke suite

The smoke suite exercises only public application/session APIs over the real
loopback actor and strict virtual receiver. It contains a deliberately small,
high-signal set:

| ID | Scenario | Required proof |
|---|---|---|
| SMK-01 | Connect and synchronize | New epoch; `PW`, `ZM`, `SI`, `MV`, `MU`, `MS`, and `Z2` queried in paced order; state becomes `Ready`. |
| SMK-02 | Physical-remote burst | External source, volume, mute, and Main Zone power frames publish immediately, merge dependency debt, and reach one settled state after quiet reconciliation. |
| SMK-03 | Several desktop controls | Source, volume, mute, and sound-mode intents are written in valid order, each outcome is evidence-based, and one merged final cycle settles all dependencies. |
| SMK-04 | Interleaved control surfaces | A desktop volume intent and later physical-remote change both remain observable; operation history and latest receiver status do not overwrite each other. |
| SMK-05 | Stale and recover | Disconnect preserves last-good values as stale; reconnect creates a new epoch and synchronization returns to `Ready`. |
| SMK-06 | Burst and slow subscriber | A subscriber stalled across more updates than channel capacity reads the final full state; the socket reader remains responsive. |
| SMK-07 | Lost event fallback | An omitted physical-remote event is found by the five-second core sweep and settles through a targeted observation. |
| SMK-08 | Supplemental isolation | A never-completing HTTP fake does not delay core synchronization, event reduction, control, or shutdown. |
| SMK-09 | Ambiguous write | A partial write yields `Indeterminate`, does not replay, and leaves state evidence intact. |
| SMK-10 | Clean close | Explicit close stops retries, completes pending waiters, closes once, and leaks no task. |

The suite fails if it imports a delivery package, contacts a non-loopback
address, requires environment configuration, or exceeds its deterministic
runtime budget.

## Comprehensive scenario catalog

Every row below becomes one or more named tests. A requirement-to-scenario
ledger is checked in CI; a Phase 5 requirement without a scenario is an error.

### Connection and epoch lifecycle

- initial refusal, DNS/selection failure before session creation, successful
  first connect, explicit close, and selected-receiver replacement;
- disconnect while idle, during synchronization, during query, before control
  write, during partial control write, after complete write, and during the
  observation window;
- repeated refusal through each backoff step, eventual recovery after the
  maximum interval, and explicit close during backoff;
- no epoch increment for failed attempts; exactly one increment per established
  socket;
- delayed query, operation, and supplemental results from an old epoch;
- receiver A events arriving after selecting receiver B; and
- second-client refusal with truthful, non-diagnostic guidance.

### Framing, parsing, and query windows

- one frame per read, fragmented frame, multiple frames per read, CR-only
  termination, maximum-size frame, oversized frame, invalid UTF-8, EOF before
  CR, empty frame, and unknown well-formed family;
- every supported `PW`, `ZM`, `Z2`, `SI`, `MV`, `MU`, and `MS` query/status
  form, documented alias, minimum, unavailable form, and boundary value;
- unrelated event while a query waits, same-family event immediately before a
  later status, duplicate matching frames, late response after timeout, and
  event burst during query;
- query timeout followed by reconnect before another query, with the late old
  frame unable to satisfy new-epoch work; and
- unknown frames retained as bounded diagnostics but unable to mutate typed
  state.

### Monitoring and state reduction

- each field first unknown, current, stale, explicitly unavailable, recovered,
  and expired;
- one field failing while all others remain current;
- last-good evidence retained through timeout, malformed stream, disconnect,
  receiver-unavailable status, and reconnect;
- duplicate observations that do not create false value changes;
- monotonic frame sequence within an epoch and monotonic publication revision;
- event-only update, reconciliation-only repair, event/reconciliation agreement,
  and external change between sequential reconciliations;
- Main Zone off while system and Zone 2 remain on, and every other meaningful
  zone-power combination;
- event storm larger than all queue capacities with several subscribers at
  different speeds; and
- field-validity expiry while user operations defer routine reconciliation.

### Activity convergence and synchronization debt

- every dependency-closure row for system power, Main Zone power, source,
  volume, mute, sound mode, Zone 2, Quick Select, and reconnect;
- changed frame publishes before reconciliation, transitions affected fields to
  `Converging`, and retains current validity/value;
- quick reconciliation at 249/250 ms boundaries, relevant activity resetting
  the deadline, and unrelated activity not resetting it;
- field queried successfully but reopened by a later relevant frame before the
  quiet window ends;
- multiple fields and causes unioned into one epoch-scoped debt set without a
  duplicate snapshot or parallel query loop;
- query-window changes expanding a dependency closure without recursively
  creating another cycle;
- local-control quick observation followed by final post-five-second-horizon
  reconciliation;
- several local writes extending one final deadline from the most recent
  related write and producing one final settlement pass;
- continuous physical-remote activity publishing every latest value, limiting
  quick reconciliations to one per second, and settling after activity stops;
- five-second core sweep repairing a deliberately omitted physical-remote
  event;
- failed quick/final query leaving debt visible and field stale/degraded rather
  than falsely settled;
- disconnect suspending debt, reconnect replacing its epoch, and full
  synchronization creating a new cycle; and
- scheduler fairness under simultaneous user controls, final settlement,
  routine sweep, and continuous receiver frames, including exact eight-user-
  transmission and 500 ms debt-service limits.

### Multi-control and mixed-controller sequences

Desktop-only sequences include:

- explicitly named system-power `PWON` followed by source, volume, and mute,
  preserving its one-second quiet period and final dependency closure;
- Main Zone `ZMON` followed by source and volume, applying only the ordinary
  documented pacing unless live evidence requires another delay;
- source followed by sound-mode intent, where source selection first changes
  the receiver-selected detailed mode;
- rapid volume targets before dispatch, proving only the newest unsent target is
  written and every superseded request completes;
- a newer overlapping target after write start, proving the old command is not
  replayed and its outcome remains indeterminate;
- repeated identical intent joining one operation and producing one write;
- independent volume/mute changes followed by one merged settlement cycle; and
- Quick Select followed by an explicit source or volume override.

Physical-remote-only sequences include:

- rapid volume up/down bursts ending at each boundary;
- source change followed by receiver-selected sound-mode changes;
- mute toggles interspersed with volume frames;
- Main Zone power changes while system and Zone 2 differ;
- repeated same-value frames and a final changed value; and
- one missing event repaired by sweep and one delayed event reopening an
  already completed quick cycle.

Mixed desktop/physical-remote sequences include:

- remote change before desktop preflight, between preflight and write, during
  write, after write but before target observation, after target observation,
  during final reconciliation, and immediately after settlement;
- both surfaces selecting the same target and different targets;
- desktop volume followed by remote volume reversal;
- desktop source followed by remote sound-mode choice, and remote source
  followed by desktop sound-mode choice;
- desktop Main Zone power while the remote changes Zone 2;
- remote Quick Select effects while an independent desktop operation is queued;
  and
- event loss, delayed event, disconnect, or receiver switch at each sequence
  boundary.

Every core sequence asserts the exact wire order, operation histories, latest
canonical values, validity, convergence/settlement state, and dependency debt.
Its canonical trace is replayed by a separately compiled desktop-projection
test; the core scenario binary never imports GUI. No expectation may use a local
target as receiver state.

### Scheduling and transport timing

- exact 50 ms boundary accepted and 49 ms forbidden for query/query,
  query/control, control/query, and reconnect/synchronization writes;
- `PWON` followed by a command at 999 ms forbidden and at one second accepted;
- user operation priority over due reconciliation without starvation;
- one pending query only and one mutating operation only;
- cancellation while queued, waiting for pacing, before first byte, during
  write, and during observation;
- actor shutdown with queued query, active query, queued operation, active
  operation, and reconnect timer; and
- real-time watchdog proves a blocked subscriber or supplemental job cannot
  deadlock the actor.

### Control outcome matrix

Every controllable field and intent runs against these transitions:

| Boundary | Expected outcome |
|---|---|
| Invalid or unsupported before queue | `RejectedBeforeDispatch(NotSupported/Invalid)` |
| Disconnected and unable to preflight | `RejectedBeforeDispatch(NotConnected)` |
| Current targeted observation equals target | `AlreadyObserved` and no control write |
| Cached value equals target but is stale | Targeted preflight required; cached no-op forbidden |
| Cached value equals target but is converging | Targeted preflight required; broader debt remains after a matching observation |
| Identical intent already pending | Join the same `OperationId`; one control write at most |
| Newer overlapping intent still queued | Older request is `SupersededBeforeDispatch`; only newest target can be written |
| Newer overlapping intent after old write starts | Older request is `Indeterminate(SupersededByNewerIntent)` and is never replayed |
| Cancelled before first write attempt | `RejectedBeforeDispatch(Cancelled)` |
| Failure after write starts, before known completion | `Indeterminate(Unknown)` and no replay |
| Complete write, matching post-write observation | `ObservedRequestedValue(CompleteWrite)` |
| Ambiguous write, matching post-write observation | `ObservedRequestedValue(Unknown)` with ambiguity retained |
| Complete write, only nonmatching observations | `Indeterminate(CompleteWrite, last_observation)` |
| Complete write, no observation by horizon | `Indeterminate(CompleteWrite, Timeout)` |
| Epoch changes after dispatch | `Indeterminate`; new-epoch value cannot confirm old operation |
| Requester is dropped after dispatch | State still reduces; command is not replayed or rolled back |

This matrix covers Main Zone power, source, exact volume, mute, each sound-mode
intent, Zone 2 power, and explicitly exposed system power. Multi-field Quick
Select uses a separate declared predicate and the same delivery boundaries.

### Power scenarios

- `PW?` never populates Main Zone power and `ZM?` never populates system power;
- `ZMON`/`ZMOFF` are the only Main Zone power writes;
- `PWSTANDBY` is reachable only from explicitly named system-power intent;
- Main Zone off does not imply system standby or Zone 2 off;
- system standby with Network Control available; and
- connection loss in standby represented as unknown cause with Network Control
  guidance, not fabricated power state.

### Volume scenarios

- Min, -79.5 dB, every half-step mapping sample, 0.0 dB, +18.0 dB, compact
  whole-dB response, and `MV---` unavailable;
- below-minimum, above-maximum (`MV985`), non-half-step, malformed, overflow,
  NaN/infinity at presentation conversion, and negative-zero input;
- configured maximum lower than model maximum with no silent clamping;
- rapid knob burst, duplicates, reversal, and external change during local
  operation; and
- exact comparison without normalized-rounding no-ops.

### Source scenarios

- every model/region-backed source ID and documented response alias;
- current static-list mismatches such as unavailable legacy/AUX entries and
  missing SAT/CBL, Network, or Bluetooth where profile evidence supports them;
- renamed source, hidden source, duplicate display labels, empty/Unicode label,
  catalog unavailable, and catalog changing after reconnect;
- protocol-supported but runtime-unknown versus explicitly disabled source;
- arbitrary/unrecognized source rejected before dispatch; and
- source change invalidating only context-dependent supplemental resources.

### Sound-mode scenarios

- direct selection and Auto/Direct/Pure Direct/Stereo intents;
- MOVIE/MUSIC/GAME recall returning the old detailed mode first, returning the
  target near five seconds, falling back to another compatible mode, or never
  changing;
- input-signal/context change altering available modes;
- unknown observed status preserved as raw evidence but not made writable;
- local intent never persisted as receiver-observed category; and
- external mode change during the operation horizon.

### Supplemental-resource scenarios

- independent success, partial success, unsupported endpoint, malformed XML,
  HTTP timeout, connection refusal, and cancellation;
- stale completion after receiver switch, reconnect, source change, or
  sound-mode context change;
- single-flight coalescing under repeated triggers;
- blocked HTTP while TCP events and controls continue;
- failure preserving only that resource's last-good result; and
- supplemental completion not changing core state revision.

### Concurrency, pressure, and shutdown

- simultaneous subscribers with no subscriber able to block publication;
- rapid repeated same-field intents, conflicting fields, control during
  reconciliation, refresh during control, and receiver switch during either;
- operation one-shot receiver dropped before and after dispatch;
- telemetry queue closed/full while state and operation completion continue;
- state subscriber closed and recreated, immediately seeing latest state;
- shutdown idempotence, no reconnect after close, all waiters resolved, socket
  closed once, and actor join completed; and
- repeated scenario execution under a leak detector with stable task/socket
  counts.

## Property-based and model-based tests

A small reference model contains only the specified state machine, without TCP
or production reducer code. Generated actions include connect success/failure,
frame, time advance, query, control, cancel, disconnect, reconnect, subscriber
lag, receiver switch, supplemental start, and supplemental completion.

For every generated trace, assert:

- accepted observations never cross receiver or epoch;
- frame sequence and state revision never regress;
- last-good evidence is preserved through failures;
- at most one socket owner, query, and mutating operation exists;
- no two transmissions violate pacing or `PWON` quiet time;
- no control is written more than once;
- synchronization debt is monotonic by union within an epoch, clears only after
  successful observation plus quiet-window rules, and never crosses epochs;
- several related controls extend one final deadline and create one final
  settlement pass;
- operation progress, targets, and terminal outcomes cannot mutate canonical
  receiver values without a parsed receiver observation;
- `AlreadyObserved` requires a current targeted observation;
- `ObservedRequestedValue` requires a matching post-write observation;
- old-epoch and supplemental results cannot mutate current resources;
- subscriber lag changes only the number of intermediate revisions seen, not
  the latest state; and
- close is terminal until a new explicit session is created.

CI uses fixed seeds plus a bounded generated case count. Nightly/manual runs use
larger counts and randomized seeds. Every failure prints the seed and minimized
trace so it can be committed as a named regression scenario.

Protocol parsers additionally receive arbitrary byte strings with limits that
exercise fragmentation, frame boundaries, Unicode errors, numeric overflow, and
unknown families. They must never panic, allocate without a configured bound,
or produce an encodable command from invalid input.

## Trace replay and live evidence

Each committed capture consists of raw bytes plus metadata:

```text
model, product variant, region, firmware, Network Control setting,
active zones, source/signal context, relevant speaker/headphone setup,
capture date, read-only/state-changing classification, redactions
```

Replay runs the raw frames through production framing, parsing, and reduction
and compares canonical observations. A live capture does not automatically
become writable capability evidence; it needs protocol review and an explicit
expected fixture.

The opt-in live smoke connects without GUI or CLI through the application
service and verifies identity, initial synchronization, event reception, one
reconciliation cycle, and clean close. It is read-only. State-changing live
scenarios require the separate controls command, explicit receiver address,
confirmation flag, safe maximum volume, and recorded restoration steps.

The armed live sequence covers several application-service controls followed by
several physical-remote actions: rapid volume changes, mute, source, sound mode,
and Main Zone power where safe. It then interleaves both surfaces on the same
and different fields. The harness records frame-to-publication latency,
converging/settled transitions, final query values, operation histories, and
whether each SLO passed. No GUI or CLI is launched.

## Headless desktop correctness tests

Core scenarios emit reusable canonical state and operation traces. The desktop
projection suite feeds those traces directly to the presentation reducer and
asserts user-visible state without rendering a window.

It covers:

- receiver switch and old-identity/old-epoch event rejection;
- unknown, synchronizing, current-but-converging, current-and-settled, stale
  last-known, unavailable, degraded, reconnecting, and disconnected
  representations for every field;
- Main Zone versus system/Zone 2 labels and controls;
- exact volume Min/-79.5/+18.0 display and draft-versus-observed separation;
- source protocol ID, duplicate label, hidden, unavailable-catalog, and current
  source behavior;
- sound-mode intent versus observed detailed status;
- operation queued, dispatched, observed, rejected, indeterminate, cancelled,
  and superseded feedback;
- external receiver updates during a local volume draft or pending operation;
- merged sync summary, oldest debt, last successful settlement, and manual
  Refresh adding canonical debt rather than starting a second state path;
- several desktop operations followed by physical-remote overrides, with
  historical outcomes remaining separate from current status;
- supplemental loading/stale/error state independent from core readiness;
- repeated events, out-of-order supplemental results, closed subscriptions,
  and app shutdown; and
- absence of synthetic request-ID filtering or optimistic values presented as
  receiver observations.

Small widget-mapping tests ensure each user intent creates the correct domain
intent, but visual baselines remain presentation checks rather than core
correctness evidence.

## Coverage ledger and quality gates

A checked-in ledger maps every overview acceptance criterion and architecture
invariant to at least one smoke, scenario, property, or live-validation ID. CI
fails for missing IDs, duplicate scenario names, ignored required scenarios, or
fixtures without provenance metadata.

Completion requires:

- all smoke and deterministic scenario tests pass repeatedly with no retries;
- the property suite passes fixed CI seeds and its configured generated count;
- every fault boundary has an asserted operation outcome and no-replay check;
- every supported command/response grammar has positive, boundary, unavailable,
  and malformed cases;
- every state-machine edge has a named scenario or generated transition;
- every dependency closure, convergence SLO, and desktop/physical-remote
  ordering boundary has named or generated coverage;
- every desktop presentation state has a headless projection test;
- required suites contain no arbitrary sleeps, external network calls, GUI/CLI
  dependency, native-window startup, or shared fixed port;
- all tests are parallel-safe and use isolated ephemeral resources;
- test failure artifacts are sufficient to replay failures locally; and
- targeted mutation testing of the reducer, parser, pacing scheduler, epoch
  checks, and operation machine leaves no surviving behavioral mutant without
  explicit review and a regression test.

Line coverage may be reported for discovery, but no percentage substitutes for
the behavior ledger, fault matrix, and invariant checks.
