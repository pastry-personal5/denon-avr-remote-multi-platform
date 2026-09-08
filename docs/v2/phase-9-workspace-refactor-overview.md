# Version 2 - Phase 9 Overview

**Status: Planned — begins after completed Phase 8.**

Phase 9 replaces the current single-package layout with a virtual Cargo
workspace and clarifies the architecture without changing supported receiver
behavior. It is an atomic internal cutover, not a published-library migration.

## Objective

Enforce dependency direction at the package level, define minimal cross-crate
contracts, and split the GUI into cohesive, independently tested units. A
whole-codebase terminology pass is included where names currently obscure
layer ownership or behavior.

## Scope

The virtual workspace has role-based packages for `domain`, `protocol`,
`application`, `infrastructure`, and `gui`, plus `cli`, `desktop`, and
`diagnostics` executable packages. The packages are internal, use
`publish = false`, and retain version `1.0.0`.

The former root `denon_avr_remote` facade is removed. Only minimal
cross-package contracts are public; implementation modules remain private or
`pub(crate)`. No compatibility layer or consumer migration guide is provided.

The GUI is split into presentation state, messages, reducers, controller/event
bridge, routes, and shared components. The GUI owns the presentation-facing
bridge and receives controller, configuration, and discovery services through
a trait-object service bundle. Concrete infrastructure wiring and runtime
construction remain in the desktop package.

The CLI and GUI executable names, Make workflows, receiver configuration
format, CLI grammar, protocol strings, and receiver semantics remain stable.
Execute-once control, confirmation, capability gating, partial status,
authority, freshness, reconnect invalidation, and stale-event rejection do not
change.

New AVR capabilities, HEOS work, additional zones, visual redesign, and
external package publication are out of scope.

## Deliverables

- A virtual workspace with role-based packages, explicit dependencies, and
  `publish = false` metadata.
- A dedicated diagnostics package for the X3800H Telnet and HTTP probes.
- Minimal cross-crate contracts, layer-owned errors, and updated package-targeted
  Make/development commands.
- A split GUI presentation layer with trait-object service injection.
- Rewritten invariant/workflow tests and automated dependency-boundary checks.
- Updated architecture and contributor documentation.

## Acceptance Criteria

- Forbidden layer dependencies fail automated checks.
- Domain and protocol compile without I/O, runtime, or presentation dependencies.
- The CLI, GUI, and diagnostics binaries retain their supported names and workflows.
- The GUI does not construct protocol frames, choose retry/confirmation policy,
  or create concrete adapters.
- The invariant/workflow matrix covers protocol framing, controller safety and
  lifecycle, configuration, CLI behavior, GUI event ordering, diagnostics, and
  stale-event rejection.
- Workspace formatting, Clippy, package builds, and all tests pass.
