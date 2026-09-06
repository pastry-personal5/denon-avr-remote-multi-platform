# Contributing

## Workflow

1. Read the relevant phase and architecture documentation under `docs/v1/` or
   `docs/v2/`.
2. Keep changes within the requested scope and preserve existing compatibility
   behavior.
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
- Prefer `ApplicationService` and `AsyncAvrTransport`; legacy direct status
  transport APIs are compatibility wrappers.
- Do not modify `.codex-firewall-hardening.ps1`.

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
under `docs/v1/` or `docs/v2/`; retired material belongs under `docs/archive/`.
Do not create phase directories. Encode phases in filenames, and keep the root
`README.md` concise.
