# Version 3 changelog

## [Unreleased]

### Fixed

- Made the X3800H HTTP information probe retain partial batch evidence and
  retry only missing read-only AppCommand observations.

### Added

- Added the initial Phase 5 canonical X3800H session, exact receiver-state
  observations, public loopback smoke test, and headless desktop projection.
- Added pure synchronization-debt bookkeeping, stale-on-reconciliation-failure
  transitions, and lower-layer tracing lifecycle diagnostics.
- Preserved malformed and unknown receiver frames as bounded diagnostics without
  mutating typed canonical state.
- Unified diagnostic and typed-frame sequencing within each connection epoch.
- Distinguished pre-dispatch command rejection from ambiguous transport delivery
  in canonical operation outcomes.
- Extended post-dispatch confirmation across the documented five-second event
  horizon without replaying the mutating command; inbound events remain reduced
  during the wait.
- Reused the domain X3800H capability tables for canonical source and sound-mode
  admission, keeping profile-backed choices aligned with presentation data.
- Integrated epoch-bound synchronization debt into the canonical session actor;
  successful full synchronization clears debt only when readiness is restored.
- Canonical sessions now service merged activity debt with a paced quick pass
  and one deferred final pass instead of waiting solely for the periodic sweep.
- Receiver-event debt now settles after a successful quick pass when no local
  control is extending the final reconciliation horizon.
- Canonical scheduling now enforces the documented eight-action/500 ms
  fairness budget before servicing due reconciliation debt.
- Initial canonical connection failures now retain their typed error kind while
  including actionable Network Control and competing-Telnet-client guidance.
- Local operations now extend one merged final-settlement deadline five seconds
  beyond the latest related write.

- Added the proposed Version 3 Phase 5 receiver-correctness plan, target
  architecture, current-code supplement, Denon/community research record,
  comprehensive headless test strategy, activity-aware status-convergence
  design, and desktop-correctness requirements.
- Added a reproducible Apple Silicon macOS packaging workflow that produces
  an unsigned `.app` and versioned drag-and-drop `.dmg`.
- Desktop composition now backs the existing GUI controller port with the
  canonical X3800H session adapter.
- The low-level AVR transport is now private to infrastructure; the obsolete
  legacy session factory and snapshot getter are no longer public production
  entry points.
- Compatibility-adapter controls now allocate distinct canonical operation IDs
  instead of reusing a process-wide constant.
- Canonical desktop sessions retain independent HTTP, source-catalog, and Quick
  Select-name refreshes through the compatibility adapter.
- X3800H source encoding now canonicalizes profile IDs and rejects unsupported
  identifiers at the protocol boundary.
- Added a canonical targeted-observation session operation; compatibility
  consumers no longer need to trigger full synchronization for one field.
- Targeted observations now open and settle their own canonical synchronization
  cycle with failure metadata preserved.
- Added named infrastructure scenario coverage for stale recovery, power-scope
  isolation, unavailable volume evidence, and ambiguous delivery.
- Expanded named smoke coverage for remote bursts, mixed-controller evidence,
  typed multi-control wire commands, slow subscribers, and clean close.
- Added a composed loopback smoke scenario proving an omitted remote event is
  repaired by canonical targeted observation.
- Added seeded infrastructure state-machine properties covering event
  interleavings, revision monotonicity, evidence retention, and epoch fencing.
- Added a reviewed X3800H raw-frame replay suite and a dedicated Make target;
  typed reduction, unknown-frame isolation, and malformed-frame handling are
  now exercised through production parser/reducer paths.
- Added an executable Phase 5 ledger gate that verifies every SMK requirement
  ID and its required test fixture before the repository check proceeds.
- Category sound-mode recalls now confirm against the validated X3800H category
  tables instead of being permanently reported as unconfirmed.
- Canonical convergence metadata now preserves the initiating cause, including
  manual refresh versus local control, for accurate state projections.
- Added an explicitly armed live-controls target with safe mute round-trip
  restoration and mandatory receiver/volume-safety environment gates.
- Recorded the first successful physical AVC-X3800H read-only synchronization run;
  armed control validation remains separate and still requires its evidence.
- Recorded a successful armed mute round-trip with receiver-confirmed
  restoration; broader live control scenarios remain separately gated.
- Recorded a successful AVC-X3800H read-only context probe on firmware
  `6000-1060-0071-9831`; the expected `DC?` timeout/reconnect was preserved as
  diagnostic evidence and no writes were issued.
- Context diagnostics now treat the AVC-X3800H's unanswered `DC?` family as
  optional unavailable evidence after reconnect, allowing the probe to exit
  successfully when all required queries pass.
- Re-ran the complete AVC-X3800H validation set: context and HTTP diagnostics,
  read-only synchronization, and armed mute restoration all passed.
- Live write validation now refuses to proceed when observed volume exceeds the
  declared half-step safety ceiling.
- Ambiguous writes now perform a read-only confirmation probe without replaying
  the mutation and can report an evidence-backed unknown-certainty success.
- Dropped operation requesters now leave the serialized actor running: a
  dispatched command is neither replayed nor rolled back, and later receiver
  evidence still reduces into canonical state.
- Canonical operations now observe requester cancellation before admission and
  again after preflight, rejecting without a mutating write when cancelled.
- Cancelled operations no longer create local-control synchronization debt while
  they are being discarded by the actor.
- Added transport-backed cancellation coverage proving a cancelled operation
  never reaches the AVR writer and leaves no reconciliation debt.
- Final synchronization settlement deadlines now begin at completed-write time,
  preserving the full five-second post-write observation horizon.
- Canonical session close is now idempotent, so repeated shutdown requests
  complete successfully without reopening or retrying transport work.
- Compatibility Zone 2 reads now use targeted canonical observation instead of
  triggering a broad synchronization pass.
- Compatibility event translation now ignores unrelated system/Zone 2 and
  validity-only snapshots instead of fabricating Main Zone values.
- Compatibility control operations now log canonical operation IDs and
  evidence/uncertainty outcomes for GUI-facing diagnostics.
- Compatibility source controls now reject invalid source identifiers as typed
  pre-dispatch errors instead of panicking during adapter conversion.
- Compatibility event translation now surfaces canonical epoch transitions as
  reconnecting/connected lifecycle events for the GUI controller.
- Added lifecycle-transition regression coverage to prevent cross-epoch noise
  from becoming a false connection event.
- Lower-layer AppCommand HTTP and SSDP discovery adapters now emit structured
  tracing for request lifecycle, connection failures, bounded responses, and
  discovery completion.
- YAML configuration load/save operations now emit structured success/failure
  diagnostics without logging receiver secrets or full configuration contents.
- Supplemental source-catalog, Quick Select-name, and HTTP-information reads
  now emit structured start/completion tracing with generation metadata.
- Malformed/unknown X3800H frames and failed canonical observations now emit
  structured diagnostics at the infrastructure boundary.
- Application control admission now logs stale-version conflicts, capability
  denials, no-op observations, and dispatch decisions with typed control data.
- Asynchronous control paths now log dispatch failures, receiver-confirmed
  outcomes, and unconfirmed results at the application boundary.
- Removed the unused synchronous Main Zone control APIs; control execution now
  follows the asynchronous Phase 5 path exclusively.
- Added a redaction-safe live validation record template covering device
  metadata, wire/timing evidence, operation outcomes, and restoration review.
- Added typed, read-only X3800H HTTP information cards and channel layouts to
  the desktop Dashboard, including automatic connected/on refreshes.
- Added the Version 3 Phase 3 clean-architecture refactor plan and automated
  workspace boundary checks.

### Changed

- Completed Version 3 Phase 5: canonical receiver monitoring/control
  correctness, deterministic headless verification, structured lower-layer
  logging, and AVC-X3800H live validation are now released; Quick Select/EQ
  writes remain explicitly deferred and gated.

- Desktop logs now use a 3 MiB budget, remove files dated three days or older,
  and mute Iced framework output by default.

- Migrated the CLI to the canonical async receiver service and removed the
  legacy synchronous TCP client; operation output is evidence-based and local
  resource-version compare-and-set is no longer accepted.

- Completed Version 3 Phase 4: Apple Silicon macOS distributable bundle
  packaging is available through `make package-macos`.
- Completed the workspace cutover: session contracts now live in application
  ports, the legacy root source tree is removed, and delivery packages do not
  encode receiver wire commands directly.
- Narrowed the application crate root to the coordinator facade, extracted
  supplemental receiver policies and GUI presentation modules, and unified
  CLI/controller control admission so listening-mode changes are not
  incorrectly suppressed as no-ops.
