# Version 3, Phase 3 — Clean architecture refactor

**Implemented — 2026-09-10. No release or version change is created by this
phase; workspace packages remain at 2.0.0.**

## Outcome

The active implementation now has one workspace architecture. The obsolete
root `src/` copy has been removed, leaving `crates/` for reusable layers and
`apps/` for delivery/composition. Receiver wire behavior, YAML receiver data,
CLI grammar, bounded I/O, and diagnostic read-only behavior are unchanged.

## Changes

- Application session factory, live session, and event contracts have one
  home in application ports. Adapters and GUI composition use that contract;
  there is no controller-local duplicate event type or forwarding conversion.
- The connection coordinator remains the sole owner of a live session. It
  serializes commands, assigns lifecycle generations, forwards unsolicited
  events, invalidates stale observations, and closes sessions on shutdown.
- Main Zone policy continues to preserve partial field results and execute
  controls once. Capability gates protect Quick Select, EQ, source catalog,
  and HTTP information reads.
- Supplemental HTTP information, source-catalog, Quick Select, and EQ state
  transitions now live in focused application policy modules. The connection
  coordinator retains only their serialized session orchestration.
- The GUI-owned controller bridge, presentation messages, dashboard route and
  controls, receiver setup, and settings/diagnostics views are separate
  modules alongside feedback, capture, components, and design primitives.
- The CLI now dispatches validated controls through the application control
  port rather than importing AVR command encoding. Diagnostics remain their
  own read-only delivery package.
- `make boundary` validates source imports, normal package dependencies, the
  absence of the legacy tree, and unique session-contract ownership.

## Verification

Run `make check`, `make clippy`, and `git diff --check`. Live receiver
validation remains optional and is never part of ordinary tests.
