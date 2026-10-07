# Version 4 roadmap

**Planned — 2026-10-07. Scope settled with the project owner the same day. No
milestone has started; no version change is made until the release
milestone.**

This roadmap sequences the work needed to implement the target design in
[Planned architecture](../planned-architecture.md). It does not restate that
design. Where it names a rule, the section of the design that owns it is
linked. Text moves from the design into [ARCHITECTURE.md](../../ARCHITECTURE.md)
as each milestone lands, as the design requires.

Each milestone is one phase. When a milestone starts it gets a paired
`phase-N-{theme}-overview.md` and `phase-N-{theme}-architecture.md` in this
directory, per [contributing](../contributing.md#documentation-maintenance).

## Scope

Decided with the project owner on 2026-10-07.

| Question | Decision |
| --- | --- |
| Quick Select recall and EQ status (D1) | Removed in milestone 2 |
| Agent hosts | OpenClaw in a UTM guest uses `mcp-http`; agents on the Mac use `mcp-stdio`. No `mcp-stdio` runs in the guest. |
| 4.0.0 contents | Milestones 1–6, 8, and 10. Approval (7) and OAuth (9) follow in later releases. |
| Agents without approval | Allow-only. Anything that would need approval ends as `approval_unavailable`. |
| Platforms | macOS only. Windows and Linux are not supported in 4.0.0; 3.0.0 is their last release. |
| Checking platform code | Manual checks; no CI. |
| Same-machine agent account | A dedicated macOS account runs the agent host |
| Receivers | One AVR-X3800H. The API is shaped for several; one is built and tested. |
| Policy limits | The owner's configuration, not shipped defaults: hard limit -20.0 dB, approval above -30.0 dB, step limit 6 dB, budget 10 dB per 10 minutes, no time-of-day rules |
| Agent write tools | All of the design's: `set_power`, `set_source`, `set_volume`, `set_mute`, `set_sound_mode`, and `cancel_operation` |
| Running the server | Started by hand, from the GUI or a terminal. No launchd in 4.0.0, as the design states. Agents see "receiver service unavailable" while it is down. |
| UTM network | Shared Network (NAT). `mcp-http` binds the vmnet gateway address, as in the design's reference deployment. This is fail-open (see Threat assumptions). A Bridged guest VLAN replaces it if S4 finds the router can do it. |
| Guest TLS trust | A private CA installed in the guest; milestone 8 documents issuing the certificate |
| `mcp-http` account | A dedicated service account, following the existing guide |
| Agent hosts tested | OpenClaw in the guest over `mcp-http`; Claude Code on the Mac, under the dedicated account, over `mcp-stdio` |
| Claude Desktop | Not connected to the receiver. It would run under the owner's account, where policy cannot bind it. |
| Live receiver | Available at any time, so each milestone's live checks run at its exit |
| Git workflow | A `v4/phase-N-{theme}` branch per milestone, merged to main when its exit criteria pass |
| Control-service port | Three traits: receiver reads, operation control, and operator administration. A read-only agent gets the first, a writing agent the first two, the GUI and CLI all three. |
| Receiver connection | On demand, released after a configurable idle time with no subscribers and no operation in flight. This frees the receiver's single control connection for other tools. |
| Receiver configuration file | A multi-receiver YAML schema. The single-receiver file is still read, then rewritten in the new schema with a one-time backup. |
| CLI with no server running | Fails with a clear message, as the design words it. It does not start a server. |
| Claude Code's tier | Read-only until S4 passes. Then it widens to the same `Allow` operations as OpenClaw. |
| S4 timing | In parallel with milestone 1 |
| Working mode | One numbered step at a time: run `make check`, review the diff, then continue |
| Timeline | No deadline. The sequence stands as written. |

Consequences for 4.0.0:

- Volume above the -30 dB ceiling needs approval, which is unavailable, so the
  ceiling is the highest level an agent can set. The -20 dB hard limit takes
  effect once approval ships.
- Agents cannot change system or Zone 2 power. They can power the main zone on
  only when the observed volume is known and at or below -30 dB.
- The design's cross-platform GUI, named-pipe transport, and Windows access
  control text no longer apply to this release. Milestone 1 amends the design
  to say so.

## Threat assumptions

Both agent hosts are untrusted and can run commands. OpenClaw runs shell by
design, and Claude Code does too. This sets what 4.0.0 can honestly claim.

- The gate, policy, and audit guard the MCP path. The boundary against an agent
  that leaves that path is where the agent runs and what it can reach, and the
  receiver's Telnet and HTTP ports have no authentication.
- `api-server` must reach the receiver, so anything that shares the Mac's network
  identity can too.
- **Guest in Shared Network (NAT):** the router sees guest traffic as coming
  from the Mac, so the only block is a host `pf` rule. The research note on
  [UTM guest LAN isolation](../research/utm-guest-lan-isolation.md) finds it
  fails open, because Apple does not support `pf` as an API and macOS can flush
  the rules silently. Fail-closed needs a Bridged guest on a VLAN or an
  isolated Wi-Fi, with the block on the router.
- **Claude Code under the dedicated account:** it shares the Mac's kernel and
  network identity, and the `pf` user rule that would keep it from the receiver
  is unvalidated. This is the lower-assurance path, and the owner chose it over
  moving Claude Code into a guest.
- A `pf`-presence check can stop `mcp-http` serving the legitimate route when
  the rules vanish. It does not stop direct access, so it is not fail-closed.
- Only a receiver-side limit bounds damage after a full bypass; S4 tests whether
  the X3800H has one.

## What the code review changed

The design assumes a starting point that the code does not quite match. These
findings drive the order below.

1. **Two session stacks exist.** The CLI opens its own `X3800hSession`
   (`apps/cli/src/main.rs`, with a constant `OperationId(1)`). The GUI reaches
   the same session through the legacy `ReceiverController`
   (`crates/application/src/controller.rs`, 1,857 lines) and a compatibility
   adapter (`crates/infrastructure/src/canonical_factory.rs`). About 120 lines
   in the GUI crate's root module and bridge mention the controller's events,
   snapshots, or bridge types. Retargeting the GUI twice, first to the
   canonical session and then to the API, would repeat that migration, so it
   happens once against a port.
2. **The receiver is treated as a single-Telnet-client device.**
   `X3800hSession`'s connection guidance tells the user to make sure no other
   Telnet client is connected. When the Control API server owns the connection,
   the GUI and CLI must already be its clients. The order is therefore: port, then
   server, then both Operator clients together, then MCP.
3. **The canonical contract covers seven fields.** The GUI also shows the
   source catalog, Quick Select names, and HTTP information, which today flow
   through the adapter. The design's session contract, control-service port, and
   API table have no home for them, and the `list_sources` tool needs the
   source catalog. This is a design gap and is closed in milestones 1 and 4.
4. **Quick Select recall and EQ status are unreachable in 3.0.0.** The
   X3800H profile sets `quick_select_recall` and `eq_status` to false, the
   desktop never supplies a validated profile, and the compatibility adapter
   does not implement either. Their only implementations are in the legacy
   `AvrSession` trait impl. Recall is also a write outside `ReceiverIntent`
   that can change volume, so it would bypass the volume rules. The owner chose
   to remove both (D1).
5. **The precondition must cover more than the target field.** The default
   policy makes main-zone power-on and unmute depend on volume, but the
   session's preflight in `operate` re-observes only the target field. The
   precondition therefore has to carry every field a matched rule consulted.
6. **Receiver identity is not stable.** The CLI derives `ReceiverId` from the
   host; the factory uses the friendly name or the host. API paths, the budget
   ledger, approval digests, and the audit log all key on it, and the Agent view
   must never expose addresses.
7. **The boundary script cannot check what the design asks.** It uses
   `cargo metadata --no-deps`, which sees direct edges only. The "no transitive
   dependency on `protocol`" and "no TLS in `mcp-stdio`" rules need the resolved
   graph.
8. **A v3 gate depends on legacy code.** `make check` runs
   `tools/check-phase5-ledger.sh`, which requires `crates/gui-lib/tests/async_apis.rs`.
   That test is written against the legacy `ReceiverSession`, `SessionFactory`,
   and `MainZoneControl`.
9. **The repository has no CI configuration.** Version 4 adds Unix-socket and
   owner-only permission code. With macOS the only supported platform, the
   checks stay manual.

## Sequencing rules

- Approval ships after 4.0.0, so everything in 4.0.0 is fail-closed: an Agent
  operation that needs approval ends as `approval_unavailable`, and agents can
  run only `Allow` operations.
- Audit lands with the gate, before any Agent write path exists. Audit failure
  rejects Agent writes, and the budget ledger is rebuilt from audit, so the
  audit schema records every dispatched volume value.
- Milestones 4 and 5 are not released separately. Milestone 4 adds a server
  nothing uses; milestone 5 switches both clients to it.
- The workspace version stays 3.0.0 until milestone 10. Version 3 phases did
  not bump per phase either.
- Each milestone updates `make boundary` for the edges it adds, and the
  milestone is not done until `make check`, `make clippy`, and `git diff --check`
  pass.
- Each milestone is developed on its own branch and merged to main only when
  its exit criteria pass, so main stays releasable.

## Overview

| # | Phase theme | Delivers | In 4.0.0 | Blocked by |
| --- | --- | --- | --- | --- |
| S | Spikes (parallel, time-boxed) | MCP SDK graph check, Agent endpoint access on macOS, approval wire contract, receiver reachability and backstop | S1, S2, S4 | none |
| 1 | `control-port` | Stable receiver id, control-service port, in-process service with the Operator gate, inspection reads, session precondition, CLI on the port | yes | none |
| 2 | `gui-on-port` | GUI on the port; legacy controller, traits, and adapter deleted | yes | 1 |
| 3 | `policy-audit-gate` | `policy` crate, audit log, Agent path through the gate, fail-closed | yes | 1 |
| 4 | `control-api-server` | `api-contract`, `api-client`, `apps/api-server`, both endpoints | yes | 1, 3, S2 |
| 5 | `operator-cutover` | CLI and GUI become API clients; server lifecycle; macOS bundle | yes | 2, 4 |
| 6 | `mcp-stdio` | `mcp-tools`, `apps/mcp-stdio`, conformance suite | yes | 5, S1 |
| 7 | `approval` | 7a broker and fake channel; 7b External Approval Service adapter | after | 5; 7b needs S3 |
| 8 | `mcp-http` | `apps/mcp-http`, TLS, static tokens, UTM guest deployment | yes | 5, `mcp-tools` from 6 |
| 9 | `oauth` | `apps/auth-server`, token exchange | after | 8, decision 4 |
| 10 | `release-4-0` | Version bump, documentation promotion, archive, validation records | yes | 1–6, 8 |

Milestone 3 depends only on the domain and application layers, so it can run in
parallel with milestone 2. Milestone 8 can start once `mcp-tools` exists, so it
does not wait for the stdio build.

## Spikes

Short research tasks whose output is a note in `docs/research/` or a decision
recorded in the owning phase document. None changes shipped code.

- **S1 — MCP SDK and revision** (design open decision 3). Build a throwaway
  crate with the official Rust SDK's stdio feature only and run
  `cargo tree -p` on it. The acceptance test is the one milestone 6 will
  enforce: no TLS library and no HTTP server framework. Also confirm that the
  SDK's Streamable HTTP server can run without per-client state, and settle the
  protocol revision, since the design's `server/discover` semantics were not
  verified. The SDK's documented feature flags separate the stdio and
  Streamable HTTP transports, which makes the split plausible; the graph claim
  is unverified until this spike.
  The owner has confirmed that OpenClaw supports a remote Streamable HTTP MCP
  server with a bearer header, so no bridge is needed in the guest.
- **S2 — Agent endpoint access** (open decision 2). Decide how the dedicated
  macOS account is admitted to the Agent endpoint (group membership or an access
  list on the socket's directory) and what the server verifies about the peer
  through Unix-socket peer credentials. Windows is out of scope.
- **S4 — Receiver reachability and backstop** (open decision 5). Three checks,
  recorded in `docs/research/`, that decide how much the 4.0.0 isolation claims
  may say. It runs in parallel with milestone 1, and its result must exist
  before the milestone 6 and 8 guides are written.
  1. *Router.* Whether your router or access point can put the guest on a VLAN
     or an isolated Wi-Fi, block it from the receiver, and still let it reach
     the Mac on one port. If so, the guest uses Bridged on that network and the
     deployment is fail-closed. If not, Shared NAT with `pf` stays and is
     documented as fail-open.
  2. *`pf` on this Mac.* Whether a `pf` rule matching the dedicated account
     blocks that account's outbound connections to the receiver, and whether the
     guest-subnet rule from the research note works with UTM and survives a
     Wi-Fi toggle and a reboot. Use the research note's validation plan.
  3. *Receiver backstop.* Whether the X3800H has a volume-limit setting, and
     whether a Telnet `MV` command above the limit is clamped. This is an armed
     live test and follows the repository's write-safety controls.
- **S3 — Approval wire contract** (open decision 1). Not needed for 4.0.0; it
  starts before milestone 7. With the External Approval Service's owner: stream
  protocol, resume cursor, signature algorithm, key distribution and rotation,
  signed encoding. Output is a frozen contract and a fake service used as a test
  fixture.

## Milestone 1 — Control port and in-process service

Refactoring; no new user-visible behavior. This is the first stage. Work goes one
numbered step at a time: run `make check`, review the diff, then continue.

0. **Amend the design before coding.** Update
   [Planned architecture](../planned-architecture.md) so it matches what this
   roadmap decided, rather than letting the roadmap fork it: the inspection
   reads on the session contract, the receiver-id rule, the typed rejection
   reason that comes with the precondition (closing open decision 6), the
   removal of Quick Select recall and EQ status (D1), and the macOS-only
   platform scope: the cross-platform GUI, named-pipe, and Windows access
   control text. The API-table additions follow in milestone 4.
1. **Stable receiver id.** The id is the saved-configuration entry name. An
   ad-hoc `--host` selection gets a derived id that the Agent view never lists.
   One function in `application` derives it, and the CLI and the connector both
   use it.
2. **Control-service port** in `application`, as the design's
   [control service port](../planned-architecture.md#control-service-port)
   defines it, in three traits: receiver reads (receiver listing, state
   subscription, inspection reads), operation control (submit, status, and
   cancel of the caller's own operations), and operator administration
   (discovery, configuration, and later tokens and the approval, audit, and
   policy views). The handle is bound to a principal. `mcp-tools` takes only the
   first two, so it cannot name an Operator method. The Agent principal exists
   as a type but nothing can construct one until milestone 3. This step proposes
   the signatures for review before any implementation sits behind them.
3. **Receiver connector port.** It replaces the legacy `SessionFactory` and
   returns a `SharedReceiverSession`. Infrastructure implements it with
   `X3800hSession`.
4. **In-process control service** in `application`: a registry with one
   session per receiver, connected on demand and released after a configurable
   idle time with no subscribers and no operation in flight, and the
   **Operation Gate for the Operator principal only**. State is stale while a
   receiver is released, and a read connects and synchronizes first. The gate allocates operation ids on the server
   side, records the owning principal, honors an idempotency key and coalesces
   identical in-flight operations, follows the lifecycle table, maps session
   outcomes to `status`, `dispatch`, and `confirmed` exactly as the
   [lifecycle](../planned-architecture.md#operation-lifecycle) specifies, and
   never abandons a call into the session. Boundary rule added now: the gate is
   the only caller of `operate` outside tests and the session implementation.
5. **Inspection reads on the canonical contract.** Typed, read-only methods on
   `CanonicalReceiverSession` for the source catalog, Quick Select names, and
   HTTP information. The HTTP clients stay in infrastructure. No third session
   contract is introduced.
6. **Session precondition** (open decision 6). `OperationRequest` gains an
   optional precondition: the receiver epoch plus the observed values of every
   field the policy decision consulted, not only the target. The session
   re-observes those fields before writing and returns `RejectedBeforeDispatch`
   with a typed reason on a mismatch, so the gate can tell a precondition
   failure from other rejections without matching strings. The change must add
   no retry path and leaves the at-most-once rule untouched. Operator operations
   carry none.
7. **CLI on the port.** The CLI composes the in-process service, so it still
   links infrastructure for now. It stops constructing a session and stops using
   `OperationId(1)`. Grammar is unchanged, and the outcome is printed in the
   design's status, dispatch, and confirmed terms.
8. **Multi-receiver configuration file.** The YAML adapter reads the
   single-receiver file and the new multi-receiver schema, and writes only the
   new one, with a version field and a one-time backup of the old file before
   the first rewrite. Entry names become the receiver ids from step 1, which also
   ends the mismatch where the GUI names an unnamed entry after its host and the
   file reloads it as `default`. 3.0.0 cannot read the new file. Nothing in 4.0
   adds a second receiver through the GUI or CLI, so a multi-receiver file comes
   from hand edits until a later release adds that. This step is independent of
   the port and comes last so it does not delay the contract steps.

There is no CI. Each phase document carries a manual macOS checklist for the
checks that `make check` does not cover.

**Exit.**

- `make check` and `make clippy` pass.
- The loopback tests in `x3800h_session.rs` cover the precondition: matching,
  mismatched target, mismatched non-target field, and epoch change.
- Every `OperationOutcome` variant is tested against the status mapping.
- Gate tests with a fake session assert at most one dispatch, including a retry
  with the same idempotency key.
- With a fake connector and clock, an idle receiver is released and the next
  request reconnects. A receiver with a subscriber or an operation in flight is
  not released.
- The configuration adapter round-trips the new schema, reads the old file, backs
  it up once, and rejects duplicate or reserved names.
- Live read-only validation and the armed, state-restoring controls run pass on
  the X3800H through the CLI, with the result recorded as in v3.

## Milestone 2 — GUI on the port; legacy retirement

Refactoring; no visual change.

1. **Behavior parity table first.** Before deleting the controller, list every
   behavior it provides (power-on quiet time, Zone 2 gating, refresh ordering
   after core status, lifecycle generation tagging, partial-status handling,
   control confirmation) and name its canonical equivalent or record it as
   intentionally dropped.
2. **Retarget `gui-lib`.** The bridge consumes the port. Presentation state is
   projected from `ReceiverState` with per-field validity, operation events, and
   inspection reads. `gui-lib` keeps its application and domain edges only.
3. **Delete the legacy path:** `ReceiverController` and its policy modules
   where the parity table shows no remaining user, the legacy `ReceiverSession`,
   `SessionFactory`, `SessionEvent`, and gateway traits, and
   `CanonicalSessionAdapter`. `AvrSession` stays, because `X3800hSession` uses it
   as its transport; only its legacy trait impl goes, including the Quick Select
   recall and EQ status implementations (decision D1).
4. **Scope the domain cleanup by inventory.** The legacy `MainZone*` types are
   still used by `protocol/src/avr/command.rs` and `response.rs`. Delete only
   what no remaining consumer needs; the canonical X3800H codec is untouched.
5. **Rewrite the guards.** Flip the single-definition rule in
   `tools/check-boundaries.sh` to `CanonicalReceiverSession`. Rewrite
   `crates/gui-lib/tests/async_apis.rs` against the port, and update
   `check-phase5-ledger.sh` and its fixture list to match. Update
   `ARCHITECTURE.md`, `AGENTS.md`, and the contributing guide to name the new
   contract.

**Exit.**

- No `ReceiverController`, legacy `ReceiverSession`, or `SessionFactory` symbol
  remains.
- `make visual-baselines` shows no pixel change on macOS.
- A live GUI session, including a forced receiver reconnect, behaves as in 3.0.0.
- `make check` and `make clippy` pass.

## Milestone 3 — Policy, audit, and the Agent path (in process)

Everything here is testable without a network, and nothing is exposed yet.

1. **`policy` crate.** A pure function of typed inputs and a declarative
   configuration, depending only on `domain`, per
   [Policy Engine](../planned-architecture.md#policy-engine). The baseline is the
   epoch plus the consulted fields. Every `ReceiverIntent` variant is
   classified, and a test enumerates the variants so a new intent cannot
   compile into an unclassified state. Per-agent narrowing must be able to
   express "deny every write for this label", because Claude Code's read-only
   tier depends on it. If the design's rule syntax cannot say that without
   naming each intent, amend the design in this milestone.
2. **Policy loading** in infrastructure: YAML to a typed `PolicyConfig`,
   rejecting `unclassified: allow`, with a digest. A load failure disables Agent
   writes. Reload semantics are defined here and exposed over the API in
   milestone 4.
3. **Audit log.** A port in `application` and an append-only JSON Lines adapter
   in infrastructure, rotated by size and file count and readable across the
   retained files. Records carry principal, request, decision and matched rules,
   approval fields, dispatch certainty, outcome, and the dispatched or possibly
   dispatched volume value, which the ledger needs.
4. **Gate: Agent path.** Policy evaluation, re-evaluation at dispatch, the
   pooled cumulative-change ledger rebuilt from audit at start, `dry_run`,
   pending and rate caps, and the precondition built from the decision's
   baseline. With no broker yet, `RequireApproval` ends as
   `approval_unavailable`. Audit failure rejects Agent writes and lets Operator
   controls continue with a warning.
5. **Boundary rules:** `policy` stays free of async runtime, serialization,
   filesystem, network, and clock reads; add the `policy` edges.

**Exit.**

- Policy tests per the design's [verification](../planned-architecture.md#verification):
  a table case per rule, and properties for monotonicity in the volume target, an
  unknown or stale baseline never yielding `Allow` for a state-dependent rule,
  `Deny` never weakened by `Allow`, and alternating steps never resetting the
  budget.
- Gate tests with a fake session, approval channel, and clock cover allow, deny,
  `approval_unavailable`, a failed precondition, supersession, and idempotent
  retry. Each asserts no dispatch on every non-allowed path.
- A rule narrowed to one agent label restricts that agent and leaves another
  agent's decisions unchanged, including a read-only label.
- A restart test shows the ledger rebuilt from audit.

## Milestone 4 — Control API server, contract, and client

Additive: new packages that no shipped delivery package uses yet.

1. **`api-contract`**: the versioned `/v1` schema. Amend the design's API table
   first to add the inspection resources (source catalog, Quick Select names,
   HTTP information), and an Agent-visible sources resource for `list_sources`.
2. **`api-client`**: implements the control-service port over local transports
   only, with no TLS dependency.
3. **`apps/api-server`**: hosts the in-process service from milestone 1. It is
   the composition root for infrastructure and the only place besides
   diagnostics that constructs receiver sessions.
4. **Endpoints.** Operator endpoint in the owner-only data directory; Agent
   endpoint in a separate location, not created if its permissions cannot be
   applied. Unix domain sockets with peer-credential checks (S2). The Windows
   named-pipe transport is out of scope. Linux shares the Unix-socket code but
   is not supported or tested in 4.0.0.
5. **Credentials and processes.** First-start Operator token file; hashed,
   labelled static Agent tokens with the `/v1/tokens` routes; header-only
   bearer authentication; one server per user with an exclusive bind; a
   parent-pipe exit mode for the GUI-owned child; the health route.
6. **Boundary rules from the resolved graph:** only `infrastructure`,
   `api-server`, and `diagnostics` reach `protocol`; only `api-server` and
   `diagnostics` reach `infrastructure`; receiver sessions are constructed only
   in `api-server` and `diagnostics`; `api-client` has no TLS dependency.

**Exit.**

- Contract tests against the schema and an in-process server.
- The Agent endpoint refuses every Operator resource, with and without an
  Operator token, and audits the attempt. The Agent view carries no receiver
  addresses.
- State snapshots coalesce for a slow client, and operation events reach only
  their owner.
- Endpoint permissions are checked by hand on macOS: the dedicated agent account
  reaches the Agent endpoint and cannot reach the Operator endpoint or the data
  directory.

## Milestone 5 — Operator clients cut over

The first milestone that changes what users run.

1. **CLI** becomes an `api-client` consumer, drops its infrastructure edge, adds
   the Agent-token command group, turns `--dry-run` into a server-side policy
   evaluation, and fails with a clear message when no server is running.
2. **GUI** becomes an `api-client` consumer. It attaches to a running server or
   launches the server executable as a child that exits when the pipe from its
   parent closes. It never stops a server it did not start, and it links no
   receiver code.
3. **New views:** operation feed, pending and decided approvals (observe and
   cancel only; empty until milestone 7), audit log, and the effective policy
   with its digest and a reload action. Add macOS visual baselines; the
   Windows and Linux baselines stay as they were in 3.0.0 and are marked
   unvalidated.
4. **macOS bundle.** `tools/package-macos.sh` currently builds only the desktop
   binary. It must also build and include `api-server` inside the app bundle;
   the existing `codesign --force --deep` step then covers the nested
   executable. The receiver connection moves into that executable, and the
   recent signing fix exists because macOS Local Network permission is tied to
   a consistent signed identity. Verify that the bundled server can reach the
   receiver when the GUI launches it, and settle what a standalone server
   started from a terminal needs, before relying on either.
5. **Boundary:** `cli` and `desktop` lose the infrastructure edge. The headline
   property now holds in the Cargo graph.
6. Update the CLI and desktop user guides and promote the process model into
   `ARCHITECTURE.md`.

**Exit.**

- With the server running, the CLI and the GUI operate the receiver
  simultaneously over one receiver connection, recorded as a live validation.
- A killed GUI ends its child server. A GUI attached to a standalone server
  leaves it running on exit.
- The GUI runs on macOS. Windows and Linux are not supported in this release.

## Milestone 6 — MCP tool surface and stdio build

1. **`mcp-tools`** with the tool table, results, annotations, and instructions
   from [Shared tool surface](../planned-architecture.md#shared-tool-surface-mcp-tools);
   no transport. `list_sources` reads the new sources resource.
2. **`apps/mcp-stdio`** per the [stdio build](../planned-architecture.md#stdio-build-appsmcp-stdio):
   credential from an owner-only file named by the environment, standard output
   reserved for the protocol, logging to standard error with redaction, exit on
   standard input close, start while the server is down, no network code.
3. **Shared conformance suite** over an in-process MCP client and a fake port,
   written so milestone 8 reuses it.
4. **Boundary rules:** `cargo tree -p` on `mcp-stdio` alone contains no TLS
   library and no HTTP server framework, with the list held in the script;
   `mcp-stdio` and `mcp-tools` write nothing to standard output outside the
   transport.
5. **Setup guide for the dedicated macOS account** that runs Claude Code,
   covering the Agent endpoint permissions from S2, where the token file lives,
   and the result of S4 for the `pf` user rule. The guide states that this path
   is lower-assurance than a guest and why.

At this point agents on the Mac work, but only through `Allow` decisions.
Operations that need approval return `approval_unavailable`.

**Exit.**

- The design's stdio tests: only valid MCP messages on standard output, exit on
  input close, start with the server down, the token never printed.
- Claude Code, under the dedicated account, reads state. Its writes are refused
  by policy until S4 passes. After S4, it performs an `Allow` operation and is
  refused a dangerous one.
- From that account, a TCP connection to the receiver's control ports fails, and
  the result is recorded as pass or fail.
- With the owner's limits configured (see Scope), the agent changes volume below
  -30 dB, is refused above it, and is refused a system power-on. Without
  configured limits every volume change and power-on needs approval, so the
  smoke test would show nothing.

## Milestone 7 — Approval

Not in 4.0.0. It starts after the release, once S3 is settled.

**7a — Broker, no external dependency.**

- The Approval Broker state machine in `application`, per
  [Approval](../planned-architecture.md#approval): tickets, canonical operation
  digest, nonce, expiry on the server clock, single use, bounded re-approval
  ending in `baseline_unstable`, per-client pending and rate caps, restart
  expiry, cancel withdrawing the ticket, and server-generated summaries.
- The `ApprovalChannel` port with a fake channel.
- The GUI approvals view shows real data.
- Gate tests complete the design's list: approve, reject, expire, cancel,
  replay, mismatched digest, state change during approval, and supersession of
  an approved operation.

**7b — External Approval Service adapter, blocked on S3.** Outbound stream with
a resumable cursor and backoff, asymmetric signature verification against the
public keys file with key ids, and `approval_unavailable` while no stream exists.
Tested against the S3 fake; integrated against the real service when it exists.

**Exit.** End to end, an agent request needing approval is approved outside the
host, dispatches exactly once, and reports `confirmed`. A replayed, expired,
mismatched, or unverifiable decision is ignored and audited.

## Milestone 8 — MCP Streamable HTTP build and deployment

This is the path for OpenClaw in the UTM guest. The guest uses static-token mode
over TLS; OAuth is not needed.

1. **`apps/mcp-http`** per the [HTTP build](../planned-architecture.md#streamable-http-build-appsmcp-http):
   TLS terminated by the server and refused without it, one bound address with
   wildcard opt-in and retry for an unassigned address, header-only static
   tokens, per-source rate limiting of failed authentication, `Origin` and
   `Host` validation, body, connection, and in-flight caps, timeouts, and
   optional mutual TLS and source allowlist. No state between requests.
2. The shared conformance suite runs over this transport, plus the design's HTTP
   tests.
3. **Deployment.** Remove the "not yet implemented" marker from
   [MCP HTTP service account](../mcp-http-service-account.md) once it is
   exercised. Record the UTM reference deployment as a validation record: the
   guest reaches the MCP port and cannot reach the receiver, with the `pf` rules
   tested rather than assumed. This is the evidence for open decision 5, not code.
4. **Dedicated service account.** `mcp-http` runs under the account the linked
   guide describes, which reaches only the Agent endpoint and its own TLS key.
5. **Private CA.** Document creating the CA, issuing a certificate that covers
   the vmnet gateway address, installing the CA in the guest, and renewing the
   certificate without touching the guest.

**Exit.**

- The design's HTTP tests pass.
- OpenClaw in the guest connects with a bearer header, reads state, and performs
  an `Allow` operation, and a dangerous one is refused.
- The deployment record shows pass or fail from the real guest, using the
  research note's checks: the receiver and the Mac's SSH port are unreachable,
  and the MCP port is reachable. The checks are rerun after every reboot, Wi-Fi
  change, and macOS update, and the record says whether the network is
  fail-closed or fail-open.

## Milestone 9 — OAuth mode (not in 4.0.0)

`apps/auth-server`, Protected Resource Metadata, token exchange, the registry
export, and signed-token acceptance on the Agent endpoint. It is blocked on open
decision 4, which needs the targeted MCP revision, the token-exchange RFC, and
the headless-agent grant read and settled first. It is outside 4.0.0 by the
owner's decision; static tokens already satisfy the design's goals for a first
release.

## Milestone 10 — Release 4.0.0

- Bump workspace packages from 3.0.0 to 4.0.0.
- Confirm every implemented section has moved from the design into
  `ARCHITECTURE.md` and is removed from `planned-architecture.md`; whatever
  remains there is the deferred work (milestones 7 and 9).
- Move the version 4 phase documents to `docs/archive/v4`, add release notes and
  a changelog, and update the documentation map.
- State in the README and user guides that Windows and Linux are not supported
  in 4.0.0 and that 3.0.0 is their last release.
- Run `make package-macos` and a macOS smoke test with the CLI, the GUI, an
  agent on the Mac, and the guest agent together.
- Record live receiver validation and the deployment checks in the archive.

## Open decisions and where they land

| Design open decision | Milestone | Kind of output |
| --- | --- | --- |
| 1 Approval wire contract | S3, then 7b (after 4.0.0) | Contract and fixture |
| 2 Agent endpoint access | S2, then 4 | macOS mechanism |
| 3 MCP revision and SDK | S1, then 6 | Note, then code |
| 4 OAuth details | 9 (after 4.0.0) | Design, then code |
| 5 Same-machine receiver isolation | S4, then 6 and 8 | Research note, then validation records |
| 6 Session precondition | 1 | Code and tests |

Gaps found by the review and the milestone that closes each:

| Gap | Closed in |
| --- | --- |
| Inspection reads have no home in the contract, port, or API | 1, 4 |
| Quick Select recall is an unclassified write | 2, by removal (D1) |
| Receiver id is host-derived | 1 |
| Audit must record dispatched volume values | 3 |
| Boundary script sees direct edges only | 3, 4 |
| Phase 5 ledger depends on the legacy GUI test | 2 |

## Open items

Nothing here blocks milestone 1.

- The default idle time before a receiver is released, and whether an Agent read
  alone keeps it connected. Settled in step 4.
- Whether your router can do a guest VLAN, which S4 answers.
- When the External Approval Service will exist. It sets the start of
  milestone 7b and does not affect 4.0.0.
- Whether Linux is worth supporting later, since the Unix-socket code is shared.

## Risks

- **Milestone 2 is the largest refactor.** The mitigations are the behavior
  parity table, the unchanged visual baselines, and doing the port before
  touching the GUI.
- **The precondition touches the session actor,** which carries the
  at-most-once guarantee. Keep the change to the preflight and add no retry.
- **The MCP revision is uncertain.** The design targets a revision whose
  discovery semantics were not verified; S1 runs before milestone 6 is planned
  in detail.
- **macOS Local Network permission may not follow the receiver connection into
  the server process.** The 3.0.0 signing fix shows how a mismatched identity
  silently blocks every receiver connection. Check this in milestone 5, before
  the packaging work is considered done.
- **Both agent hosts are untrusted and can run commands, and the receiver has no
  authentication.** In 4.0.0 the guest's isolation is fail-open unless S4 finds a
  router that can do better, and Claude Code's rests on an unvalidated `pf` user
  rule and a shared kernel. The gate does not help against an agent that goes
  around it. The release notes must say so, and S4's receiver backstop is the
  only thing that bounds the damage.
- **The policy ships no numeric limits.** The owner's limits are configuration.
  Until they are set, agents need approval for every volume change and
  power-on, which in 4.0.0 means they are refused.
- **Agents are allow-only in 4.0.0.** The -20 dB hard limit is inert until
  approval ships, and an agent cannot change system or Zone 2 power. The release
  notes must say so, so a refusal is not mistaken for a fault.
- **Dropping Windows and Linux strands those users on 3.0.0.** The README, user
  guides, and visual-baseline records must say so, and the Unix-only code needs
  a clear failure on Windows rather than a confusing build error.
- **The External Approval Service is outside this repository.** It gates
  milestone 7b only, after 4.0.0.
