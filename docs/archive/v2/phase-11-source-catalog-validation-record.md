# Phase 11 source-catalog validation record

**Status: Open — product code is capability-gated; no validated profile is enabled.**

This is the provenance record required before enabling source-catalog reads.
Do not replace these fields with invented responses or promote the candidate
request shape to validated protocol behavior.

## Read-only capture procedure

Use one dedicated, unused source. For every observation run:

```text
cargo run -p denon-avr-diagnostics --bin source-catalog-probe -- \
  http://RECEIVER:8080/goform/AppCommand.xml TIMEOUT-MS MODEL FIRMWARE SCENARIO
```

The endpoint is deliberately explicit. The probe sends only `GetRenameSource`
and `GetDeletedSource` candidate reads, prints request XML, HTTP status, and
raw response XML, and performs no receiver write.

Record sanitized raw output and derived fixtures for each required scenario:

| Scenario | Required result | Captured |
| --- | --- | --- |
| baseline | Original name and shown state | No |
| rename | Receiver menu rename visible in raw response | No |
| shown | Explicit shown row | No |
| selected-then-hidden | Active source remains observable | No |
| hidden | Explicit hidden row | No |
| shown/restored | Source reappears after restore | No |
| reconnect | Same response behavior after reconnect | No |
| restoration confirmation | Original name/visibility restored before session end | No |

HDMI auto-renaming is recorded when available; it is not required release
hardware.

## Required provenance for the committed artifact

- Receiver model and exact firmware
- Capture date, timezone, endpoint, timeout, and elapsed timing
- Sanitized complete request XML, HTTP status, and raw response XML
- Original and restored receiver settings, plus the dedicated source used
- Any parser-derived fixture and the mapping rules used to produce it
- Reviewer confirmation that no command changed receiver state

Only after every required row is captured may this record enable a
validated-profile-specific, bounded catalog reader. Partial or malformed
responses must remain failure fixtures, not a reason to infer hidden entries.
Rename, Reset, Hide, and Show desktop writes remain out of scope pending a
separate mutation-evidence record.
