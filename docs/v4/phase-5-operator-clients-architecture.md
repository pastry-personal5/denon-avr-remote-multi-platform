# Version 4, Phase 5 — Operator clients cut over, architecture

This document fixes the types and rules that
[Planned architecture](../planned-architecture.md) and the
[roadmap](roadmap.md#milestone-5--operator-clients-cut-over) leave to phase 5. It does
not restate the design; where a rule is owned there it is linked. Text that is
implemented moves into [ARCHITECTURE.md](../../ARCHITECTURE.md) when the milestone's exit
criteria pass.

It was written on 2026-10-09 from the code at the head of `main` (`79fd1dd`) and changes
no code. Step 0 of the [overview](phase-5-operator-clients-overview.md) amended the
roadmap and the design where the review found them wrong.

"Phase 5" here is the fifth milestone of version 4. The Makefile target `phase5-ledger`
and `tools/check-phase5-ledger.sh` belong to version 3's fifth phase and are unrelated.
No target added by this phase has "phase5" in its name.

## Starting point

- Both delivery packages already speak only the control-service port. The CLI works
  through `&dyn OperatorControl` (`apps/cli/src/{commands,target}.rs`) and the GUI
  through `GuiServices { control: SharedOperatorControl, shutdown: ShutdownHook }`
  (`crates/gui-lib/src/bridge.rs`). What each composes is one expression, in
  `apps/cli/src/main.rs` and `apps/desktop/src/main.rs`:
  `ControlService::new(X3800hConnector, YamlConfigRepository::default(),
  SsdpDiscoveryAdapter, ServiceConfig::default())`. `ControlService::new` has no audit
  log and no Agent path (phase 3, D6).
- Those four constructors, and `DEFAULT_DISCOVERY_TIMEOUT` (`apps/cli/src/{target,commands}.rs`),
  are the only `infrastructure` names in `apps/cli/src` and `apps/desktop/src`.
  `apps/desktop/src/logging.rs` imports none and chooses its own log directory
  (`~/Library/Logs/Denon AVR Remote` on macOS).
- `ApiClient` implements the whole port over a Unix socket and opens one connection per
  request, so one built from a socket path and a token keeps working across a server
  restart. It takes `Token::new(text)`. Nothing reads the Operator token file, and the only
  definition of the default data directory is `infrastructure::data_directory()`, which
  falls back to the literal path `~` when `HOME` is unset.
- `ApiClient::save_configuration` asks for the configuration's `ETag` and then sends it
  as `If-Match` in the same call. The route compares `etag_of` (SHA-256 of the canonical
  JSON, computed in `apps/api-server/src/routes.rs`) under `config_lock`. So two clients
  cannot interleave inside one call, but a client that read the configuration earlier can
  still write over a change made since.
- The server takes `--data-dir DIR` and `--exit-with-parent` (it shuts down when
  standard input ends), exits with status 75 when another server holds the lock, prints no
  readiness line, and logs with `tracing` to standard error. Its Operator endpoint answers
  `GET /v1/health` only to a caller with the token, so a caller with no token learns
  nothing but that something accepted the connection.
- `ControlError::Unavailable` means "the service, or the receiver behind it, cannot be
  reached" (`control.rs`). Over the wire a refused connection and a receiver that is down
  can both reach a client as `Unavailable`, so a client cannot tell the server's absence
  from the receiver's by the variant.
- The GUI's shutdown hook runs on window close and on `Message::Shutdown` (which no view
  sends; only tests do), waits up to `SHUTDOWN_GRACE` of 3 seconds, and today closes the
  in-process service. Boot calls `control.configuration()` first. The server's own shutdown
  grace is 5 seconds.
- The navigation rail has five routes (Dashboard, Receivers, Settings, Advanced,
  Diagnostics) and appears on every screen but the launch animation. `Advanced` holds one
  panel, "Receiver status". `tests/visual-baselines/macos` holds 14 PNGs (seven scenarios
  at 100% and 200%); no other platform has ever had a baseline.
- `OperationSnapshot` carries no principal. `AuditRecord.principal` does.
- `main` contains phases 1 to 4 and the clean-architecture refactor, which changed the
  session actor's parameters (`Feed`). Phase 4's armed live run and its account-boundary
  run have not been done.

## What the review changed

The roadmap's milestone 5 and the design had these faults. Each is fixed in the design
(step 0) or in this document.

| # | Fault | Fix |
| --- | --- | --- |
| R1 | The roadmap has the CLI and GUI "become `api-client` consumers", but nothing in the workspace can find the server or read the Operator token outside `infrastructure`; the data directory is defined there, and its fallback for an unset `HOME` makes a directory named `~` | A client bootstrap (step 2): one definition of the data directory beside `EndpointPaths`, a missing `HOME` an error, and a token-file reader that refuses a file others can read (D51, D52) |
| R2 | Phase 4's `ETag` guarantee (D36) does not cover a caller's earlier read, because `ApiClient::save_configuration` fetches its own `ETag` just before it writes. The CLI rewrites the configuration after every command, and this phase's exit has the CLI and GUI running together | The revision goes through the port (D44, the owner's choice): `configuration()` returns it, `save_configuration` takes the one it was read under, and a stale write fails with a conflict. One helper reads, edits, and saves, and skips a write that changes nothing (D48 to D50) |
| R3 | "Attaches to a running server or launches the server executable" has no contract: no readiness signal exists, two GUIs can start at once, a child's standard error that nobody drains stalls it, and the executable's location is unspecified | The launcher's algorithm (D53 to D58): probe the socket, spawn with an explicit data directory, wait on health, treat exit status 75 as "attach", send the child's standard error to a file, find the executable beside the GUI's own |
| R4 | `GET /v1/health` needs the token, and the first server start is what creates it, so a GUI that has never run a server cannot probe by health | The probe is a plain connect first. A socket that accepts means the token file exists, because the server opens the token store before it binds. Health is read only after that, with the token (D58) |
| R5 | `--dry-run` "becomes a server-side policy evaluation", but the Operator's own dry run is always `Allow` because the Operator skips policy (phase 3) | `--dry-run` alone reports that; `--dry-run --as-agent LABEL` is the server's `dry_run_as`, which is the evaluation worth running (D64) |
| R6 | "A killed GUI ends its child" holds only if the GUI holds the write end of the child's standard input for its whole life and nothing else keeps it open | The pipe is held by the launcher's value, never cloned or inherited by a second child, and a test kills the GUI process and watches the child go (D55) |
| R7 | The packaging step assumes `codesign --force --deep --identifier` gives the nested server the bundle's identity. The 3.0.0 fix (`25401e7`) shows what a mismatch costs: macOS Local Network permission is keyed to one identity while the kernel checks another, and every receiver connection fails silently | S5 is a hand check before packaging is designed (step 0b). The script signs the nested executable first and the bundle after it, and the identity it gives each is S5's output: **B**, the server as `com.denonavr.remote.server` (S5, 2026-10-10, D80) |
| R8 | The approvals view has no route and no port method (D29), and the External Approval Service is not built. ApproveHub, which the owner is building, is not yet usable for requests of the kind Denon needs approved | A static panel that says approval is not available and where refused requests show (D45). No contract or port change; milestone 7 and S3 stay after 4.0.0 |
| R9 | The roadmap puts the new views in the GUI without saying where. Any new rail item redraws all 14 existing baselines | The views live inside `Advanced`, under a segmented control; the rail does not change (D46) |
| R10 | The roadmap says to keep the Windows and Linux baselines "marked unvalidated"; none exists | Dropped. The documents say macOS only. `tools/check-visual-baselines.sh` is left as it is |
| R11 | `ControlError::Unavailable` cannot tell a stopped server from an unreachable receiver, so a GUI that shows "server unavailable" on every `Unavailable` would blame the wrong thing | The server link probes the socket when a call fails with `Unavailable`; the GUI says the server is unavailable only when the probe also fails (D61) |
| R12 | The design leaves open what a GUI does when the server it was using disappears. Restarting one silently would undo an owner who stopped it to cut agent access | The GUI says so, keeps trying to re-attach, and starts a server only when the owner presses a button (D47, the owner's choice) |
| R13 | The CLI no longer frees the receiver's control connection when it exits (the server keeps it for the 60-second idle time), and the "close the receiver connection" action means something different when the server is not the GUI's | The owner accepted the idle time and asked for a `release` command (D77). Both are documented as behavior changes in the user guides and the known limits; `Message::Shutdown` releases the GUI's own leases, and ends the server only if the GUI started it (D60) |
| R14 | Phase 3 said the CLI and GUI are unaudited until milestone 5 (D6) and phase 3's D7 puts Operator volume changes in the agent budget. From this phase both are true of everything they do | The text in `ARCHITECTURE.md`, the planned design, and the user guides is corrected at promotion; the Audit view makes it visible |
| R15 | Phase 4 left one check for this phase: whether the GUI's projection and its `FieldBaseline` guard treat a state rebuilt from the wire as they treat the original, which `api-contract` cannot test because it cannot depend on `gui-lib` | A round-trip test in `gui-lib` with `api-contract` as a dev-dependency (D67) |
| R16 | Phase 4's armed live run is open and now applies to the refactored session actor. A cutover built on a server that was never run against the receiver would find its defects in the users' hands | Step 0c: the owner's live runs gate the CLI cutover and everything after it, and the merge (D73) |

## Packages and edges

Before and after, for the packages this phase changes:

```text
cli      → application, domain, infrastructure          before
cli      → api-client, application, domain              after
desktop  → application, domain, gui-lib, infrastructure  before
desktop  → api-client, api-contract, application, domain, gui-lib   after
gui-lib  → application, domain                          unchanged
```

`gui-lib` gains `api-contract` as a **dev-dependency** only (D67). `cli` and `desktop` gain
`denon-avr-api-server` as a dev-dependency with its `testing` feature (see
[Test support](#test-support)). The boundary script reads normal edges; the rules below
add checks that these dev edges stay out of the normal graph.

`desktop` names `api-contract` for `EndpointPaths` and the data directory (the launcher
needs the run directory); `cli` reaches both through `api-client`, which re-exports
nothing and offers `Endpoint::operator(&Path)` (see [Client bootstrap](#client-bootstrap-step-2)).

After this phase the headline property holds in the resolved graph: only `api-server` and
`diagnostics` reach `infrastructure` and `protocol`.

## Configuration revision (step 1)

This is the one change to phase 4's code. It is small on the wire (nothing changes there)
and wide in the tree, because the port's two methods change signature.

**Types, in `application`.**

```rust
/// Opaque. Equal revisions mean equal configurations.
pub struct ConfigRevision(String);                       // 16 lower-case hex digits
impl ConfigRevision { pub fn of(&ConfiguredReceivers) -> Self; pub fn parse(&str) -> Option<Self>; pub fn as_str(&self) -> &str }

pub struct RevisedConfiguration { pub configuration: ConfiguredReceivers, pub revision: ConfigRevision }

// OperatorAdmin
fn configuration(&self) -> BoxFuture<'_, Result<RevisedConfiguration, ControlError>>;
fn save_configuration<'a>(&'a self, configuration: &'a ConfiguredReceivers, base: &'a ConfigRevision)
    -> BoxFuture<'a, Result<ConfigRevision, ControlError>>;
```

`ConfigRevision::of` is FNV-1a over 64 bits of a length-prefixed encoding of the current
receiver name, each saved receiver (name, host, model, friendly name), and each receiver's
sound-mode favorites, in the order the maps already keep. It is a detector for a lost
update and not a security boundary, so it needs no hash crate in `application` (D48). The
function destructures `ConfiguredReceivers` and `ReceiverIdentity` without `..`, so a field
added later is a compile error and not a field that silently falls outside the revision. The
wire's `ETag` is the revision in quotes.

**Behavior, in the service.** `save_configuration` takes a lock held across the
comparison and the save (replacing the server's `config_lock`), loads the stored
configuration, and compares its revision with `base`. A mismatch is
`ControlError::ConfigurationChanged` and writes nothing. A match validates, saves,
retires sessions whose address changed (as today), re-reads what was stored, and returns
its revision, so a saving client holds the revision of the file as written and not of what
it sent. A hand edit of the file while the server runs changes the revision at the next
read, so it is caught as well.

**Wire.** `GET /v1/config` sets `ETag` from the revision. `PUT /v1/config` requires
`If-Match` (428 without), parses it with `ConfigRevision::parse` (400 for text that is
not a revision), and calls the port. `ConfigurationChanged` maps to 412
`precondition_failed`, and `ApiClient` maps a 412 back to it. `ApiClient::configuration`
keeps the `ETag` it was given; `save_configuration(config, base)` sends `base` as
`If-Match` and does not fetch another.

**The helper, in `application`.**

```rust
pub async fn edit_configuration(
    admin: &dyn OperatorAdmin,
    edit: impl FnMut(&mut ConfiguredReceivers) -> bool,   // true if it changed something
) -> Result<EditOutcome, ControlError>;

pub enum EditOutcome { Unchanged(RevisedConfiguration), Saved(RevisedConfiguration) }
```

It reads, applies `edit` to a copy, returns `Unchanged` without writing when `edit`
reports nothing changed or the result equals what was read, saves with the revision it read,
and on `ConfigurationChanged` reads again and applies `edit` again, up to three attempts, then
returns the error. `edit` must be a function of the configuration it is given and nothing it
captured from an earlier read. The CLI's `remember` and every configuration save in
`gui-lib` (`setup.rs`, `sound_mode.rs`, the setup flows in `lib.rs`) go through it.

## Principal on the operation snapshot (step 1b)

The owner wants the Operations feed to say who acted (D78). The service already knows: the
operation table stores each entry's owner, and `publish` filters events by it. So the owner
becomes part of the value:

```rust
pub struct OperationSnapshot { /* … */ pub principal: Principal }   // Operator, or Agent(label)
```

- **Service.** The snapshot is built with the submitting handle's principal (the one site in
  `service/operations.rs` that builds an `OperationSnapshot` for a new operation), and every
  later snapshot of the operation keeps it.
- **Wire.** `OperationDto` gains `principal: PrincipalDto`, the type the audit records already
  use, on both endpoints. An Agent sees only its own operations and events, so the value is its
  own label, which it holds already; the Agent view still carries no address, path, raw frame,
  or error text, and the leak tests gain a case that the principal of every operation an Agent
  can read is that Agent's. The field is required, with no default: no release has ever shipped
  a server, so there is no older server to be compatible with, and the contract version stays 1.
  The goldens for operation DTOs are regenerated.
- **Client.** `TryFrom<OperationDto> for OperationSnapshot` reads it, so an Agent client and an
  in-process Agent handle return the same snapshot and the conformance run needs no exception.
- **Callers.** Eighteen sites build an `OperationSnapshot` literal (the service, the fixtures of
  the Agent tests, the GUI's `FastPort`, `feedback.rs`, and tests). They gain the field.
- **Not a tool result.** Milestone 6's `mcp-tools` maps a snapshot to a tool result and must not
  copy the principal into it; that is that milestone's mapping to write, and its tests will say so.

## Receiver release (step 1c)

The owner accepts that the server keeps a receiver for the 60-second idle time after a CLI
command, and asked for a CLI command that frees it at once (D77). The service already has the
mechanism: `release_if_idle` closes a session under the slot lock when no lease is held. The
new method is its manual form.

```rust
// OperatorAdmin
fn release<'a>(&'a self, receiver: &'a ReceiverId) -> BoxFuture<'a, Result<ReleaseOutcome, ControlError>>;

pub enum ReleaseOutcome {
    Released,                                    // there was a session, and it is closed
    NotConnected,                                // there was none
    InUse { leases: usize, operations: usize },  // something holds it; nothing was closed
}
```

- **Service.** `identity(receiver)` first, so an unknown receiver is `NotFound`. With no slot, or a
  slot holding no session, `NotConnected`. Otherwise it takes the slot lock; a session with any
  lease (a read, a held state subscription, an operation in flight) is `InUse`, with the counts;
  one with none is taken out and closed as `release_if_idle` does, and the result is `Released`. It
  **never closes under a lease** and has no force option: the GUI's subscription would only
  reconnect at once, and an operation in flight must keep its outcome (D79).
- **Wire.** `POST /v1/receivers/{id}/release`, no body, Operator endpoint only (a row in the route
  table with the Operator audience, so the generated refusal matrix covers it). `200` with
  `{"result":"released"}`, `{"result":"not_connected"}`, or
  `{"result":"in_use","leases":N,"operations":M}`. `InUse` is a result and not an error, because the
  CLI words it.
- **Not audited.** It sends nothing to the receiver and reads no policy state, and the idle release it
  stands in for is not audited either.
- **CLI.** See [The CLI](#the-cli-step-3). The command waits up to 20 seconds for a lease to end
  before it reports `InUse`, because the lease of the command just before it may still be closing:
  the server finds a client that closed its end of an event stream by its next write, within 15
  seconds, and that is when the lease drops (phase 4, D35).

## Client bootstrap (step 2)

**The data directory** has one definition, `api_contract::paths::default_data_directory()
-> Option<PathBuf>`: `$HOME/Library/Application Support/Denon AVR Remote`, and `None` when
`HOME` is unset or empty (D51). It reads the environment and nothing else, so
`api-contract`'s rule against `std::fs` still holds. The non-macOS branch is dropped:
`api-client` and `api-server` already refuse to build off Unix and the product is macOS only.
`infrastructure::data_directory` loses its path functions: once steps 3 and 5 have removed
the CLI's and the GUI's `YamlConfigRepository::default()`, that, `default_config_path`,
`audit_directory`, and `policy_path` have no caller outside the module and its tests (a
search on 2026-10-09 found none for the last two), and the server builds its paths from
`--data-dir` as it does today.
`ensure_private_directory` stays. The server's default for `--data-dir` becomes
`default_data_directory()`, and a `None` is a usage error that names `HOME`.

**The Operator endpoint.** `api-client` gains

```rust
impl Endpoint {
    /// The Operator endpoint of the server whose data directory is `dir`.
    pub fn operator(dir: &Path) -> Result<Endpoint, BootstrapError>;
}
pub fn read_operator_token(path: &Path) -> Result<Token, TokenFileError>;
```

`read_operator_token` takes `lstat` first and refuses, with a message that names the file
and what is wrong, anything that is not a regular file owned by the caller's effective uid,
has a mode with any group or other bit, is empty, is over 256 bytes, or holds characters a
bearer token cannot (D52). The caller's uid comes from `rustix::process::geteuid`
(`rustix` 1.x is already in `Cargo.lock`; the step checks the feature name against its
documentation). Only then does it open the file, so a file replaced between the two calls
is not read as another's. The result is a `Token`, whose `Debug` prints `<redacted>`.

`BootstrapError` has four cases, each with the text a person acts on: no data directory
(`HOME`), the socket path over the platform's limit (the existing `ConnectError`), the
token file missing, and the token file unsafe or unreadable. The CLI's "no server is
running" message is the first of these when the socket does not accept: *"No Denon AVR
Remote server is running (looked for `<socket>`). Start the Denon AVR Remote app, or run
`denon-avr-api-server`."* The socket path is the Operator's own, so it is safe to print.

**The discovery timeout.** `DEFAULT_DISCOVERY_TIMEOUT` (5 seconds) moves to
`application::ports` beside the discovery port; `infrastructure` and the server's
`routes.rs` import it from there. `ApiClient::discover` already adds the timeout to its
request timeout, so the 10-second request limit does not cut a 5-second scan.

## The CLI (step 3)

The CLI composes `ApiClient::connect(Endpoint::operator(dir)?)` and runs the commands it
runs today against `&dyn OperatorControl`. It no longer builds a service, so it has nothing
to shut down and the receiver is released by the server's idle time, not on exit (R13).

**Grammar.** Everything that exists stays. Added:

```text
denon-avr-remote [--data-dir DIR] get ...           # --data-dir is accepted before the command
denon-avr-remote set <operation> <value> --dry-run [--as-agent LABEL] [selector]
denon-avr-remote tokens issue <label>
denon-avr-remote tokens list
denon-avr-remote tokens revoke <token-id>
denon-avr-remote release [selector]                 # free the receiver's one control connection now
```

- `--data-dir DIR` names the server's data directory (default `default_data_directory()`).
  It exists for tests and for a second server on the same Mac.
- `set --dry-run` without `--as-agent` calls the Operator's `dry_run` and prints that the
  Operator is not subject to policy and the request would be submitted. It contacts the
  server and the receiver (it resolves the target as `set` does) and writes nothing: it
  skips `remember`, so a dry run never saves the configuration.
  With `--as-agent LABEL` it calls `dry_run_as` and prints the decision: allowed, or the
  reasons and rule ids if it needs approval or is denied. The label is checked with
  `policy::label_is_well_formed` before any request. A mistyped label that names no token
  gets the server's answer for a label no rule mentions, which is the general rules; the
  output says which rules were considered so a misspelling is visible (phase 3's input
  most likely to bite). `--as-agent` works on the saved current receiver only: a receiver
  chosen with `--host` or `--receiver N` gets an `adhoc:` id that no Agent view lists, so
  the combination is refused at parse time with that reason (D76).
- `tokens issue` prints the token once, on standard output, as one line, after a line on
  standard error saying it will not be shown again (D65). `tokens list` prints id, label,
  state, and creation time and never a secret. `tokens revoke` takes the id `list` prints.
  There is no `--out` until milestone 6's setup guide needs a file (D65).
- `release` resolves its receiver as `get` does (a saved current receiver, `--host`, or
  `--receiver N`) and calls `release`. `Released` prints "Released <name>; its control connection
  is free." and `NotConnected` prints "The server was not connected to <name>.", both exit 0.
  `InUse` is retried every 500 ms for up to 20 seconds, with one line on standard error ("Waiting
  for open connections to end…"), and then prints "Not released: <name> is in use (N open reads or
  subscriptions, M operations in flight), for example by the app's window. Close it, or wait for
  the operation, and try again." and exits 2. It never saves the configuration.

**Output.** A usage error prints the message and the usage text and exits 2, as now. Any
other failure prints one line and exits 2, as now, without the usage text (the usage text
after "no server is running" is noise). The exit codes are unchanged.

**An operation whose answer is lost.** The CLI submits once and waits with
`operation(id, wait)`. If the connection drops while it waits, it does **not** submit again
(the session's at-most-once rule is the server's, and a retry could double a write). It
prints *"The server stopped before the outcome of operation N was known. Run `get` to see the
receiver's state."* and exits 2 (D66). A wait that ends because the server shut down is the
same case.

## The launcher and the server link (steps 4 and 5)

### Where it lives

`apps/desktop/src/launcher.rs` holds the launcher, and `apps/desktop/src/link.rs` the
`OperatorLink` and the `ServerLink` value the GUI sees. They are in `desktop` and not in
`api-client`, because `mcp-stdio` inherits `api-client` and must never be able to start the
server (D53). A boundary rule makes the launcher the only code outside tests that spawns a
process (see [Boundary rules](#boundary-rules-added-and-changed)).

### What `gui-lib` sees

```rust
pub struct GuiServices {
    pub control: SharedOperatorControl,   // the OperatorLink
    pub server: SharedServerLink,         // new
    pub shutdown: ShutdownHook,           // runs ServerLink::release
}

pub trait ServerLink: Send + Sync {
    fn status(&self) -> ServerStatus;                           // cheap, no I/O
    fn start(&self) -> BoxFuture<'_, Result<(), String>>;       // attach, else spawn
    fn release(&self) -> BoxFuture<'_, ()>;                     // see Shutdown
    fn summary(&self) -> BoxFuture<'_, Option<ServerSummary>>;  // for Diagnostics
}

pub enum ServerStatus {
    Starting,
    Running { owned: bool },        // owned: the GUI started it
    Unavailable { since: Instant },
    Incompatible { server: u32, client: u32 },
    ExecutableMissing(PathBuf),
    Failed(String),                 // a message a person can act on
}

pub struct ServerSummary { pub socket: PathBuf, pub owned: bool, pub agent_endpoint: Option<String>, pub token_store_ok: bool }
```

`gui-lib` owns these types and imports no wire type; `desktop` maps the server's health
(`ApiClient::server_health`) into `ServerSummary` (`agent_endpoint` is `Some(reason)` when
the endpoint is off).

### `OperatorLink`

`OperatorLink` implements `OperatorControl` by delegating to the `ApiClient` it holds.
Before the first attach it holds none, and every call returns `ControlError::Unavailable`
with the text "the Control API server is not running". After a successful attach it holds
the client, which keeps working across a server restart because each request opens its own
connection (D63). It is a plain `RwLock<Option<Arc<ApiClient>>>`.

### The algorithm

`Launcher::ensure()` is `start()`'s body and runs at GUI start. Its inputs are the data
directory, the server executable's path, and the log file's path, all parameters so a test
can point it at a scratch directory and the real binary.

1. **Probe.** `connect()` to `EndpointPaths::under(dir).operator_socket` with a 2-second
   timeout.
   - Accepted: go to *attach*.
   - `ConnectionRefused` or `NotFound` (nothing listening, or a socket left by a killed
     server): go to *spawn*.
   - Anything else (`PermissionDenied`, a timeout, a path that is not a socket):
     `Failed("the socket at <path> cannot be used: <why>")`. **Do not spawn**: a server
     that holds the lock and does not answer would only exit 75, and a socket someone else
     owns is not ours to remove (D58).
2. **Attach.** Read the token (`read_operator_token`), build the `ApiClient`, and call
   `health()`. If `health.contract` differs from `CONTRACT_VERSION`, report
   `Incompatible { server, client }` and do nothing else (D59). Otherwise the status is
   `Running { owned: false }`.
3. **Spawn.** Run `<directory of current_exe()>/denon-avr-api-server --data-dir <dir>
   --exit-with-parent`, with standard input a pipe the launcher keeps, standard output
   null, and standard error the log file opened for append with mode `0600` (D55). The log
   is `~/Library/Logs/Denon AVR Remote/api-server.log`, truncated at spawn when it is over
   1 MiB. A missing executable is `ExecutableMissing(path)`; there is no `PATH` lookup and
   no override (D54).
4. **Wait.** Every 100 ms for up to 15 seconds: `try_wait()` on the child, then *probe*.
   - The child exited with status 75: another server holds the lock. *Probe* again. If it
     accepts, that server won the race: go to *attach*. If the probe is refused or the
     socket is gone, the holder is shutting down (it stops accepting, so connections are
     refused, but it keeps the lock until its connections have ended, its sessions are closed,
     and its operations in flight are done, which can take its 5-second grace and more) or
     has just died. Wait 250 ms, doubling to 1 second, and go back to *spawn*, all within the
     one 15-second deadline. This is what makes quitting the app and opening it again at once
     work (D74).
   - The child exited with any other status: `Failed("the control server stopped (status N);
     see <log path>")`.
   - The probe accepts: go to *attach*, and the status is `Running { owned: true }`.
   - The deadline passes: drop the pipe, which makes the child exit, and report
     `Failed("the control server did not become ready in 15 seconds; see <log path>")`.

   The deadline covers the policy load and the budget ledger's rebuild from the audit log,
   which happen before the sockets bind.

### Lost-server detection (R11)

The GUI wraps the port, not the other way round. When a call through `OperatorLink` fails
with `Unavailable`, the link sets a flag. A task, started once, wakes on the flag and also
every 2 seconds while the status is `Unavailable`, and probes with a plain `connect()`
(1 second timeout, no token). A failed probe sets `Unavailable { since }`. An accepted one
runs the **attach** steps above (read the token, build the client and install it in
`OperatorLink`, call `health()`, check the contract version), and only a successful attach
sets `Running` and reports a `ServerBack` event. So a server from another build that
appears after a loss is `Incompatible` and not used, and a GUI whose first launch failed
gets its client the first time any server appears (D75). The GUI reacts to `Unavailable`
by showing a banner (below) and to `ServerBack` by selecting the current receiver again,
which restarts its state subscription. The GUI starts a server only from the **Start
server** button, which calls `start()` (D47).

### Shutdown (D60)

Window close and `Message::Shutdown` run `ServerLink::release()`, inside the GUI's
existing 3-second grace:

- A **spawned** child: drop the held pipe. The server sees the end of standard input and
  runs its own shutdown (stop accepting, tell streams `shutdown`, close sessions within its
  5-second grace). The GUI waits for the child to exit up to the 3 seconds and then lets the
  window go; the child finishes alone. No signal is sent and the child is never killed.
- An **attached** server: nothing. The GUI drops its state subscription so its leases are
  released, and leaves the server running.

A GUI that is killed closes the pipe by dying, and the child shuts down the same way.

### Capture mode

When `DENON_AVR_CAPTURE_DIR` is set, `desktop` composes an inert port and server link: every
call fails with a typed error, and the status is `Running { owned: false }`. It never calls
the launcher, so the baselines are captured with no server and no receiver, as today.
Capture scenarios put their data straight into the GUI's state, as `configure_capture_scenario`
does now.

## The GUI

### Server state in the views

`Lifecycle` gains no variant. A new `server: ServerStatus` field on `Gui` is set from the
link. While it is not `Running`:

- The Dashboard shows a banner above the receiver cards: "Control server unavailable. This
  app is waiting for it to come back." with a **Start server** button, or the message of
  `Failed`, `Incompatible`, or `ExecutableMissing` with no button for the last two.
- Controls fail with the port's error as they do now; they are not disabled, so the
  banner's text is not the only signal.
- **Diagnostics** gains a "Control server" panel: the status, who started it, the socket
  path, whether the Agent endpoint is on (and the reason if it is off), and whether the token
  store is healthy.

### Advanced

`Route::Advanced` stays. Its body becomes a segmented control with five tabs, kept in
`AdvancedTab { Status, Operations, Approvals, Audit, Policy }` and changed by
`Message::AdvancedTab(tab)`:

- **Status**: today's "Receiver status" panel.
- **Operations**: the operation feed.
- **Approvals**: the placeholder.
- **Audit**: the audit log.
- **Policy**: the effective policy.

**Operations.** The bridge opens `operation_events()` once the server is `Running` and
re-opens it on `ServerBack` or when the stream ends. The feed keeps the newest 200
operations, keyed by id, showing the latest status of each, newest first: when the GUI
received the update, the id, the receiver, who submitted it (the Operator, or the agent's
label), the intent in words, the status, whether the dispatch is certain, whether it is
confirmed, and the reason. A `Missed` event adds the line "Some events were missed; the
audit log has every operation." The feed shows operations from the CLI, the GUI, and agents
alike, and the principal column is what tells them apart (D78). The CLI and the GUI are both
the Operator, so they are not told apart from each other.

**Audit.** The tab reads `audit(AuditQuery::new(50))` when opened and on **Refresh**, newest
first, and **Older** continues from the page's `next` cursor. It keeps at most 500 entries
(ten pages) and then disables **Older** with "Showing the newest 500 entries." Each row is
the time, the principal (Operator, an agent label, or "none" for a refused caller), the
receiver, and one line for the event and its decision. A failed read shows the port's
error in place of the table.

**Policy.** The tab reads `policy()` when opened. It shows the digest (the first 12
characters, with the whole digest in a tooltip-like secondary line), when it was loaded,
the file's text read-only in a scrollable monospace block capped at 300 lines with a note,
and the load error if the last load failed. **Reload** calls `reload_policy()` and replaces
the view with the result; a failed reload shows its error and keeps the policy in force
(the service's behavior, shown, not decided here).

**Approvals.** A panel with no data: "Approval is not available in this release. A request
that would need approval is refused with the status `approval_unavailable`. See the Audit tab
for refused requests." It has no button and calls nothing (D45).

### Bridge

`PortBridge` gains one task for `operation_events()`, and its subscription loop treats an
`Unavailable` from `state()` as the server possibly being gone: it reports
`PortEvent::ConnectFailed` as before and leaves the decision to the server link, which
probes (R11). On `ServerBack` the GUI sends `BridgeCommand::Select` for the selected
receiver again.

### Projection round trip (D67)

`crates/gui-lib/tests/wire_round_trip.rs`, with `api-contract` as a dev-dependency, takes a
set of `ReceiverState` values (every field in each validity class, with and without an
issue, a stale reason, an epoch and a revision) through the Operator state view and back
(`StateView` and its inverse, as `api-client` does) and asserts that the projection the
views read (`projection.rs`), the values the dashboard shows, and `FieldBaseline::capture`
for each of the seven fields are equal for the original and the rebuilt state. It is the
check phase 4 left open.

## Packaging (step 10)

`tools/package-macos.sh` builds both binaries
(`-p denon-avr-desktop --bin denon-avr-remote-gui -p denon-avr-api-server --bin
denon-avr-api-server`), copies the server to `Contents/MacOS/denon-avr-api-server`, signs
**inside out** (the nested executable, then the bundle, instead of `--deep`, which Apple
recommends against for signing), and verifies with `codesign --verify --strict` and
`codesign -dvvv` on each, printing both identifiers. What identity each gets was S5's output, and **S5 chose B**
(the owner confirmed it on 2026-10-10, D80):

- **A.** The server is signed with the bundle's identifier (`com.denonavr.remote`). This was
  the hope: one grant covers both. **Not chosen:** on every fresh id S5 tried, this shape was
  blocked silently, with no dialog and no row.
- **B.** The server is signed with its own identifier (`com.denonavr.remote.server`). S5's six runs
  show that the system then asks once, with one dialog and one Settings row, both named for the
  app and never for the server, so the user guide has one thing to say. The first connection is
  refused with `os error 65` while the dialog is up and works from the retry after Allow, and again
  once after an update (no dialog, a retry about 5 s later passes), so the launcher and the server
  status must show "waiting for permission" and retry, not report a hard failure. An update of the
  same bundle keeps the grant, and two identical copies at two paths share it. Under the real id
  both A's and B's shapes ask once for a user who already has a record; only B worked on every fresh
  id. **S5 chose B** (see the research note's Outcome). The script therefore signs the server with
  `--identifier com.denonavr.remote.server` and then the bundle with `--identifier
  com.denonavr.remote`, and prints both. An in-place update of the installed app, a real disk image
  and a restart are not tested.
- **C.** Neither works from the bundle, and the GUI-owned server cannot reach the receiver.
  The plan stops and returns to the owner: the options are then a server that is a login
  item with its own bundle, or a GUI that hosts the server in a thread. Both change the
  process model, so they are not planned here. **Not needed.**

The script also adds a smoke step that needs no network: it starts the packaged server with
`--data-dir` in the script's work directory, waits up to 5 seconds for the Operator socket,
stops it with SIGTERM, and checks status 0 and that both the socket and the lock are gone.
`Info.plist` gains `NSLocalNetworkUsageDescription` if S5 finds that the permission text is
needed.

`make run-gui` builds `denon-avr-api-server` before it runs the GUI, so the GUI finds it
beside itself under `target/debug`.

## S5: what the hand check decides

Run by the owner on the Mac, before step 10 starts, with the receiver reachable. Step 10's
script does not exist yet, so S5 had its own (archived on 2026-10-10 in `docs/archive/s5/tools/`, with
the paths below as they were): `tools/s5/build-bundles.sh` built test bundles
that have the shape the package will have (a main executable and the real server nested beside
it, signed three ways), `tools/s5/app` is a small AppKit program that stands in for the GUI (the
first run, with a plain executable in that place, was blocked and showed no dialog, so it settled
nothing), and `tools/s5/host` is the helper it starts, which starts the nested server and drives
it. Neither is part of the product or of the Cargo workspace. The step by
step instructions, the recording tables, and the rule that turns results into outcome A, B, or
C are in [the runbook](../research/local-network-permission-server-macos.md). The checks:

1. **Identity.** `codesign -dvvv "Denon AVR Remote.app/Contents/MacOS/denon-avr-api-server"`
   after the current `codesign --force --deep --sign - --identifier com.denonavr.remote`:
   what `Identifier=` is for the server and the GUI. *A dry run on 2026-10-09 answered this:
   `--deep --identifier` gives the nested server the bundle's identifier, so the current command
   already produces outcome A's identity. The system did not honor it on fresh ids, where it blocked that shape silently (see the research note).*
2. **Finder launch.** Open the app from Finder with the server missing from the Local
   Network list. Does the prompt appear once, name the app, and, once granted, does the
   GUI-owned server reach the receiver (a `get state` from the CLI succeeds)? Repeat after
   removing the grant with `tccutil reset` (or the System Settings toggle) and again after
   rebuilding the app, to see whether the grant survives a new ad-hoc signature.
3. **Discovery.** The server's SSDP scan from the bundle finds the receiver.
4. **Terminal launch.** Run the packaged server from a terminal. Which application does the
   prompt name, and does it work once granted? What does the same server do when started
   from the build directory?
5. **The alternative signings** A and B above, if 1 to 4 fail under the current command.

The output is a note in `docs/research/` that picks A, B, or C and records the commands.
The script of step 10 takes the signing as data, so either of A and B fits.

## Test support

`apps/api-server/tests/support/mod.rs` holds a fake connector and session, a real
`ControlService::start` over a temporary audit directory and policy, and a raw Unix-socket
helper. Step 3 moves the first two to `apps/api-server/src/testing.rs`, public, behind a cargo
feature `testing` that is off by default, so that `cli` and `desktop` can start a real server
in process from their tests. The server's own tests use the same module. The boundary script
requires that no normal build enables the feature
(`cargo tree -p <pkg> --edges normal,features` shows no `denon-avr-api-server feature "testing"`).

The launcher's tests need the **real** server binary, and `CARGO_BIN_EXE_*` only names the
binaries of the package under test. They look for `denon-avr-api-server` beside the test's
own executable's profile directory (`target/<profile>/`). `cargo test --all-targets`, which
`make check` runs, builds every package's binaries first, so the file is there. Run alone
(`cargo test -p denon-avr-desktop`) it may not be, and the test then fails with a message that
says to build the server. `make test-launcher` builds `denon-avr-api-server` and runs those
tests.

## Boundary rules added and changed

Each lands with the step that makes it true, because the script fails if either half
lands alone.

- **Step 3.** Delete `denon-avr-cli:denon-avr-infrastructure` from the edge allowlist and add
  `denon-avr-cli:denon-avr-api-client`. `apps/cli/src` names neither
  `denon_avr_infrastructure` nor `ControlService`.
- **Step 5.** Delete `denon-avr-desktop:denon-avr-infrastructure`; add
  `denon-avr-desktop:denon-avr-api-client` and `denon-avr-desktop:denon-avr-api-contract`.
  `apps/desktop/src` names neither `denon_avr_infrastructure` nor `ControlService`. The
  temporary exceptions that phase 4 wrote into the script are removed, and the general rule
  is switched on: `cargo tree -p P --edges normal` for each of `denon-avr-cli`,
  `denon-avr-desktop`, `denon-avr-gui-lib`, `denon-avr-api-client`, and
  `denon-avr-api-contract` lists neither `denon-avr-infrastructure` nor `denon-avr-protocol`.
  The graphs of `cli` and `desktop` list no `denon-avr-api-server` (the GUI launches the
  server; it does not link it), and no normal build enables its `testing` feature.
- **Step 5.** `crates/gui-lib/src` names no `denon_avr_api_`; the dev-dependency is for the
  one test.
- **Step 4.** `std::process::Command` appears outside tests only in
  `apps/desktop/src/launcher.rs`. Nothing but the launcher spawns a process, which is what
  keeps `mcp-stdio` from being able to start the server later.
- The `operate` allowlist and the single-definition rule are unchanged.

## Files

| File | Change |
| --- | --- |
| `crates/application/src/{control,config_edit,ports,lib}.rs`, `service/ports.rs` | `ConfigRevision`, `RevisedConfiguration`, the two changed methods, `ControlError::ConfigurationChanged`, `edit_configuration`, `DEFAULT_DISCOVERY_TIMEOUT`; `OperationSnapshot.principal` (step 1b); `release` and `ReleaseOutcome`, with `release_if_idle`'s close shared (step 1c) |
| `crates/api-contract/src/{error,paths,admin/config}.rs`, `tests/` | The 412 mapping, `default_data_directory`, the `ETag` from the revision; `OperationDto.principal` (goldens for operation DTOs regenerated); the `release` route and its result type |
| `crates/api-client/src/{lib,bootstrap,transport}.rs`, `Cargo.toml` | `Endpoint::operator`, `read_operator_token`, `BootstrapError`, `save_configuration(config, base)`, `release`, the principal in `TryFrom<OperationDto>`, `rustix` |
| `apps/api-server/src/{routes,main,testing,lib}.rs`, `Cargo.toml`, `tests/` | Revision in the route, the lock removed, the `release` route, `--data-dir` default, the `testing` feature |
| `crates/infrastructure/src/{data_directory,config_yaml,discovery_ssdp,lib}.rs` | The path functions go; the discovery constant moves |
| `apps/cli/src/{main,args,commands,target,render}.rs`, `Cargo.toml`, `tests/` | The client, `--data-dir`, `--as-agent`, `tokens`, `release`, the edit helper |
| `apps/desktop/src/{main,launcher,link}.rs`, `Cargo.toml`, `tests/launcher.rs` | The launcher, the link, composition, capture mode |
| `crates/gui-lib/src/{bridge,lib,messages,state,shell,settings_diagnostics,setup,sound_mode,session}.rs` and new `advanced.rs`, `operations.rs`, `audit_view.rs`, `policy_view.rs`, `server_status.rs` | `ServerLink`, server banner, the Advanced tabs, the three views and the placeholder, bridge task for operation events |
| `crates/gui-lib/tests/wire_round_trip.rs`, `Cargo.toml` | The projection check, with `api-contract` as a dev-dependency |
| `tests/visual-baselines/macos/*`, `tools/capture-visual-baselines.sh`, `crates/gui-lib/src/capture_scenario.rs` | New scenarios and baselines; `diagnostics-*` re-captured |
| `tools/{package-macos,check-boundaries}.sh`, `Makefile` | Packaging, the rules above, `run-gui` builds the server |
| `docs/research/local-network-permission-server-macos.md`, `docs/archive/s5/tools/{build-bundles.sh,host/,app/}` | S5's note and results, and the programs it ran (archived on 2026-10-10 as reference; the screenshots were deleted) |
| `ARCHITECTURE.md`, `AGENTS.md`, `docs/{README,cli-user-guide,desktop-user-guide,development}.md`, `docs/planned-architecture.md`, `docs/v4/roadmap.md`, `docs/archive/v4/phase-5-live-validation-record.md` | Promotion at step 11 |

## Decisions

**Answered by the owner on 2026-10-09.**

- **D44. The configuration revision goes through the port.** The alternative skipped
  writes that change nothing and documented the rest. The revision closes the race and
  reopens phase 4's port, client, route, and test fakes.
- **D45. The approvals view is a static panel.** No contract or port method; milestone 7 and
  S3 stay after 4.0.0. ApproveHub, the owner's separate approval app, is in its first phase,
  whose requests exclude the sensitive ones Denon needs approved, so nothing in this phase
  should fix an approval shape.
- **D46. The new views live inside Advanced.** The rail does not change, so the 12
  baselines of unchanged screens stay byte-identical and only `diagnostics-*` is
  re-captured.
- **D47. A lost server is not respawned.** The GUI shows it, keeps trying to re-attach, and
  starts a server only when the owner presses **Start server**.
- **D77. The 60-second hold is accepted, and a `release` command is added.** The alternatives
  were accepting it alone and a shorter idle for CLI-only use. `release` adds a port method
  and a route to phase 4's contract (step 1c).
- **D78. The Operations feed shows who acted.** The principal becomes part of
  `OperationSnapshot`, on the wire in both views. The alternative pointed to the Audit tab and
  left the port alone (step 1b). The owner chose to decide the S5 fallback (outcome C) when S5
  reports, so no fallback is planned here.

**Settled by the review.** Any can be reversed in a follow-up commit.

| # | Decision | Reason |
| --- | --- | --- |
| D48 | `ConfigRevision::of` is FNV-1a 64 over a length-prefixed encoding of the configuration; the `ETag` is its hex in quotes | `application` has no hash crate or `serde`. The revision detects a lost update; it is not a security boundary, and a collision needs two edits in one window that also collide in 64 bits |
| D49 | `ControlError::ConfigurationChanged` is a new unit variant, 412 `precondition_failed` on the wire | It is the one place a client must tell "read again" from "failed". A string-carrying variant would need the closed-set mapping of D30 |
| D50 | `edit_configuration` reads, edits a copy, skips a no-op, saves under the revision it read, retries three times | The CLI's `remember` and the GUI's saves are read-modify-write; the retry re-applies the edit to fresh data instead of resending stale data |
| D51 | One data-directory definition in `api-contract`; macOS layout only; an unset `HOME` is `None` | D39 planned the move. The `~` fallback creates a directory by that name |
| D52 | The token reader refuses a symlink, a file not owned by the caller, any group or other mode bit, an empty file, one over 256 bytes, and a malformed token, and reads only after `lstat` | A token others can read is not a token. The design accepts that anything that can read the file is the Operator, and this keeps the file honest |
| D53 | The launcher is in `desktop`, not `api-client` | `mcp-stdio` inherits `api-client` and must never start the server |
| D54 | The server is found beside `current_exe()`; no `PATH` lookup, no environment override | A lookup that can be redirected is a way to run something else as the Operator's server. `make run-gui` builds both binaries |
| D55 | The child gets `--data-dir`, `--exit-with-parent`, a held stdin pipe, null stdout, and standard error appended to a `0600` file in the GUI's log directory | An undrained pipe stalls the server once it fills. The pipe is what ends the child when the GUI dies |
| D56 | Readiness is a 15-second wait polling every 100 ms | The policy load and the ledger rebuild run before the sockets bind |
| D57 | Exit status 75 means "attach"; any other early exit is a failure with the log path | Two GUIs can start at once; the lock decides |
| D58 | The probe is a plain `connect()` with a 2-second timeout; only `ConnectionRefused` and `NotFound` spawn | A timeout or a permission error is not a reason to start a second server or remove a socket |
| D59 | A contract-version mismatch on attach is `Incompatible` and nothing else happens | A client must not speak to a server whose wire it does not know |
| D60 | Close and `Message::Shutdown` drop the pipe of a spawned child and wait up to the existing 3 seconds; they do nothing to an attached server but drop the GUI's leases | The child finishes on its own; the GUI never kills it and never stops a server it did not start |
| D61 | An `Unavailable` makes the link probe the socket; "server unavailable" is shown only when the probe fails | `Unavailable` also means the receiver is unreachable |
| D62 | `GuiServices` gains `server: SharedServerLink`; `shutdown` stays | The GUI keeps seeing no process code |
| D63 | `OperatorLink` holds an `Option<ApiClient>` and replaces it at attach | The token exists only once a server has started once, and `ApiClient` is built from it |
| D64 | `set --dry-run [--as-agent LABEL]`; `--as-agent` maps to `dry_run_as` | Phase 3 decided the mapping; without a label the Operator's dry run is `Allow` |
| D65 | `tokens issue` prints the token once on standard output with a warning on standard error; no `--out` yet | The design shows the value once. A file option belongs with milestone 6's setup guide, which is its consumer |
| D66 | The CLI never resubmits; a lost connection while waiting is "outcome unknown" | At-most-once is the session's guarantee, and a retry could double a write |
| D67 | The round-trip test is in `gui-lib` with `api-contract` as a dev-dependency | It is the check phase 4 could not make; the boundary script reads normal edges, and a text rule keeps `gui-lib/src` free of the crate |
| D68 | Advanced's tabs are `Status`, `Operations`, `Approvals`, `Audit`, `Policy`; the feed keeps 200 operations and the audit tab 500 entries | A bounded memory per view |
| D69 | Only `diagnostics-*` is re-captured, and the new views get new baselines at 100% and 200% | D46 |
| D70 | Signing is inside out, not `--deep`, and the identity is S5's output | R7 |
| D71 | `apps/api-server` exposes its fake connector and session behind a `testing` feature | `cli` and `desktop` tests need a real server in process; the feature is checked to stay out of normal builds |
| D72 | A server that does not become ready within the deadline is reported as failed; the GUI drops the pipe and does not kill it | A slow server and a dead one look the same from outside. Closing the pipe asks it to stop, and the lock keeps a wedged one from being doubled |
| D73 | Phase 4's armed live run and account-boundary run gate step 3 onward and the merge | R16. Steps 1 and 2 need no receiver and can be built first; the cutover needs a server that has been run against the real one |
| D74 | After a child exits 75, a refused or missing socket means spawn again with a growing pause, inside the same deadline; an accepting socket means attach | The old server stops accepting before it releases the lock, so a quick reopen finds a refused socket and a held lock. Treating the holder as "the winner" would wait for a server that is leaving |
| D75 | Only a completed attach (token, health, contract check) sets `Running`, at launch and when a lost server returns | A bare connect proves that something listens, not that it is a server this build may speak to |
| D76 | `--as-agent` is refused with `--host` and with `--receiver N` at parse time, and `--dry-run` never saves the configuration | A receiver chosen by address gets an `adhoc:` id that no Agent view lists, so the agent's dry run would answer "not found". A dry run that saved the configuration would not be one |
| D80 | S5's outcome is B (the owner confirmed it on 2026-10-10): the nested server is signed inside out with `--identifier com.denonavr.remote.server` and the bundle with `com.denonavr.remote`; the launcher and the server status treat the first connection after an install or an update as "waiting for permission" and retry, not as a failure; the guides name one dialog and one Settings row, both named for the app | R7. S5's six runs: the signing used today was blocked silently on fresh ids, and B was asked once and passed on every one |
| D79 | `release` never closes a session that holds a lease and has no force option; the CLI waits up to 20 seconds for leases to end before reporting `InUse` | A forced close under the GUI's subscription would only reconnect at once, and one under an operation in flight loses its outcome. The wait covers the lease of the command just before it, which the server finds closed within 15 seconds |

## Live checks

There is no new armed test. `make test-live-x3800h-api` (phase 4) is unchanged and is step
0c. The new risk is the process model, which `tests/launcher.rs` covers with the real binary
and which the owner checks by hand (the overview's [checklist](phase-5-operator-clients-overview.md#manual-macos-checklist)):
the CLI and the GUI operating the receiver together over one connection, a killed GUI ending
its child, and an attached GUI leaving a standalone server running.
