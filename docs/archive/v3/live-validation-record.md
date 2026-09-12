# Phase 5 live AVC-X3800H validation record

Copy this template for each approved hardware run. Do not commit receiver
addresses, account data, or other identifying values; use the redacted host
label and retain raw captures outside the repository when necessary.

## Device and environment

- Date (UTC): 2026-09-11 (workspace run date; confirm against operator notes)
- Redacted receiver label: physical AVC-X3800H (host withheld)
- Model reported by receiver: AVC-X3800H (confirm from receiver information)
- Firmware version: 6000-1060-0071-9831
- Region: South Korea
- Network Control setting: On
- Active Main Zone / Zone 2 state: Main Zone observed responding; Zone 2 state not exposed
- Input and signal context: GAME input; HDMI PCM, 48 kHz; Multi Ch Stereo; video 4K60
- Speaker/headphone context: Speaker output; active FL/C/FR/SL/SR; input map also reported FHL/FHR/FWL/FWR/SBL/SBR/EXT
- Safe volume ceiling (half-steps): -40
- Tool revision / commit: workspace revision `825a964` (working tree changes uncommitted)

## Read-only synchronization

- Command order: `SI?`, `SD?`, `DC?`, `MS?`, `CV?`, `SYSDA ?`, `OPINFINS ?`, `OPINFASP ?`, `SYSMI ?`, `SSINFAISFSV ?`, Audyssey/Dirac queries
- Raw response frames (redacted): captured in `/tmp/avc-x3800h-context.log` and `/tmp/avc-x3800h-http.log`; not committed because they include receiver-specific data
- Transmission timestamps (monotonic ms): per-command elapsed timings captured in the context log; HTTP batch timestamp `1789134018`
- Connection epoch transitions: one reconnect after optional `DC?` timeout; subsequent queries completed
- Readiness result: required context families passed; optional `DC?` unavailable
- Unknown/malformed diagnostics: unsolicited status frames were ignored; no malformed required responses

Recorded context-probe run: model `AVC-X3800H`, firmware
`6000-1060-0071-9831`. Read-only responses were obtained for source, signal,
mode, channel-volume, speaker layout, sample rate, and Audyssey/Dirac fields.
The `DC?` query reached the one-second timeout and caused one reconnect; the
probe resumed and completed all subsequent read-only queries. The optional
family is reported as unavailable rather than making the overall diagnostic
run fail. A subsequent run exited successfully with all required queries
passing. No writes were issued.

## Armed control validation

- Arm command and environment gates: `ALLOW_RECEIVER_WRITES=1`; safe ceiling `-40` half-steps
- Initial observed values: recorded by the live control test; receiver mute state restored
- Desktop operations and operation IDs: not exercised by this hardware-only fixture
- Physical-remote actions: none during the armed mute fixture
- Raw event frames and timestamps: retained in external run output; not committed
- Confirmation outcomes: receiver-confirmed mute round trip
- Final canonical values and validity: original mute state restored and confirmed
- Restoration commands and outcomes: restoration completed successfully
- Safe-volume invariant result: passed; observed volume did not exceed `-40`

## Review

- Reviewer: Human (anonymous)
- Capture fixture name (if approved): external AVC-X3800H run logs (not committed)
- Deviations / follow-up scenarios: optional `DC?` is unavailable and requires
  reconnect; Quick Select/EQ writes are explicitly deferred to a later phase

## Recorded execution

- Test: `cargo run -p denon-avr-diagnostics --bin x3800h-context-probe -- HOST 1000 AVC-X3800H 6000-1060-0071-9831`
- Result: passed; required read-only families completed, with optional `DC?`
  reported unavailable after reconnect (run timestamp `1789133988`)
- Test: `cargo run -p denon-avr-diagnostics --bin x3800h-http-info-probe -- HOST 2000 AVC-X3800H 6000-1060-0071-9831`
- Result: passed; HTTP `200 OK`, `parsed=true`, and `usable=true` (run
  timestamp `1789134018`)
- Test: `cargo test -p denon-avr-infrastructure --test live_x3800h -- --ignored` (AVC-X3800H hardware)
- Result: passed (`read_only_core_sync_against_live_x3800h`)
- Duration: approximately 1.32 seconds
- Runner: `../denon-contrib/run-real-test.sh`
- Device metadata and raw-frame details: complete the fields above from the
  redacted run notes before treating this record as release evidence.
- Test: `cargo test -p denon-avr-infrastructure --test live_x3800h_controls -- --ignored` (AVC-X3800H hardware)
- Result: passed (`armed_live_mute_round_trip_restores_original_state`)
- Duration: approximately 1.61 seconds
- Runner: `../denon-contrib/run-real-test-2.sh`
- Restoration: original mute state was restored and confirmed by the receiver.
