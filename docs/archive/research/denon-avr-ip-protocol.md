# Denon and Marantz AVR IP protocol research

Status: research/reference documentation. The primary project target is the Denon AVR-X3800H, with Denon/Marantz variation recorded rather than assumed away. Facts are classified as **official**, **community-observed**, or **live validation required**.

## Scope and evidence

This document covers IP control of Denon/Marantz AV receivers and HEOS features. It does not specify RS-232 electrical details, except where Denon uses the same command vocabulary over Ethernet and serial.

The numbered references at the end are the evidence base. Reference [1] is an older, model-specific AVR protocol specification and is used for command shape and examples; it is not a guarantee that every AVR-X3800H or Marantz command is identical. References [2]-[4] are official Denon documentation. Reference [5] is a community implementation and is secondary evidence only.

## 1. AVR control protocol: TCP 23

### Wire format

**Official for the reference protocol.** Denon's AVR protocol describes Ethernet control as a half-duplex TCP connection on port 23, with ASCII command data and a maximum message length of 135 bytes [1]. A command has the shape `COMMAND + PARAMETER + CR`, where CR is carriage return (`0x0D`). The protocol defines commands sent by the controller, responses to request commands whose parameter is `?`, and events sent when the receiver's state changes, including changes made from the front panel or remote.

The receiver can accept commands while transmitting events. Request responses are specified to arrive within 200 ms for the reference model [1].

Representative messages, shown with `<CR>` for byte `0x0D`:

```text
PWON<CR>       # power on
PWSTANDBY<CR>  # standby
MVUP<CR>       # master volume up
MV80<CR>       # master volume at 0 dB in the reference encoding
SICD<CR>       # select CD input
MSSTEREO<CR>   # select Stereo surround mode
SI?<CR>        # request current input; response resembles SICD<CR>
ZM?<CR>        # request main-zone power status
```

**Live validation required for AVR-X3800H.** The examples above are official for the AVR-3313CI/AVR-3313 reference model [1]. The X3800H and other models may omit, rename, or add inputs, modes, zones, and status fields. Unsupported-command behavior and exact event coverage must be tested against the receiver.

### State areas and encoding

**Official for the reference protocol; X3800H capability requires validation.** The reference command table covers main-zone power (`PW`/`ZM`), master volume (`MV`), mute (`MU`), input selection (`SI`), surround mode (`MS`), signal/input mode (`SD` and `DC`), channel volume (`CV`), and status queries such as `PW?`, `MV?`, `SI?`, `MS?`, `MU?`, `CV?`, and `ZM?` [1].

Master volume is not sent as a decimal dB string. In the reference protocol, the normal range is encoded as `00` through `98`, with `80` representing 0 dB and `00` the minimum. With 0.5 dB steps, half-step values use three characters: `MV805` is +0.5 dB, `MV795` is -0.5 dB, and `MV005` is -79.5 dB. Whole-dB values use two characters, such as `MV80` [1]. Channel-volume encoding is also receiver-dependent; the reference table uses `50` for 0 dB and documents a typical `38`-`62` range [1].

Events are not guaranteed to be a complete snapshot. For example, changing an input can also change surround mode and channel-volume state, while some events are suppressed when a value remains unchanged [1]. A client should merge events into cached state and re-query authoritative fields after connection and reconnect.

### Initial capability matrix

The matrix separates protocol families from the currently unverified X3800H result. “Reference” means listed in [1]; “X3800H” is deliberately pending live testing.

| Capability | Reference command family | AVR-X3800H status | Evidence/next action |
| --- | --- | --- | --- |
| Main-zone power | `PW`, `ZM` | Validate | Test `PW?`, `PWON`, `PWSTANDBY`, `ZM?`; record standby behavior. |
| Master volume | `MV` | Validate | Test `MV?`, up/down, direct values, and 0.5 dB encoding. |
| Mute | `MU` | Validate | Test `MU?`, `MUON`, `MUOFF`, and unsolicited events. |
| Input selection | `SI` | Validate | Enumerate actual X3800H input names; do not assume the reference list. |
| Surround mode | `MS` | Validate | Query and set modes exposed by the X3800H; record unsupported responses. |
| Signal/input mode | `SD`, `DC` | Validate | Test HDMI, digital, analog, and auto modes where exposed. |
| Channel volume | `CV` | Validate | Record available channels and value range for the installed speaker layout. |
| Main-zone status events | command-specific events | Validate | Compare network commands with front-panel/remote changes. |
| Additional zones | zone-prefixed commands such as `Z2`/`Z3` | Validate | Test only if enabled and document exact X3800H names/fields. |

### Connection lifecycle

**Official for the reference protocol.** The reference protocol asks the controller to wait one second after sending `PWON` before sending the next command [1]. The X3800H manual documents DHCP/manual network configuration and network-control settings [4]. Whether the X3800H accepts TCP control in each standby configuration is **live validation required**.

**Implementation recommendation.** Treat TCP 23 as a persistent session, but expect disconnects during standby, power transitions, network changes, and firmware behavior differences. Serialize writes, frame reads at CR, preserve unsolicited lines, apply bounded timeouts, and reconnect with backoff. After reconnecting or powering on, query state rather than assuming the last event was delivered. Keep connection failure, timeout, malformed line, unsupported command, and receiver-reported error as distinct structured errors.

## 2. HEOS control protocol

HEOS is a separate protocol and should not be folded into the AVR ASCII parser.

### Discovery and connection

**Official.** HEOS devices can be discovered with UPnP SSDP. The HEOS specification gives the M-SEARCH target `urn:schemas-denon-com:device:ACT-Denon:1`; the response supplies an IP address, after which the controller opens HEOS CLI on TCP 1255. A static/manual IP is also supported. Denon's service reference additionally lists secure HEOS CLI on TCP 1256 where supported [2][3]. One HEOS connection can control the wider HEOS system; the specification recommends not connecting separately to every player [2].

**Live validation required for AVR-X3800H.** Test whether the receiver exposes TCP 1255 and/or 1256, answers SSDP on the expected interface, and remains reachable in standby. SSDP packet-field details are intentionally outside this document.

### Commands, framing, and responses

**Official.** HEOS commands are human-readable URLs terminated by CRLF (`\r\n`) [2]:

```text
heos://player/get_now_playing_media?pid=1\r\n
heos://player/get_volume?pid=1\r\n
```

The general form is `heos://command_group/command?attribute=value&...`. Responses are JSON objects with a `heos` envelope (`command`, `result`, and `message`) and an optional `payload`. The protocol percent-encodes `&`, `=`, and `%` in attribute/value data. Responses may report that a command is still in progress, and commands may produce unsolicited events [2].

Typical capabilities include player discovery, source discovery, browsing/search, artwork, now-playing metadata, play state, transport, volume/mute, and player/group state events [2]. Service-specific fields and supported playback operations are **live validation required**.

### Recommended initialization

1. Connect to a manually configured IP, or use SSDP discovery.
2. Send `heos://player/get_players` and, if needed, `heos://player/get_groups` to build the player-ID model.
3. Query sources, player information, volume/mute, play state, and now-playing state for the players the UI needs.
4. Send `heos://system/register_for_change_events?enable=on` once the event reader is ready; events are disabled by default in the official specification [2].
5. Correlate command responses by command and player ID, while routing `event/...` messages independently.
6. Use heartbeat/reconnect behavior and refresh player/group state after reconnect.

Do not assume that the AVR is the only HEOS player or that player IDs are stable hard-coded identities. Do not take ownership of or alter a user's queue or Connect session as part of ordinary state discovery.

## 3. Discovery and exposed services

There are three distinct concerns:

| Concern | Mechanism | Meaning |
| --- | --- | --- |
| AVR/HEOS discovery | SSDP M-SEARCH to `239.255.255.250:1900`, target `urn:schemas-denon-com:device:ACT-Denon:1`; Denon's exposed-services table separately lists UDP 1800 | Finds a reachable Denon/HEOS device and address; it does not prove every endpoint is enabled. |
| AVR control | Manual IP plus TCP 23, when receiver network control permits it | Denon ASCII zone/control protocol. |
| HEOS control | Manual IP or SSDP plus TCP 1255; TCP 1256 where exposed | HEOS CLI and its player/source model. |

**Official.** Denon's exposed-services reference lists SSDP on UDP 1800, HEOS CLI on TCP 1255, secure HEOS CLI on TCP 1256, and the HEOS Web API/AVR Remote app interface on TCP 8080 [3]. The HEOS specification names UPnP SSDP and the Denon search target but does not override SSDP's standard multicast endpoint [2]. The X3800H manual documents network operation and web control, including the “Network Control” setting [4]. Discovery and port exposure can vary with model, region, firmware, network-control settings, and standby policy.

**Implementation evidence.** Interoperable HEOS discovery implementations send M-SEARCH to the standard SSDP endpoint `239.255.255.250:1900`, commonly with `MX: 3`, multicast TTL 3, retries, and about a five-second response window [6]. The project therefore treats UDP 1900 as primary and probes UDP 1800 only for compatibility with Denon's exposed-services listing.

Manual IP is the first project capability because it is deterministic and useful where multicast is filtered. SSDP discovery is a separate optional capability with interface selection, timeout, duplicate suppression, and clear presentation of discovered model/address data.

## 4. HTTP/XML and AppCommand observations

This section is background research only; HTTP/XML/AppCommand is not a planned primary transport for the project.

**Official.** The X3800H manual documents web control, and Denon's service reference identifies TCP 8080 as the AVR Remote/HEOS Web API interface [3][4]. These references establish that web-related services exist; they do not define a stable cross-model HTTP API contract.

**Community-observed.** Open-source implementations such as [5] use model-specific web-control and XML/AppCommand/status endpoints, response fields, and workarounds. Endpoint paths, HTTP ports, authentication expectations, XML completeness, and available fields vary by receiver and firmware. Treat this material as compatibility evidence only, not an implementation commitment or a guaranteed public API.

## 5. Rust project implementation guidance

- Build a persistent AVR TCP session with CR line framing, serialized commands, response/event parsing, reconnects, timeouts, and structured errors.
- Build HEOS as a separate client: its CRLF framing, JSON payloads, player IDs, browsing model, and event registration differ materially.
- Keep command and status capabilities data-driven by model/firmware. Do not expose an action solely because it appears in the older AVR-3313 reference table.
- Query receiver state after connection and reconnect, then consume events for live updates. Treat event delivery as incremental rather than a complete snapshot.
- Support manual IP first; add SSDP discovery separately.
- Never assume ownership of a user's HEOS queue or Connect service. Display receiver-reported state and avoid mutating playback state during initialization.
- Maintain a tested compatibility matrix beginning with the Denon AVR-X3800H, recording model, firmware, network-control standby setting, supported commands, status fields, event behavior, and HEOS exposure.

## 6. AVR-X3800H validation checklist

Record each result as `pass`, `fail`, or `not exposed`, with firmware and network-control settings.

- [x] TCP 23: `pass` on AVR-X3800H for `PW?`, `SI?`, `MV?`, `MU?`, and `MS?` on 2026-09-06. Observed values included power on, TV input, volume code `00` (-80.0 dB), mute off, and MCH STEREO. Firmware and Network Control setting still need to be recorded.
- [ ] TCP 23: verify input, mute, volume, surround mode, signal mode, channel-volume, and power events caused by network commands and front-panel/remote actions.
- [ ] TCP 23: test `PWON` timing, standby behavior, socket closure, reconnect, and unsupported commands.
- [ ] Populate the capability matrix with actual X3800H input names, modes, zones, channel fields, and volume encoding.
- [ ] HEOS TCP 1255: connect, enumerate players/sources, query now-playing and volume, and verify CRLF JSON framing.
- [ ] HEOS events: register for change events and verify player, playback, volume, mute, group, and source notifications.
- [ ] HEOS TCP 1256: test only if live probing shows it is exposed; do not assume it is available.
- [x] SSDP: `pass` on AVR-X3800H on 2026-09-06 using the Denon target on UDP 1900. Initial discovery failed because Windows selected a WSL virtual adapter; binding the search to each private IPv4 interface found the receiver and its AIOS description. The manual-IP path also passed.
- [ ] HTTP/XML: record availability only as background compatibility evidence; do not make it a project transport requirement.

## References

1. [Denon AVR-3313CI/AVR-3313 control protocol (official PDF)](https://downloads.denon.com/documentmaster/us/avr3313ci_avr3313_protocol_v04.pdf)
2. [HEOS CLI Protocol Specification (official PDF)](https://assets.denon.com/documentmaster/us/heos_cli_protocol_specification_290616.pdf)
3. [Denon exposed network interfaces and services](https://manuals.denon.com/EUsecurity/EU/EN/index.php)
4. [Denon AVR-X3800H network and web-control manual](https://manuals.denon.com/AVRX3800H/NA/EN/GFNFSYqfevlqjv.php)
5. [ol-iver/denonavr community implementation](https://github.com/ol-iver/denonavr/blob/main/denonavr/denonavr.py)
6. [Pytheos HEOS discovery implementation documentation](https://endlesscoil.github.io/pytheos/pytheos_networking.html)
