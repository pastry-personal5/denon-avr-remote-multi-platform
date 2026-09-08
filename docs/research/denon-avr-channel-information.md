# Denon AVR input and output channel information

## Conclusion

For an AVR-X3800H-class receiver, “channel information” has three distinct
meanings:

1. **Selected input** — the source selected by the main zone, such as `Blu-ray`
   or `TV Audio`.
2. **Input signal layout** — the channels present in the decoded incoming
   signal, for example 2.0, 5.1, 7.1, or an object-based format.
3. **Output channel layout** — the channels currently being rendered after the
   sound mode, speaker configuration, bass management, virtualization, and
   zone/headphone state are applied.

Denon documents a user-facing way to inspect the first two. The receiver’s
`Information` > `Audio` view reports `Sound Mode`, `Input Signal`, `Format`,
`Sample Rate`, `Offset`, and `Flag`. Denon defines `Format` as the number of
input channels, including the presence of front, surround, and LFE channels.
The X3800H manual also documents a `Channel Indicators` setting that chooses
whether the front-panel channel indicators show input channels or output
channels; `Output` is the default. [1]

The documented AVR TCP protocol does not provide an equivalent, explicit
“current input channel map” or “current output channel map” query. It provides
source, input-mode, digital-mode, surround-mode, and channel-volume commands.
Those values are useful context, but they are not sufficient to prove the
actual live output map. The output map should therefore be treated as
**unknown** unless it is obtained from a model-specific status interface or
validated through the receiver’s information display.

## User-visible procedure

On an X3800H:

1. Start playback on the input being investigated.
2. Press `INFO` on the Denon remote. The remote manual says that `INFO`
   displays the receiver status information on the TV screen. [2]
3. Open the Audio information page and record `Input Signal`, `Format`, and
   `Sound Mode`.
4. To inspect the output indication on the front panel, use Setup > General >
   Front Display > Channel Indicators and select `Output` or `Input`. The
   indicator selection changes what the channel icons mean; it does not change
   the audio processing itself. [1]

The display is the authoritative practical check for a particular receiver and
firmware, but it is not a machine-readable API.

## Network/TCP protocol findings

Denon’s AVR control protocol uses an ASCII command followed by carriage return
(`CR`, byte `0x0d`) over the receiver’s control connection. The exact command
set varies by model and protocol revision, so commands documented for an older
AVR must not automatically be advertised as X3800H capabilities. [3]

The relevant command families are:

| Query | Meaning | What it tells us |
| --- | --- | --- |
| `SI?` | Main-zone input/source | Selected source name, e.g. `SIBD` or `SITV AUDIO`; not the decoded channel layout. |
| `SD?` | Input mode | Whether the source is using automatic, HDMI, digital, analog, or another input mode where supported; not the channel count. |
| `DC?` | Digital input mode | Automatic/PCM/DTS selection on protocol revisions that expose it; not the live decoded layout. |
| `MS?` | Surround/sound mode | Selected processing mode, such as `MSSTEREO`, `MSDIRECT`, or a model-specific mode; not a channel map. |
| `CV?` | Channel-volume status | Channel trim values, normally returned as a sequence of `CV` channel records on supported models; this can reveal configured channel names, but not which channels currently carry program audio. |
| `MV?` | Main volume | Master volume only; unrelated to channel presence. |

The official reference protocol lists `SI`, `SD`, `DC`, `MS`, and `CV` as
separate state areas. Its `CV` records describe channel-volume adjustment, not
signal detection or speaker activity. [3]

Consequently, a TCP status collector can safely report:

```text
selected_input = SI?
input_mode     = SD?
digital_mode   = DC?
sound_mode     = MS?
channel_trims  = CV?
```

It should label the resulting channel information as configuration/context,
not as the actual input or output channel map.

## Other interfaces

The X3800H provides IP control, RS-232 control, web control, and app control as
product features. [4] Denon's web-control documentation directs a browser to
the receiver address and requires Network Control to be enabled for standby
access. Denon's exposed-services reference identifies TCP 80 for HTTP, TCP 443
for HTTPS, and TCP 8080 for the AVR Remote app interface. [5][6] These official
sources document the transports but do not publish a stable, cross-model
AppCommand XML schema for live input/output channel maps.

A live AVR-X3800H probe on firmware `6000-1060-0071-9831` successfully posted
read-only `GetAudioInfo`, `GetInputSignal`, and `GetActiveSpeaker` queries to
`/goform/AppCommand0300.xml` on TCP 8080. The HTTP/1.0 response reported PCM,
48 kHz, and Multi Ch Stereo. `control="2"` selected `FL`/`FR` for the input and
`FL`/`C`/`FR`/`SL`/`SR` for active speakers. This is useful model-specific
evidence, not an official Denon schema guarantee.

The Denon Remote app and the receiver’s on-screen `Information` page are useful
for human verification. They should not be assumed to define a supported
public API for this project.

## Recommended implementation strategy

### Minimum reliable status

Implement the documented AVR queries already used by the project:

```text
SI?\r
MS?\r
```

Optionally add `SD?`, `DC?`, and `CV?` behind model-capability gates. Preserve
each response as an independently typed field. Do not infer `5.1` merely from
`MS` or infer active surround speakers from the existence of `CV` records.

### Input channel information

Prefer a model-specific status endpoint only after live validation. Otherwise,
represent input signal layout as unavailable and direct the user to the
receiver’s `INFO` > `Audio` screen. If a future validated endpoint returns
fields such as codec, sample rate, and channel presence, keep those fields
separate from the selected input and sound mode.

### Output channel information

Treat output as a derived result requiring more than one input. At minimum it
depends on:

- the input signal layout;
- the selected sound mode and decoder;
- speaker layout and amp assignment;
- crossover/bass-management and subwoofer settings;
- virtualization, headphones, and zone routing;
- model and firmware behavior.

Do not calculate a definitive output map from the input channel count alone.
For a UI, use `output_channels: Unknown` unless a validated status source is
available. A separate “configured channels” field may be populated from
speaker-layout or channel-volume data, but it must not be named “active output
channels.”

### Validation plan for the AVR-X3800H

For each test, record firmware, speaker layout, amp assignment, sound mode,
input connector, and the exact response lines:

1. Query `SI?`, `SD?`, `DC?`, `MS?`, and `CV?` during stereo PCM playback.
2. Repeat with 5.1 Dolby/DTS and 7.1 content.
3. Repeat with Dolby Atmos or another object-based format if available.
4. Change sound modes and compare the TCP responses with the on-screen Audio
   Information page and front-panel `Input`/`Output` channel indicators.
5. Test headphones, Zone 2, speaker reassignment, and standby/reconnect.
6. Mark each field as `pass`, `not exposed`, or `model-specific`; do not turn
   a single successful observation into a cross-model capability claim.

## References

1. [Denon AVC-X3800H owner’s manual: Channel Indicators and Audio information](https://manuals.denon.com/AVRX3800H/NA/EN/pdf/AVRX3800H_NA_EN.pdf)
2. [Denon AVR-X3800H remote control: INFO status display](https://manuals.denon.com/avrx3800h/na/en/GFNFSYaubbfivv.php)
3. [Denon AVR control protocol reference PDF](https://downloads.denon.com/documentmaster/us/avr-2308ciserialprotocol_ver540.pdf)
4. [Denon AVR-X3800H product specifications](https://www.denon.com/en-us/product/av-receivers/avr-x3800h/300609-01-00-101.html)
5. [Denon AVR-X3800H web control](https://manuals.denon.com/AVRX3800H/NA/EN/RQIFSYzprtydut.php)
6. [Denon exposed network interfaces and services](https://manuals.denon.com/EUsecurity/EU/EN/index.php)
