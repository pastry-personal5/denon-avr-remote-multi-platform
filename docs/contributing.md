# Contributing

## Workflow

1. Read the relevant phase and architecture documentation under `docs/v1/`,
   `docs/v2/`, or `docs/v3/`.
2. Keep changes within the requested scope and preserve documented behavior.
3. Add protocol and transport tests for behavior changes.
4. Run the checks below before submitting the change.

## Engineering rules

- Keep AVR and HEOS wire formats and parsers separate.
- Keep protocol code independent of sockets, operating systems, and async
  runtimes.
- Preserve serialized writes, unsolicited-event routing, bounded I/O, and
  partial-status behavior.
- Do not claim receiver capabilities without protocol evidence or live
  validation.
- Prefer application use cases and infrastructure adapters; protocol modules
  stay independent of concrete transports.
- Keep the package graph directed inward: domain has no workspace dependency;
  protocol and application depend only on domain; infrastructure composes
  application/domain/protocol; presentation imports application and domain,
  never protocol or infrastructure.
- Define cross-package contracts once in `crates/application/src/ports.rs`.
  In particular, a receiver session is owned by the serialized coordinator,
  not by GUI or adapter forwarding layers.

## Verification

Run from the repository root:

```text
make format-check
make check
make clippy
git diff --check
```

Use deterministic fakes or local servers for tests. Live receiver validation
must record the model, firmware, settings, commands, responses, and date.

## Documentation

Active documentation belongs under `docs/`. Version-specific material belongs
under `docs/v1/`, `docs/v2/`, or `docs/v3/`; retired material belongs under
`docs/archive/`.
Do not create phase directories. Encode phases in filenames, and keep the root
`README.md` concise. Name paired phase plans
`phase-x-{main theme}-overview.md` and
`phase-x-{main theme}-architecture.md`.
