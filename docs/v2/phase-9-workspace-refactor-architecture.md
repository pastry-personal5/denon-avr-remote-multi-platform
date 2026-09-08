# Version 2 - Phase 9 Architecture

**Status: Implemented.**

```text
apps/cli ───────────────┐
apps/diagnostics ───────┼──> infrastructure ──> protocol
apps/desktop ──> gui ───┘          ^                 ^
                  |                |                 |
                  └────────> application ───────> domain
```

## Workspace boundaries

| Package | May depend on | Owns |
| --- | --- | --- |
| `denon-avr-domain` | Rust standard library | Receiver state, values, capabilities, and evidence. |
| `denon-avr-protocol` | Rust standard library | AVR, HEOS, and AppCommand framing/parsing. |
| `denon-avr-application` | `denon-avr-domain` | Ports, use cases, controller policy, commands, events, and application errors. |
| `denon-avr-infrastructure` | application, domain, protocol | Network, filesystem, runtime, and concrete adapters. |
| `denon-avr-gui-lib` | application, Iced | Presentation state, messages, reducers, views, components, and the bridge. |
| `denon-avr-cli` | application, infrastructure | CLI parsing, dependency injection, and process entry point. |
| `denon-avr-desktop` | application, infrastructure, gui | Iced lifecycle and concrete GUI service wiring. |
| `denon-avr-diagnostics` | infrastructure, protocol, domain | X3800H diagnostic probe entry points and evidence output. |

All packages are workspace-internal and set `publish = false`. Keep package
versions at `1.0.0`. The workspace root is virtual and contains no library
facade or executable package.

Neither domain nor protocol imports runtime, socket, filesystem, platform, or
presentation APIs. Application owns port traits and receiver policy;
infrastructure implements those ports. Executable packages create the runtime
and select concrete adapters.

## Cross-crate contracts

Expose only contracts required by another package. Keep implementation modules
private or `pub(crate)`. Domain, application, protocol, and infrastructure own
their boundary-specific error types; infrastructure translates adapter errors
into application operation errors at port boundaries. Do not introduce one
global error type.

Remove the root `denon_avr_remote` facade and do not retain deprecated aliases,
compatibility imports, or a consumer migration guide. The refactor may rename
internal helpers, tests, and cross-package symbols for consistent terminology,
but it must not change protocol strings, configuration data, CLI grammar, or
receiver behavior.

## GUI boundary

`denon-avr-gui-lib` owns the presentation-facing controller/event bridge and maps
user intent to typed application contracts. It receives a small trait-object
service bundle for controller, configuration, and discovery operations. It does
not construct `AvrSession`, YAML, SSDP, Tokio, or protocol frames.

`denon-avr-desktop` supplies concrete services, owns Iced application startup
and shutdown, and keeps runtime composition outside the GUI crate. Request,
receiver-context, and connection-generation identity remain attached to bridge
events; reducers discard superseded events before changing presentation state.
Execute-once and confirmation decisions remain in application.

## Developer workflow and verification

Make targets retain the existing `make run` and `make run-gui` entry points by
calling explicit package targets. Add a diagnostics target for
`denon-avr-diagnostics`; direct Cargo commands use `cargo run -p ...`.

Use an invariant/workflow test matrix rather than preserving test files
one-for-one. Keep protocol fixtures with protocol/adapters, controller fakes
with application tests, GUI reducer/bridge tests with the GUI package, CLI
tests with the CLI package, and probe/evidence tests with diagnostics. Run
workspace formatting, Clippy, package builds, dependency checks, and all tests;
live receiver validation remains a release gate.
