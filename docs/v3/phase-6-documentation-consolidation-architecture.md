# V3 Phase 6: Documentation consolidation architecture

## Document ownership

| Concern | Owner |
| --- | --- |
| Current implementation architecture and invariants | [ARCHITECTURE.md](../../ARCHITECTURE.md) |
| Documentation navigation | [docs/README.md](../README.md) |
| Engineering policy and documentation maintenance | [contributing.md](../contributing.md) |
| Prerequisites, commands, and verification gates | [development.md](../development.md) |
| End-user operation | [CLI guide](../cli-user-guide.md) and [desktop guide](../desktop-user-guide.md) |
| Protocol and receiver evidence | `docs/research/` |
| Completed decisions and validation history | `docs/archive/` |

Documents link to their owner instead of duplicating its rules. In particular,
phase documents may describe scope and acceptance criteria but cannot establish
a competing current architecture.

## Archive policy

Completed V1 and V2 material and completed V3 Phases 1–5 reside beneath
`docs/archive/v1/`, `docs/archive/v2/`, and `docs/archive/v3/`. Global
historical changelog material and retired research also live beneath
`docs/archive/`. Archive records remain useful for audit, evidence, and
historical links, but are explicitly non-authoritative.

Relocation preserves same-version relative links where possible and updates
links that cross from an archived record to a current document or repository
file. New active documents must never use an archived plan as their primary
architecture or policy reference.

## Boundary of this phase

This phase changes documentation organization and wording only. It does not
change Rust APIs, package dependencies, receiver behavior, or release version.
The repository boundary checker remains the executable enforcement of package
and session-contract rules.
