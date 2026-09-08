# Phase 8 live validation record

This record is intentionally a release gate, not a parser fixture. Do not set
`ModelCapabilities::quick_select_recall` or `eq_status` to `true` until each
row has been completed against the target receiver.

The existing diagnostic probe emits the read-only rows with:

```text
cargo run --bin x3800h-context-probe -- HOST TIMEOUT-MS MODEL FIRMWARE
```

Quick Select recall is deliberately not part of that read-only probe. It must
be exercised as an explicit, separately reviewed operation and recorded as
execute-once evidence.

## Receiver

- Model:
- Firmware:
- Date and timezone:
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

Validation must also cover timeout/reconnect behavior: a recall that may have
been dispatched is reported unconfirmed and is never replayed automatically.
