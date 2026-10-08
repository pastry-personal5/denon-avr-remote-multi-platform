# Agent endpoint access on macOS (S2)

Run by the owner by hand on 2026-10-09, macOS 26.7.1 (25G241), on the machine the
project is developed on. This is the output of step 0b of
[phase 4](../v4/phase-4-control-api-server-overview.md#step-0b--s2-the-agent-endpoints-directory-and-admission-done):
it fixes the keys of `agent_endpoint` and the directory's default, and says how the
dedicated account is admitted. The code of step 9 reads it.

## What was run

| Account | uid | Primary group | Notes |
| --- | --- | --- | --- |
| `user1`, the owner | 501 | `staff` | Administrator; runs the server |
| `agent0000`, the dedicated agent account | 503 | `staff` | Standard account; in neither `admin`, Remote Login, nor Screen Sharing; home `0700` |
| `_denonmcp` | 480 | `_denonmcp` | The MCP service account from [mcp-http-service-account.md](../mcp-http-service-account.md); used only as an account that is not admitted |

The directory was `/Users/Shared/Denon AVR Remote`, created by the owner with
`mkdir` and `chmod 700`. A throwaway Python script (about a hundred lines, not kept)
bound `s2.sock` in it as the owner, read each connection's peer with
`getsockopt(SOL_LOCAL, LOCAL_PEERCRED)` and `LOCAL_PEERPID`, and answered with the uid it
saw. A second terminal ran `sudo -u agent0000 -H -i` and tried to connect. The checks
need nothing from the project's code: the operating system decides every result below.

## Results

| # | Check | Result |
| --- | --- | --- |
| 1 | Directory ACL `agent0000 allow search`, socket mode `600` | `agent0000` is refused (`EACCES`): a connect needs write permission on the socket file |
| 1 | The same ACL, socket mode `666` | `agent0000` connects; the server reads **uid 503** |
| 1 | ACL entries `agent0000 allow search` and `agent0000 allow write,file_inherit`, socket mode `600` | `agent0000` connects. The new socket carries `user:agent0000 inherited allow write` (`ls -le`), and stays `srw-------` owned by the owner |
| 1 | An account that is not in the ACL (`_denonmcp`), socket mode `666` | Refused (`EACCES`): the directory is `0700`, so nothing else can search it, however open the socket file is |
| 3 | Peer credentials, from `sudo -u agent0000 -H -i` | The kernel reports uid 503 |
| 4 | `agent0000` listing, reading, or connecting to the owner's home, the data directory `~/Library/Application Support/Denon AVR Remote`, and the socket directory | Every one refused (`EACCES`; the data directory is refused at `lstat`, because the home is `0700`). The ACL grants `search` only, so the agent can name the socket but not list the directory |
| 5 | The server stopped and started again with the ACL untouched | `agent0000` connects again: the recreated socket inherits the entry |

Not checked: admission by a shared group (the ACL works, and `staff` cannot be that
group, because both accounts are in it); the uid reported for a process an agent host
starts (it is the same account, and the uid is the process's, not the shell's); the
account hardening below.

## Decisions

**Admission is a directory ACL, and the owner sets it once.** The server cannot, and
does not try to, change who is admitted. With the socket directory at `0700` and these
two entries on it, the account is admitted:

```sh
D="/Users/Shared/Denon AVR Remote"
mkdir "$D" && chmod 700 "$D"
chmod +a "agent0000 allow search" "$D"
chmod +a "agent0000 allow write,file_inherit" "$D"   # macOS lists it as add_file,file_inherit
```

The first entry lets the account reach the socket's name; the second is inherited by
every socket the server creates in the directory, which is what lets it connect to a
socket that stays `0600` and owned by the owner. Set both **before** the server starts,
or restart it afterwards: a socket made earlier does not gain an entry the directory
acquired later. Replace `agent0000` with the account's short name, and `ls -led "$D"`
and `ls -le "$D/s2.sock"` show the result.

**Location: `/Users/Shared/Denon AVR Remote`.** `/Users/Shared` is sticky and writable
by everyone, so the owner can make the directory without `sudo`, and the account's
home and the owner's home are not involved. The server checks the directory itself
(it is a real directory, owned by the server's uid, and not group- or world-writable)
and creates it at `0700` when it is missing, because the sticky parent is exactly where
another user could plant a link or a directory. The new directory's group is `wheel`
(inherited from `/Users/Shared`); at `0700` that does not matter.

**Keys.** `server.yaml` in the data directory:

```yaml
agent_endpoint:
  directory: /Users/Shared/Denon AVR Remote   # optional; this is the default
  uids: [503]                                 # required: the accounts admitted
```

The socket's mode is not a key. It is `0600`, because the directory's ACL and the
server's check of the peer's uid decide who gets in, and the socket file carries the
inherited entry. A `666` socket was also admitted by the ACL and refused to everyone
else, so the mode is defence in depth and not the control; `0600` leaves the least open.

**Defence in depth.** Three things must each hold for an agent to be served: the
operating system lets the account reach the socket (the ACL), the server finds the
peer's uid in `uids` (`LOCAL_PEERCRED`, which reported 503), and the request carries a
valid Agent token.

## Recommended account hardening (not verified)

Not part of the checks above; none of it was reported as done.

- `agent0000` is in `staff`, as every account made in System Settings is. Group
  `staff` can read any file that is group-readable, and `/Users/sugar` on the
  development machine is `drwxr-x---` `staff`, so the agent account can list that home.
  A private primary group removes that: `sudo dseditgroup -o create -i 503 -r "Agent 0000"
  agent0000`, then `sudo dscl . -create /Users/agent0000 PrimaryGroupID 503`, with the
  account logged out. The check that was run does not depend on it, because the owner's
  home is `0700`.
- `sudo -l -U agent0000` should say the account may not run `sudo`.
- Keep the account out of Remote Login and Screen Sharing.
- Keeping the account from the receiver itself is a firewall question that this check
  did not touch.

## Caveats

- One macOS version on one machine. Re-run the connect from the agent account after a
  major macOS update.
- An ACL is not carried by every copy tool. Restoring the directory from a backup, or
  recreating it, needs the two `chmod +a` commands again; the server's health response
  does not show whether the ACL is there, so a missing entry looks like the agent being
  unable to connect (`EACCES`), not like a server fault.
- Removing the account's access is `chmod -a` on both entries, or deleting the
  directory.
