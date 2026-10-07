# Version 4, Phase 2 — GUI on the port architecture

This document fixes what the GUI must keep, change, or drop when it moves from
the legacy controller to the control-service port. It does not restate the
design; where a rule is owned elsewhere it is linked. Text that is implemented
moves into [ARCHITECTURE.md](../../ARCHITECTURE.md) when the milestone's exit
criteria pass.

Step 1 of the [overview](phase-2-gui-on-port-overview.md) wrote this document
and changed no code. Everything below was read from the code on 2026-10-08 at
the head of main (`5ef9b68`), except the items listed under
[Live checks](#live-checks), which only the receiver can settle.

## Three layers, not one

In 3.0.0 the GUI's behavior comes from three layers stacked, and reading only
the controller misleads:

```text
gui-lib bridge  ->  ReceiverController      (application/controller.rs)
                ->  CanonicalSessionAdapter (infrastructure/canonical_factory.rs)
                ->  X3800hSession           (infrastructure/x3800h_session.rs)
                ->  AvrSession transport    (infrastructure/avr_session.rs)
```

The adapter turns controller calls into canonical ones and canonical state back
into legacy events. Two consequences shape the table:

- **Controls were confirmed twice.** The adapter's `execute_once` awaits
  `X3800hSession::operate`, which dispatches and then re-queries the target for
  up to 5 s (rechecking every 250 ms). The controller then ran its own targeted
  query under a 2 s deadline and compared again.
- **Outcomes were collapsed.** The adapter returned `Ok` only for
  `ObservedRequestedValue` and `AlreadyObserved`. Every other outcome
  (`RejectedBeforeDispatch`, `Indeterminate`, `Cancelled`,
  `SupersededBeforeDispatch`) became `Err`, which the controller reported as
  `TransportFailure` ("Command failed to send"), formatted from the outcome's
  `Debug` text. `Unconfirmed` was reachable only from the controller's own
  second check.

## Behavior parity table

Mechanism names the layer that produces the 3.0.0 behavior. Disposition is one
of:

- **Kept**: the canonical session or service already provides it.
- **Moves**: the behavior stays, in presentation or the bridge.
- **Replaced**: a different mechanism gives the same user-visible result.
- **Changed**: the user can observe a difference. Owner decision or acceptance
  is named.
- **Dropped**: the behavior goes away. A decision is named when it is not dead
  code.

### Connection and lifecycle

| ID | Behavior in 3.0.0 (mechanism) | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| L1 | `Select` closes the previous session first and carries on if the close fails; emits `Selected` (controller) | The GUI drops its `StateSubscription` for the old receiver. The service releases the session after the idle time and logs a failed close without blocking any other receiver (`release_if_idle`) | Replaced. The old receiver stays connected for up to 60 s, not at once. With one receiver this is invisible |
| L2 | `Connect` then `Refresh`; `Connecting` is emitted before the attempt; a failed connect emits `Disconnected`, invalidates every field and returns the error (controller) | `ReceiverReads::state` connects and synchronizes before it resolves, and fails with `ControlError::Unavailable`. The GUI shows `Connecting` while the call is pending and `Disconnected` with the message when it fails | Moves. `Connecting` and `Disconnected` become presentation state |
| L3 | The launch screen leaves when `Lifecycle::Connected` arrives, which is right after the socket opens and before any status query (`complete_launch_if_ready`) | `state` resolves only after two full synchronization passes: the session's actor runs one at startup, and the service's `connect` then calls `synchronize`, which queues behind it. Each pass queries the seven core fields in sequence | Changed. That is 14 paced queries, at least 0.7 s at the 50 ms transmission interval plus round trips, before the dashboard opens. Dropping one pass is a possible step 2 fix. Confirm live (L3 below) |
| L4 | Reconnect: the transport reconnects forever. The adapter maps an epoch change to `Reconnecting` then `Connected`; the controller bumps its generation, invalidates every field and supplemental read, and on `Connected` refreshes the core fields, then HTTP information, then Quick Select names (adapter, controller) | The session marks every field `Stale { Disconnected }`, clears `epoch`, and installs the next epoch on reconnect, keeping `last_good`. It requests no resynchronization at that point; the 5 s sweep restores the fields | Closed: the session reads every core field again as soon as it reconnects (owner's decision, 2026-10-08), so every client recovers at once and the GUI asks for nothing. A reconnect that happens during an operation's confirmation is covered by the next sweep. The forced-reconnect check is exit criterion 3 |
| L5 | The `Reconnecting` lifecycle is visible to the GUI | Visible through the port as state: `epoch` is `None` while disconnected and a higher epoch afterwards. `watch` coalesces, so a disconnect and reconnect can arrive as one epoch-to-epoch step; the projection treats any epoch increase as a reconnect, as the adapter did (`previous_epoch != current_epoch`) | Moves. No port change; the projection rule and a test are required |
| L6 | The generation (connection counter) tags events and supplemental reads, and the GUI drops events older than the one it holds (bridge, `lib.rs` `Message::Bridge`) | `Epoch` on state and `epoch - 1` on inspection reads (`X3800hSession::generation`). `DesktopProjection::accept_state` already refuses an older epoch or a revision that is not newer | Replaced. The bridge's request-id and generation tagging goes; GUI request ids stay for UI timers only |
| L7 | Terminal `Disconnected` after a failed connect or session error, and a `Retry Status` button that re-runs `Refresh`, which reconnects (controller `refresh` connects when there is no session) | A failed `state` call is the disconnected condition. Retry calls `state` again when there is no subscription, and `refresh` ([P1](#port-changes)) when there is one | Moves, with P1 |
| L8 | `Connect`, `Disconnect`, and `Shutdown` messages exist | `Connect` and `Disconnect` have no emitter and are gone. `Shutdown` had none either (the desktop never sent it; exit closed the socket through the operating system). On the owner's instruction closing the window now closes the receiver connection first: the window's close request starts the shutdown hook the composition root supplies, bounded by a three-second grace period, then closes the window. A second request closes at once, and Cmd-W takes the same path | Dropped for `Connect` and `Disconnect`. Changed for `Shutdown` (owner's decision): the receiver's single control connection is free the moment the window closes |
| L9 | `Stopping` and `Stopped` lifecycle states | Rendered as "waiting" by `lifecycle_label`'s catch-all | Dropped |
| L10 | Close deadline 1 s (`ControllerConfig::close_timeout`) | `X3800hSession::close` bounds the close at 250 ms and then aborts the actor; the service logs a failed close | Replaced |

### State

| ID | Behavior in 3.0.0 | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| S1 | Five Main Zone fields as `FieldStatus<T>`, plus Zone 2 power separately (`MainZoneSnapshot`, `Zone2Snapshot`) | `ReceiverState`: seven fields, each with `last_good`, `validity`, `synchronization`, and `last_issue`. The GUI ignores `system_power` in this milestone | Moves. The projection builds what each view needs from `ReceiverState` |
| S2 | A value is usable only when current: the adapter filtered `last_good` by `is_current` | Projection rule, stated once: a field is usable for display and for enabling controls only when its validity is `Current`. A stale value may be shown as stale, never used as a baseline. An observation is valid for 10 s (`reduce_line`) and the session re-reads every field every 5 s, so a healthy receiver never shows `Stale { Expired }`; a stalled one does, which is accurate | Moves |
| S3 | "Not queried" marks a field that has not been read, and drives the 5 s status wait (`status_waiting`) | `FieldValidity::Unknown` with `FieldSynchronization::NotStarted` | Replaced |
| S4 | `resource_version` orders snapshots and is the optimistic concurrency token | `StateRevision` with `Epoch`. It also changes on diagnostics and synchronization marks, so it is not usable as a concurrency token (see C6) | Replaced |
| S5 | Partial status: a failed field query emits `FieldError` and the rest of the refresh continues; the GUI announces "{field} unavailable: {message}" for each event | `FieldValidity::Unavailable` or `Stale { QueryFailed }` with `last_issue`, per field in the same state. The session sweeps every 5 s, so announcing on every state would repeat the message | Replaced. Announce a field only when it enters a failed condition, and again only after it recovered |
| S6 | Unsolicited updates drained every 25 ms, at most 32 per pass, so commands and refreshes are not starved; a snapshot is published only when the version changed, because a full bounded event channel once deadlocked a control that was waiting for confirmation (controller) | The session actor handles receiver lines inline and publishes through a `watch` channel. A slow reader sees the newest state and cannot block the socket owner | Replaced. The deadlock cannot occur. The projection ignores a state whose revision is not newer |
| S7 | Periodic reconcile every 15 s: five fields, Zone 2, HTTP information (controller) | The session sweeps all seven core fields every 5 s and reconciles quickly after unsolicited events and after operations (`SyncDebt`) | Replaced for core state. HTTP information has no push: the bridge keeps a 15 s timer (see R1) |
| S8 | Zone 2: read only when the model supports it; refreshed with the core fields; unsolicited Zone 2 changes were never delivered, because the adapter's `changed_value` covers only the five Main Zone fields | `zone2_power` is a core field in `ReceiverState`, so an unsolicited change appears at once | Changed (accept). A change made on the receiver now shows without waiting for the next refresh |
| S9 | The status snapshot carries `audio_context` and a `StateAuthority` | `audio_context` is always the default (`query_audio_context` returns it) and is not shown. Canonical observations carry an origin but there is no snapshot-level authority | Dropped. The Diagnostics "Snapshot authority" row goes (see D4) |

### Controls

| ID | Behavior in 3.0.0 | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| C1 | Admission before dispatch: capability check, no-op when the value is already observed, conflict check (`admit_main_zone_control`) | `OperationSubmission` to the gate. The session's `admit_intent` admits sources and sound modes. The session does not gate Zone 2 by model, and neither did the adapter, so the GUI keeps checking `ModelCapabilities` before it offers the control | Moves for capability checks; Replaced for the rest |
| C2 | Already at the target: `NoOp`, "Receiver already has the requested value", decided from the controller's local snapshot without touching the receiver | `AlreadyInState`, decided by a preflight query of the target field | Replaced. One extra read, and the answer is authoritative |
| C3 | Dispatch is `execute_once`, never retried | The gate calls `operate` once on a task the service owns; shutdown waits for it | Kept |
| C4 | After a power-on the controller waits `power_on_quiet_time` (1 s) before its own confirmation query | There is no controller query to protect any more. The transport's own 1 s quiet period applies only to the `PWON` command (`avr_session.rs`), and the GUI's power control encodes as `ZMON`, so the canonical path adds no pause after a main-zone power-on beyond `transmission_interval` pacing | Dropped, with a live check (C4 below). If the receiver needs the pause after `ZMON`, extend the transport rule in step 2; do not add it to the GUI |
| C5 | Confirmation: a targeted query within `confirmation_timeout` (2 s) after the adapter's own 5 s confirmation | One confirmation by the session: 5 s window, 250 ms rechecks, and the gate maps the outcome to `status`, `dispatch`, and `confirmed` | Replaced. One confirmation, one deadline |
| C6 | Optimistic concurrency: the GUI sends `resource_version` with every control except power, and the controller returns `Conflict` ("Receiver state changed before the command; retry") when it differs | Operator operations carry no precondition (roadmap, milestone 1 step 6). A projection `revision` cannot stand in, because it also moves on diagnostics and synchronization marks | Changed. Owner decision D2 |
| C7 | Outcome wording: `Confirmed`, `NoOp`, `Conflict`, `Unsupported`, `Rejected`, `TransportFailure`, `Unconfirmed`, `Cancelled` (`feedback::control_message`) | `OperationSnapshot.status` and `dispatch`. See the [outcome mapping](#outcome-mapping) | Changed (accept, D5). A write that went out but was not confirmed no longer reads "failed to send" |
| C8 | Sound mode, three controls. `SelectSoundMode { category, mode }` reached the receiver as the mode only. `RecallSoundModeCategory` always dispatched and counted as confirmed only if the reported mode changed. After confirmation the controller paired the category with the reported mode (`confirm_sound_mode_category`), because `MS?` reports a mode and no category | `SoundModeIntent::Select(mode)` and `RecallMovie/Music/Game/PureDirect`. A category recall is `AlreadyObserved` when the current mode is already in that category, and confirmed as soon as the reported mode is in it. The category pairing becomes presentation state: set from the submitted control when the operation is `Completed` or `AlreadyInState`, cleared when the reported mode changes or the epoch changes | Moves for the pairing. Changed for recall: pressing a category the receiver is already in no longer writes. Owner decision D3 |
| C9 | Input select validates against `ModelCapabilities`; the controller refreshed HTTP information when an input control confirmed | `ReceiverIntent::Source(SourceId)`; the session rejects an unsupported source with `UnsupportedIntent`, and re-reads sound mode after a source change | Kept. The HTTP trigger is R1 |
| C10 | Volume slider −80.0 to +18.5 dB, in legacy `VolumeLevel` native units. The adapter converted to and from the canonical level (`volume`, `master_volume`) | `MasterVolume::{Minimum, DbHalfSteps}`, −79.5 to +18.0 dB. The slider's −80.0 maps to `Minimum`. In 3.0.0 a +18.5 request was dispatched as +18.0 and then reported unconfirmed, because the controller compared native code 985 with the reported 980 | Changed (owner's decision): the slider and its label now stop at +18.0 dB, the receiver's maximum, so the top step is real and confirms. The conversions move into the projection and are unit-tested at −80.0, −79.5, 0, and +18.0, and a level above +18.0 clamps. The volume row moves, so the screens that show it need new baselines (D4) |
| C11 | The first volume press with no known level starts from the safe minimum (`volume_baseline_initialized`) | Unchanged presentation logic; the baseline comes from the current volume field | Kept |
| C12 | The sound-mode request has a 3 s UI timeout (`SOUND_MODE_CONTROL_TIMEOUT`) and a wait overlay | A GUI timer, independent of the session's 5 s window | Kept, in presentation |
| C13 | Zone 2 power control: unsupported off the X3800H, a no-op when equal, confirmation by re-querying Zone 2 within 2 s | `ReceiverIntent::Zone2Power` through the gate; one confirmation by the session | Replaced |
| C14 | A control connects first if there is no session (`control`) | `submit` leases the receiver, which connects and synchronizes on demand | Kept |

### Supplemental reads

| ID | Behavior in 3.0.0 | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| R1 | HTTP information is read only when the model supports it and the receiver is a saved one, and only while main-zone power is on (otherwise invalidated). It is refreshed after `Refresh` replies, after a reconnect, on a 15 s tick, when input, sound mode, or power-on changes arrive unsolicited, and after a control for which `may_have_changed` holds (`http_information` helpers, controller) | `OperatorAdmin::http_information`, a one-shot read stamped with the epoch. The helper functions are pure and stay in `application` (`should_read`, `may_have_changed`, `merge_refresh`); the bridge calls them. The triggers become state transitions in the projection plus the 15 s timer | Moves. No port change. "Saved" becomes "the receiver id is not ad hoc" |
| R2 | Source catalog: read after the first usable snapshot when the freshness is unknown or invalidated, and when the source picker opens; gated by `source_catalog_read` | `ReceiverReads::source_catalog`, with the same triggers in the bridge. `ModelCapabilities::for_model` already sets `source_catalog_read` for the X3800H, and the desktop passes no validation override, so `ControllerConfig::validated_source_catalog` has no effect today | Moves. The `validated_*` plumbing is dropped |
| R3 | Quick Select names: read after `Refresh` replies and after a reconnect | `OperatorAdmin::quick_select_names`, same triggers | Moves |
| R4 | Quick Select recall and EQ status | Removed (roadmap decision D1) | Dropped. The recall button is never rendered (`quick_select_recall` is false and never validated), but the Diagnostics screen shows the EQ summary and evidence lines, so the removal is visible. See D4 |
| R5 | Invalidating the catalog, names, and HTTP information with every disconnect or reconnect, tagged with the generation | The bridge invalidates them when the projection sees a disconnect or a new epoch | Moves |

### Configuration, discovery, selection

| ID | Behavior in 3.0.0 | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| G1 | Startup loads the configuration and selects `current` | `OperatorAdmin::configuration` | Kept |
| G2 | Manual setup and "save discovered receiver" build a new `ConfiguredReceivers` holding one receiver and no favorites, and save it. This replaces the whole file: it removes other receivers and every sound mode favorite. This is the limit phase 1 recorded | Read-modify-write: `configuration()`, add or replace the named entry, set `current`, keep every other entry and all favorites, then `save_configuration()`. The service validates before saving. No new port method is needed | Changed. The limit closes. The GUI re-reads before each write so a hand edit is not overwritten. A write from another process between the read and the write can still be lost; with one user that is accepted |
| G3 | Sound mode favorites save the in-memory copy, serialized so rapid clicks build on each other | The same, through `save_configuration`, against the configuration the GUI last loaded or saved | Kept |
| G4 | Discovery with a 3 s timeout | `OperatorAdmin::discover` | Kept |
| G5 | `ReceiverSelection` has three forms | The GUI only ever creates `Saved`. The selection becomes a `ReceiverId` (the entry name) plus the identity for display; ad hoc registration is not needed by the GUI | Replaced |
| G6 | An unnamed entry is named by its host | Unchanged; the name must still pass `ReceiverId::new` (the service's `validate` rejects the reserved `adhoc:` prefix) | Kept |
| G7 | A receiver saved again under the same name with a new address takes effect at once, because every `Select` closed the old session and connected again. "Save discovered receiver" names the entry by model, so a receiver that DHCP moved is saved under its old name and then selected | `save_configuration` stores the file and leaves any open session alone (its own comment says an address change waits for release). The GUI holds a subscription for as long as the receiver is selected, so the release never comes, and the session reconnects forever (`reconnect_indefinitely`) to the address that no longer answers | Changed, unless fixed. Closed by [P2](#port-changes) |

### Diagnostics and observability

| ID | Behavior in 3.0.0 | Canonical equivalent | Disposition |
| --- | --- | --- | --- |
| O1 | `Diagnostic` events (connection generation, reconnect attempt, timeout, malformed frame, queue pressure, shutdown) go to `tracing` through `TracingObservability`, and a bridge failure is announced as "Diagnostic: …" | The session and transport log connect, disconnect, and reconnect. Unsupported frames are kept in `ReceiverState.diagnostics` (bounded to 32). A failed port call is announced from `ControlError`'s message | Replaced. Reconnect-attempt and queue-pressure logs are not reproduced; the transport's own logs remain |
| O2 | The Diagnostics screen shows lifecycle, generation, selected host, snapshot authority, Quick Select freshness, catalog freshness and count, and EQ lines | Lifecycle and generation come from the projection (epoch). Snapshot authority has no counterpart. Catalog and Quick Select name freshness remain. EQ lines go with D1 | Changed. D4 |
| O3 | Capture scenarios build `MainZoneSnapshot` values directly (`configure_capture_scenario`) | They must build a `ReceiverState` fixture and run it through the same projection, and produce identical pixels for the six screens that do not change | Moves. This is the main pixel risk |

### Outcome mapping

The GUI maps an `OperationSnapshot` to the message it announces. Wording of the
existing messages is kept where the meaning is kept.

| Operation status | Dispatch | Message |
| --- | --- | --- |
| `completed` | any | "Command confirmed by the receiver." |
| `already_in_state` | `not_dispatched` | "Receiver already has the requested value." |
| `rejected` | `not_dispatched` | "Command rejected: {reason}" |
| `indeterminate` | `complete_write`, `possibly_dispatched`, `unknown` | "Command sent, but confirmation is unavailable: {reason}" |
| `indeterminate` | `not_dispatched` | "Command failed to send: {reason}" |
| `cancelled`, `superseded` | `not_dispatched` | "Command cancelled." |
| submit or wait returned `ControlError` | none | "Operation failed: {error}" |

## Step 3 outcomes

Implementing the retarget settled these, and found the differences below that the
table did not predict. Each is a deliberate result, tested in `gui-lib`.

**Display model.** The views keep reading `MainZoneSnapshot` and `Zone2Snapshot`.
`projection::project` fills them from `ReceiverState`, applying S2. Replacing the
display model would change every view and its pixels for no behavior gain, so it
is left for a separate refactor. `desktop_projection.rs`, which only the tests
used, is deleted; its epoch and revision ordering is replaced by tagging every
bridge event with the `Select` request it belongs to, and dropping the events of
an earlier one.

**Decisions applied.** D2(b): a control carries the value its target field showed
(`FieldBaseline::capture`), and the bridge refuses it with "Receiver state changed
before the command; retry." if the subscription now shows another. Power controls
carry none, as in 3.0.0. D3: category recall is `RecallMovie`/`RecallMusic`/
`RecallGame`/`PureDirect`. D4: the EQ lines and the "Snapshot authority" row are
removed from Diagnostics, and the Quick Select recall button is removed from the
dashboard (it was never rendered).

**Found while implementing:**

| ID | What | Disposition |
| --- | --- | --- |
| I1 | The category a sound mode control pairs with the reported mode (C8) is read by no view: the panel uses its own local filter. | Dropped with no replacement |
| I2 | 3.0.0 converted a volume request with `native / 10`, which rounded a half-dB step down to the whole dB: +0.5 dB asked for 0 dB and then read as unconfirmed. | Fixed. The conversion is `native / 5`, tested at 5, 800, 805, and 985 |
| I3 | 3.0.0 showed volume at `Minimum` as 0.0 dB on the slider, because the adapter gave it a `0` dB value. | Fixed. It shows at the bottom of the slider, -80.0 dB |
| I4 | A saved receiver that could not be reached at launch left the launch screen on for ever, because only a successful connection opened it. | Fixed. A failed connection opens the unavailable screen with its retry |
| I5 | The source catalog was asked for at every state while its freshness was unknown, so a receiver that could not answer would be asked at the rate of the state updates. | Changed. The GUI asks once per connection on its own; the picker and the refresh button still ask on demand |
| I6 | The HTTP information timer, the HTTP information triggers, and the Quick Select name read are driven by the GUI (R1 to R3) and are tested for ordering, one read at a time, and dropping a read that finishes after a reconnect. | As designed |
| I7 | The favorites save re-reads the stored configuration and replaces only the favorites, so a hand edit to the receivers is not overwritten by a favorite click (G3 only promised the in-memory copy). | Changed (improvement) |
| I8 | A control was reported finished from the task that ran it, while state reached the window from the loop that watches the subscription, in no fixed order. A report could reach the window before the state it produced, and the next click would be built on a stale display and refused as a conflict. 3.0.0 sent the snapshot first on one channel. | Fixed. A task hands its result to the loop, which forwards any newer state before the report. A single-threaded test with a port that writes and resolves in one poll failed on its first round before the fix |

## Port changes

The table found two gaps in the port and confirmed that the rest is sufficient.
P1 adds a method; P2 changes the service's behavior and no signature. Step 2
records both in the Control service section of
[ARCHITECTURE.md](../../ARCHITECTURE.md#control-service), where phase 1 moved the
implemented port, and adds the Operator `refresh` resource to the Control API
table amendments that milestone 4 already makes.

**P1. Manual refresh.** The 3.0.0 `Refresh` action ("Refresh Status", "Retry
Status") and the refresh after a reconnect have no port equivalent: a
subscription is passive, and `synchronize` exists only on the session.

```text
OperatorAdmin::refresh(receiver: &ReceiverId) -> Result<Readiness, ControlError>
```

It leases the receiver (connecting if needed) and calls the session's
`synchronize`. It is Operator-only because it causes receiver traffic, like
`http_information`. It is a read; it dispatches nothing and does not touch the
gate. This is the only addition to `control.rs`.

**P2. Retire a session whose entry changed (G7).** A saved entry whose host
changed, or that was removed, retires its open session: the service closes it,
and the next lease connects at the new address. Step 2 settles the mechanism
and must meet three constraints:

- **Never under an operation in flight.** `X3800hSession::close` waits 250 ms
  and then aborts the actor, which would resolve an in-flight operation as
  indeterminate with `not_dispatched` even if the write went out. Retirement
  waits for operations in flight, as idle release already waits for every lease.
  It does not wait for subscriptions, which can be held indefinitely.
- **Never two connections for one receiver.** The receiver accepts one control
  connection, so the close finishes before the next connect begins. The slot
  lock already serializes connect and release; retirement takes the same lock.
- **Subscribers learn that the session ended.** A closed session leaves
  `StateSubscription::changed` pending forever, because the session itself owns
  the `watch` sender. The service ends the subscriptions of every session it
  closes, so `changed` returns the existing "session closed" error, and the
  bridge treats that error as a lost connection and calls `state` again. This
  does not depend on the order in which the GUI drops and re-takes its
  subscription, and it works for any session implementation.

This lives in the service, so the CLI and later clients get it too. Tests with the
fake connector: a save with a new host, then a lease, connects with the new
identity and closes the old session; an operation in flight on the old session
completes before it closes; two sessions are never open for one receiver at the
same time; a held subscription receives the closed error; a save that does not
change the identity retires nothing.

Considered and not needed:

- **Explicit release.** No view emits `Disconnect`. A selection change drops the
  subscription and the 60 s idle release applies, which still frees the
  receiver's single control connection for other tools. Only the changed-address
  case (P2) needs more than idle release.
- **Reconnect visibility.** Already visible as state (L5). The session also
  reads every core field again when it reconnects, which helps every client and
  is what the policy engine of milestone 3 needs; this was first left to the
  GUI through P1 and moved into the session on the owner's instruction.
- **Operation completion.** `submit` followed by `operation(id, wait)` is
  enough. The Operator event stream carries every client's operations, so the
  GUI would have to filter it; the operation feed is milestone 5.
- **Shutdown on the trait.** `ControlService::shutdown` stays inherent (L8).
- **Evicting a stopped session.** Settled in step 2: not needed. The canonical
  session sets `reconnect_indefinitely`, and the transport's reconnect loop then
  returns only when its own event receiver has been dropped, which happens only
  when the session itself is dropped or closed. A session's actor therefore
  cannot end by itself, and the service never holds a dead session.

## Presentation design

Step 3 is written against these rules. They are fixed here so the table's
"Moves" rows have a place to land.

- **Projection.** `desktop_projection.rs` is the seed. It exports a reducer for
  system power, main-zone power, and volume, and nothing outside its own tests
  uses it. It grows to every field the views show, applies S2 (usable only when
  `Current`), and keeps the epoch and revision ordering it already has. It has
  no Iced or transport dependency, so it is tested headless.
- **Bridge.** The controller bridge becomes a bridge to the port. It holds one
  `StateSubscription` for the selected receiver, turns each new state into a
  `Message`, submits operations, waits for them with `operation(id, wait)`, and
  owns the supplemental-read triggers (R1 to R3) and the 15 s HTTP timer. It
  remains the single serialized owner of the port handle.
- **Operations.** A control becomes an `OperationSubmission` with an idempotency
  key. The id the gate returns correlates the result; the GUI's own request ids
  remain for UI timers only (volume value display, sound-mode timeout).
- **What stays in `application`.** The pure helpers (`http_information`,
  `source_catalog`, `quick_select` names merge) stay while they have a user.
  The controller, `main_zone_control`, `main_zone_status`, and the legacy
  ports go in step 4 where nothing uses them.

### GUI services

`GuiServices` stops carrying a session factory:

```text
GuiServices { control: SharedOperatorControl, shutdown: Arc<dyn Fn() -> BoxFuture<()>> }
```

The desktop composes `ControlService` from `X3800hConnector`, the YAML
repository, and SSDP discovery, passes the operator handle as `control`, and
supplies `shutdown`. `gui-lib` therefore never names `ControlService`, and
milestone 5 changes the supplier and not the GUI. The `desktop → infrastructure`
edge stays until then.

## Deletion inventory

Consumers outside the file that defines each symbol, from `rg` on 2026-10-08.
Step 4 deletes a symbol only when this list is empty for it.

| Symbol | Other users today | Fate |
| --- | --- | --- |
| `ReceiverController`, `ControllerHandle`, `ControllerConfig`, `ReceiverEvent`, `ControlResult`, `Lifecycle` | `gui-lib` (bridge, root, views, tests) | Deleted. `Lifecycle` is replaced by a presentation enum |
| `CanonicalSessionFactory`, `CanonicalSessionAdapter` | `apps/desktop` | Deleted |
| `SessionFactory`, `ReceiverSession`, `SessionEvent` | `gui-lib`, `infrastructure/avr_session.rs` (the legacy trait impl), `tools/check-boundaries.sh` | Deleted, with the `AvrSession` impl. `AvrSession` itself stays as the transport |
| `StatusGateway`, `ControlGateway`, `AsyncStatusGateway`, `AsyncControlGateway` | `main_zone_control.rs`, `main_zone_status.rs`, `avr_session.rs` | Deleted with those modules |
| `execute_main_zone_control_async`, `query_main_zone_status*` | only each other | Deleted |
| `MainZoneControl`, `MainZoneEvent` | `protocol/src/avr/command.rs` and `response.rs`, `domain/capabilities.rs` | Step 5 inventory. Kept while the protocol crate uses them |
| `MainZoneSnapshot`, `FieldStatus` | `gui-lib` (the display model), `domain/http_information.rs`, `protocol/http_information.rs` | Kept. The GUI display model stays (step 3). The transport's own copy was write-only once the legacy impls went, and is removed (step 4) |
| `QuickSelectRecallOutcome`, `EqStatus`, `QuickSelectEqCapabilities`, and the recall and EQ code in `AvrSession` | `gui-lib`, `application/quick_select.rs`, `infrastructure/avr_session.rs` | Deleted (D1) |

## Step 4 outcomes

The legacy path is deleted, and `make boundary` now fails if it returns.

- **Deleted:** `ReceiverController` and its configuration, handle, events, and
  results (`controller.rs`); `main_zone_control.rs` and `main_zone_status.rs`;
  `CanonicalSessionFactory` and `CanonicalSessionAdapter` (`canonical_factory.rs`);
  the `SessionEvent`, `ReceiverSession`, `SessionFactory`, `StatusGateway`,
  `ControlGateway`, `AsyncStatusGateway`, `AsyncControlGateway`, and
  `SourceCatalogReader` traits; and the `ReceiverSession` implementation of
  `AvrSession`, with the Quick Select recall and EQ status code in it (D1).
- **`AvrSession` stays as the transport.** It keeps `request`, `dispatch`,
  `next_event`, `connection_generation`, and now an inherent `close`. Its
  shadow `MainZoneSnapshot` was updated by every line and read by nothing once the
  legacy impls were gone, so it and the code that fed it are removed; no event,
  timing, or write path changed. `request_query`, a retrying read that only the
  legacy path used, is removed with its test.
- **Application helpers kept:** the HTTP information, source catalog, and Quick
  Select name policy the GUI calls (`should_read`, `intent_may_have_changed`,
  `merge_refresh`, `apply_names`), each now public and tested.
- **Guards:** the single-definition rule names `CanonicalReceiverSession` in
  `session_v3.rs`; the retired names are forbidden in `crates` and `apps`; and
  `canonical_factory.rs` is gone from the `operate` allowlist. Each rule was
  checked by adding a violation and seeing it fail. `AGENTS.md` now says the
  contract is in `session_v3.rs`, which avoids moving a trait that every import
  in infrastructure and the CLI names.
- **Live tests:** `make test-live-x3800h` and `-controls` are ordinary `--test`
  targets, compiled by `make check`, and they use no legacy symbol.

## Step 5 outcomes

Scoped by inventory: a scan of every exported name for a user outside its own
definition, run after step 4. Only what the legacy path used, or what D1
removed, went; the canonical X3800H codec and the canonical state types are
untouched.

- **Protocol:** `quick_select_eq.rs` (Quick Select recall and EQ commands and
  parser) is deleted. In `avr`, the MainZone query, control, Zone 2, volume, and
  audio-context encoders and parsers are deleted, as are the error variants only
  they used. `AvrCommand`, `AvrProtocolError`, `get_command_family`,
  `response_matches`, and the `x3800h` codec remain: the transport and the
  canonical session use them.
- **Domain:** `eq_status.rs` and `audio_context.rs` are deleted; so are
  `QuickSelectRecallOutcome`, `QuickSelectRecallConfirmation`,
  `QuickSelectEqCapabilities`, `SourceCatalogCapabilities` and the
  `with_validated_*` builders, the `quick_select_recall` and `eq_status`
  capability fields, `ConnectionState`, `MainZoneEvent`, `Zone2Control`, and the
  audio-context types.
- **`MainZoneSnapshot` keeps** the five fields, the HTTP information, and
  `set_value`, `set_error`, `value`, `invalidate`, and the HTTP setters, because
  it is the GUI's display model. It loses the unread audio context, the unread
  sound mode category and its confirm method, the event reducers, the version
  counter, and `probe_succeeded`.
- **`MainZoneControl`, `MainZoneField`, and `MainZoneValue` stay.** The GUI uses
  them to ask a model what it supports and to name a field; they no longer have a
  protocol consumer.
- **Left alone on purpose:** exported names with no user that belong to the
  canonical design and not to the legacy path (`TransmissionSchedule`,
  `FieldIssue`, `FieldValue`), and the HEOS and AppCommand items outside this
  milestone's scope.

## Guard changes

These land with the step that makes them necessary, so `make check` passes at
every commit.

- **Step 3 rewrites `crates/gui-lib/tests/async_apis.rs`.** It builds
  `GuiServices { factory, … }` and fakes `ReceiverSession`, so retargeting the
  GUI breaks it. It is rewritten against the port with a fake connector. The
  file keeps its name and path, so `tools/check-phase5-ledger.sh`, which only
  tests that the file exists, passes unchanged.
- **Step 3 rewrites the roughly 750 lines of unit tests in the `gui-lib` root
  module (`lib.rs`)** that construct `ControllerBridge::new(TestSessionFactory)`.
- **Step 4 flips the single-definition rule** in `tools/check-boundaries.sh`
  from `SessionEvent|ReceiverSession|SessionFactory` in `ports.rs` to
  `CanonicalReceiverSession`, and removes `canonical_factory.rs` from the
  `operate` caller allowlist, in the same commit that deletes the three traits.
  Either alone fails `make boundary`.
- **`AGENTS.md` is wrong about where the contract lives.** It says the
  canonical receiver-session contract is defined in `ports.rs` in the
  `application` crate; `CanonicalReceiverSession` is in `session_v3.rs`. Step 4
  moves the trait into `ports.rs` (the module name `session_v3` is a phase
  label) or corrects the sentence, together with `ARCHITECTURE.md` and the
  contributing guide, which step 4 updates anyway. Moving it is preferred,
  because the boundary rule's path check then needs no exception.

## Owner decisions

D1 (remove Quick Select recall and EQ status) is in the roadmap. These are new.
Each has a recommendation. The owner asked on 2026-10-08 for the milestone to be
completed without answering them, so the recommendations were taken as the
decisions and step 3 implements them. Any can be reversed in a follow-up commit.

**Answered in the interview that followed (2026-10-08).** D2 to D5 all stand.
Further:

- **Graceful close.** Closing the window closes the receiver connection first,
  within a short grace period, instead of dropping the socket.
- **Reconnect re-read.** In the session, for every client (see L4).
- **`ZMON` pause.** The transport pauses after a main-zone power-on as it does
  after `PWON`, now, without waiting for the live check (C4).
- **Slider range.** The volume slider and its label run to +18.0 dB, the
  receiver's maximum. Every baseline that shows the volume row changes: connected,
  source-picker, and messages, at both scales.
- **Launch delay (L3).** Measure live first; both synchronization passes stay.
- **Display model.** `MainZoneSnapshot` stays; the roadmap records it as debt.
- **`config/` backups** are ignored by Git.
- **Merge.** A person merges the branch once the owner's checks pass.

**D2. Optimistic concurrency (C6).** 3.0.0 rejected a control whose snapshot was
stale. Operator operations carry no precondition. The exposure is the volume
control: it steps relative to the displayed level, so a stale display can send a
large jump. Absolute controls (power, input, mute, mode) are harmless.

- (a) Drop the check.
- (b) **Recommended.** The bridge compares the displayed value of the target
  field with the latest observed value before it submits, and reports the same
  "Receiver state changed before the command; retry." Field-level, so it is more
  precise than 3.0.0's whole-snapshot version. No contract change.
- (c) Let Operator submissions carry an optional `Precondition`, checked by the
  session. Race-free, but it widens the contract that milestones 3 and 4 are
  about to freeze.

**D3. Category recall when already in the category (C8).** Pressing Movie while
in a Movie mode re-sent `MSMOVIE` and reported "unconfirmed" if the mode did not
change. The canonical path reports "already has the requested value" and writes
nothing. Recommended: accept. Keeping the re-send needs a new sound-mode intent.

**D4. Diagnostics screen (O2, S9, R4).** The Diagnostics baselines
(`diagnostics-100pct.png`, `-200pct.png`) cannot stay pixel-identical: D1 removes
the EQ lines, and "Snapshot authority" has no canonical source. Recommended:
remove the EQ lines and the authority row, keep the Quick Select names freshness
row, recapture those two baselines, and amend the exit criterion to read: the
other baselines are byte-identical, and the two Diagnostics baselines differ only
in the rows named here. (The owner's later slider change adds the six baselines
that show the volume row to the ones that differ; the overview lists them.)

**D5. Accepted differences.** Confirm or veto each:

1. Outcome wording follows the real outcome (C7).
2. A Zone 2 change made on the receiver appears at once (S8).
3. No extra wait after a main-zone power-on and no second confirmation query (C4,
   C5), which shortens the time to "confirmed".
4. The dashboard opens after two synchronization passes, 14 paced queries, and
   not when the socket opens (L3). Step 2 kept both passes: the actor's startup
   pass is not observable from outside, and the service's own pass is what
   reports a failed first read and closes the session. Measure it live.
5. Manual setup and "save discovered" keep other receivers and all favorites
   (G2).
6. A receiver saved again under its old name with a new address reconnects at
   the new address without a restart (G7, P2). This restores 3.0.0 behavior
   that the port would otherwise lose, so it is listed for completeness.
7. A half-dB volume step now asks for the half step; 3.0.0 rounded it down to the
   whole dB (I2).
8. Volume at `Minimum` shows at the bottom of the slider, not at 0.0 dB (I3).
9. An unreachable receiver at launch opens the unavailable screen and its retry,
   not a launch screen that never ends (I4).
10. The source catalog is asked for once per connection on its own, not at every
    state (I5).
11. A favorite click re-reads the stored configuration, so a hand edit to the
    receivers is not overwritten (I7).
12. The volume slider and its label end at +18.0 dB, the receiver's maximum,
    where 3.0.0 offered +18.5 dB and sent +18.0 (C10).
13. A main-zone power-on is followed by the same one-second pause as `PWON`
    (C4), where 3.0.0 paused in the controller after the confirmation.
14. Closing the window closes the receiver connection first, so it is free for
    other tools at once (L8).

## Live checks

These need the receiver. Step 6 runs them at the milestone's exit and records
the result in the archive, as in version 3.

- **L3.** Time from launch to the dashboard, against 3.0.0.
- **L4.** Forced reconnect (network off and on, or a receiver power cycle):
  fields go stale, then recover without a restart. Compare with 3.0.0's recovery
  time.
- **C4.** Main-zone power-on from standby through the GUI. Confirm the
  confirmation completes without a timeout and that no command is refused. If the
  receiver needs a quiet period after `ZMON`, extend the transport rule.
- **C8.** Each category recall, from inside and outside the category.
- **R1.** HTTP information refreshes after an input change, a sound-mode change,
  and a power-on.
- **G2.** With a hand-edited two-receiver file, "manual setup" keeps both and the
  favorites.
- **G7.** With the GUI connected, edit the saved host to a second address that
  reaches the same receiver and re-save it. The GUI moves to the new address
  without a restart, and the receiver never shows two control connections.
