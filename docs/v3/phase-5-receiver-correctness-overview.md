# Version 3, Phase 5 — Receiver monitoring and control correctness

**Complete — 2026-09-11.** The canonical domain/session vocabulary,
X3800H wire profile, CLI composition, reconciliation scheduling, deterministic
fault harnesses, desktop cutover, and live AVC-X3800H validation are implemented.
This phase may break every receiver-state and control API. Its target is
truthful, deterministic monitoring and control of AVC-X3800H and AVR-X3800H,
not compatibility with abstractions that currently overstate correctness.

The [current-behavior supplement](phase-5-receiver-correctness-supplement.md)
records the code and protocol research behind this plan. The companion
[architecture](phase-5-receiver-correctness-architecture.md) is the normative
technical design, and the
[headless test plan](phase-5-receiver-correctness-test-plan.md) defines its
executable evidence.

## Outcome

Phase 5 will establish one long-lived receiver session as the source of truth
for both observation and control. It will model Denon protocol semantics
directly, preserve last-good evidence through faults, bind every observation to
a receiver and connection epoch, and report only what can actually be proved.

The defining user-visible changes are:

- Main Zone power controls Main Zone with `ZM`; it no longer aliases system
  power `PW`.
- Receiver state remains visible but is explicitly stale while disconnected or
  after failed reconciliation.
- A control succeeds only when the requested value is observed after dispatch.
  Ambiguous delivery or an expired observation window is reported as
  indeterminate, never as a definite failure or acknowledgement.
- Volume is represented in exact dB half-steps for X3800H, not a normalized
  0–100 value.
- Source and sound-mode choices reflect protocol identity and receiver context
  rather than unconditional static allowlists.
- Desktop and CLI use the same session and application semantics.
- A delivery-independent smoke suite verifies the whole underlying stack in
  seconds, while deterministic scenario and property suites exercise every
  specified transition and fault boundary without starting GUI or CLI.
- The desktop becomes a thin, truthful projection of canonical state: it no
  longer manufactures state causality, hides stale evidence, or presents local
  control intent as an observed receiver result.

## Why replacement is necessary

The existing design has local safety mechanisms—serialized writes, bounded
queues, periodic refresh, generations, and execute-once controls—but their
composition is contradictory. Infrastructure and application maintain
different snapshots; the queue between them may drop the only public copy of
an update. Aggregate freshness hides per-field age. GUI request IDs are attached
to unrelated receiver events. A process-local mutation counter is exposed as
if it were receiver-side optimistic concurrency.

There are also direct protocol mismatches. The Main Zone power abstraction uses
Denon's system-power family, ordinary transmissions are not globally spaced by
the documented minimum, the control observation window is shorter than the
documented event allowance, and the volume type admits a value above the
X3800H's documented maximum.

These faults share one cause: wire observations, domain state, operation
progress, and presentation bookkeeping do not have explicit identities and
lifetimes. Phase 5 replaces those contracts together.

## Status synchronization contract

Phase 5 optimizes for convergence after activity from either control surface:

```text
desktop controls ----+
                     +--> canonical receiver frames --> desktop state
physical remote -----+              |
                                    v
                         merged synchronization debt
                                    |
                     quick targeted + final reconciliation
```

- Every receiver frame updates canonical state before command replies,
  debounce, or presentation work.
- Every local write and changed receiver event opens or extends one
  epoch-scoped synchronization cycle for the affected field and its documented
  dependencies.
- Repeated controls and remote-event bursts merge debt. They do not start
  competing full refreshes or create a snapshot per control.
- The desktop displays the latest receiver observation immediately and marks
  affected fields `Converging` until a quiet targeted reconciliation succeeds.
- Local-control debt receives a final post-event-horizon query at least five
  seconds after the last related write. This catches delayed status lines while
  allowing operation feedback to complete earlier when the target is observed.
- Physical-remote events trigger a 250 ms quiet-window targeted reconciliation.
  Late events reopen the cycle. A five-second periodic core sweep bounds
  recovery when an event is lost.
- Operation outcomes are historical evidence. A later physical-remote action
  always wins current desktop status without making the earlier outcome false
  or suppressing the later observation.

The detailed debt algorithm, dependency closure, scheduling fairness, and SLOs
are normative in the
[architecture](phase-5-receiver-correctness-architecture.md#synchronization-debt-and-dependency-closure).

## Scope

### In scope

- Exact AVC-X3800H and AVR-X3800H Main Zone semantics for system power, zone
  power, source, master volume, mute, and sound-mode status/intents.
- Existing Zone 2 power behavior, moved onto the same observation model.
- One asynchronous TCP session actor with protocol pacing, response windows,
  reconnect policy, state reduction, and operation tracking.
- Per-field last-good evidence and validity, plus receiver identity, connection
  epoch, and monotonic frame/state sequence.
- Initial synchronization, unsolicited-event handling, targeted reads, periodic
  reconciliation, and bounded reconnect with indefinite retry until explicitly
  stopped.
- Activity-aware synchronization debt, dependency-closure reads, rapid
  quiet-window reconciliation, and a post-control final settlement pass.
- Targeted control preflight and post-dispatch observation with honest delivery
  certainty.
- Independent freshness and execution for HTTP information, source catalog,
  Quick Select metadata, EQ evidence, and other supplemental resources.
- Migration of desktop, GUI library, CLI, and read-only diagnostics to the new
  contracts while preserving repository dependency direction.
- Deterministic protocol simulations and a recorded live-AVC-X3800H validation
  matrix.
- A strict virtual X3800H, deterministic clock, byte-level fault injection,
  trace replay, property/state-machine testing, and a checked requirement-to-
  scenario coverage ledger.
- Fast core smoke tests and comprehensive core scenario tests that import no
  GUI or CLI package and require no physical receiver.
- Headless desktop projection and interaction-state tests that launch neither
  an Iced window nor a delivery executable.
- Desktop lifecycle, receiver-switch, stale-state, operation-feedback, volume
  draft, supplemental-state, and shutdown correctness.

### Out of scope

- Broad Denon or Marantz model support without equivalent model-specific
  evidence.
- HEOS playback, browsing, or media-queue redesign.
- Undocumented HTTP control writes as a fallback for TCP port 23.
- A multi-process socket broker or arbitration with other control applications.
- Claiming that an observed target value proves this client caused it.
- Automatically changing the receiver's Network Control setting.
- Preserving current public Rust APIs, serialized state shapes, GUI bridge
  messages, CLI concurrency flags, or normalized volume syntax.

## Breaking contract changes

| Current contract | Phase 5 contract |
|---|---|
| `MainZoneField::Power` / `PowerState` maps to `PW` | Separate `SystemPower` (`PW`) and `ZonePower` (`ZM`, `Z2`) |
| `MainZoneSnapshot` mixes core and supplemental state | `ReceiverState` contains core observations; supplemental resources have independent state |
| One aggregate `freshness` and `authority` | Each field has last-good observation, validity, convergence/settlement, error, epoch, sequence, origin, and time |
| `VolumeLevel(0..=1000)` normalization | Exact `VolumeDb` in 0.5 dB steps, plus distinct minimum and unavailable status |
| Arbitrary `Input` plus static model list | Protocol `SourceId` separated from receiver label/visibility and effective catalog |
| Sound-mode category stored as receiver state | `SoundModeIntent` is operation input; `SoundModeStatus` is receiver observation |
| `resource_version` / `expected_version` | Removed from control admission; internal state revision is display synchronization only |
| `Confirmed`, `TransportFailure`, `Unconfirmed` | Evidence-based outcome with `AlreadyObserved`, `ObservedRequestedValue`, `RejectedBeforeDispatch`, `SupersededBeforeDispatch`, or `Indeterminate` |
| GUI request ID attached to snapshots/events | Independent `OperationId`; state uses `ReceiverId`, `Epoch`, and state revision |
| GUI bridge owns command/event forwarding and stale filtering | Desktop subscribes to latest canonical state and reduces operation feedback separately |
| Separate `SyncAvrClient` behavior | CLI runs the same async receiver service in a short-lived runtime |

Configuration file compatibility may be retained where it does not weaken the
new semantics, but it is not a phase requirement. A migration note must call out
every user-facing CLI or configuration change before implementation is marked
complete.

## Work plan

### 1. Freeze evidence and executable protocol fixtures

- Record model, region, firmware, Network Control setting, command, response,
  timing, and date for an AVC-X3800H test receiver.
- Capture `PW?` versus `ZM?`, Main Zone off with another zone active, source
  identifiers, volume endpoints, mute, sound-mode transition ordering, standby
  reconnect, and event latency.
- Convert approved captures into redacted, deterministic fixtures. Do not make
  live hardware part of ordinary tests.
- Resolve any disagreement between the mirrored model chart and current
  firmware before encoding a writable capability.

Exit: every Phase 5 write command has either exact model documentation or a
reviewed live validation record, preferably both.

### 2. Replace the domain vocabulary

- Introduce receiver/epoch/sequence identity and per-field observation state.
- Separate system power from zone power and exact status from operation intent.
- Replace normalized volume, source, sound-mode, aggregate freshness, aggregate
  authority, and public resource-version contracts.
- Split core state from supplemental resource state.

Exit: domain tests express stale preservation, epoch rejection, exact volume,
and distinct power scopes without runtime or protocol dependencies.

### 3. Build the X3800H protocol profile

- Define strict query, control, and response/event grammars for supported
  families.
- Encode model limits, aliases, unavailable values, and context-sensitive
  operations explicitly.
- Keep raw unknown frames as diagnostics without allowing them to mutate typed
  state.
- Add parser and encoder round-trip, boundary, and malformed-input tests.

Exit: the protocol layer cannot encode `MV985` or map Main Zone power to `PW`.

### 4. Replace transport and state ownership

- Build one actor that exclusively owns the TCP reader/writer, pacing clock,
  frame sequence, connection epoch, and canonical state reducer.
- Publish full current state through a coalescing watch channel and operation
  progress through a separate bounded stream.
- Remove the infrastructure snapshot/application snapshot split and silent
  delta loss.
- Implement indefinite bounded-exponential reconnect with jitter and explicit
  shutdown.
- Track synchronization debt in the canonical actor and publish per-field
  `Converging`/`Settled` metadata without creating another state copy.

Exit: burst, reconnect, timeout, and slow-consumer simulations converge on the
same latest state without mixing epochs or blocking the socket reader.

### 5. Establish the headless correctness laboratory

- Add a strict loopback virtual X3800H and a separate scripted byte-transport
  seam for faults that real TCP cannot reproduce deterministically.
- Inject a manually advanced monotonic clock into pacing, query, confirmation,
  validity, reconciliation, and reconnect scheduling.
- Add public-API core smoke tests, the complete named scenario catalog,
  property/state-machine generation, raw trace replay, and rich failure
  artifacts.
- Add `make test-core-smoke`, `make test-core-scenarios`, and
  `make test-core-properties`; include all deterministic suites in `make check`.
- Enforce that these suites cannot import GUI, CLI, desktop, or diagnostics.

Exit: the test harness can independently demonstrate pacing, epoch isolation,
state convergence, ambiguous delivery, no replay, no backpressure deadlock, and
clean shutdown. The complete matrix is defined in the
[headless test plan](phase-5-receiver-correctness-test-plan.md).

### 6. Replace control transactions

- Query only the target field for preflight; use cached `AlreadyObserved` only
  when that field is demonstrably current, settled, and within the strict
  preflight age.
- Enforce global 50 ms transmission spacing and the one-second `PWON` quiet
  period in the socket owner.
- Never automatically replay a control after ambiguous delivery.
- Observe target-field events first and perform paced targeted reconciliation
  through a window that includes Denon's five-second event allowance.
- Preserve intermediate nonmatching sound-mode observations instead of ending
  confirmation early.
- Coalesce superseded, not-yet-written overlapping intents; never erase or
  replay a write that already began.
- Merge every dispatched control's affected/dependent fields into a shared final
  settlement cycle so several controls produce one coherent reconciliation.

Exit: every terminal operation result states dispatch certainty and supporting
observation without claiming protocol acknowledgement or causal proof.

### 7. Decouple supplemental resources and migrate delivery composition

- Run CLI commands through the same async application service and remove the
  synchronous protocol/control duplicate.
- Move HTTP information, source catalog, Quick Select names, and EQ evidence to
  independent single-flight jobs. Reject old-epoch or obsolete-context results.
- Keep diagnostics read-only and prevent them from opening a second connection
  when the app owns the receiver session.

Exit: supplemental jobs are isolated from core TCP work, CLI uses the shared
async service, and no synchronous production control path remains.

### 8. Rebuild desktop behavior as a truthful projection

- Remove the serialized bridge's synthetic request/generation tagging and all
  presentation-owned filtering of receiver state.
- Subscribe once to canonical core state and independently to supplemental
  resources; reduce only receiver identity, epoch, and revision supplied by
  their owners.
- Separate observed values, editable drafts, pending operations, and operation
  outcomes. Never render a local draft or dispatched command as observed state.
- Define exact behavior for receiver switch, reconnect, stale/unavailable
  fields, external changes during local interaction, duplicate controls,
  cancelled tasks, and shutdown.
- Add headless desktop projection and operation-scenario tests driven by the
  same canonical traces as the core suite. No native window is required.

Exit: all desktop states and interactions in the architecture have deterministic
headless tests, and the desktop executable contains composition/bootstrap only.

### 9. Validate, document, and cut over

- Run formatting, boundaries, compile, Clippy, smoke, deterministic scenarios,
  generated properties, trace replay, desktop-model tests, targeted mutation
  testing, and the approved live-receiver matrix.
- Run an explicitly armed headless live sequence combining several application
  controls with several physical-remote controls and record convergence/SLO
  evidence; never launch GUI or CLI for this gate.
- Document Network Control prerequisites, single-client port contention, stale
  state, indeterminate outcomes, source labels, and new volume syntax.
- Remove old APIs and adapters in the same cutover; do not maintain two state
  machines behind compatibility wrappers.
- Update active user guides and the Version 3 changelog only after behavior is
  implemented and validated.

Exit: all acceptance criteria below pass and no production delivery path uses
the legacy contracts.

## Acceptance criteria

### Monitoring

- Main Zone power is read from `ZM?`; system power and Zone 2 cannot overwrite
  it.
- Initial synchronization finishes only after every required field is current,
  explicitly unavailable, or failed with preserved evidence. Any local failure
  yields `Degraded`, not `Ready`.
- Each accepted observation belongs to the selected receiver and current
  connection epoch; delayed old-epoch work cannot mutate current state.
- A disconnect preserves last-good values as stale, and a reconnect continues
  until shutdown without requiring the user to recreate the controller.
- An unsolicited burst larger than every notification queue still leaves all
  consumers able to read the latest full canonical state.
- Periodic reconciliation repairs a deliberately lost simulated event.
- A changed receiver event publishes immediately, marks its dependency closure
  `Converging`, and reaches `Settled` only after a quiet targeted cycle.
- Several desktop controls merge into one final post-horizon reconciliation;
  they do not trigger one full refresh per control.
- Physical-remote and desktop controls interleaved on the same field never
  suppress each other's receiver frames. The latest frame remains visible while
  the field is converging.
- With a healthy responsive receiver, a lost remote event is repaired by the
  five-second core sweep plus its bounded query duration.
- A stalled supplemental HTTP endpoint does not delay TCP reads, controls, core
  readiness, or state publication.

### Control

- No ordinary pair of writes is transmitted less than 50 ms apart, and no
  command follows `PWON` for at least one second.
- A stale or converging cached target never causes `AlreadyObserved`; a targeted
  preflight observation is required.
- Disconnect before write is `RejectedBeforeDispatch`; disconnect during or
  after write is `Indeterminate` and the control is not replayed.
- A target event near five seconds can complete successfully; an old `MS` value
  followed by the target value does not fail early.
- Confirmation reads only the affected field unless the operation's documented
  semantics affect multiple fields, such as Quick Select.
- The result exposes the observation that matched the requested value and does
  not describe a same-family line as an acknowledgement.
- An identical pending intent joins existing work; a newer unsent overlapping
  target supersedes the older one without writing it; a newer target after
  dispatch leaves the older outcome explicitly indeterminate and never replays
  it.

### Model semantics and delivery parity

- X3800H volume accepts minimum or half-dB values from -79.5 through +18.0 dB
  and rejects values outside the documented model range before dispatch.
- Source choices preserve protocol ID, display label, and visibility separately;
  controls are restricted to the effective observed/model-backed catalog.
- Sound-mode recall intent is not persisted as receiver-observed category.
- Desktop and CLI produce equivalent state and outcomes from an identical
  fixture trace.
- A second-client refusal produces actionable port-contention guidance and
  never causes an undocumented HTTP write fallback.

### Headless core verification

- `make test-core-smoke` composes domain, application, protocol, and
  infrastructure through public APIs and completes without GUI, CLI, a LAN, or
  a physical receiver.
- `make test-core-scenarios` covers every monitoring state, supported protocol
  family, control outcome, timing boundary, disconnect point, queue-pressure
  case, receiver switch, cancellation point, supplemental-resource transition,
  and shutdown path in the checked scenario ledger.
- The harness uses logical time rather than arbitrary sleeps and has a real-time
  watchdog only for deadlock detection.
- Property failures report reproducible seeds and minimized traces; scenario
  failures report wire bytes, logical time, epochs, state revisions, and
  operation transitions.
- Core tests have a mechanically enforced prohibition on dependencies or
  imports from GUI and delivery packages.
- Every stale-epoch result, partial-write boundary, and completion outcome has
  an explicit no-replay assertion.

### Desktop correctness

- Receiver events are never discarded because of a GUI request ID and cannot
  be accepted for the wrong receiver or epoch.
- The UI distinguishes unknown, synchronizing, current-but-converging,
  current-and-settled, stale last-known, receiver-unavailable, degraded,
  reconnecting, and disconnected states.
- Observed receiver value, local edit/draft, pending operation, and terminal
  outcome are separate presentation states for every control.
- External changes remain visible during local interaction; a volume draft is
  never labeled as observed and rebases after operation completion.
- The desktop shows merged synchronization progress and last settlement from
  canonical metadata; its manual Refresh action adds core debt rather than
  starting a second query/state path.
- A historical “requested value observed” result never replaces a newer
  physical-remote value or claims that the displayed value is still current.
- Main Zone controls issue only Main Zone intents; system and Zone 2 scope are
  unmistakable in labels, availability, and feedback.
- Slow or failed supplemental resources do not change core readiness or block
  controls, event updates, receiver switching, or shutdown.
- Headless projection tests cover all canonical traces without starting a
  native window, GUI executable, CLI, or real receiver.

### Engineering gates

- `make format-check`, `make boundary`, `make check`, `make clippy`, and
  `git diff --check` pass.
- `make check` includes all deterministic Phase 5 smoke, scenario, property,
  trace-replay, and desktop-model suites; opt-in live tests remain separate.
- Domain and protocol remain runtime-free; application depends only on domain;
  infrastructure owns concrete TCP/HTTP; GUI library imports neither protocol
  nor infrastructure.
- No legacy synchronous AVR control client, duplicate core snapshot, public
  receiver CAS token, or synthetic state/request correlation remains.
- The live validation record includes model, firmware, region, settings,
  commands, responses, timing, and date.

## Release rule

Phase 5 is complete only after the new design is the sole production path, the
headless correctness ledger is complete and green, desktop projection scenarios
pass, and the live AVC-X3800H matrix passes. Documentation or isolated unit tests
alone do not make the phase implemented. Any protocol behavior that remains
unverified must be read-only, disabled, or labeled experimental rather than
inferred from a nearby model.
