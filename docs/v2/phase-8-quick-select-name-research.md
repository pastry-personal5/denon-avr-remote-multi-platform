# Phase 8 — Quick Select name HTTP research

**Status: Candidate discovery only — no product capability is enabled.**

## Conclusion

The AVR-X3800H owner’s manual establishes that every Main Zone Quick Select
slot has a user-editable display name (up to 16 characters), but it does not
document a network API that reads those names. The documented/control-protocol
evidence establishes `MSQUICK1` through `MSQUICK4` recall only; it does not
establish a query for a slot name or preset contents.

The existing AppCommand source-catalog reader is not a substitute. Its
`GetRenameSource` response maps canonical input identifiers to receiver input
labels, such as `GAME` to `PlayStation 5`; it does not map a Quick Select slot
to its own name.

## Candidate read surfaces

Community Denon integrations use the following HTTP XML resources as
read-only status/configuration resources. They are candidates to inspect, not
evidence that an AVR-X3800H returns Quick Select data from them:

| Resource | Reason to capture | Status |
| --- | --- | --- |
| `/goform/Deviceinfo.xml` | Modern receiver configuration/status XML candidate | Unvalidated |
| `/goform/formMainZone_MainZoneXml.xml` | Main Zone configuration/status candidate | Unvalidated |
| `/goform/formMainZone_MainZoneXmlStatus.xml` | Main Zone status candidate | Unvalidated |
| `/goform/formMainZone_MainZoneXmlStatusLite.xml` | Reduced Main Zone status candidate | Unvalidated |

Live AVC-X3800H firmware `6` evidence (2026-09-09) adds a fifth, stronger
candidate: its `Deviceinfo.xml` explicitly advertises `GetQuickSelectName`,
`SetQuickSelectName`, and `SetQuickSelectNameDefault` for Main Zone. The
diagnostic probe therefore sends the receiver-advertised `GetQuickSelectName`
operation as a separate bounded AppCommand POST after the four GET captures:

```xml
<tx>
  <cmd id="1">GetQuickSelectName</cmd>
</tx>
```

It does not send Telnet or any `Set*` operation. The request began as an
evidence probe; the baseline below is now sufficient to enable the read-only
name capability for the X3800H profile. Recall and all Quick Select writes
remain independently gated.

## Recorded baseline evidence

On 2026-09-09, an AVC-X3800H identifying firmware as `6` returned HTTP 200
for the line-delimited request above at `/goform/AppCommand.xml`. Its response
was a single `<cmd>` with the following stable field layout:

```xml
<rx>
  <cmd>
    <Name1>…</Name1><Name2>…</Name2><Name3>…</Name3><Name4>…</Name4>
    <Source1>…</Source1><Source2>…</Source2><Source3>…</Source3><Source4>…</Source4>
  </cmd>
</rx>
```

`Name1` through `Name4` are the four Quick Select display names, and
`Source1` through `Source4` are companion source labels. Both values were
right-padded by the receiver and must be trimmed for presentation while raw
diagnostic evidence remains intact. A compact, otherwise equivalent XML
request returned an empty `<rx/>`; the line-delimited framing is therefore
part of the observed wire contract.

```text
cargo run -p denon-avr-diagnostics --bin quick-select-name-probe -- \
  http://HOST:PORT TIMEOUT-MS AVR-X3800H FIRMWARE baseline
```

Run and retain at least these scenarios: baseline; a uniquely renamed slot;
each of the four slots selected; reconnect; and the renamed slot restored to
its prior name. Capture the exact raw responses, HTTP statuses, model,
firmware, date/timezone, and receiver settings. Do not commit personal network
addresses or user-assigned names without sanitizing them.

## Promotion criteria

Only add an HTTP Quick Select-name reader after the baseline evidence is
supplemented by a unique rename, all-four-slot, reconnect, timeout, and
restoration trace. The parser must distinguish absent names, malformed
responses, and transport failure. It must preserve generation/freshness and
must never derive a slot name from the active input or recall a slot to learn
it.

## Sources

- [Denon AVR-X3800H Quick Select settings](https://manuals.denon.com/AVCX3800H/EU/EN/GFNFSYmgamgxii.php): receiver-side names and 16-character limit.
- [Denon control-protocol reference](https://downloads.denon.com/documentmaster/us/avr2113ci_avr1913_protocol_v04.pdf): established `MSQUICKn` recall command family; not model-specific evidence for a name query.
- [Community `denonavr2016` endpoint inventory](https://github.com/JPHutchins/denonavr2016/blob/master/denonavr2016/denonavr.py): candidate HTTP resource paths only.
