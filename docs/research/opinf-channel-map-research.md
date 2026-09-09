# Denon OPINFINS / OPINFASP and channel-map research

## Executive conclusion

`OPINFINS` and `OPINFASP` are useful diagnostic observations, but they are not
currently a safe source for a speaker-name channel map.

The strongest available evidence describes them as undocumented masks:

- `OPINFINS` is reported as an input-availability mask.
- `OPINFASP` is reported as an audio-stream/output-property mask.

The bit/character positions are not documented, and no independent source
found a stable position-to-channel table for the AVR-X3800H. The values from
the current receiver (`22222222111111000000` and
`22202200000000000000000000000000`) therefore should be retained as raw
evidence, not decoded into `FL`, `C`, `SR`, height, or LFE labels.

The best channel-map interface found is the receiver's HTTP AppCommand API:
`GetInputSignal` returns named input-channel slots, and `GetActiveSpeaker`
returns named output-speaker slots. Both include a `control` attribute. A
recent independent capture reports `control="0"` as absent, `control="1"`
as configured/available, and `control="2"` as currently active. This is the
only source found that exposes both the channel name and its state in one
response.

## Evidence for OPINFINS and OPINFASP

The community-maintained Marantz/Denon TCP guide labels both fields
“observed but undocumented.” It records examples such as
`OPINFINS 11111111111111000000` and
`OPINFASP 11111100000000000000000000000000`, and explicitly says their exact
semantics are inferred rather than confirmed. The same guide associates them
with `MSQUICK?`, together with `SYSMI` (human-readable mode) and `SYSDA`
(stream format).[^tcp-guide]

That guide is valuable corroboration that the fields occur on modern Denon /
Marantz firmware, but it does not provide a channel ordering or explain the
three observed symbols (`0`, `1`, `2`). It should therefore not be treated as
a decoder specification.

The current X3800H capture is consistent with a three-state mask, but it is
not enough to identify positions. For example, `OPINFASP` has 32 positions,
while the receiver has multiple possible layouts (front, center, surround,
surround-back, height, wide, LFE, and extension channels). A single Atmos
capture cannot distinguish all of those positions.

## Direct channel-map interface: AppCommand0300.xml

The strongest independent evidence is the `denonavr` issue “Expose audio
signal information.” It gives a complete POST body for
`/goform/AppCommand0300.xml` using `id="3"` and these queries:

```xml
<cmd id="3">
  <name>GetInputSignal</name>
  <list><param name="inputsigall"></param></list>
</cmd>
<cmd id="3">
  <name>GetActiveSpeaker</name>
  <list><param name="activespall"></param></list>
</cmd>
```

The captured `GetInputSignal` response contains named slots such as `FHL`,
`LFE`, `FL`, `C`, `FR`, `SL`, `SR`, `SBL`, and `SBR`; each slot has a
`control` value. The captured `GetActiveSpeaker` response contains named
output slots such as `SW`, `FL`, `C`, `FR`, `SL`, `SR`, `TRL`, `SBL`, `SBR`,
and `TRR`, again with `control` values.[^denonavr-issue]

The issue's interpretation is:

| `control` | Practical meaning | Safe use |
|---:|---|---|
| 0 | no channel/speaker in that slot | omit |
| 1 | available/configured, but not active for this signal | include in configured map only |
| 2 | active for the current signal/mode | include in active map |

This interpretation is independently echoed by a Home Assistant community
capture of `GetActiveSpeaker`, which shows the same named slots and says that
`1` means present/configured and `2` means present and currently active.[^reddit]
The evidence is not an official Denon protocol definition, so the parser
should preserve the raw control code and source firmware/model alongside the
derived state.

The AppCommand response is a better fit than OPINF masks for two distinct
maps:

1. **Input signal map:** use `GetInputSignal`; `control=2` identifies channels
   carried by the current input signal, while `control=1` identifies known
   channel slots that are not carried by that signal.
2. **Output speaker map:** use `GetActiveSpeaker`; `control=1` identifies
   configured/available speakers and `control=2` identifies speakers active
   for the current processing mode.

The API is still undocumented and firmware-dependent. Treat a missing or
empty response as `Unavailable`, not as an all-zero map.

## Why the current HTTP probe returned an empty `<rx>`

The request shape currently used by this project is substantially the same as
the independently captured request: port 8080, `POST`,
`/goform/AppCommand0300.xml`, `id="3"`, and the three `Get*` commands. The
empty response therefore needs targeted troubleshooting rather than a new
channel decoder. Plausible causes include:

- the receiver was not in an active Main Zone playback state;
- this firmware accepts only a particular command ordering or one command per
  request;
- the receiver's AppCommand service is selectively disabled or behaves
  differently from the X4800H capture;
- the response was requested while the receiver was busy or between source /
  mode changes.

The next diagnostic should send the exact two-command XML above as separate
requests, then the full five-command body from the issue, and print the exact
request body as well as the response. A one-command `curl` reproduction is
useful because it removes client serialization and parser differences.

## Other ways to obtain channel information

### Official on-screen/front-panel information

Denon's X3800H manual documents an Information → Audio page containing
**Sound Mode**, **Input Signal**, **Format**, **Sample Rate**, and **Flag**.
The manual defines Format as the number of input channels, including the
presence of front, surround, and LFE channels.[^x3800h-info]

Denon also documents a **Channel Indicators** setting. With `Output` (the
default), the front-panel indicators show channels being output; with `Input`,
they show channels present in the input signal.[^x4700h-display] This is a
reliable manual-validation oracle, but not a convenient machine API. It can
be used to validate AppCommand control codes across stereo, 5.1, 7.1, and
Atmos test material.

### Speaker configuration / Amp Assign

The X3800H manual documents multiple speaker layouts and Amp Assign modes,
including 5.1, 7.1, 9.1, 11.1, bi-amp, Front B, and preamplifier layouts.[^x3800h-speakers]
This is the authoritative source for the *configured capability* of the
receiver, but it is not the current signal map: a configured speaker may be
idle for a stereo or 5.1 source. It should be used as a separate
`configured_speakers` snapshot, not substituted for `GetActiveSpeaker`.

### Telnet status and unsolicited lines

`MS?`, `SYSDA`, `SYSMI`, and sample-rate/status fields are useful context, but
they do not name individual channels. `CV?` enumerates channel trims and can
show which trim families exist, but trim presence is not proof that a channel
is carrying signal. The historical Denon protocol documentation describes
`CV` as channel-volume control/status, not an input/output signal map.[^denon-protocol]

`MSQUICK?` is worth probing because modern community captures show it emitting
`SYSMI`, `SYSDA`, `OPINFINS`, and `OPINFASP`; however, it remains a raw-mask
fallback, not a replacement for AppCommand.

## Recommended implementation strategy

1. **Primary:** implement and probe `GetInputSignal` and
   `GetActiveSpeaker`. Preserve every XML parameter name, value, and control
   code.
2. **Derived maps:** expose `input_signal_channels`,
   `configured_output_speakers`, and `active_output_speakers` separately.
   Do not collapse configured and active output into one field.
3. **Fallback:** retain `OPINFINS` and `OPINFASP` as raw diagnostic fields.
   Decode them only behind a model/firmware-specific mapping table after
   correlation against the front-panel Input/Output indicators.
4. **Validation matrix:** collect AppCommand + Telnet + front-panel results
   for PCM stereo, PCM 5.1/7.1, Dolby Digital, Dolby TrueHD/Atmos, DTS/DTS:X,
   Multi Ch Stereo, Direct/Pure Direct, headphones, and each speaker preset.
5. **Failure semantics:** empty XML, unsupported command, timeout, and
   disconnected session must remain distinguishable. Never convert an empty
   response into an empty channel map.

## Confidence assessment

| Finding | Confidence | Reason |
|---|---|---|
| `OPINFINS`/`OPINFASP` are real modern Denon/Marantz responses | Medium-high | repeated community captures, including the current X3800H probe |
| They are compact masks rather than named channel lists | Medium | observed fixed-length strings and community labels |
| Their positions have a stable public channel ordering | Low | no published ordering found |
| AppCommand exposes named input slots | High | complete independent XML capture |
| AppCommand exposes named active-output slots | High | complete independent XML capture and corroborating capture |
| `control=0/1/2` means absent/configured/active | Medium-high | two independent community sources; still undocumented by Denon |
| Official manual can validate input/output maps | High | Denon documents both Format and Channel Indicators modes |

[^tcp-guide]: [Marantz / Denon AVR — TCP Port 23 Control Guide](https://github.com/alsak0de/marantz-denon-mqtt/blob/main/docs/telnet/MARANTZ_DENON_TELNET_PROTOCOL.md), especially “Observed but undocumented responses” and model-compatibility sections.
[^denonavr-issue]: [ol-iver/denonavr issue #363, “Expose audio signal information”](https://github.com/ol-iver/denonavr/issues/363), request XML and X4800H response capture.
[^reddit]: [Home Assistant community capture of `GetActiveSpeaker`](https://www.reddit.com/r/homeassistant/comments/10b5g56/denon_httprest_command_to_get_active_speakers/), January 2023.
[^x3800h-info]: [Denon AVR-X3800H Information → Audio manual section](https://manuals.denon.com/AVRX3800H/NA/EN/GFNFSYcsmwwkkn.php), Audio information fields.
[^x4700h-display]: [Denon AVR-X4700H manual, input/output signal channel indicators](https://manuals.denon.com/avrx4700h/na/en/WBSPSYszuacfuh.php).
[^x3800h-speakers]: [Denon AVC-X3800H manual, speaker configuration and Amp Assign](https://manuals.denon.com/AVCX3800H/EU/EN/DRDZSYhylpohod.php).
[^denon-protocol]: [Denon AVR control protocol reference](https://downloads.denon.com/documentmaster/us/avr-2308ciserialprotocol_ver540.pdf), channel-volume (`CV`) command/status definitions.
