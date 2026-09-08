# Phase 7 — Audio signal and channel context

## Status

**Complete — 2026-09-08**

The Phase 7 diagnostic surface is implemented: the reusable AppCommand
protocol and bounded HTTP adapter are in place, the Telnet and HTTP diagnostic
probes are available, and the audio-context query primitives feed the
persistent session snapshot. Group recall remains distinct from individual
mode selection, and individual listening-mode choices stay disabled without
validated signal and speaker context. Live TCP and HTTP traces have been
collected from an AVR-X3800H, but receiver-specific AppCommand meaning is not
promoted to the supported API without scenario-based validation.

## Diagnostic procedure

Build and run `x3800h-context-probe <receiver-host> <timeout-ms> <model> <firmware>` while
playing PCM stereo, Dolby Digital/Plus 5.1, DTS 5.1, multichannel PCM 5.1/7.1,
TrueHD/Atmos, DTS-HD/DTS:X, and paused/no signal. Repeat relevant inputs and
sound modes. The probe sends the documented baseline queries `SI?`, `SD?`,
`DC?`, `MS?`, and `CV?`, one at a time with CR framing. It also sends the
diagnostic candidate queries
`SYSDA ?`, `OPINFINS ?`, `OPINFASP ?`, `SYSMI ?`, and `SSINFAISFSV ?`.
Output records receiver identity, epoch timestamp,
elapsed time, status, and raw response; failures produce a nonzero exit status.
If a query times out or encounters a read error, the probe reconnects before
issuing the next query. This prevents a delayed AVR response from being
mistaken for the response to a later command. Lines from other command
families are retained as `UNSOLICITED` records.

The companion HTTP probe uses the receiver's read-only
`/goform/AppCommand0300.xml` endpoint (port `8080` by default):

```text
x3800h-http-info-probe <receiver-host> <timeout-ms> <model> <firmware> [port]
```

It requests `GetAudioInfo`, `GetInputSignal`, and `GetActiveSpeaker`, then
prints the raw XML and each returned parameter's `name`, `control`, and value.
The `control` attribute is deliberately retained as raw evidence; its meaning
must be correlated with front-panel indicators before it is used as a channel
map. The HTTP request contains no state-changing operation. XML construction
and parsing live in `src/protocol/app_command.rs`; socket and bounded-response
handling live in `src/infrastructure/app_command_http.rs`.

Denon's X3800H manual documents browser-based web control and instructs users
to enable `Network Control` → `Always On` for web and app access. Denon's
exposed-services reference lists HTTP on TCP 80, HTTPS on TCP 443, and the AVR
Remote app interface on TCP 8080. These official documents establish transport
availability, but they do not publish the `AppCommand0300.xml` request or field
schema. The endpoint and command names therefore remain model/firmware-scoped
implementation evidence rather than a cross-model Denon contract.

Sources: [X3800H web control](https://manuals.denon.com/AVRX3800H/NA/EN/RQIFSYzprtydut.php),
[X3800H Network Control](https://manuals.denon.com/AVRX3800H/NA/EN/HJWMSYmehwmguq.php),
and [Denon exposed network services](https://manuals.denon.com/EUsecurity/EU/EN/index.php).

For every run, transcribe X3800H `INFO` → `Audio` (signal type,
format/channel layout, sample rate, flag, and sound mode), and record front
panel indicators with Channel Indicators set to Input and Output. Also record
firmware, input, content, speakers, amp assignment, subwoofer, virtualization,
headphones, and test date. Redact network identity before committing records.

## Availability matrix

| Field | Diagnostic source | Supported interpretation |
| --- | --- | --- |
| Input selection | `SI?` | Known source selection only |
| Input/digital settings | `SD?`, `DC?` | Raw observed settings; not signal format |
| Current mode | `MS?` | Known selected mode only |
| Channel trims | `CV?` | Configuration/trim data only; never active channels |
| Codec/encoding candidate | `SYSDA ?` | Diagnostic candidate; not promoted |
| Input channel candidate | `OPINFINS ?` | Diagnostic candidate; retain raw status string |
| Output channel candidate | `OPINFASP ?` | Diagnostic candidate; retain raw status string |
| Output sound candidate | `SYSMI ?` | Diagnostic candidate; not input codec |
| Sample-rate candidate | `SSINFAISFSV ?` and event | Diagnostic candidate; validate against INFO Audio |
| Format, flags | No validated TCP field | `NotValidated` until a repeatable adapter record exists |
| Web-control Information page | Diagnostic investigation only | Model/firmware-specific, unsupported |
| HTTP `GetAudioInfo` | `AppCommand0300.xml` | Diagnostic candidate; retain raw fields |
| HTTP `GetInputSignal` | `AppCommand0300.xml` | Diagnostic candidate; retain raw channel fields |
| HTTP `GetActiveSpeaker` | `AppCommand0300.xml` | Diagnostic candidate; retain raw channel/control fields |

Every observation uses `Known`, `Unavailable`, or `NotValidated`, with raw
response and provenance where applicable. Partial results are retained.
Individual listening-mode choices remain disabled without validated signal and
speaker context. Group recall (`MSMOVIE`, `MSMUSIC`, `MSGAME`) remains distinct.

## Initial X3800H TCP observation

One AVR-X3800H trace (firmware `6000-1060-0071-9831`) returned the following
candidate values while the receiver reported an Atmos program:

```text
SYSDA Dolby Atmos
OPINFINS 11111111111111000000
OPINFASP 11101100000000000000000000000000
SYSMI Multi Ch Stereo
SSINFAISFSV NON
```

`SYSDA Dolby Atmos` is useful evidence for the input encoding, and `SYSMI
Multi Ch Stereo` is evidence about the selected output sound mode. The two
`OPINF` strings remain raw status maps: their `0`/`1` encoding has not been
correlated with the X3800H front-panel Input/Output indicators. `NON` is also
retained as an unavailable/unresolved sample-rate observation rather than
treated as a numeric rate. The same trace produced a delayed `SYSDA` line
while waiting for `DC?`, demonstrating that unsolicited/event traffic and
query responses are not strictly ordered.

## Initial X3800H HTTP observation

An AVR-X3800H running firmware `6000-1060-0071-9831` returned HTTP/1.0 `200 OK`
from TCP 8080. `GetAudioInfo` reported:

```text
inputmode = HDMI
output    = Speaker
signal    = PCM
sound     = Multi Ch Stereo
fs        = 48 kHz
```

The raw `signal` value contained trailing spaces. The protocol parser therefore
preserves exact decoded text and exposes a separate trimmed view instead of
silently normalizing evidence.

In the same response, `GetInputSignal` marked `FL` and `FR` with `control="2"`;
other named channel positions carried `control="1"` or `control="0"`.
`GetActiveSpeaker` marked `FL`, `C`, `FR`, `SL`, and `SR` with `control="2"`.
This is internally consistent with a stereo PCM input rendered through Multi
Ch Stereo, and is strong X3800H evidence that code `2` identifies selected or
active positions for these two operations. Codes `0` and `1` remain separate
raw states until additional speaker layouts and input formats establish their
exact meaning. The generic AppCommand parser stores every attribute and exposes
numeric control codes without assigning global semantics.

## Validation record template

```text
model / firmware:
test date:
input / connector / content:
speaker layout / amp assignment / subwoofer / virtualizer / headphones:
sound mode:
INFO Audio: signal type / format / sample rate / flag / mode:
front panel Input indicators:
front panel Output indicators:
all query commands and exact responses and timings:
conclusion and reviewer:
```
