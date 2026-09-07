# Version 2 — Phase 2 GUI Design

## Authority and design direction

This is the visual and interaction specification for the
[information architecture](phase-2-information-architecture.md). The
[Phase 2 overview](phase-2-overview.md) sets delivery scope and the
[architecture](phase-2-architecture.md) sets implementation boundaries.
All layouts and measurements below are design targets awaiting implementation
and usability verification.

The experience should feel like a focused desktop utility: clear values,
familiar controls, a stable sense of place, and quiet feedback. Apple’s
[Feedback](https://developer.apple.com/design/human-interface-guidelines/feedback)
guidance informs contextual updates, and
[Typography](https://developer.apple.com/design/human-interface-guidelines/typography)
informs legible hierarchy that survives scaling. Our layouts, sizes, and
receiver-specific interactions are product decisions.

## Iced implementation contract

The GUI targets Iced 0.14. The implementation should use the current
application builder and keep the unidirectional loop visible:

```text
boot/configuration ──> State + initial Task<Message>
                            │
user input ──> Message ──> update(State, Message) ──> Task<Message>
                            │                         │
                            └──── view(&State) <──────┘
worker events ─────────────> Subscription<Message>
```

Use `iced::application(boot, update, view)` as the composition root, attach a
`subscription` for the receiver worker and window events, and configure the
window through the application settings. `view` must be a side-effect-free
projection of state. It must not start discovery, open a socket, query status,
write configuration, or mutate drafts. `update` maps user intent to application
commands and returns `Task<Message>` for one-shot work. Long-lived receiver
output belongs in a `Subscription`; it must not be polled from `view` or from a
periodic timer created for each field. See the official Iced references for the
[application builder](https://docs.rs/iced/latest/iced/fn.application.html),
[Task](https://docs.rs/iced/latest/iced/struct.Task.html), and
[Subscription](https://docs.rs/iced/latest/iced/struct.Subscription.html).

Keep the GUI state separate from receiver policy. A practical state shape is
`route`, `window_class`, `receiver_selection`, `connection`, `main_zone_status`,
`partial_status`, `setup_form`, `diagnostics`, `focus_intent`, and
`announcement`. Message variants should make scope explicit, for example
`Navigate(Route)`, `SelectReceiver(ReceiverId)`,
`RefreshRequested`, `Worker(WorkerEvent)`, `AddressChanged(String)`, and
`WindowResized(Size)`. A worker event carries receiver identity, Main Zone scope, and
connection generation; `update` drops an event that does not match the current
context before touching visible state.

A `Task` completion must carry an operation/request identity as well as its
result. `update` accepts a completion only when that identity is still current;
late discovery, connection, query, or focus work cannot overwrite a newer
selection. Cancel or supersede the previous in-flight task when the app state
requires it. Do not encode policy in widget closures or create network futures
inside a button's `on_press` callback.

The worker owns one session and exposes typed commands/events through a bounded
channel. The subscription is keyed by worker lifetime and remains stable while
ordinary status fields change. Rebuilding the subscription for every render can
duplicate event streams or lose events. A subscription ending, a closed worker
channel, and a receiver disconnect are distinct messages and map to distinct
user-facing states.

Maintain the established dark appearance with opaque, readable surfaces.
Platform-specific glass, blur, or decorative animation is not a requirement.
Apple’s [Dark Mode](https://developer.apple.com/design/human-interface-guidelines/dark-mode)
guidance treats a dark-only interface as a special choice and emphasizes testing
contrast. Here, dark-only remains a product constraint; higher contrast and
reduced motion must still work. No archived visual reference constrains this design.

## Design decisions users should notice

| Decision | Benefit |
| --- | --- |
| One receiver selector and fixed Main Zone context | The target stays clear without implying unsupported zone features. |
| Three everyday content groups | Power/source, volume/mute, and sound mode form a readable sequence. |
| Read-only values look like values | Phase 2 feels complete without suggesting inactive controls can be used. |
| Text status near the affected item | People understand partial failures without opening Diagnostics. |
| A direct recovery action | Setup, disconnection, and failed reads never end in a dead screen. |
| Background updates preserve focus and page | Reading or editing is not interrupted by network activity. |
| Measurable contrast, scaling, and input behavior | Accessibility is verifiable at implementation time. |

## Shell and responsive layout

### Wide layout

Target a default client area of 1,080 × 720 logical pixels. At client widths
of 960 or more, show a 240 px leading sidebar with:

1. Receiver selector: friendly name with a text connection summary below.
2. Dashboard, Receivers, Settings in that order.

Keep branding small. Do not repeat model, host, or Disconnect
in a large sidebar footer. Full identity is available through Receivers.

The remaining area has a persistent context toolbar containing the fixed “Main
Zone” label, Refresh status, and a labelled Connection menu.
Below it, the destination owns one vertical content scroller. Its title,
banner, and groups scroll together. The content has 24 px padding and a
maximum width of 920 px, aligned toward the leading edge.

### Compact layout

From 720–959 px client width, replace the sidebar with a labelled Navigation
button and receiver selector above the context toolbar. Navigation opens a
drawer with the same destination names and order. Indicate the current
destination on the button or adjacent title. Selecting a page closes the drawer;
Escape returns focus to Navigation.

Do not shrink three destinations to unexplained icons. Apple notes that sidebars
consume space and that compact navigation can work better in constrained
layouts. The labelled drawer is this app’s adaptation of that principle.
Source: [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars).

The toolbar wraps if names or larger text require it. The fixed Main Zone context and selected receiver remain visible. All essential values and actions remain reachable in
a 720 × 520 client area at default text size.

### Reflow and scroll ownership

Use a single reading column; related fields can share a row when the actual
content width is at least 600 px. Below that width, or when enlarged text no
longer fits, stack in semantic order. Avoid masonry, fixed-height cards, or
reordering volume ahead of the receiver/Main Zone context.

At larger text sizes or short windows, allow the toolbar to wrap and the page
to scroll; never clip content to preserve a nominal card height. Sidebar
navigation stays reachable. Long menus and the open navigation drawer may have
their own bounded scroller; ordinary cards and diagnostic tables do not.

Maintain page scroll on refresh and restore it when returning from Diagnostics.
Scrolling a page must not change a slider under the pointer. Menus, tooltips,
banners, and window chrome must not obscure the keyboard focus target.

## Page compositions

### Dashboard

The shell supplies receiver and Main Zone context. The page title is “Dashboard”.
Immediately below it, show a compact summary such as “Status checked 20 seconds
ago · Read-only”. “Read-only” is explained once: “You can view receiver status
in this version.”

Use three groups in this order:

1. **Power and source:** clear Power and Source labels with their values.
2. **Volume and mute:** prominent tabular volume, visible dB unit, and separate
   “Muted” or “Not muted” text. Muting never turns the volume display into zero.
3. **Sound mode:** exact observed value, with room for wrapping.

In Phase 2, these are labelled values, not switches, disabled sliders, or
dropdowns. A small View details disclosure exposes each field's evidence/age.
Exceptions are already visible beside their field. Healthy fields do not each
need a bright badge.

Later controls replace the values in place, retaining an observed value and
separate requested target while pending. Only validated choices are editable.
A reported source or sound mode outside the setter allowlist is still readable.


### Receivers

Show Saved receivers, then Find a receiver with a Discover button and Enter
address alternative. A candidate row contains name, model if known, host,
and one explicit connection action. The selected row remains in place while
connecting; newly discovered rows append without moving keyboard focus.

Manual setup expands inline with Receiver address, optional local Name, Save
and connect, and Cancel. Use examples as supporting text, not as replacements
for field labels. Port 23 is an implementation detail; no port input ships in
Phase 2. Show validation on submit or after leaving an edited field, never on
every incomplete keystroke.

Later, labelled Receiver actions menus contain Rename and Remove. Removal uses
the precise effect and confirmation behavior specified in the IA.

### Settings and Diagnostics

Settings presents Appearance, Receiver configuration, and Advanced as short
sections. Configuration paths and migration mechanics are disclosed on demand.
A native Settings menu item leads to this same destination.

Diagnostics is a separate page with an explicit return path, receiver/Main Zone
scope, status-reading details, connection details, and recorded errors.
At wide sizes use a labelled table; at narrow sizes render the same rows as
label/value blocks in the same order. Expand long errors vertically. Do not
turn missing diagnostic support into a fabricated zero or “No errors”.

## Wireframes

Square brackets below denote interactive controls only. All values are
illustrative. State variants reuse the same shell and content groups.

```text
WIDE — PHASE 2
+----------------------+-----------------------------------------------+
| [Living Room AVR v]  | Main Zone   [Refresh status] [Connection v]   |
| Connected            +-----------------------------------------------+
|                      | Dashboard                                     |
| > Dashboard          | Status checked 20 seconds ago · Read-only     |
|   Receivers          | Power and source                              |
|   Settings           | Power   On              Source   Blu-ray      |
|                      |                                               |
|                      | Volume and mute                               |
|                      | -35.0 dB                Not muted             |
|                      |                                               |
|                      | Sound mode                                    |
|                      | Dolby Surround              [View details]   |
+----------------------+-----------------------------------------------+

COMPACT — PHASE 2
+---------------------------------------------------------------------+
| [Navigation: Dashboard]   [Living Room AVR v]  Connected             |
| Main Zone (fixed)           [Refresh status] [Connection v]       |
+---------------------------------------------------------------------+
| Dashboard                                                           |
| Status checked 20 seconds ago · Read-only                            |
| Power and source                                                    |
| Power  On                                                           |
| Source Blu-ray                                                      |
| Volume and mute                                                     |
| -35.0 dB · Not muted                                                |
| Sound mode  Dolby Surround                         [View details]   |
+---------------------------------------------------------------------+

RECEIVERS — FIRST SETUP
Receivers
Connect a receiver to see its status. This does not turn it on.
No saved receivers
Find a receiver       [Discover] [Enter address]
Found on your network
Living Room · AVR-X3800H · 192.0.2.20           [Save and connect]

MANUAL SETUP — SAME PAGE
Receiver address  [__________________________]
Example: 192.0.2.20 or receiver hostname
Name (optional)   [__________________________]
[Save and connect] [Cancel]

SETTINGS
Settings
Appearance              Dark
Receiver configuration  Imported previous configuration
[Configuration details] [Manage receivers]
Advanced                [Diagnostics]

DIAGNOSTICS
[Back to Settings] [Return to Dashboard]
Diagnostics · Living Room AVR · Main Zone
Status readings
Field   Value      Evidence    Last observation   Last successful query
Volume  -35.0 dB   Event       10:42:08           10:41:50
Connection
Connected · generation 7                       [Refresh status]
Errors
Volume query timed out at 10:41:55              [Details]

STARTUP — SAVED RECEIVER
Living Room AVR · Connecting
Dashboard
Connecting to your receiver…
Power  Waiting for status     Source  Waiting for status
Volume Waiting for status     Mute    Waiting for status
Sound mode Waiting for status                  [Choose receiver]

DISCONNECTED
Living Room AVR · Not connected
Dashboard
Connection lost. Previous readings may have changed.
[Try again] [Choose receiver]
Volume  Last known: -35.0 dB

RECONNECTING
Living Room AVR · Reconnecting
Dashboard
Trying to reconnect. Previous readings may have changed.
Volume  Last known: -35.0 dB                     [Disconnect]

PARTIAL ERROR — STILL CONNECTED
Dashboard
4 of 5 readings checked · volume unavailable    [Refresh status]
Power On · Source Blu-ray
Volume Couldn’t read volume                    [View details]
Mute Not muted · Sound mode Dolby Surround

LATER CONTROL — UNCONFIRMED
Volume and mute
Last observed -35.0 dB · Requested -34.5 dB
Couldn’t confirm the change. It may have taken effect.
Living Room AVR · Main Zone                    [Refresh status]
```

## Visual system

### Color and contrast

Use semantic roles. The following opaque sRGB tokens are the baseline; platform
contrast settings may strengthen them. Every state also has text or a structural
cue. A decorative separator is different from a control boundary.

| Role | Token | Use |
| --- | --- | --- |
| Canvas | `#0D1117` | Window content backdrop |
| Surface | `#161C24` | Group and sidebar background |
| Raised surface | `#222B36` | Menus and temporary overlays |
| Decorative divider | `#36414F` | Nonessential section separation |
| Control boundary | `#768391` | Essential input/slider edges |
| Primary text | `#F3F5F7` | Values, headings, action labels on dark surfaces |
| Secondary text | `#B3BEC9` | Supporting labels and timestamps |
| Disabled text | `#96A3B0` | Readable inactive label, with reason nearby |
| Interactive accent | `#70B7FF` | Links, selection marker, primary action fill |
| On-accent text | `#0D1117` | Text on the bright action fill |
| Brand red | `#C7353F` | Restrained identity detail; not a connection or error signal |
| Success | `#7DD3A5` | Confirmed-result symbol and text |
| Warning | `#F5CC75` | Unconfirmed/attention symbol and text |
| Error | `#FF9797` | Failed-operation symbol and text |
| Focus | `#9BD0FF` | 2 px outer ring with 2 px dark separation |

Require at least 4.5:1 for all specified text pairs, including useful inactive
labels, and 3:1 for essential control boundaries and focus against adjacent
surfaces. Never use white text on the bright blue fill or the divider token
as the only input boundary. Verify hover, pressed, selected, disabled, error,
and higher-contrast variants in the rendered interface.

These are product acceptance thresholds informed by
[WCAG 2.2 contrast and interaction criteria](https://www.w3.org/TR/WCAG22/).
For this native app they are a testing baseline, not a claim of web conformance.

### Typography, spacing, and shape

Use the host platform's system sans-serif with ordinary licensed fallbacks.
Prefer regular and semibold weights. Default roles in logical pixels are:
page title 28/34, group title 18/24, body 16/24, label 14/20, supporting text
14/20, and volume 40/48. Tabular numerals stabilize changing values. Use the
minus sign and a visible dB unit; assistive output speaks “minus 35 decibels”.
Do not abbreviate unavailable values to a dash without accompanying text.

Text must scale to 200% and wrap while preserving hierarchy. Host DPI and
supported text-size preferences take priority. If the toolkit cannot consume
text-size preferences, expose equivalent session text scaling through View
without changing Phase 2 receiver configuration. Reserve room for longer
localized labels, names, translated messages, and right-to-left layouts.

Use 4/8/12/16/24/32 spacing, 16–24 px group padding, 8 px group corners, and
6 px control corners. Shadows are limited to overlays; normal groups use
spacing and subtle surfaces. Avoid a separate elevated card for every label.

Interactive hit areas are at least 36 × 36 logical pixels; primary controls
and volume adjustment targets use 44 × 44. Keep 8 px between adjacent small
actions. These are product targets; glyph size can remain smaller.

## Reusable components

All components inherit the color, scaling, focus, and phase rules above.
Static values remain available to assistive reading but do not add Tab stops.
Interactive controls have role, accessible name containing the visible label,
value/selection state, enabled state, and any reason for being disabled.

### Content and appearance

| Component | Purpose, content, and visual treatment |
| --- | --- |
| Application shell | Persistent receiver/context plus selected destination; one leading selection marker, restrained surfaces. |
| Receiver selector | Friendly name and connection text; menu reveals saved choices and Manage receivers with host disambiguation. |
| Connection badge | Plain text plus optional symbol; Connected, Connecting, Reconnecting, Not connected. Errors are explained separately. |
| Main Zone context | Fixed text label in every v2 phase; there is no zone selector. |
| Status group | Heading, related labels/values, local exceptions, View details; one surface per meaningful group. |
| Power | On/Standby text in Phase 2; later explicit Turn on/Standby action showing the observed state. |
| Source selector | Exact observed source; later a labelled menu with checked current item and validated alternatives. |
| Volume display and slider | Visible observed dB value; later target preview, endpoint labels, slider, and minus/plus buttons. |
| Mute | Muted/Not muted text; later a labelled Mute/Unmute action with observed state. |
| Sound-mode selector | Exact mode with wrapping; later validated choices and checked current item. |
| Field availability | Inline reason replaces a missing value; older data, if shown, has a Last known prefix. |
| Freshness/evidence | Compact page summary plus per-field disclosure; neutral text except actionable exceptions. |
| Pending indicator | Setting/Checking text beside the item; target remains distinct from observed value. |
| Error banner | One scoped cause and primary recovery action, optional Details; no stack of repeated network errors. |
| Discovery list | Stable, distinguishable candidates and explicit Connect action; searching/empty/error text within the list section. |
| Manual form | Persistent labels, example, inline error, Save and connect/Cancel; no placeholder-only label. |
| Diagnostic table | Semantic headers and grouped rows; narrow layout becomes ordered label/value blocks. |
| Result panel | Target, receiver, zone, confirmed/rejected/unconfirmed outcome; persistent recovery for unresolved results. |

### State, enablement, and input

| Component | Rules and accessible example |
| --- | --- |
| Application shell | Destinations remain available through loading/error; “Dashboard, selected”. Opening navigation changes no receiver state. |
| Receiver selector | Browse with arrows; Enter commits and Escape dismisses. Block only an actual transition/decision, not the whole network timeout. “Receiver, Living Room AVR”. |
| Connection badge | Readable status; announce a meaningful transition once, without focus movement. It is not a button unless a details action is explicitly labelled. |
| Status group | Loading, partial, unknown, failed, historical, or available fields remain readable. Details uses Enter/Space and announces expanded state. |
| Power | Later only: explicit action enabled by capability and application policy, including the quiet period. “Turn on Main Zone”. |
| Source selector | Later only: arrows/type-ahead browse; Enter commits, Escape cancels. No command on hover/focus. “Source, Blu-ray, Main Zone”. |
| Volume display and slider | Later only: validated range/0.5 dB steps, release-to-commit and equivalent step buttons. “Main Zone volume, minus 35 decibels”. |
| Mute | Later only: Enter/Space sends one request; pending is not an optimistic toggle. “Mute Main Zone”, with current Not muted state. |
| Sound-mode selector | Later only: same menu behavior as Source; unavailable allowlist gives a reason, not arbitrary options. “Sound mode, Dolby Surround”. |
| Field availability | Reason is in the accessible description and visible text. Failed read is not automatically Unsupported. |
| Freshness/evidence | No live announcement every second; exact times available through details. “Volume, updated by receiver, 20 seconds ago”. |
| Pending indicator | Announce once on a user request; prevent duplicates and serialize conflicts. Navigation and inspection remain possible. |
| Error banner | Action names match the remedy. Background error uses a polite announcement; focus moves only under the rules below. |
| Discovery list | Keyboard highlighting does not connect; stable row identity survives incoming results. “Living Room, 192.0.2.20, Connect”. |
| Manual form | Editing and paste work normally; Enter submits valid input. Duplicate submit disabled, reason visible; invalid submit focuses its first error. |
| Diagnostic table | Screen readers associate headers/values; rows are not Tab stops unless they contain a disclosure/action. |
| Result panel | Preserve focus and expose exact outcome. Refresh observes state; it never silently retries a setter or reclassifies history. |

## Interaction details

### Connection and feedback

Refresh has one in-flight state: “Checking status…”. Disable duplicate refresh
activation, keep current values, and update fields independently. While
disconnected, Refresh is disabled with a reason and Try again is directly
available in the banner. Following an unconfirmed command, allow Refresh as
soon as the connection and application sequencing permit.

Represent work in the next UI update; aim for feedback within 100 ms of input.
Avoid skeleton shimmer. Show progress text immediately; introduce a small
indeterminate indicator only if work continues beyond roughly 300 ms to avoid
flashing. Network duration and timeout come from application policy, never a
percentage guessed by the view.

Ordinary success stays local. Unresolved failure/unconfirmed text persists until
resolved or explicitly dismissed; dismissal clears presentation, not the stored
outcome. Coalesce repeated reconnect errors into one banner. Do not use alerts
for routine connection loss or startup. Apple recommends reserving alerts for
necessary interruptions; see
[Alerts](https://developer.apple.com/design/human-interface-guidelines/alerts).

### Confirmed controls, later

Enable controls only for validated capabilities. Preserve exact observed values
outside setter allowlists; do not invent or preselect an alternative. A generic
“Not available in this version” explanation describes Phase 2 without showing
a bank of disabled controls.

A submitted operation keeps observed state visible, labels the requested target,
and remains pending until the agreeing query. Other-page navigation is allowed.
Conflicting commands are disabled/serialized by application policy. A receiver
context change discards unsent drafts; switching after sending requires the IA's
pending-operation decision. Socket delivery and a matching event alone are not
authoritative confirmation.

Power-on's one-second quiet period precedes all subsequent AVR traffic, including
Refresh. Use “Waiting for receiver…” with the existing requested operation.
Mute does not change the volume number, power does not imply a connection
result, and Disconnect never means Standby.

### Volume interaction, later

Volume represents an external amplifier, so preview and commit must be explicit.

1. Pointer drag adjusts a clearly marked target preview within the validated
   range in 0.5 dB steps. Preserve the observed numeric reading alongside it.
2. Pointer release submits one absolute target. Escape, losing the interaction
   before release, or changing context cancels the unsent draft.
3. Minus/plus buttons provide a non-drag alternative. Each activation requests
   one step based on the latest eligible observed value; ignore duplicate
   activations while the operation is pending.
4. With slider focus, arrow keydown previews one step; keyup commits it. Coalesce
   repeats held in one gesture into a single absolute target on release.
   Losing focus before keyup cancels the draft. Show the commit behavior in
   the slider description. Home/End may preview validated
   endpoints but require Enter to submit; focus loss cancels that draft.
5. A disconnect, newly unavailable field, or receiver update during an unsent
   gesture cancels the draft with a local explanation. A late keyup/release
   cannot submit into another receiver or connection generation.
6. A failed confirmation retains the observed value, target, and Unconfirmed
   result. No optimistic success, accumulated repeat queue, or automatic replay.

Scroll-wheel input never changes volume. Label endpoints from capability
evidence, not hardcoded guesses. Keep step buttons accessible even when precise
dragging is difficult. The visible scale and exact value apply Apple's
[Sliders](https://developer.apple.com/design/human-interface-guidelines/sliders)
guidance; deferred network commit is this product's behavior.

## Focus and keyboard

Tab/Shift-Tab traverse interactive controls in reading order: navigation trigger
if present, receiver selector, destination navigation,
toolbar actions, recovery actions, content controls, details links. A
programmatically focused page heading supplies an entry point without making
every status value a Tab stop. Standard assistive reading reaches all text.

| Event | Focus behavior |
| --- | --- |
| First render | Focus the Dashboard or receiver-setup heading once. |
| User activates navigation | Announce the new page; keyboard activation moves to its heading. Restore the previously focused content on an explicit Return from Diagnostics. |
| Receiver menu commit | Return to its trigger; update the named receiver, preserve destination, and announce the change once. Main Zone remains fixed. |
| Query completes, event arrives, connection drops | Preserve focus, selection, draft, caret, and scroll. Use a concise status announcement if meaningful. |
| Invalid submitted form | Focus the first invalid field and associate its message. |
| Error after user has moved elsewhere | Preserve the new focus; leave recovery visible. |
| Focused item is removed | Move to the next surviving row, previous row, or section heading, in that order. |
| Menu/drawer/dialog closes | Return to its invoking control; if gone, use the nearest surviving context control. |

Use Enter/Space for buttons, arrows and type-ahead for menus, Escape for an
uncommitted menu/draft, and platform text-editing shortcuts unchanged. Expose
Refresh status in a menu with Command-R on macOS and Ctrl-R on Windows/Linux.
On macOS provide Settings via Command-comma and standard Close/Quit commands.
No global single-key binding, system volume key, or unrelated scroll changes
the AVR. Platform expectations inform these bindings:
[Keyboards](https://developer.apple.com/design/human-interface-guidelines/keyboards).

## Accessibility, motion, and native behavior

Provide semantic names, roles, values, selected/expanded/busy states, group
headings, and error associations through the actual platform accessibility
bridge. Iced-rendered labels alone do not establish screen-reader support.
Treat accessibility as an Iced capability gate: verify which widgets and custom
containers expose semantics in the selected Iced version, then add a supported
accessibility integration or narrow the support claim before release. Never
ship a visual-only “accessible” label. Verify VoiceOver on macOS, Narrator on
Windows, and Orca on Linux where the chosen runtime is supported. Record and
address any toolkit limitations before claiming support. Apple recommends auditing the actual accessible interface;
see [Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility/).

Normal status updates use a polite, coalesced announcement. Avoid reading every
volume event, ticking timestamp, or spinner frame. A user-requested result is
announced once with receiver and Main Zone scope. Avoid duplicate speech when focus
already reads the result.

Honor Reduce Motion and Increase Contrast when available. Reduced motion removes
animated transitions and spinners while retaining static progress text.
If host preferences cannot be read, provide session overrides alongside text
scaling. Opaque surfaces satisfy reduced-transparency needs. Never pulse or
flash to indicate a healthy connection; sound is not required for feedback.

Use native window chrome and host menus; reserve their space in layout. Keep
resize, minimize, maximize/zoom, standard editing shortcuts, and platform
Close/Quit behavior. The app has no hidden always-running tray service. Closing
the sole receiver window releases its session; on a host that keeps the process
alive, reopening follows the normal saved-receiver flow. Phase 2 uses bounded
worker teardown; Phase 7 supplies explicit graceful cancellation. Do not block
window closure on an unbounded network call or claim closure undid a command.

Iced view renders state. Update maps user intent, handles presentation state,
and requests tasks. Subscriptions carry worker updates tagged with context.
Receiver capability, discovery, validation, query sequencing, retry, confirmation,
and transport policy remain in the application/worker boundary described in
the architecture. The GUI discards stale-context messages and never implements
a second policy engine.

## Acceptance and handoff

Validate the [IA user tasks](phase-2-information-architecture.md#usability-acceptance)
and capture these implementation cases before release. These are acceptance
requirements, not evidence that a GUI has already passed.

| Area | Required evidence |
| --- | --- |
| Layout | All destinations at 1,080 × 720 and 720 × 520, 100% and 200% text; wrapped long receiver/source/mode names; no clipped focus or essential controls. |
| Visual contrast | Measured text, essential boundaries, and focus pairs across default, hover, pressed, selected, disabled, error, and higher-contrast states. |
| Phase 2 honesty | Five read-only Main Zone values, no fake sliders or unsupported diagnostic values. |
| First connection | Saved, unconfigured, duplicate discovery names, zero results, invalid address, failed save, and unreachable receiver all retain usable recovery. |
| Partial status | Query timeout, malformed response, unsupported field, and receiver event affect only their own values. |
| Lifecycle | Initial refresh, disconnect, reconnect, and delayed old-generation updates never relabel old data as current. |
| Focus and semantics | Complete keyboard workflow and platform screen-reader reading of value, scope, availability, and outcome; no focus theft during typing; record Iced/widget accessibility limitations. |
| Later volume | Pointer/keyboard release, Escape, focus loss, held key, wheel scrolling, receiver switch, and incoming receiver event during a draft. |
| Later confirmation | Pending, rejected/not-sent, confirmed, and unconfirmed cases; power-on quiet period; no automatic setter replay. |
| Motion and native behavior | Reduced motion, increased contrast, resize, minimize/restore, Close/Quit, and reopening on each supported host. |

Visual review alone is insufficient for focus, assistive semantics,
external-device timing, and confirmation behavior. Use deterministic worker
scenarios plus hands-on platform checks; record usability findings and revise
the specification where user behavior contradicts the intended outcome.
