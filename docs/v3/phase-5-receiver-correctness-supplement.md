# Version 3, Phase 5 supplement — current receiver monitoring and control

**Research and implementation baseline — 2026-09-11.** This document records
what the current code actually does before Phase 5 changes it. It is descriptive,
not a promise that the behavior is correct. The companion
[overview](phase-5-receiver-correctness-overview.md) defines the phase scope and
the [architecture](phase-5-receiver-correctness-architecture.md) defines the
replacement design. The
[headless test plan](phase-5-receiver-correctness-test-plan.md) turns its
requirements into executable coverage.

## Evidence and confidence

The code review covers the active workspace, especially
[`controller.rs`](../../crates/application/src/controller.rs),
[`ports.rs`](../../crates/application/src/ports.rs),
[`avr_session.rs`](../../crates/infrastructure/src/avr_session.rs),
[`tcp_avr.rs`](../../crates/infrastructure/src/tcp_avr.rs),
[`command.rs`](../../crates/protocol/src/avr/command.rs),
[`response.rs`](../../crates/protocol/src/avr/response.rs),
[`main_zone.rs`](../../crates/domain/src/main_zone.rs), and
[`bridge.rs`](../../crates/gui-lib/src/bridge.rs). The verification review also
covers the inline tests, [`async_apis.rs`](../../crates/gui-lib/tests/async_apis.rs),
the root [`Makefile`](../../Makefile), and the current boundary checker.

Protocol conclusions use this evidence order:

1. the current AVC-X3800H manual and Denon-hosted protocol material;
2. the model-specific 2022 control chart covering AVC/AVR-X3800H, available as
   a third-party mirror and therefore treated as strong but mirrored evidence;
3. repeatable captures from the project's target receiver;
4. mature open-source implementations and forum reports, used only to identify
   field conditions and test cases.

The model-specific chart is more useful than older generic charts for command
availability, but it still must be checked against a real receiver's region,
firmware, and setup. Community reports are not normative protocol definitions.

## Current component flow

```text
desktop UI -> GUI bridge -> application Controller -> ReceiverSession port
                                                   -> AvrSession TCP actor
                                                   -> HTTP helper calls

CLI -> synchronous application policies -> SyncAvrClient

AVR TCP lines -> AvrSession private snapshot + bounded event queue
              -> application Controller snapshot + bounded event queue
              -> GUI bridge tagging/filtering -> Iced state
```

The asynchronous desktop path has two mutable Main Zone snapshots: one inside
`AvrSession` and one inside the application controller. The synchronous CLI has
a separate socket client and status/control sequence. Consequently, the three
delivery applications do not consume one canonical receiver state machine.

## How monitoring currently happens

### Connection and initial read

1. The desktop selects a receiver and asks the GUI bridge to connect and then
   refresh.
2. The application controller opens an `AvrSession`, increments its own
   generation, and reports `Connected` before core status is read.
3. A refresh serially queries five `MainZoneField` values in this order:
   `PW?`, `SI?`, `MV?`, `MU?`, and `MS?`.
4. For each query, the TCP actor writes a CR-terminated command and waits for
   the first line whose text has the expected family. Other lines are parsed as
   unsolicited events while it waits.
5. The application controller applies successful query values as
   `Authoritative`; a failed field is replaced by `Unavailable(error)`. It then
   publishes the whole `MainZoneSnapshot`.
6. Zone 2 is queried separately. HTTP information and Quick Select names are
   supplemental reads after the core refresh.

`Connected` therefore means that a TCP socket was established, not that a
usable state baseline exists. A refresh is also not atomic: each field mutates
the aggregate as it arrives.

### Continuous observation

- The TCP actor reads unsolicited CR-terminated lines whenever no request is
  waiting. While a query is waiting, nonmatching lines take the same event
  path.
- Every recognized line first updates the TCP actor's private snapshot.
  Main-zone line notifications then use `try_send` into a queue of 32 and are
  silently discarded when full. Lifecycle notifications use a blocking send.
- The application controller wakes every 25 ms and drains at most 32 session
  events, with a 1 ms wait for each item. Recognized changes update its separate
  snapshot and may cause a full snapshot publication.
- Every 15 seconds the same controller actor performs a five-field Main Zone
  refresh, a Zone 2 refresh, and an HTTP information refresh.
- The default query response timeout is one second. A read-only query is issued
  once more after a timeout or disconnect causes a reconnect.
- Automatic reconnect uses at most three attempts with a fixed 250 ms delay by
  default. If they are exhausted, the TCP actor ends instead of continuing to
  monitor in the background.
- Disconnect or reconnect notifications invalidate controller state. The
  infrastructure actor also invalidates its private snapshot when a response
  times out or the stream is malformed or disconnected.

The private infrastructure snapshot makes dropped line notifications look safe
inside `publish_event`, but the application never consumes that snapshot. A
burst can therefore leave the UI state behind the receiver until a later
reconciliation happens to repair it.

### Supplemental information

Audio context, HTTP information, source catalog, Quick Select names, and EQ
status have partially independent policy modules, but their calls still pass
through the serialized application controller and some are embedded in
`MainZoneSnapshot`. Awaiting a slow HTTP result can delay processing of TCP
events and commands. Supplemental invalidation and completion also increment
the Main Zone `resource_version`, even when no controllable Main Zone field
changed.

## How control currently happens

### Desktop path

1. The controller admits a `MainZoneControl` against model capabilities, an
   optional `expected_version`, and the controller's cached snapshot.
2. If the cached field already equals the target, the operation returns
   `NoOp`; it does not first prove that the cached observation is current.
3. The session encodes and writes one command. A successful write is reported
   as dispatched; there is no protocol-level acknowledgement distinct from a
   status line.
4. Power-on controls wait one second.
5. The controller starts a full five-field refresh under a two-second outer
   timeout, rather than querying only the affected field.
6. If the target field in the refreshed aggregate matches, the operation is
   `Confirmed`; otherwise it is `Unconfirmed`.

The command result and state stream are then forwarded through the GUI bridge.
The bridge associates later events with its most recent or pending GUI request
identifier, even though unsolicited receiver events have no such causal
relationship. Most snapshot events do not contain a lifecycle generation and
are tagged as generation zero or inherit bridge state. GUI stale-event filters
therefore operate on metadata that the receiver did not establish.

### CLI path

The CLI opens `SyncAvrClient`, performs a fresh five-field query, runs the same
admission policy, and calls its synchronous `execute_once`. Despite that name,
this implementation writes the control and waits for a same-family line through
`request_raw`; it does not use the asynchronous desktop transaction. The CLI
then prints that the command was dispatched but not confirmed.

The CLI's `resource_version` is reconstructed from a new process-local
snapshot. It is a count of local mutations, not a receiver revision. The same
number can recur after unrelated receiver changes, so `--resource-version`
cannot provide receiver-side compare-and-set protection.

### Several controls and the physical remote

The current implementation has no explicit “receiver is converging” state or
merged synchronization debt after activity. Each desktop control performs its
own admission, one write, and full refresh. Several controls therefore create a
sequence of independent confirmations instead of one dependency-aware final
settlement pass.

This matters when desktop and physical-remote actions overlap:

- Receiver events have no transaction ID, but the GUI bridge associates them
  with the latest desktop request ID. A physical-remote event can therefore be
  presented or filtered as if it belonged to a desktop operation.
- A source, power, sound-mode, or Quick Select change can cause other receiver
  fields to change. The first event updates only the field it carries; related
  fields wait for the current full refresh or the 15-second periodic refresh.
- Volume and other event bursts can overflow the droppable session event queue,
  leaving the application snapshot at an intermediate value.
- A full refresh interleaves sequential queries with unsolicited activity. The
  resulting aggregate can contain values observed before and after another
  desktop or physical-remote action without exposing that it is still
  converging.
- A target observed during control confirmation may immediately be replaced by
  a later physical-remote value. The operation result and current status are
  not modeled as separate time-scoped facts.

Periodic polling can eventually repair some outcomes, but the application has
no bound or explicit debt showing which fields still require reconciliation.
Phase 5 therefore needs an activity-aware convergence cycle in addition to
single-operation confirmation.

## Current verification shape

The repository has useful unit tests in every reusable layer, fake-session
tests around the application controller, several loopback TCP tests inside
`avr_session.rs`, and headless GUI-library tests. `make check` compiles and runs
all targets. These tests protect many individual cases, but they do not yet
form one explicit correctness argument for monitoring and control.

In particular, the current workspace has no:

- named smoke suite that composes domain, application, protocol, and
  infrastructure without launching CLI or GUI;
- reusable strict virtual X3800H shared by full-stack integration tests;
- scenario manifest mapping every documented invariant and failure transition
  to an executable test;
- deterministic clock covering 50 ms pacing, five-second event windows,
  validity expiry, and reconnect backoff without wall-clock sleeps;
- byte-level fault injector for disconnect-before-write, partial-write,
  write-complete, late-frame, and malformed-stream boundaries;
- property/state-machine suite that generates event, query, control,
  cancellation, and reconnect interleavings;
- replay format joining captured AVR traces to expected state revisions and
  operation outcomes; or
- separate fast-smoke, comprehensive-scenario, property, and opt-in live
  hardware commands.

The GUI tests necessarily exercise current bridge behavior, so they cannot
substitute for a delivery-independent test of the underlying stack. Phase 5
needs both: a core suite that cannot import GUI or CLI, and headless desktop
projection tests that do not launch a window or executable.

## Correctness findings

| Priority | Finding | Current consequence |
|---|---|---|
| Critical | `MainZoneField::Power` uses `PW?`, `PWON`, and `PWSTANDBY`. Denon's protocol defines `PW` as system power and `ZM` as Main Zone power. | A Main Zone UI can observe or change the wrong power scope; standby can affect more than Main Zone. |
| Critical | Droppable line events feed a snapshot that only infrastructure reads, while application owns another snapshot. | Event bursts can silently diverge public state from receiver state. |
| Critical | Ordinary commands are not globally paced to the documented minimum interval. | Back-to-back refresh and control traffic can violate the receiver's wire contract. |
| High | Confirmation allows two seconds, while the protocol allows an event as late as five seconds; an `MS` transition can first report the previous mode. | A successful receiver change can be reported as unconfirmed, especially for sound mode. |
| High | Refresh mutates one aggregate across sequential requests without binding every result to one connection epoch. | Reconnect during refresh can produce a cross-connection snapshot. |
| High | A successful socket write is treated as definite dispatch and write failure as definite transport failure. | A partial write or disconnect can leave delivery unknowable, but the API cannot express that. |
| High | Errors replace last-good field values; aggregate `freshness` and `authority` are overwritten by whichever field changed last. | The UI loses useful evidence and can describe a mixed snapshot as wholly live or authoritative. |
| High | `resource_version` changes for unrelated and supplemental fields but is presented as an optimistic concurrency token. | Valid controls conflict, while stale controls can still pass; no atomic receiver comparison exists. |
| High | The GUI bridge assigns command request IDs to unrelated receiver events. | Stale filtering and control feedback can suppress or attribute real external changes incorrectly. |
| High | The serialized controller can await optional HTTP work and block on a bounded publication channel while the bridge awaits a command. | TCP event handling and commands can be delayed; under pressure the ownership cycle can stall. |
| Medium | Desktop and CLI use different socket clients and dispatch semantics. | Fixes can land in one path while the other remains behaviorally different. |
| Medium | Generic normalized volume permits native `985`; the X3800H documentation ends at `98` (+18.0 dB) and distinguishes Min from -79.5 dB. | The public type admits a target outside this model's documented range and obscures receiver units. |
| Medium | Static source and sound-mode allowlists conflate chart support, region, setup, and current signal compatibility. | The UI can offer sources or modes the configured receiver cannot select and omit ones it can. |
| Medium | A GUI-selected sound-mode category is stored in the receiver snapshot although `MS?` does not report that category. | Local intent is presented as observed receiver state. |

These findings are contract defects, not merely missing retries. Increasing
queue sizes or extending one timeout would leave the contradictory ownership
and semantics intact.

## External protocol and field research

### Transport timing and response meaning

[Denon's public older IP protocol](https://assets.denon.com/documentmaster/de/ip_protocol_avr-xx100.pdf)
and the [mirrored 2022 model chart](https://allonis.com/media/kunena/attachments/814/DenonFY23-CY2022_AVR_PROTOCOL_V0221.pdf)
agree on the important transport constraints: TCP port 23, ASCII commands
terminated by CR, a 135-byte maximum message, at least 50 ms between
transmissions, a nominal 200 ms response period, events allowed up to five
seconds, and a one-second quiet period after `PWON`. The chart also warns that
a sound-mode change may first return the present mode and later the changed
mode.

The protocol uses status-shaped lines for both query responses and unsolicited
events. It does not provide client transaction IDs. A client can establish that
the requested value was observed after its write, but it generally cannot prove
that its write caused the value when another remote may also be active.

### Power is two different concepts

The model chart defines `PW` as system power and `ZM` as Main Zone power. Zone 2
uses `Z2`. Phase 5 must model all three explicitly. The dashboard's Main Zone
toggle must use `ZM?`, `ZMON`, and `ZMOFF`; system standby must be a separate,
deliberately named operation if exposed at all.

The [X3800H Network Control manual](https://manuals.denon.com/avcx3800h/eu/en/HJWMSYmehwmguq.php)
also says Network Control must be **Always On** for web, remote-app, and HEOS
access during standby. If it is **Off in Standby**, a lost connection may be
configured behavior rather than a transport defect. The app must surface that
distinction as guidance without pretending it can diagnose the setting from a
failed socket alone.

### Exact volume units

The [X3800H owner's manual](https://manuals.denon.com/avcx3800h/eu/en/download.php?filename=%2FAVCX3800H%2FEU%2FEN%2Fpdf%2FAVCX3800H_EU_EN.pdf)
documents either a 0–98 display scale or a dB scale from -79.5 dB through
+18.0 dB, plus the minimum/unavailable display. The wire form uses half-dB
steps. The domain should therefore store exact dB half-steps (and
minimum/unavailable), not a generic percentage-like 0–100 value. A configured
volume limit may make the effective maximum lower than the model maximum.

### Sources and sound modes are contextual

The 2022 chart does not support the current X3800H static source list as-is: it
shows protocol identifiers such as SAT/CBL, Media Player, Network, Bluetooth,
and a model-dependent AUX set, while some legacy names are unavailable on the
target family. User renaming, hiding, region, and setup further affect what
should be displayed. The safe design is an exact chart-backed command grammar
intersected with a receiver-observed catalog, preserving protocol identifier,
display label, and visibility as separate data.

The X3800H manual says [available sound modes depend on the input signal,
channel count, speaker setup, and headphones](https://manuals.denon.com/AVCX3800H/EU/EN/DRDZSYhrhvtgzs.php).
Its [MOVIE, MUSIC, and GAME actions recall remembered choices](https://manuals.denon.com/avcx3800h/eu/en/GFNFSYvhhpesdv.php)
and can fall back when the old choice is incompatible. Those actions are
intents, not stable observed modes. Phase 5 must separate settable sound-mode
intent from the detailed `MS` status reported by the AVR.

### One TCP owner is an operational assumption

The [`denonavr` project](https://github.com/ol-iver/denonavr) and
[long-running](https://community.symcon.de/t/modul-denon-marantz-avr/40873)
[automation forum threads](https://forum.universal-devices.com/topic/8833-how-to-control-denon-avr-using-official-control-protocol/)
report that Denon receivers permit only one active Telnet/IP-control client.
Reports describe another client or a stuck old connection preventing control.
Phase 5 must preserve one socket owner inside the process, report port
contention clearly, and avoid opening an extra probe connection. A cross-process
broker is useful future work but not required by this phase.

Community implementations commonly combine Telnet events with periodic HTTP or
Telnet polling. That is evidence for reconciliation as a safety net, not for
letting HTTP become the authority for core control. A recent X3800H report also
found a legacy unauthenticated HTTP control endpoint returning 403. Undocumented
HTTP writes are therefore not a correctness fallback.

## Phase 5 design constraints derived from the audit

- One actor owns the TCP stream, wire pacing, frame sequence, connection epoch,
  and the only canonical core state reducer.
- Consumers receive a coalescing full-state watch, so notification pressure can
  skip intermediate renders without losing the latest state.
- Every field carries its own last-good observation, epoch, sequence, time,
  origin, validity, convergence/settlement, and error. Failure makes evidence
  stale; it does not erase it.
- Desktop writes and physical-remote events merge dependency-aware
  synchronization debt; quick and post-event-horizon targeted reads converge
  the canonical mirror without a per-control full refresh.
- Controls use targeted preflight and post-dispatch observation. Results say
  `ObservedRequestedValue` or `Indeterminate`, not a causally stronger
  “acknowledged” claim.
- The API removes receiver-wide optimistic versions, separates operation IDs
  from state identity, and rejects old-epoch asynchronous results.
- Core Telnet state, source catalog, HTTP information, Quick Select metadata,
  and EQ evidence are separate resources with separate freshness.
- CLI and desktop compose the same asynchronous application service. No second
  protocol implementation remains.
- Correct AVC/AVR-X3800H behavior takes precedence over API compatibility and
  broad, unvalidated model support.

## Research sources

- [Denon FY23/CY2022 AVR control protocol V02 mirror](https://allonis.com/media/kunena/attachments/814/DenonFY23-CY2022_AVR_PROTOCOL_V0221.pdf)
  — model-specific chart that includes AVC/AVR-X3800H; third-party hosted.
- [Denon AVR control protocol V06](https://assets.denon.com/documentmaster/de/ip_protocol_avr-xx100.pdf)
  — Denon-hosted corroboration for framing, timing, events, and power-on delay.
- [AVC-X3800H Network Control manual](https://manuals.denon.com/avcx3800h/eu/en/HJWMSYmehwmguq.php)
  — standby network behavior.
- [AVC-X3800H owner's manual PDF](https://manuals.denon.com/avcx3800h/eu/en/download.php?filename=%2FAVCX3800H%2FEU%2FEN%2Fpdf%2FAVCX3800H_EU_EN.pdf)
  — volume range and receiver behavior.
- [AVC-X3800H sound-mode selection](https://manuals.denon.com/avcx3800h/eu/en/GFNFSYvhhpesdv.php)
  and [sound-mode availability](https://manuals.denon.com/AVCX3800H/EU/EN/DRDZSYhrhvtgzs.php)
  — recalled intents and context-dependent mode availability.
- [`denonavr` implementation notes](https://github.com/ol-iver/denonavr)
  and [Home Assistant Denon integration](https://github.com/home-assistant/core/blob/dev/homeassistant/components/denonavr/media_player.py)
  — mature implementation comparisons, not protocol authority.
- [Symcon Denon/Marantz module discussion](https://community.symcon.de/t/modul-denon-marantz-avr/40873),
  [Universal Devices control discussion](https://forum.universal-devices.com/topic/8833-how-to-control-denon-avr-using-official-control-protocol/),
  and [FHEM Denon thread](https://forum.fhem.de/index.php?topic=58452.240)
  — field reports about single-client behavior and reconciliation.
- [X3800H Telnet/HTTP field report](https://www.heimkinoverein.de/forum/thread/27843-denon-x3800h-homeassistant-steuerung-mit-telnet-http-https-befehle/)
  and [`denonavr` sound-mode timing report](https://github.com/ol-iver/denonavr/issues/274)
  — test hypotheses for current hardware, not normative claims.
