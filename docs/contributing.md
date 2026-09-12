# Contributing

## Workflow

1. Start with the [documentation map](README.md), then read the current
   [architecture](../ARCHITECTURE.md) for the affected boundary.
2. Keep the change within its agreed scope and add deterministic tests for
   behavior changes.
3. Run the relevant verification gates before submitting.
4. Record live receiver evidence only when it is needed; do not make live
   validation part of ordinary tests.

## Engineering policy

- Keep AVR and HEOS wire formats and parsers separate.
- Preserve serialized session ownership, bounded I/O, unsolicited-event
  routing, and partial-status behavior.
- Do not claim capabilities without protocol evidence or live validation.
- Keep state-changing commands one-shot; distinguish acknowledgement from
  authoritative receiver confirmation.
- Use application ports and policies at delivery boundaries. Protocol remains
  independent of concrete transports.
- Preserve the workspace boundaries and session-contract ownership defined in
  [ARCHITECTURE.md](../ARCHITECTURE.md). Do not restate or fork that architecture
  in feature documents.

## Verification

Run from the repository root:

```text
make format-check
make boundary
make check
make clippy
git diff --check
```

Use deterministic fakes or local servers for normal testing. A live validation
record must include model, firmware, settings, commands, responses, and date.
Receiver writes require the explicit safety controls described by the Makefile.

## Documentation maintenance

The [documentation map](README.md) is the active entry point.
`ARCHITECTURE.md` owns current architecture; this guide owns engineering policy;
`development.md` owns commands and verification instructions; user guides own
their respective operation instructions. Keep the root README concise.

Put supporting protocol evidence in `docs/research/`. Put completed plans,
changelogs, release notes, and validation records in `docs/archive/`, grouped
by version when applicable. Archive material is historical and non-authoritative.
When moving or changing Markdown, update local links and check them before
submission. Do not create phase directories; use paired
`phase-x-{theme}-overview.md` and `phase-x-{theme}-architecture.md` filenames
for active phase documents.
