# Version 2 — Phase 2 Information Architecture

## Authority and user outcomes

This is the authority for content, navigation, terminology, and information
states. The [GUI design](phase-2-gui-design.md) specifies their presentation and
interaction. The [overview](phase-2-overview.md) and
[architecture](phase-2-architecture.md) define delivery and runtime boundaries.
These are design requirements; they do not claim an implemented or user-tested
GUI.

The primary user is a home-theater owner. Design for these tasks:

| User goal | Successful experience |
| --- | --- |
| Check the theater before watching | Identify the receiver, Main Zone, connection, power, source, volume, mute, and sound mode on one screen. |
| Connect for the first time | Find a receiver or enter its address without understanding network protocols. |
| Recover from a problem | See what is affected, retain useful values, and find one clear recovery action. |
| Change the listening setup, later | Know that a control applies to Main Zone and whether the receiver confirmed the change. |
| Investigate a persistent issue | Reach technical details without interrupting everyday use. |

Apple's guidance supports shallow navigation and feedback near the item it
describes. Here, those ideas become stable destinations, persistent context, and
local status explanations. The specific structure and copy below are product
decisions. Sources: [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars)
and [Feedback](https://developer.apple.com/design/human-interface-guidelines/feedback).

## Delivery and product boundaries

One receiver is selected for the session, and at most one receiver connection
is active. Saved receivers are choices, not a fleet being monitored. Main Zone is
the only zone context in every v2 phase.

| Surface | Phase 2 requirement | Later requirement |
| --- | --- | --- |
| Dashboard / Main Zone | Five Main Zone fields, read-only; refresh and events | Confirmed Main Zone controls under [Phase 3](phase-3-overview.md) |
| Receivers | Load saved identities, explicit discovery selection, manual setup, connect/disconnect/reselection | Complete local rename/remove management as described below |
| Settings | Configuration health and fixed dark appearance | Additional preferences only when useful and implemented |
| Advanced Diagnostics | Existing lifecycle and five-field data only | Validated input/signal/channel details and richer reconnect history |
| Lifecycle | Bounded worker/session behavior in the Phase 2 architecture | Explicit cancellation and reusable controller in [Phase 7](phase-7-overview.md) |

HEOS, cloud access, credentials, remote-control services, macros, automation,
raw command entry, and simultaneous multi-receiver monitoring are excluded.
Keep the established dark-only appearance. The design has no dependency on
archived visual assets.

## Navigation and persistent context

### Sitemap

- [Dashboard](#dashboard): everyday status and, later, everyday controls.
- [Receivers](#receivers): saved identities and connection setup.
- [Settings](#settings): application configuration.
  - [Advanced Diagnostics](#advanced-diagnostics): troubleshooting details.

Keep these three top-level labels and their order stable. Advanced Diagnostics
is reached through Settings > Advanced; its page title is “Diagnostics”.
Diagnostic links from a field open that page with the relevant row highlighted.
“Back to Settings” and “Return to Dashboard” make the exits explicit.

### Shell contract

The leading sidebar contains the receiver selector and its connection summary,
then destination navigation. A persistent context toolbar above the content
contains the fixed “Main Zone” context, Refresh status, and a labelled Connection menu.
Disconnect lives in that menu and in Receivers. During setup or connection
failure, Connect/Try again is directly visible in the content.

There is one receiver selector and no zone selector. Main Zone is a fixed
context label shown in the shell and Dashboard. It is text, not a disabled
dropdown or a future-feature affordance.

Navigation never connects or powers on the receiver merely because an item
receives focus. Activating a destination changes the page; committing a receiver
choice changes context. Looking at discovery candidates does neither.
Do not navigate automatically when a background connection or query finishes.

### Persistence and scope

Preserve destination, local form drafts, and page scroll position during the
running session. Keep receiver and Main Zone context visible when navigating into
Settings or Diagnostics. Diagnostic pages name the scope they inspect.

Persist receiver identity and the chosen startup receiver using the existing
configuration shape. Do not add window, appearance, port, or zone-preference
fields to that shape in Phase 2. Start with Main Zone. An intentional
Disconnect lasts for this running session; the next launch uses saved
auto-connect behavior. Explain this alongside Disconnect.

A committed receiver switch clears the old displayed snapshot and restores the
fixed Main Zone context. Late results must remain attached to their original
receiver and connection generation; they cannot populate the new context.

## Information states and user language

Connection, availability, evidence, freshness, and operation outcome are
independent dimensions. Do not flatten them into one field enum: a field can
have a usable event-derived value while a refresh is pending and its latest
query has failed.

| Dimension | Meaning and presentation rule |
| --- | --- |
| Connection | No receiver selected, Connecting, Connected, Reconnecting, or Not connected. Connected describes transport, not complete status or power. |
| Availability | Unknown until queried; usable value; temporarily unavailable with a reason; malformed response; or explicitly unsupported. A timeout alone never proves unsupported capability. |
| Evidence / authority | Successful query or validated event, scoped to the current connection. A disconnect invalidates prior authority. |
| Freshness | Time of the last accepted observation and last successful query, independently. Age alone does not change the evidence source. |
| Operation | Idle, pending, confirmed, rejected/not sent, or unconfirmed. Control outcomes are separate from displayed observations. |

Use these translations wherever those conditions need to be visible:

| Internal condition | Everyday wording | Detail and recovery |
| --- | --- | --- |
| Unknown, query pending | “Waiting for status” | No fabricated zero, Off, or default source. |
| Explicitly unsupported field | “Not supported by this receiver” | Explain the field; no retry that cannot help. |
| Feature not shipped | “Not available in this version” | Distinguish software coverage from hardware evidence. |
| Query unavailable/timeout | “Couldn’t read volume” | Refresh status; retain an older value only as “Last known”. |
| Malformed reply | “Couldn’t read the receiver’s reply” | Show the affected field and a Diagnostics link. |
| Disconnected / invalidated | “Last known: −35.0 dB” | “Not connected — this value may have changed.” |
| Event-derived | “Updated by receiver” | An event confirms an observation, not completion of a requested control. |
| Authoritative query | “Checked with receiver” | Time of that field’s successful query. |
| Pending control, later | “Setting volume…” | Requested target is separate from the observed value. |
| Confirmed control, later | “Volume set to −35.0 dB” | Only after an agreeing follow-up query. |
| Unconfirmed control, later | “Couldn’t confirm the change” | “It may have taken effect.” Offer Refresh status, never blind retry. |

The normal Dashboard summary is compact: “Status checked 20 seconds ago” after
a successful full query. After a partial query use “4 of 5 readings checked ·
volume unavailable”; after subsequent events add “Newer receiver updates”.
Provide each field's evidence and age in an adjacent details disclosure and its
accessible description. Always show an exception inline. Do not use one recent
timestamp to imply that all fields are equally current.

Use elapsed age without an invented expiration threshold. “Last known” follows
a failed read or lost connection; a running refresh can say “Checking…” while
retaining the existing value. Silence from the receiver does not prove that
its state changed or that the connection failed.

Events update only the affected validated field. A reconnect starts a new
generation, invalidates all old authority, and requests the five-field snapshot.
Successful fields resolve independently. One failed field never hides valid
siblings. Receiver standby and network disconnection are different conditions.

## Destination contracts

### Dashboard

Purpose: answer the everyday questions in the user-outcomes table. Enter from
startup with a saved receiver, navigation, or the explicit “Open Dashboard”
action after setup. Leave through any destination, context selector, or field
details link.

Information priority is receiver/connection, Main Zone, power, source,
volume/mute, sound mode, freshness/evidence, then the last operation result.
Use three content groups: Power and source; Volume and mute; Sound mode.
Receiver identity belongs in the shell, not another large card.

Phase 2 fields are readable values. Do not display inert sliders, switches, or
menu arrows to advertise future controls. Later controls use these same groups
and are enabled only by validated capabilities, connection state, and operation
policy. Refresh applies to Main Zone; Phase 2 queries Main Zone only.

- Loading: name the selected receiver, retain field labels, show “Waiting for
  status”, and resolve each field independently.
- Empty: with no selected receiver, show “Connect a receiver to see its status”
  and Choose receiver; with a selected receiver, show the initial query state.
- Unavailable: keep the field and its specific reason; if all fields fail, retain
  the group structure and show one summary recovery action.
- Error: field failures stay local; connection failure uses one page banner.
  Offer Refresh status if connected and Try again if disconnected.
- Disabled: state-changing controls are absent in Phase 2. Refresh is disabled
  without a connected receiver or while the same refresh is running, with the
  reason visible. Navigation and setup remain available.

### Receivers

Purpose: find and select the physical receiver without changing playback.
Entry points are first launch without a saved receiver, navigation, Choose
receiver, and connection-recovery links.

Separate “Saved receivers” from “Found on your network”. Rows show name,
model when known, and host to distinguish devices with identical names. The
selected row says “Selected”; only the connected receiver says “Connected”.
Never mark other saved rows Online or Offline without querying them. Deduplicate
discovery results by available device identity/endpoint and keep row order
stable while someone is navigating it.

The primary row action is Connect, or “Use this receiver” during an active
connection. Merely highlighting a row is not a connection. Discover starts a
bounded search on request, keeps manual setup available, and never automatically
selects even a single result. Selection explains “Connects this app; does not
turn on the receiver.”

Manual setup asks for “Receiver address” with a hostname/IP example and an
optional local name. Accept paste, trim surrounding whitespace, and pass through
application validation. Reject unsupported address forms and control characters
with an inline explanation. Phase 2 uses TCP 23 internally; there is no port
editor because saved identities do not store a custom port. Submit is “Save
and connect”, with no duplicate submit while it is running.

Saved connection metadata is local configuration, not a change to the device.
If saving fails, preserve the form and show “Couldn’t save receiver” with Retry
save; do not report that startup persistence succeeded. A saved but unreachable
receiver remains editable for recovery. Configuration failures must not prevent
the architecture's manual-host recovery path.

Later local management: Rename edits the local display name and keeps a stable
selection, with Save/Cancel and validation. Remove shows the exact identity and
explains the local effect; when active, “Remove and disconnect” also closes the
session. Until a reliable undo exists, confirm removal with an explicit action
and Cancel. Do not automatically select another saved receiver. A write failure
leaves the original row intact.

- Loading: “Searching your network…” or a connecting marker on the chosen row.
  Keep saved rows, navigation, and manual setup usable.
- Empty: “No saved receivers” or “No receivers found”, with Discover/Find again
  and Enter address. These are different states.
- Unavailable: unknown model metadata remains “Model unknown”; controls stay
  read-only until supported by evidence.
- Error: explain discovery, save, or connection failure at that location.
  Suggest checking the address and same local network; show a permission remedy
  only when permission denial is actually known.
- Disabled: prevent duplicate searches/connect commits. A working connection
  remains untouched while browsing candidates.

Leaving for another destination does not discard a manual form draft. Background
success stays on the chosen page and offers Open Dashboard.

### Settings

Purpose: understand app configuration and reach help for it. Entry is sidebar
Settings or the platform Settings command; exits are navigation and Diagnostics.
Keep groups short: Appearance, Receiver configuration, Advanced.

Appearance reads “Dark”. General behavior explains startup auto-connect and
that Disconnect lasts until reconnect or the next launch. Configuration health
shows “Saved”, “Imported previous configuration”, “No saved receiver”, or a
specific failure. Put the complete platform path and migration explanation in
a disclosure, with a link to Manage receivers. A valid native file takes
precedence; a legacy import leaves its source intact.

- Loading: only the configuration section waits; navigation remains available.
- Empty: configuration is absent; Add receiver is the next action.
- Unavailable: fixed appearance is descriptive text, not a disabled selector.
- Error: distinguish read, parse, and write failure and preserve the source file.
  Retry the failed configuration step, or open manual receiver recovery.
- Disabled: no fictional preferences or raw-command interface.

### Advanced Diagnostics

Purpose: explain a suspicious reading or connection problem. Enter through
Settings > Advanced > Diagnostics or a field's details link; preserve the
originating destination, scroll, and focus for an explicit Return action.

Show receiver, Main Zone, and current connection scope above the details. Group
Status readings (value, availability, evidence, observation time, query time),
Connection (lifecycle, generation, reconnects), and Errors (time, affected
operation, understandable cause). Later validated rows include input mode,
signal/channel details, and sound mode. Phase 2 renders only data that its
worker exposes; never fill unimplemented diagnostics with plausible numbers or
“None” that suggests a successful inspection.

- Loading: rows resolve independently; navigating here alone starts no extra
  unsupported queries.
- Empty: “No connection activity yet” and Choose receiver.
- Unavailable: “Not collected in this version” for unimplemented diagnostics;
  “Not reported by this receiver” only when evidence supports that statement.
- Error: keep other rows readable; retain an actionable summary on Dashboard.
- Actions: Refresh status when allowed, Return, Back to Settings. There is no
  raw command editor, telemetry upload, or hidden mutation.

## Lifecycle and action rules

Each flow proceeds in the listed order. Focus policy is defined precisely in
the [GUI interaction specification](phase-2-gui-design.md#focus-and-keyboard).
Main Zone is fixed, so context focus rules apply to receiver and page changes only.
“Preserve focus” includes background failure, not just background success.

| Flow | Visible sequence and focus | Available actions and receiver effect |
| --- | --- | --- |
| Saved startup | Load/migrate → named receiver Connecting → fields resolve. Initial focus is the page heading; later updates preserve it. | Auto-connect and five read queries; on failure Try again/Choose receiver. No power command. |
| Unconfigured startup | Receivers → explanation → Discover or Enter address. Initial focus is the setup heading. | Explicit discovery/manual setup; no automatic candidate selection. |
| Discovery | Discover → bounded search → stable results/empty/error. Keep focus on the initiating control. | Manual entry and navigation remain usable; discovery does not change playback or active receiver. |
| Manual entry | Enter address → validate → save/connect → inline outcome. Focus invalid input only on submit; preserve input on failure. | Save and connect; configuration changes and read queries only. Session-only recovery must be labelled unsaved. |
| Receiver switch | Commit choice → clear old values/reset Main Zone → connect new target → resolve fields. Keep the destination. | Close old session, query new; no automatic fallback to another receiver. |
| Initial connection | Connected transport → “Checking status…” → full/partial summary. | Per-field queries; Refresh running state prevents duplicate work. |
| Reconnect | Reconnecting → old readings “Last known” → new connection → fresh per-field query. | Existing application retry policy; Disconnect and Choose receiver stay available. |
| Disconnect | User action → Not connected, last-known values. Preserve page; move focus to Connection menu trigger if its item disappears. | End session/retries for this session; never standby or forget configuration. |
| Manual refresh | Refresh status → Checking… with previous values → independent results. Preserve focus. | Read queries only. Do not infer success for failed siblings. |
| Partial failure | Affected field explains failed read; others remain useful. No modal or focus theft. | Refresh status if connected; View details for investigation. |
| Unsupported model | Model identity and supported read-only fields with coverage explanation. | No state-changing command or forced model override. |
| Pending control, later | Requested target + observed value → Setting/checking. Keep initiating focus. | Execute once, serialize conflicts. Navigation stays available. |
| Confirmed control, later | Agreeing query → observed value and concise confirmation. Preserve focus. | Re-enable eligible actions; no automatic replay. |
| Unconfirmed control, later | Bounded confirmation ends → “May have taken effect” with receiver/Main Zone/target. Preserve focus. | Refresh status or reconnect; refreshing observes current state and does not retroactively prove the original command succeeded. |
| Rejected/not-sent control, later | Explicit failure with target and reason. | A new request is possible only if supported and still wanted; never call this unconfirmed delivery. |
| Shutdown | Close/quit requested → stop accepting intents → release worker/session. | Phase 2 bounded close; Phase 7 explicit cancellation. Do not send standby or claim a pending control was undone. |

Refresh is disabled while disconnected; show Connect/Try again instead.
After an unconfirmed result, Refresh is available as soon as the connection and
application sequencing allow it. A power-on quiet period or confirmation in
flight temporarily explains “Waiting for receiver”; no extra AVR traffic may
bypass that sequencing.

Later, context switching during a sent control requires a decision: stay and
wait, or disconnect/switch with a message that the change may have taken effect.
Never carry a pending command to the new target. Dismiss unsent drafts on a
context switch; reconnect never replays a command.

## Content and privacy

Use “Source” and “Sound mode” consistently in everyday screens; retain exact
receiver-reported source/mode values rather than inventing familiar names.
“Standby” is a power value; “Not connected” is a connection state. Mute is
“Muted” or “Not muted”, not an ambiguous lit speaker icon.

Keep local network addresses visible where they identify the intended receiver.
Put protocol errors in details, sanitize them for display, and never automatically
send diagnostic data away. No permission or network dialog appears without a
related user action or a known platform requirement.

## Usability acceptance

Before implementation is accepted, run these tasks with representative
home-theater owners, including keyboard and assistive-technology users. Start
with 5–8 participants as a formative study, iterate on failures, and record
observations. These are proposed evaluation targets, not measured results.

| Task / scenario | Evidence required |
| --- | --- |
| Glance at Dashboard | Most participants identify receiver, zone, source, volume, and mute within 10 seconds without opening details. |
| Connect with two identically named discoveries | Everyone can distinguish the target by host/model and connect explicitly; nobody thinks discovery powered it on. |
| Recover from one failed field | People can use the remaining fields and find recovery within two actions without technical help. |
| Reconnect after a dropped connection | No participant interprets a last-known reading as freshly checked. |
| Main Zone scope | Every participant understands that all displayed status belongs to Main Zone. |
| Unconfirmed volume, later | People understand that the change may have happened and choose to check status rather than assume failure. |
| Keyboard-only and larger-text setup | Complete setup, navigation, refresh, recovery, and return from Diagnostics without a pointer or clipped essential text. |
| Background update while entering an address | Draft, cursor, focus, and selected destination remain unchanged. |

The companion [GUI acceptance criteria](phase-2-gui-design.md#acceptance-and-handoff)
cover presentation, contrast, platform behavior, and interaction verification.
