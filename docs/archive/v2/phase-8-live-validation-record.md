# Phase 8 live validation record

This record is intentionally a release gate, not a parser fixture. Do not set
`ModelCapabilities::quick_select_recall` or `eq_status` to `true` until each
row has been completed against the target receiver.

The existing diagnostic probe emits the read-only rows for the physical
AVC-X3800H with:

```text
cargo run -p denon-avr-diagnostics --bin x3800h-context-probe -- HOST TIMEOUT-MS AVC-X3800H FIRMWARE
```

Quick Select recall is deliberately not part of that read-only probe. It must
be exercised as an explicit, separately reviewed operation and recorded as
execute-once evidence.

## Receiver

- Model: AVC-X3800H
- Firmware: 6000-1060-0071-9831
- Date and timezone: 2026-09-11 (workspace run date; confirm operator timezone)
- Main Zone settings (source, mode, calibration, speakers):
- Connection and test environment:

## Command record

For every command, record the exact bytes including the query marker, the raw
CR-terminated response, elapsed time, and whether a reconnect occurred.

| Operation | Request | Raw response | Elapsed | Result / applicability | Notes |
| --- | --- | --- | ---: | --- | --- |
| Quick Select 1 recall (`MSQUICK1`) | | | | | |
| Quick Select 2 recall (`MSQUICK2`) | | | | | |
| Quick Select 3 recall (`MSQUICK3`) | | | | | |
| Quick Select 4 recall (`MSQUICK4`) | | | | | |
| MultEQ XT32 (`PSMULTEQ: ?`) | | | | | |
| Dynamic EQ (`PSDYNEQ ?`) | | | | | |
| Dynamic EQ reference level (`PSREFLEV ?`) | | | | | |
| Dynamic Volume (`PSDYNVOL ?`) | | | | | |
| Audyssey LFC (`PSLFC ?`) | | | | | |
| Dirac Live (`PSDIRAC ?`) | | | | | |

The read-only context probe completed with successful responses for `SI?`,
`SD?`, `MS?`, `CV?`, `SYSDA ?`, `OPINFINS ?`, `OPINFASP ?`, `SYSMI ?`,
`SSINFAISFSV ?`, and all Audyssey/Dirac queries. `DC?` reached the one-second
read deadline, was reported as unavailable, and triggered a reconnect; the
probe then continued successfully with the remaining commands. The probe
reports this optional family as unavailable and exits successfully when it is
the only failure. No writes were issued.

The AVC-X3800H HTTP information probe also returned `200 OK` with
`parsed=true` and `usable=true` using firmware `6000-1060-0071-9831`.

Validation must also cover timeout/reconnect behavior: a recall that may have
been dispatched is reported unconfirmed and is never replayed automatically.
