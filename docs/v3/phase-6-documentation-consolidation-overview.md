# V3 Phase 6: Documentation consolidation overview

## Outcome

Phase 6 makes current guidance discoverable without treating completed phase
plans as active design authority. The [documentation map](../README.md) is the
entry point, [ARCHITECTURE.md](../../ARCHITECTURE.md) is the current
architecture source of truth, and the contribution and development guides own
policy and commands.

## Delivered migration

- Added `docs/README.md` as the current-document map.
- Rewrote `ARCHITECTURE.md` around the implemented workspace graph, package
  ownership, session boundary, correctness invariants, delivery composition,
  and enforcement.
- Reduced root README and agent guidance to orientation and current links.
- Updated the CLI guide from current source behavior, including canonical async
  service use, evidence-based write outcomes, selectors, exact dB range, and
  removal of `--resource-version`.
- Moved completed V1, V2, and V3 Phases 1–5 records, release notes,
  changelogs, and validation material to `docs/archive/`.
- Kept `docs/research/` as supporting evidence rather than current policy.

## Acceptance criteria

1. An engineer or agent can reach architecture, policy, commands, user guides,
   research, current work, and archival status from `docs/README.md`.
2. Active entry points do not direct readers to V1/V2 or completed V3 plans as
   current architecture guidance.
3. Archive material is retained, marked non-authoritative, grouped by version,
   and its local links resolve after relocation.
4. Current CLI instructions match `apps/cli/src/main.rs`.
5. `make format-check`, `make boundary`, and `git diff --check` pass.

The paired [Phase 6 architecture](phase-6-documentation-consolidation-architecture.md)
defines document ownership and archive policy; it does not replace the
canonical architecture.
