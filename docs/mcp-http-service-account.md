# MCP HTTP service account (macOS)

Written 2026-10-07 for a component that does not exist yet. The `mcp-http` build
is described in [Planned architecture](planned-architecture.md); nothing in this
document is part of the implemented system. Only read-only checks were run on a
Mac while writing it, and none of the creation commands were executed. See
[Evidence status](#evidence-status).

## Purpose

`mcp-http` listens on the network and parses requests from agent hosts. If a
defect in it let an attacker run code, the attacker would hold whatever account
the process runs as. Under the owner's login that includes the Operator token,
which would give Operator authority and defeat the policy gate. A separate
account without access to the owner's files keeps a compromised `mcp-http`
within Agent authority. The reasoning is in
[Choosing a build](planned-architecture.md#choosing-a-build).

This document covers macOS only, which is the reference deployment. Linux and
Windows are not covered.

## The account

| Property | Value | Reason |
| --- | --- | --- |
| Short name | `_denonmcp` | The underscore prefix is the macOS convention for service accounts and keeps the account out of the login window. `denon` ties it to this project and `mcp` names the role |
| User and group id | 480 | Free as both a user id and a group id on the author's Mac on 2026-10-07. Ids below 500 are the range macOS uses for service accounts. Check again before use |
| Primary group | `_denonmcp`, same id | A group of its own, so no shared group grants access to anything else |
| Login shell | `/usr/bin/false` | Nobody can open a shell as this account. A program is run as it with `sudo -u` |
| Password field | `*` | No password can match, so the account cannot log in |
| Home and data directory | `/usr/local/var/denon-mcp`, mode 700 | Holds only this service's files. System Integrity Protection leaves `/usr/local` writable |

If the authorization server is enabled it gets its own account, for example
`_denonauth`, created the same way with a different id. The two services must not
share an account, because they hold different keys.

## What the account may and may not do

| May | Must not |
| --- | --- |
| Read and write its own data directory: the TLS certificate and key, its configuration, and in OAuth mode its exchange credential | Read the owner's home folder, the Operator token file, the policy, or the audit log |
| Bind the single address that its configuration names | Reach the receiver directly. The rule that enforces this has not been validated |
| Connect to the Control API server's Agent endpoint. How that access is granted is not decided; see open decision 2 in the architecture document | Connect to the Operator endpoint |

## Create it

Run this in Terminal as an admin user. It asks for the sudo password.

First confirm that the id is still free, and choose another value below 500 if it
is not. Both commands should print nothing:

```sh
dscl . -list /Users UniqueID | awk '$2 == 480'
dscl . -list /Groups PrimaryGroupID | awk '$2 == 480'
```

Then create the group, the user, and the data directory:

```sh
NAME=_denonmcp
ID=480
HOME_DIR=/usr/local/var/denon-mcp

sudo dscl . -create /Groups/$NAME
sudo dscl . -create /Groups/$NAME PrimaryGroupID $ID
sudo dscl . -create /Groups/$NAME RealName "Denon MCP HTTP"

sudo dscl . -create /Users/$NAME
sudo dscl . -create /Users/$NAME UniqueID $ID
sudo dscl . -create /Users/$NAME PrimaryGroupID $ID
sudo dscl . -create /Users/$NAME RealName "Denon MCP HTTP"
sudo dscl . -create /Users/$NAME UserShell /usr/bin/false
sudo dscl . -create /Users/$NAME NFSHomeDirectory $HOME_DIR
sudo dscl . -create /Users/$NAME Password '*'
sudo dscl . -create /Users/$NAME IsHidden 1

sudo mkdir -p $HOME_DIR
sudo chown $NAME:$NAME $HOME_DIR
sudo chmod 700 $HOME_DIR
```

Place the TLS certificate and key, which the user provides, in the data
directory with owner-only access, for example:

```sh
sudo install -o _denonmcp -g _denonmcp -m 600 tls.key /usr/local/var/denon-mcp/tls.key
sudo install -o _denonmcp -g _denonmcp -m 644 tls.crt /usr/local/var/denon-mcp/tls.crt
```

## Check it

Record the macOS version and the date with the results, and mark each check
`pass` or `fail`.

| Command | Expected |
| --- | --- |
| `id _denonmcp` | `uid=480(_denonmcp) gid=480(_denonmcp)`, and the group list contains neither `admin` nor `staff` |
| `dscl . -read /Users/_denonmcp UserShell NFSHomeDirectory` | `/usr/bin/false` and `/usr/local/var/denon-mcp` |
| `sudo -u _denonmcp whoami` | `_denonmcp` |
| `sudo -u _denonmcp ls "/Users/$USER"` | Permission denied |
| `sudo -u _denonmcp ls /usr/local/var/denon-mcp` | Lists the service's own files |
| `su _denonmcp` | Cannot log in |

The fourth check is the one that matters: the account must not see the owner's
home folder. Repeat it against the real location of the Operator token file once
that location exists. Do not claim isolation unless every check passed.

## Run the service as the account

The command below is illustrative. The program and its options do not exist yet:

```sh
sudo -u _denonmcp -H /path/to/mcp-http --config /usr/local/var/denon-mcp/config.yaml
```

The design uses no launchd or other operating-system service, so the user starts
the process from a terminal. `sudo -u` works only for an admin user.

## Remove it

```sh
sudo dscl . -delete /Users/_denonmcp
sudo dscl . -delete /Groups/_denonmcp
sudo rm -rf /usr/local/var/denon-mcp
```

The data directory holds the TLS key, so remove it deliberately. Revoke the
agent tokens first if the account is being retired because of a compromise.

## Alternatives

| Alternative | Why it is weaker |
| --- | --- |
| System Settings, Users & Groups, Add Account (Standard) | Creates an account with a password and a home folder under `/Users` that appears in the login window. It works, but it is a login-capable account and not a service account |
| `sysadminctl -addUser` | Same result as the settings pane for this purpose |
| Run `mcp-http` as the owner | No setup, but a remote-code-execution defect in it yields Operator authority. The architecture document lists this as accepted when the account is not created |

## Open issues

- **Agent endpoint access.** The mechanism that lets this account reach the
  Agent endpoint, and only that endpoint, is open decision 2 in the
  architecture document. Creating the account does not depend on it.
- **Firewall.** If the macOS application firewall is on, it may block or prompt
  for an unsigned command-line program that accepts connections. This was not
  tested.
- **Receiver reachability.** Keeping this account from reaching the receiver
  needs an operating-system rule that matches the account, which is
  unvalidated. The network rules in
  [UTM guest LAN isolation](research/utm-guest-lan-isolation.md) cover the agent
  host, not this account.

## Evidence status

| Claim | Basis |
| --- | --- |
| Id 480 was free as a user and group id; Apple's accounts at the time occupied 300 to 308 and 441 | Read-only `dscl` listing on macOS 26.7.1, 2026-10-07 |
| `dscl`, `dseditgroup`, and `sysadminctl` are present | `command -v` on the same Mac |
| The creation commands produce a non-login service account | Standard `dscl` usage, not executed |
| The account cannot read the owner's home folder | Expected from default home-folder permissions, not tested |
| Ids below 500 are the service-account range | General macOS convention, not verified here. Apple adds its own accounts over time, so a collision with a future one is possible |
| Behavior of the application firewall for this service | Untested |
| A rule that keeps the account from the receiver | Unvalidated |
