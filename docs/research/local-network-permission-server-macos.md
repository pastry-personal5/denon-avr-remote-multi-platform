# Local Network permission for the Control API server (spike S5)

**Six runs done on 2026-10-10. The owner confirmed outcome B** (see "Sixth run"
below and the Outcome at the end; [the runbook](../runbook-for-human.md) has the last checklist). Written 2026-10-09 with the programs it uses. The owner's hand check on the
Mac, with the receiver. Its output is the **Results** and **Outcome** sections at the end
of this file, which fix how step 10 of
[phase 5](../v4/phase-5-operator-clients-overview.md) packages and signs the server. See
the architecture's [S5 section](../v4/phase-5-operator-clients-architecture.md#s5-what-the-hand-check-decides)
for why the check exists.

> **Archived 2026-10-10.** The programs this note runs (`tools/s5/build-bundles.sh`, `tools/s5/host/`,
> `tools/s5/app/`) were moved to [`docs/archive/s5/tools/`](../archive/s5/README.md), for reference only.
> The paths below are their original ones: move that directory back to `tools/s5/` to run them. The
> twelve screenshots that were in `docs/archive/s5/` were deleted, so the run records below are the only
> record of what they showed. The receiver's address is written `<receiver address>`.

## The question

In 3.0.0 the app itself talks to the receiver. In phase 5 the GUI starts
`denon-avr-api-server` as a child, and the **server** opens the receiver connection. macOS
asks the user once to let an app "find and connect to devices on your local network", and
it is the 3.0.0 signing fix (`25401e7`) that taught us this can fail silently: the
permission was stored under one identity while the kernel checked another, and every
connection to the receiver failed with "No route to host" and no dialog.

The checks answer these, in this order:

1. When the server is a child nested in the signed app, does the permission the user gives
   the app cover the server's connection to the receiver, and to the SSDP scan?
2. If not, does the server need a grant of its own, and does it get asked?
3. Does a grant survive the next build of the app? Does the grant that 3.0.0 users already
   have (`com.denonavr.remote`) cover the new nested server?
4. What does a server started from a terminal, which is how a standalone server is started,
   need?

## What the programs are, and are not

- `tools/s5/build-bundles.sh` builds test app bundles: `own` and `same` by default, `deep` only
  if `--cases` names it, and `--cases` also sets the order the build prints its `open` lines in
  (the order to run them). Each holds `s5-app` (the
  bundle's main executable, a small AppKit application with a window: the stand-in for the GUI),
  `s5-host` (the helper `s5-app` starts), and the real `denon-avr-api-server`, and each is signed
  differently:

  | Case | How the bundle is signed |
  | --- | --- |
  | `deep` | `codesign --force --deep --sign - --identifier <id> <app>`, which is what `tools/package-macos.sh` does today |
  | `same` | the nested programs first with `--identifier <id>`, then the bundle with `<id>`, inside out |
  | `own` | the server first with `--identifier <id>.server`, the helper with `<id>`, then the bundle with `<id>` |

- `s5-app` (`tools/s5/app/main.swift`, built with `swiftc`) shows a window, starts `s5-host`, and
  prints what it prints, live. The chain is `s5-app` -> `s5-host` -> server, so the application
  macOS holds responsible for the server's network use is `s5-app`, a real AppKit app, as the GUI
  will be. The window says FINISHED when the run is over.
- `s5-host` (`tools/s5/host`, standard library only, not part of the Cargo workspace) does what the
  planned launcher does: it starts the nested server with `--data-dir <dir> --exit-with-parent`, a
  held standard-input pipe, and standard error in a file, waits until the Operator socket accepts,
  then asks the server, over that socket, to (a) check its health, (b) register the receiver by
  address, (c) read its state, which makes **the server** open a TCP connection to the receiver, and
  (d) run an SSDP scan, which is UDP multicast. It records each result in
  `/tmp/s5/result-<case>.txt`; an earlier report of the same name is kept as
  `result-<case>.<time>.txt`, never overwritten.
- The state read is retried every 5 seconds for about 90 seconds, so you have time to answer
  a permission dialog while the run is going. **Let every run reach its SUMMARY line.**
- `s5-host --drive` starts nothing and only drives a server that is already running. It is for
  the terminal cases. Give it a `--label` that matches its `--data-dir` (`d-t1` and `t1`); it
  notes a mismatch in the report.
- `S5_MAIN=plain bash tools/s5/build-bundles.sh` builds the first run's shape instead, with
  `s5-host` as the main executable. Use it only to compare.
- **Added after run 4** (2026-10-10 evening), none of it changing what the probe sends or when:
  - The build script refuses an `--id` that is not one it printed (`com.denonavr.s5.<14 digits>`)
    unless a `--tag` is given, and `--production` without a tag, because run 4's "rebuild" used a
    12-digit id and reused the file names of runs 1 and 2. `--rebuild` reads the last fresh build's
    id, tag and cases from `target/s5/last-build.txt`, so no id is retyped.
  - `s5-host` writes `started at <time>`, puts a wall-clock time on each attempt line, checks before
    it starts the server whether other S5 or Denon programs are running (by executable name, never
    by command line, because the repository path contains `denon-avr` and the `open -W` that waits
    for the run names the bundle), says so with a `NOTE` if `state` was refused and then passed, and
    keeps an earlier server log as it keeps an earlier report.
  - The window has three buttons ("A dialog appeared", "I clicked Allow", "I clicked Don't
    Allow") and a note box. They write time-stamped lines to `/tmp/s5/notes-<case>-<tag>.txt`, a file
    of their own, because three runs lost the dialogs' names and clicks to memory. **The window was
    compiled and never run when this was written.**
  - A system-log capture was considered for the helper and left out: the server uses ordinary BSD
    sockets, so the permission decision is logged by system daemons whose message text is not
    known, and the code path could not be exercised in the sandbox. The runbook has one manual
    `log show` at the end of a session instead.
- **Added after run 5** (2026-10-10 evening), none of it changing what the probe sends or when:
  - Every program in a bundle (the host, the server, the window program) gets a **Mach-O UUID of its
    own** in every build, patched into the copy in the bundle by `s5-host --retag-uuid` before the
    bundle is signed (the original builds are never touched). A rebuild therefore changes each
    program's code hash and UUID, as a release does, and no two bundles share a program's UUID, which
    runs 1 to 5 did not guarantee. **This is a change of shape from run 5**; run 6's fresh `own` is the control.
    `S5_SAME_BYTES=1` skips both this and the run-path. An old host without `--retag-uuid` makes the
    build fail, and an unfinished bundle is removed.
  - `--copy` (one case) makes `target/s5-copy/` hold a byte-identical copy of the bundle, for the
    install-flow test. A bundle in a `…-copy` directory gets `-copy` in its label (window title,
    report, data directory, notes), in the host and in the window alike.
  - Reports carry `UUID` lines (host, window program, server); `s5-host --print-uuid FILE` prints one.
    The window shows where its bundle is.
- Nothing here writes to the receiver. The state read and the scan are reads.

## What a dry run already showed (2026-10-09, no receiver)

Run on the development Mac (macOS 26.7.1, arm64) with the receiver address set to
the loopback address, so every receiver connection was refused by the Mac itself:

- All three bundles build, verify (`codesign --verify --strict`), and run. The server starts
  in 0.1 s, answers health, takes the ad hoc receiver, and stops with status 0 when the host
  closes its pipe.
- The identifiers `codesign -dvvv` reports:

  | Case | `s5-host` | nested server |
  | --- | --- | --- |
  | `deep` | `<id>` | `<id>` |
  | `same` | `<id>` | `<id>` |
  | `own` | `<id>` | `<id>.server` |

  So `--deep --identifier` already gives the nested server the bundle's identifier. **Check 1
  of the architecture's S5 list (identity) is answered; what remains is how the system behaves,
  which only a run with the receiver and a person can show.**
- A refused connection to the receiver comes back as
  `503 … AVR connection failed: Connection refused (os error 61)`. The symptom of a Local Network
  block is different: `No route to host (os error 65)`, which the program marks
  "looks like the OS refused it". A receiver that refuses because Network Control is off or because
  another Telnet client holds its single connection also says "Connection refused", so read the
  error number.

## First run, 2026-10-10 (plain executable as the main program)

The owner ran Parts 2 and 5 with the first version of the tools, in which `s5-host`, an
executable with no window, was the bundle's main program. The files are in `/tmp/s5`.

| What | Result |
| --- | --- |
| Terminal, server on `/tmp/s5/d-t2` (`result-t1.txt`) | `state` passed on attempt 1 and `discovery` found the receiver. The report is labeled `t1` but its data directory is `d-t2`; one of the two terminal reports was overwritten, so there is one terminal result and the other is **not known** |
| `deep`, `open` | `No route to host (os error 65)` on every attempt for 25 s, with no success. Stopped by the owner, `ParentGone` in the server log |
| `same`, `open` | The same, for 15 s; the owner stopped it (`SIGTERM` to the server) |
| `own`, `open` | The same, for 15 s; the owner stopped it (`ParentGone`) |
| Dialog | The owner saw one permission dialog, once, and clicked Allow. It was not recorded **which run** it came in or **what name** it showed. After that no dialog appeared |
| Settings → Local Network | **No row for any S5 app** |

What this does and does not show:

- Each `open`-launched bundle was blocked the way the 3.0.0 bug blocked the app (`os error 65`), and
  the three signings did not differ, but each run was stopped well inside the 90 seconds the
  state read waits, and `discovery` never ran. So no case had its chance to pass.
- A server started from a terminal reached the receiver at once, because the terminal already had
  the permission. That is the expected result for Part 5 and says nothing about bundles.
- With no row for any S5 app in Settings, the system never registered the S5 bundles as asking. The
  one dialog may have been for the terminal application (the Part 5 runs came first), and not for
  a bundle. If so, the plain executable never got a dialog at all. That is the doubt the AppKit
  stand-in removes: the real GUI is an AppKit application, and a plain no-window program may simply
  not be offered the dialog.
- So run 1 settles no outcome. **Run 2 uses the AppKit stand-in below.**

## Second run, 2026-10-10 (AppKit stand-in `s5-app`)

Base id `com.denonavr.s5.20261010001703`, built 00:17. Files in `/tmp/s5`.

| Case | Time | `state` | `discovery` |
| --- | --- | --- | --- |
| `deep` | 00:17 | **Blocked** for the whole run: `No route to host (os error 65)` on all 18 attempts, 85 s | **FAIL**, 502: `SSDP discovery failed: No route to host (os error 65)` |
| `same` | 00:19 | **PASS, attempt 1** | PASS |
| `own` | 00:20 | **PASS, attempt 1** | PASS |
| `t1` (driven from a terminal, `d-t1`) | 00:21 | PASS, attempt 1 | PASS |
| `t2` (driven from a terminal, `d-t2`) | 00:21 | PASS, attempt 1 | PASS |

What can be said:

- **Both kinds of traffic are governed together.** `deep` was blocked for TCP to the receiver and for
  the SSDP scan alike, with the same error, so the permission covers the multicast scan too. The
  question "does discovery need something TCP does not" is answered: not in this run.
- **`own` passed at attempt 1 with a server whose identity (`<id>.server`) differs from the
  bundle's**, and nobody had answered a dialog for that identity. If the permission were keyed to
  the nested executable's own identity, `own` would have blocked until its server was granted. It
  did not, so the permission is attributed to something other than the nested server's own
  signature: the app that started it, or something broader. That is the point that decides
  between outcomes A and B, and it favors A.
- **What the owner reported afterwards.** `same` and `own` were started with `open -W`, like `deep`,
  so the launch method is not the difference. During `deep` a dialog appeared and the owner clicked
  Allow before the run ended; the name it showed was not recorded, and `deep` still failed every
  attempt. Settings → Local Network now lists **S5-same** and **S5-own**, and no S5-deep.
- **What this points to.** Every case that passed has a row in Settings and the one that did not has
  none. The row names are the bundles' file names (`S5-same`), not the bundle names (`S5 same`).
  Two things are consistent with that and with nothing else seen so far: (a) the system keys a
  grant to the app's **name or path**, not only to its bundle id, so `same` and `own`, rebuilt in run 2
  at the same paths as run 1 under new ids, inherited what run 1 gave them; or (b) the dialog during
  `deep` was for something other than the app that was running.
  *Run 1 and run 2 reused the file names, so run 2 cannot tell these apart from a real first grant.*
- **What it does not yet show:** that a **new** app, never seen by the system, is asked once and then
  works, and for which entity (the app, the helper, or the server) the dialog is. That is what the
  next run must show, and it needs names and ids that have not been used before.
- The terminal cases pass, as expected. The report does not show which server was in the first tab
  for `t1` and `t2`; the bundle path of the helper that drove them is the same.

## Third run, 2026-10-10 (fresh names and ids, tag `003044`)

Base id `com.denonavr.s5.20261010003044`; bundles `S5-same-003044.app`, `S5-own-003044.app`,
`S5-deep-003044.app`. Several starts were stopped after two attempts (the owner trying things); the
complete runs are:

| Case | Server's identifier | Complete runs | `state` | `discovery` |
| --- | --- | --- | --- | --- |
| `own` | `<id>.server` (differs from the bundle's) | 00:33, 00:35 | **PASS, attempt 1, both times** | PASS |
| `same` | `<id>` (equals the bundle's) | 00:33, 00:38 | **Blocked all 18 attempts, both times** | **FAIL**, `SSDP … No route to host (os error 65)` |
| `deep` | `<id>` (equals the bundle's) | 00:36 | **Blocked all 18 attempts** | **FAIL**, the same error |

Every report's "started by" chain is `s5-host` → `s5-app` → launchd (pid 1), for passing and failing
cases alike, so the launch method does not differ.

What the three runs together show:

| Case | Run 1 (plain main program) | Run 2 (AppKit, reused names) | Run 3 (AppKit, fresh names) |
| --- | --- | --- | --- |
| `own` | blocked (stopped at 15 s) | pass | **pass, twice** |
| `same` | blocked (stopped at 15 s) | pass | **blocked, twice** |
| `deep` | blocked (stopped at 25 s) | blocked | **blocked** |

- **The one signing difference between `own` and `same` is the server's identifier** (`<id>.server`
  against `<id>`); the helper and the app are signed identically. And `own` is the only case that
  passes on a brand-new name and id. So a server that shares the bundle's identifier is blocked, and
  one with its own identifier is not, in this run.
- **`same` passed in run 2 only where the names had been seen before** (rows `S5-same` and `S5-own`
  existed). That is the earlier name-keyed observation again: an identifier-equal server needs the
  app's row, and had it by name.
- A **dialog and an Allow did not unblock** an identifier-equal server in run 2 (`deep`). What the
  dialogs in run 3 said, and which rows exist now, is **not yet recorded**.
- **Consequence for step 10, if it holds:** `tools/package-macos.sh` signs everything with one
  identifier (`--deep --identifier com.denonavr.remote`), which is the shape of `deep` and `same`,
  the blocked ones. Outcome B (the server signed as `com.denonavr.remote.server`, the bundle as
  `com.denonavr.remote`) is the shape of `own`. The 3.0.0 app is unaffected: it has no nested server.
- **What is still missing:** whether `own` needed a dialog at all (it passed at attempt 1 on its first
  run); whether the real identifier `com.denonavr.remote` behaves like the fresh ones (Part 4); and
  whether the pass survives a rebuild (Part 3).

## Fourth run, 2026-10-10 evening (a fresh id, then a second fresh id by mistake)

Read from the reports in `/tmp/s5` (macOS 26.7.1, receiver at `<receiver address>`). The bundles carry the
per-build run-path on the nested programs, as built after the patch described above. The
launch chain in every report is `s5-host` -> `s5-app` -> launchd, as in run 3.

| Run | Id and file names | Case | Ended | `state` | `discovery` |
| --- | --- | --- | --- | --- | --- |
| 4a | `…20261010165709`, `S5-<case>-165709` | `same` | 16:59 | Blocked, 18 attempts (`os error 65`) | FAIL, `os error 65` |
| 4a | | `own` | 17:00 | **Blocked, 18 attempts** | FAIL |
| 4a | | `deep` | 17:05 | Blocked | FAIL |
| "4b" | `…202610101708`, `S5-own.app`, `S5-same.app`, `S5-deep.app` | `own` | 17:08 | **PASS, attempt 3 (10 s in)** | PASS |
| "4b" | | `same` | 17:10 | Blocked | FAIL |
| "4b" | | `deep` | 17:12 | Blocked | FAIL |

What can be said:

- **The order was not the one the runbook gave.** 4a ran `same`, `own`, `deep` (the build printed
  "suggested order: same, own, deep"; the runbook said `own` first and skip `deep`). "4b" ran `own`,
  `same`, `deep`.
- **The stop rule applied and was not followed, which did no harm.** 4a did not repeat run 3
  (`own` blocked), so 4b and 4c should not have run. No production bundle was built, so the installed
  app's grant was not touched.
- **"4b" was not a rebuild.** Its id has 12 digits (`202610101708`), not the 14 of 4a's, so it was a
  new fresh id, and without a tag the bundles got the names `S5-own.app`, `S5-same.app` and
  `S5-deep.app` that runs 1 and 2 had used. Whether a grant survives a rebuild is still untested.
  The build script now refuses such an id.
- **Dialogs, as the owner told it:** one opened when `same` ran in 4a, and one when `own` ran in
  "4b". Their names, the clicks and the timing were not recorded. In "4b" `own` was refused twice and
  connected about 10 s in (`state` attempt 3), which fits an answered dialog; in 4a `own` got no
  dialog that is known of and stayed blocked for the whole run.
- **The run-path is not the cause of 4a's block**: "4b" `own` had the same shape and passed.
- **What differs between 4a's `own` (blocked) and "4b"'s `own` (passed after a dialog):** 4a's ran
  8 s after a `same` run that had opened a dialog, with names the system had never seen; "4b"'s ran
  with names that had Settings rows from run 2. The data cannot separate these yet.

Across runs 2 to 4: `same` and `deep` have never passed on a fresh id, even after a dialog was
answered (run 2 `deep`, run 3, 4a, "4b"). `own` passed in run 2, run 3 (twice) and "4b", and was
blocked in 4a. So the case for B is not closed: if `own` can be blocked silently for some users,
B reproduces the 3.0.0 failure for them. (Screenshots taken later correct one point above: the dialog
in 4a was for a bundle that was not running; see the fifth run.)

## Fifth run, 2026-10-10 evening (two fresh ids, and the screenshots)

Read from the reports in `/tmp/s5`, the notes files the window wrote, and the screenshots
(deleted afterwards). Tags `174424` and `174720` (ids `…20261010174424` and `…20261010174720`). Every
program in these bundles shares its Mach-O UUID with the same program in the other bundles (checked
on all four), as in runs 1 to 4; the signatures differ by identifier.

| Id | Run (in order run) | `state` | `discovery` | Dialog |
| --- | --- | --- | --- | --- |
| `174424` | `own`, first | attempt 1 refused (`os error 65`), **PASS at attempt 2** (5 s in) | PASS | yes; screenshot at 17:44:38 names `S5-own-174424`; the Allow was marked late, at 17:44:46 |
| `174424` | `own` again | **PASS at attempt 1** | PASS | none |
| `174424` | `same`, third | Blocked, 18 attempts (`os error 65`) | FAIL | none marked |
| `174720` | `own`, first | attempt 1 refused, **PASS at attempt 2** | PASS | yes; marks: appeared 17:47:34, Allow 17:47:35, pass at 17:47:37 |
| `174720` | `own` again | **PASS at attempt 1** | PASS | none |
| `174720` | `same`, third | Blocked, 18 attempts | FAIL | none marked |

Not done as written: the rebuild (there is no third `own` run under `174424`), and run 4a's order
(on `174720` the build printed `same` first, and the runs were `own`, `own`, `same`).

The screenshots:

- **The dialogs** read: *Allow "<name>" to find devices on local networks? This will allow the app to
  discover, connect to, and collect data from devices on your networks.* [Don't Allow] [Allow]. The name
  is the **bundle's file name** (`S5-own-174424`), never the display name and never the nested server.
- **Run 4a's dialog was for a different bundle.** At 16:57:48, 12 s into `S5-same-165709`, the dialog
  is titled `S5-own-165700`: the first of two builds made 9 s apart, with no report and no data
  directory. The dialogs at 17:08:48 (`S5-own`) and 17:44:38 name the bundle that was running.
- **Settings, before run 4 (16:40):** **Denon AVR Remote** is listed and **on**. No S5 rows, though
  `S5-own-003044` (run 3) was on disk and had passed twice.
- **Settings, after run 5 (17:56):** rows `S5-own-174424` and `S5-own-174720`, both on. **No rows for
  `S5-same-174424` or `S5-same-174720`**, whose bundles were on disk. No rows for the apps allowed in
  run 4 (their bundles had been deleted), as run 2's rows were gone by 16:40: a row disappears with its
  bundle (inferred from these two cases).

What can be said:

- **Outcome B's behaviour reproduced cleanly, twice**: the first connection is refused while the
  dialog is up, Allow, the next retry passes, the next launch passes with no dialog, and a same-shape
  program beside it stays blocked. Nobody answered a dialog for `same`.
- **One dialog and one row, named for the app.** The earlier expectation that B makes the user allow
  the server separately ("two entries") is wrong, and is corrected in the reading table and in the
  architecture's step 10.
- **`same` and `deep` never ask and never get a row**, so there is nothing for a user to switch on:
  the 3.0.0 failure, and what `tools/package-macos.sh` produces today.
- **Run 4a's blocked `own` is explained only in part.** Its one dialog named a bundle that was not
  running, so `S5-own-165709` never had a dialog of its own answered. Why the system named
  `S5-own-165700` is not known. The two bundles had different ids and shared their programs' UUIDs.
  With a UUID of its own in every bundle that shape no longer arises in S5, and run 6 does not test it.
- **Run 3's missing row** (`own-003044` passed twice and had no row at 16:40): either it passed
  without being asked, or a restart between run 3 and 16:40 cost it. Not relied on.
- **A consequence for the product, recorded and not designed here:** the first connection fails with
  `os error 65` while the dialog is up, and the retry works from about 5 s after Allow in both runs.
  The launcher and the server status must show "waiting for permission" and retry, not report a hard
  failure.

## Sixth run, 2026-10-10 evening (the update, the install flow, the real id)

Read from the reports in `/tmp/s5`, the notes files and the screenshots (deleted afterwards). Every
bundle in this run has a Mach-O UUID of its own for each program; the fresh `own` of Part 1 was the
control for that change and went as in run 5.

| Step | Bundle | `state` | `discovery` | Dialog |
| --- | --- | --- | --- | --- |
| 0.1 control | `own-174720` (run 5's, shared UUIDs) | PASS at attempt 1 | PASS | none; its pre-flight named a leftover `S5-same-174720` window from run 5 (pid 29337), gone by 18:19 |
| 1.1 fresh `own` | `181926` | attempt 1 refused, PASS at attempt 2 | PASS | `S5-own-181926` (screenshot 18:19:36), Allow 18:19:40 |
| 1.2 rebuilt `own` | `181926`, new hashes and UUIDs, same id, name and path | attempt 1 refused, **PASS at attempt 2**, 5 s later | PASS | **none marked, none photographed** |
| 2.1 the copy | `182040` in `target/s5-copy` | attempt 1 refused, PASS at attempt 2 | PASS | yes, Allow 18:20:56 (no screenshot) |
| 2.2 the original, copy present | `182040` | PASS at attempt 1 | PASS | none |
| 2.4 the original, copy deleted | `182040` | PASS at attempt 1 | PASS | none |
| extra: original first | `182157` | attempt 1 refused, PASS at attempt 2 | PASS | `S5-own-182157` (screenshot 18:22:21), Allow 18:22:23 |
| 3.3 production `same` | `S5-same-ps`, id `com.denonavr.remote`, server id equal | attempt 1 refused, PASS at attempt 2 | PASS | **"Denon AVR Remote"** (screenshot 18:25:20), Allow 18:25:23 |
| 3.2 production `own` | `S5-own-po`, server `com.denonavr.remote.server` | attempt 1 refused, PASS at attempt 2 | PASS | yes, Allow 18:29:52 (no screenshot; the name is unknown) |

The owner ran `same-ps` before `own-po`; the runbook said `own` first. Part 2 was run as written, and
then once more with the original first (`182157`). The installed 3.0.0 app, opened afterwards, **reached
the receiver** (the owner's word; whether before or after the test bundles were deleted was not said).
The extra build `182341` and the copy of `182157` have no reports.

Settings (screenshots 18:22:59, 18:28:44 and 18:32:02): **Denon AVR Remote** on; rows `S5-own-174720`,
`-181926`, `-182040` and `-182157`, all on, **one row per identity** (the copy and the original of
`182040` share `S5-own-182040`); no row for a bundle that was built and never run (`181912`), or for
one that was deleted (`174424`). After the production runs there are **three rows named "Denon AVR
Remote"**, all on, all with the real app's icon: the original, and one appended at the bottom for each of
`S5-same-ps` and `S5-own-po`. There is no row under the names `S5-same-ps` or `S5-own-po`, and the one
for `same-ps` stayed after that bundle was deleted. So under the real id each bundle that differs from
the earlier ones (other programs, another path and name) is recorded as a new record named for the
app: a reinstall at another path can leave extra rows of the same name, harmless and all on.
The 18:32 screenshot was taken with `S5-own-po.app` still on disk.

What can be said:

- **The UUID change did not alter the behaviour.** A fresh `own` still goes dialog, Allow, pass.
- **An update keeps the grant, but the first connection after it is refused once.** The rebuilt
  `own` made no dialog (none was marked or photographed, which the owner has not confirmed) and no
  second row, and passed at the retry 5 s later. In the control and in Part 2 the pass came at attempt 1.
  So after an install or an update the first connect can fail and the next one works.
- **An install from a copy is safe.** Two byte-identical copies at two paths share one grant and one
  row, the original started when the copy was there and when it was gone, and the reports show the right
  bundle path each time. The feared wrong-window case did not occur. A real `hdiutil attach` is untested.
- **Under the real id both shapes ask once and pass.** The 3.0.0 grant covered neither `same-ps` nor
  `own-po`, which have the real id but other programs at another path and name. `same` asks where it
  never asked on fresh ids, and its dialog and row carry the real app's name and icon, not the test
  bundle's. The installed app still works.
- **So the shapes differ only for a user with no record of the id.** On fresh ids (the nearest
  thing to a new user that this Mac can show) `own` asks and passes and `same` never asks and is
  blocked, about six times. A new user of the real id cannot be tested here, because this Mac has the
  record. For an existing user the two shapes look alike.
- **Run 4a's block did not recur.** `own-po` passed right after `same-ps`. Why 4a's `own` was blocked
  and its dialog named a bundle that was not running is still unexplained. The owner does not remember
  whether `S5-own-165700` was opened, whether `same` asked in run 5, or whether the Mac was restarted
  between run 3 and 16:40, so those stay unknown.
- **Not tested:** an in-place update of the installed `/Applications/Denon AVR Remote.app` (replacing it,
  with a backup, is the owner's choice); a real `hdiutil attach`; a restart (the kept `S5-own-<T>.app`
  can be run after the next one). The `own-po` dialog was not photographed, but its Settings row is named
  "Denon AVR Remote", so the dialog most likely was too.

**What S5 supports (the owner confirmed it on 2026-10-10).** Outcome **B**: the packaging signs the nested server
inside out with its own identifier, `com.denonavr.remote.server`, and the bundle with
`com.denonavr.remote`.

- It is the only shape that worked on every fresh id, and the current shape (`same`, which
  `tools/package-macos.sh` produces) did not on any.
- The update and the install from a copy keep the grant.
- Under the real id it costs an existing user nothing over the current shape: both ask once.

What step 10 and the guides then say: the user sees **one dialog and one Settings row, both named for the
app** (never the server). The launcher and the server status must treat the first connection after an
install or an update as "waiting for permission" and **retry**: it fails with `os error 65` once, and
works from the next attempt about 5 s later, with no action by the user after an update.

## What you need

- The Mac that ran 3.0.0 (it has the real `com.denonavr.remote` grant), logged in at the
  keyboard, with the screen unlocked. Dialogs appear only in a logged-in session.
- The receiver on, **Network Control enabled** in its setup menu, and **no other client holding
  its control connection**: quit the Denon AVR Remote app and the CLI, stop any
  `denon-avr-api-server`, and close any Telnet session. The receiver accepts one at a time.
  Check with `pgrep -l 'denon-avr|s5-'` (should print nothing; it matches program names, so the `open -W` waiting in your terminal does not show).
- The receiver's IP address. The development Mac's scan saw one AVC-X3800H;
  use yours, and confirm it with `make run ARGS="get receivers"` if you are not sure.
- Xcode or its command line tools (`codesign`, and `xcrun swiftc` for the AppKit stand-in; this Mac
  has Xcode 26.1.1), Rust with the `aarch64-apple-darwin` target, and the crates the server needs
  (a build with network access the first time, as in phase 4).
- About an hour. Each run takes up to two minutes if it is blocked, much less if it works.

## Part 0 — Look before you touch anything

1. Open **System Settings → Privacy & Security → Local Network**. Write down every row, its
   name, and its toggle (on or off), in the **Before** column of the Results. In particular:
   is there a row for **Denon AVR Remote** (the 3.0.0 app, bundle id `com.denonavr.remote`)? Is
   there a row for your terminal application (Terminal, iTerm, …)?
2. Open a terminal and go to the repository:

   ```sh
   cd /Volumes/Work_Volume/denon-avr-remote-multi-platform
   pgrep -l 'denon-avr|s5-'      # nothing
   rm -rf /tmp/s5               # start clean
   ```

## Part 1 — Build

```sh
export S5_RECEIVER_HOST=<receiver address>      # your receiver
bash tools/s5/build-bundles.sh
```

**The bundles' names include a tag** (`S5-same-002739.app`, report `result-same-002739.txt`). Wherever
this runbook says `S5-V.app` or `result-V.txt`, use the paths and the order the script prints at the
end. Settings lists apps by file name, so the tag also keeps this run's rows apart from earlier ones.

Delete the old bundles first (`rm -rf target/s5/S5-*.app`) so none can be opened by accident, and
`rm -rf /tmp/s5` if you want the result files fresh (an earlier report is kept either way, under
its own time). The build compiles the server, the host, and the Swift app, and takes a few
minutes the first time. It ends by printing the base id (`com.denonavr.s5.<14 digits>`), the tag,
and the `open` lines, in the order to run them. **Write down the base id and the tag; Part 3 no
longer needs the id typed, because `--rebuild` reads it from `target/s5/last-build.txt`.** The
bundles are in `target/s5/`, named `S5-own-<tag>.app` and `S5-same-<tag>.app` (and `S5-deep-<tag>.app`
if `--cases` names it). Each case has its own bundle id, so a grant given to one does not cover
another.

If the script stops, read its last line: a missing Rust target (`rustup target add
aarch64-apple-darwin`) or tool is named.

## Part 2 — A bundle launched like a user launches it (cases `deep`, `same`, `own`)

`open` starts a bundle through Launch Services, as a double-click in Finder does, so the app
and not your terminal is what macOS holds responsible for what the server does. Do **not**
start `s5-host` or `s5-app` by typing its path for this part: then your terminal is responsible
and the result says nothing about the app.

**Run one case at a time and let it finish.** A window titled "S5 V" opens and prints the run
as it goes. It ends with `SUMMARY V: …` and then `FINISHED`. If you stop a run early (the first
run's mistake), it proves nothing about that case; start it again. Between cases, quit the
previous window.

Do the cases in this order. For each case `V` (`deep`, then `same`, then `own`):

1. Make sure nothing else holds the receiver (`pgrep -l 'denon-avr|s5-'` prints nothing).
2. Run:

   ```sh
   open -W target/s5/S5-V.app
   ```

   It waits until the app exits, which is when you close the window. (Double-clicking the bundle
   in Finder is the same.) The window shows the run live.
3. **Watch the screen.** Within the first seconds a system dialog may appear that says
   something like *“S5 V” would like to find and connect to devices on your local network*
   (the wording varies with the macOS version). If it appears, write down the **exact name**
   it shows (it should be the app, `S5 V`; anything else is a finding), and click **Allow**.
   You have about 90 seconds. Do not click *Don't Allow*; if you did, note it, turn the app's
   switch on in Settings, and say so in the Results.
4. When the window says FINISHED, close it and read the result:

   ```sh
   cat /tmp/s5/result-V.txt
   ```

   The lines you care about start with `PASS`, `FAIL`, or `WARN`: `start`, `health`, `ad-hoc`,
   `state` (the receiver connection), and `discovery` (the scan). Note the attempt number at
   which `state` passed, and whether any attempt says *looks like the OS refused it*.
5. Open **Local Network** in System Settings again. Write down which rows appeared, their exact
   names, and their toggles.
6. **Run the same bundle a second time** (`open -W target/s5/S5-V.app`). Expected: no dialog and
   `PASS state` on attempt 1. Record whether that happened.

How to read what you see:

| What happened | What it means |
| --- | --- |
| A dialog named the app, you allowed it, `state` and `discovery` then pass | The grant for the app covers the nested server for this case |
| `state` fails every attempt with `os error 65` and **no dialog** appeared | Blocked silently: the 3.0.0 failure mode |
| ...and a row for the app now exists with its switch **off** | The system registered the attempt but did not ask. Turn it on, run the case again, and record "needed a manual switch" |
| The dialog names something other than the app (the server's executable name, say) | The server is being treated as its own app: record the name |
| `state` passes but `discovery` fails or is empty | TCP is allowed and multicast is not: record it, it matters |
| `state` fails with `os error 61` or `Connection refused` | Not the permission. Network Control is off, or another client holds the receiver |

## Part 3 — Does a grant survive the next build?

Rebuild **under the same ids**. The builder keeps each program's identity and gives every
program in the bundle, the nested ones too, a new code hash: it adds a run-path load command
that holds a version string, then signs again. (Until 2026-10-10 only the main executable and
the bundle changed; the nested server and helper kept their hash, so a pass here said nothing
about the server's grant. Since run 6's tooling every program in the bundle also gets a **new
Mach-O UUID**, patched into the copy before it is signed, so a rebuild differs as a release does and
no two bundles share a program's UUID. A real release changes the UUID because a link does.)

```sh
bash tools/s5/build-bundles.sh --rebuild      # the last fresh build's id, tag and cases; no id to type
```

(Typing the id was run 4's mistake: a 12-digit id made a new one. The script now refuses an id it did
not print.) Then for each case that passed in Part 2, run the `open` line it prints and read
`/tmp/s5/result-V-<tag>.txt`. Record whether it passed on attempt 1 with no dialog (the grant survived),
or whether a dialog came again (it did not). This is what a user sees when the app is updated.

## Part 4 — The real identifier, `com.denonavr.remote`

The id the installed 3.0.0 app has. **Denon AVR Remote** is listed in Settings and on (the screenshots
of 2026-10-10). Run this only after run 6's Part 1 (a rebuilt `own`) and Part 2 (a copy beside the
original) have passed, because these bundles share their id with the installed app. Run `own` first,
then `same`, with tags that no earlier run used:

```sh
bash tools/s5/build-bundles.sh --production own --tag po
bash tools/s5/build-bundles.sh --production same --tag ps
```

Each makes one bundle (`S5-own-po.app`, `S5-same-ps.app`) and prints its one `open` line. Before
each run, quit the real app and check that `pgrep -l 'denon-avr|s5-'` prints nothing. **Every
production bundle has the bundle id `com.denonavr.remote`, like the installed app.** If one of them,
or the installed app, is still running, `open` on another only brings the running one forward and no
new run happens. Look at the window's title (`S5 own-po`, `S5 same-ps`) each time, and quit a bundle
before starting the next.

`own` goes first because it is the case the product needs, and run 4a showed that starting with a
`same` run can precede an `own` block. If `own-po` passes with no dialog, a 3.0.0 user's grant covers
an `own` server under that id; if a dialog appears, record what it names; if `same-ps` passes where it
was blocked on a fresh id, the current signing works for upgraders only.

**What a pass means:** the bundle has the real id but a new file name, a new path, and new programs,
so the result is "an app with that id and a new name is covered", not "an in-place update keeps the
grant". The in-place check (replacing the installed app, with a backup) is stronger; it is the
owner's choice and is not part of the run.

**The installed app's grant is at risk.** Never click Don't Allow here, and touch no row. Write down
the installed app's path first (`mdfind "kMDItemCFBundleIdentifier == 'com.denonavr.remote'"` lists
every copy, including any `target/release/macos/Denon AVR Remote.app`). Open it **by that path** with
the test bundles still on disk, and check it reaches the receiver; then delete the test bundles, look
at Settings, and check it again. That separates a cause that is the test bundles being present from
one that is their deletion removing a row that shares the id. If it asks again, Allow. If it is
silently blocked, or its row is gone or off, stop; do not try fixes.

## Part 5 — A server started from a terminal

A standalone server is started by hand, so the application macOS holds responsible is your
terminal application. These cases use two terminal tabs.

Use the labels exactly as written, `t1` with `d-t1` and `t2` with `d-t2`: the first run's
terminal report was overwritten because both cases used one label. The host now notes a mismatch
and keeps an earlier report, but the right label is still yours to type.

**T1, the server inside a bundle** (use any bundle; note which):

```sh
# tab A
target/s5/S5-own-<tag>.app/Contents/MacOS/denon-avr-api-server --data-dir /tmp/s5/d-t1
# tab B
target/s5/S5-own-<tag>.app/Contents/MacOS/s5-host --drive --label t1 --host $S5_RECEIVER_HOST --data-dir /tmp/s5/d-t1
cat /tmp/s5/result-t1.txt
```

**T2, the server straight from the build** (the way `make run-gui` and a developer start it):

```sh
# tab A
target/aarch64-apple-darwin/release/denon-avr-api-server --data-dir /tmp/s5/d-t2
# tab B
target/s5/S5-own-<tag>.app/Contents/MacOS/s5-host --drive --label t2 --host $S5_RECEIVER_HOST --data-dir /tmp/s5/d-t2
cat /tmp/s5/result-t2.txt
```

Stop the server in tab A with Ctrl-C after each. Record: did a dialog appear, and what
application did it name? If your terminal already had Local Network permission in Part 0, there will
be no dialog and `state` will pass on attempt 1. That is a result too: it means a standalone server
needs the **terminal's** grant and not the server's.

## Part 6 — Only if Part 2 showed a problem with the dialog

Build once with the usage text in the bundle's `Info.plist`, which the production bundle does not
have today:

```sh
S5_USAGE_TEXT=1 bash tools/s5/build-bundles.sh
```

Repeat Part 2 for `same`. Record whether the dialog now carries the text, and whether a case that
failed silently without it now asks. If every case in Part 2 worked, skip this part.

## Reading the results into an outcome

Choose the first line that fits. These are the outcomes the phase 5 architecture names.

| Result | Outcome | What step 10 does |
| --- | --- | --- |
| `same` passes in Part 2 and survives Part 3 (and Part 4 passes or is a clean second grant) | **A**: one identity, one grant | Sign the server inside out with the bundle's identifier. `deep` passing too is fine and changes nothing, since the packaging script signs inside out either way |
| `same` fails or never asks, but `own` passes in Part 2 | **B**: the server has its own identity | Sign the server with `<id>.server`; the user guide names the one dialog and the one row, both named for the app (run 5's screenshots; no second entry for the server) |
| Parts 2 and 4 pass only after a **manual switch** in Settings, with no dialog | **A or B plus a guide step** | As A or B, and the desktop guide tells the user to switch the entry on |
| No case passes in Part 2, but Part 5 passes | **C**: launching from the bundle is the problem | The plan stops and returns to the owner, who chooses between a login-item server and a server hosted in the GUI (the owner decided on 2026-10-09 to choose then) |
| A grant does not survive Part 3 | **A or B, with a cost** | Ad-hoc signing changes the identity's requirement on every build; say so in the release notes, and consider whether the project can sign with a stable certificate |
| `discovery` fails where `state` passes | **Open**: multicast needs something TCP does not | Report it before anything else; the SSDP scan might need an entitlement |

## Cleaning up

```sh
pgrep -l denon-avr-api-server        # stop any left over with Ctrl-C or kill <pid>
rm -rf /tmp/s5 target/s5
```

The test rows in **Local Network** stay in Settings after the bundles are deleted; switch them off or
leave them. They are harmless. Do not delete or edit the **Denon AVR Remote** row.

## What to send back

1. The result files: `cat /tmp/s5/result-*.txt` before cleaning up, pasted whole.
2. The **Results** tables below, filled in.
3. Anything surprising: a dialog with an unexpected name, a Settings row that appeared or did not,
   a case that behaved differently the second time.

## If something goes wrong

| What you see | What it means |
| --- | --- |
| `s5-host: no --host and no /tmp/s5/config.txt` | Run the builder with `S5_RECEIVER_HOST=<address>` set, or write `host=<address>` into `/tmp/s5/config.txt` |
| `FAIL start: the server exited early` and a log line | The log tail is printed under it. A path longer than the socket limit, or a directory it cannot make, are the usual causes |
| macOS says the app "can't be opened" or "is damaged" | Run `xattr -cr target/s5`; a locally built bundle should not need it |
| `open` returns at once and there is no result file | Run the bundle's `s5-host` by path **once** to see the error, then go back to `open` |
| The run is stuck at `attempt 1` and a dialog is on a different screen or space | The dialog is waiting for you; the run keeps trying for about 90 seconds |
| `Connection refused (os error 61)` on every attempt | Not the permission: see the table in Part 2 |
| `s5-host --drive` says nothing accepted on the socket | The server in tab A is not running, or its `--data-dir` differs from the host's |

## Results (to fill)

### Before (Part 0)

| Row in Local Network | Switch |
| --- | --- |
| Denon AVR Remote (`com.denonavr.remote`) | To fill |
| Terminal application | To fill |
| Others | To fill |

### Part 2: a bundle launched with `open`

| Case | Dialog? | Name it showed | `state` passed at attempt | `discovery` | Second launch passed with no dialog? | Rows in Settings after |
| --- | --- | --- | --- | --- | --- | --- |
| `deep` | To fill | To fill | To fill | To fill | To fill | To fill |
| `same` | To fill | To fill | To fill | To fill | To fill | To fill |
| `own` | To fill | To fill | To fill | To fill | To fill | To fill |

### Part 3: after a rebuild under the same ids

| Case | Passed on attempt 1 with no dialog? | Dialog came again? |
| --- | --- | --- |
| `deep` | To fill | To fill |
| `same` | To fill | To fill |
| `own` | To fill | To fill |

### Part 4: the real identifier

| Was Denon AVR Remote listed and on in Part 0? | Passed on attempt 1 with no dialog? | If a dialog: the name |
| --- | --- | --- |
| To fill | To fill | To fill |

### Part 5: terminal

| Case | Dialog? | Application it named | `state` | `discovery` |
| --- | --- | --- | --- | --- |
| T1, server in a bundle | To fill | To fill | To fill | To fill |
| T2, server from the build | To fill | To fill | To fill | To fill |

### Part 6 (only if run)

To fill.

### Outcome

**B** (macOS 26.7.1, arm64, 2026-10-10), **confirmed by the owner on 2026-10-10**: sign the nested server
with `<bundle id>.server` (`com.denonavr.remote.server`), the bundle with `com.denonavr.remote`, inside out. The evidence is in the fifth and
sixth runs above. What is not yet shown: an in-place update of the installed app, a real disk image, and a
restart; the three runbook questions about runs 4 and 5 are unanswerable. **Outcome C is not needed.**
