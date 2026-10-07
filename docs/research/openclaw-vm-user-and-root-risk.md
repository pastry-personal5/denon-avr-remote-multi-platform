# OpenClaw user and root risk in a Linux VM

Researched 2026-10-07 from OpenClaw documentation and third-party security
write-ups. Nothing here was run against a live OpenClaw install; see
[Evidence status](#evidence-status).

## Conclusion

OpenClaw has no fixed Linux user. A native install runs as the account that
installed it, and only the official Docker image fixes a user (`node`, uid
1000). [1][2][3]

No source I read says an OpenClaw vulnerability gives **Linux root**. The
Cyera advisory states that none of its four vulnerabilities does. [4] But
OpenClaw already runs shell commands as its own user by design, so the practical
question is whether that user can reach root through the VM's configuration.
Group membership such as `sudo` or `docker` is the most likely path, and it is a
setup issue, not an OpenClaw bug.

Root inside the VM does not by itself give control of the macOS host. It does
remove any firewall rule that lives inside the guest, which is why
[UTM guest LAN isolation](utm-guest-lan-isolation.md) puts the enforceable rule
on the host.

## Which user runs OpenClaw

| Install method | User | Source |
| --- | --- | --- |
| Native on Linux (`openclaw gateway install`) | The account that ran the install. The gateway is a `systemd --user` unit. The docs name no default user or uid. | [1] |
| Native, recommended setup | A dedicated non-root `openclaw` user. State is under `/home/openclaw/.openclaw/`. | [2] |
| Official Docker image | `node`, uid 1000. State is under `/home/node/.openclaw`. | [3][5] |
| Rootless Podman | The invoking non-root user (`keep-id`). | [5] |

- The Linux uid of a native user is assigned by the system. On a fresh Ubuntu
  VM the first normal user is usually 1000, and a later `openclaw` user usually
  gets the next free number. This is general knowledge, not from the docs.
- The setup wizard tries `loginctl enable-linger <user>` so the service
  survives logout. [6]
- Running as root is possible but is treated as a deliberate choice. The Docker
  troubleshooting notes describe file-ownership warnings when uid 1000 and the
  mounted state disagree, and say to prefer the default uid. [3]

"Latest" cannot be confirmed from this research. The indexed releases go up to
`v2026.4.9` plus `main`-branch docs, and the research date is 2026-10-07.

## What OpenClaw already has

- **Host shell access is by design.** Exec is host-first, and
  `agents.defaults.sandbox.mode` defaults to `off`. [7]
- **One trusted operator.** OpenClaw is "not designed as a shared multi-tenant
  boundary between adversarial users on one gateway". If users are not fully
  trusted, use one VPS or OS user per user. [7]
- **The gateway is never sandboxed.** The sandbox covers tool execution, and
  `tools.elevated` lets an agent deliberately run outside it. [8][9]
- **Approvals can be turned off.** The troubleshooting page shows
  `tools.exec.ask off` with `tools.exec.security full`. [10]
- **Prompt injection is the realistic trigger.** Content from chat channels or
  web pages can push the agent into running commands. This is an assessment, not
  a documented claim.

An attacker who takes over the agent therefore gets that user's files, network
access, and credentials. That is already serious without root.

## Paths from the gateway user to root

These depend on how the VM is set up, not on an OpenClaw bug:

| Path | Why it matters | Check |
| --- | --- | --- |
| `sudo` group or sudoers rule | Root at once. On Ubuntu the first user is usually in `sudo`. | `groups <user>`; `sudo -l -U <user>` |
| `docker` group (also `lxd`, `libvirt`) | Root-equivalent. Docker is the default sandbox backend, so enabling the sandbox can create this path. [11] | `groups <user>` |
| Shared interactive account | If a person also logs in and uses `sudo`, the agent could capture the password with a PATH or alias shim. | Use an account nobody logs into. |
| Kernel or setuid local privilege escalation | Ordinary Linux risk. | Keep the VM patched. |
| Running as root on purpose | Some Docker setups do this. [3] | `ps -eo user,uid,args \| grep -i [o]penclaw` |
| Docker socket or home bind-mounted into a sandbox | The exposure runbook says to avoid credential, home, Docker socket, and system paths in bind mounts. [9] | Review the sandbox mounts. |

The Ansible playbook documents `NoNewPrivileges`, `PrivateTmp`, and an
unprivileged user. It does not say whether that user joins the `docker` group.
[11]

## Published vulnerabilities

Most advisories say "privilege escalation", but that usually means escalation
inside OpenClaw (operator scopes, owner role, leaving the sandbox), not Linux
root. Only the Cyera page and the project's own documentation were read
directly. The other rows come from search-result summaries of third-party
write-ups and must be verified before relying on them.

| CVE | Reported effect | Fixed | Source |
| --- | --- | --- | --- |
| CVE-2026-25253 | One-click remote code execution through an unvalidated local WebSocket from any website | not stated | [12] |
| CVE-2026-32048 | Sandbox escape through `sessions_spawn`, which did not enforce sandbox inheritance | 2026.3.1 | [13] |
| CVE-2026-41329 | Sandbox bypass leading to privilege escalation (CVSS 9.9) through the heartbeat `senderIsOwner` handling | after 2026.3.28 | [14] |
| CVE-2026-32922 | Token rotation grants `operator.admin` to a caller holding only `operator.pairing` (CVSS 9.9) | not stated | [15] |
| CVE-2026-44112 | OpenShell write-escape race: configuration tampering, backdoor placement | April 23 patches [4] | [4] |
| CVE-2026-44113 | Read-escape race: access to system files and credentials | April 23 patches [4] | [4] |
| CVE-2026-44115 | Execution allowlist leaks environment variables, including API keys | April 23 patches [4] | [4] |
| CVE-2026-44118 | MCP loopback owner-level access to gateway configuration, cron, and execution environment | April 23 patches [4] | [4] |

- Cyera says none of its four gives Linux root. [4]
- Another summary gives the fixed version for the same set as 2026.4.22. I did
  not resolve the mismatch.
- For the third-party rows I only know what the summaries say, so I cannot rule
  out root there.

## Hardening

1. Run OpenClaw as a dedicated non-root user that is not in `sudo`, `docker`,
   `lxd`, or `libvirt`, and that nobody logs into interactively.
2. Keep elevated mode off and keep host exec approvals on.
3. Turn the sandbox on for any untrusted input, using rootless Docker or Podman
   rather than the `docker` group.
4. Keep bind mounts narrow and never mount the Docker socket.
5. Update OpenClaw past the fixes above and run `openclaw security audit --deep`.
   [16]
6. Keep `NoNewPrivileges` on the service. [11]
7. Disable UTM shared folders and clipboard sharing unless needed. This is
   general knowledge about UTM, not from the docs read here. These can expose
   host data to the guest without any root.
8. Enforce network restrictions on the host, not in the guest.

## Validation plan

On the VM, as the user that runs the gateway:

```text
id
groups
sudo -l
ps -eo user:16,uid,args | grep -i [o]penclaw
openclaw gateway status
openclaw security audit --deep
```

Pass criteria: a non-zero uid, no `sudo`, `docker`, `lxd`, or `libvirt` group,
`sudo -l` asking for a password or listing nothing, and an audit with no
high-severity findings. Record the OpenClaw version, Ubuntu version, install
method, and date, and rerun after each OpenClaw or kernel update. For Docker,
also run `docker exec <container> id` and expect `uid=1000(node)`.

## Evidence status

| Claim | Basis |
| --- | --- |
| Native install runs as the installing account | OpenClaw Linux and gateway docs [1] |
| Dedicated `openclaw` user recommended | OpenClaw DigitalOcean guide [2] |
| Docker image runs as `node` uid 1000 | OpenClaw Docker docs [3][5] |
| Trust model, host-first exec, sandbox off by default | OpenClaw SECURITY.md [7] |
| Cyera's four CVEs do not yield Linux root | Cyera advisory [4] |
| Other CVE details | search-result summaries of third-party write-ups, not verified |
| `sudo` and `docker` groups are root-equivalent; shared account password capture; UTM shared folder exposure | general knowledge, not verified here |
| Which uid a new native user receives | general knowledge |
| Whether the Ansible user joins `docker` | not stated in the page read |

## References

1. [OpenClaw gateway docs](https://github.com/openclaw/openclaw/blob/main/docs/gateway/index.md) and [Linux platform page](https://docs.openclaw.ai/platforms/linux.md)
2. [OpenClaw on DigitalOcean](https://github.com/openclaw/openclaw/blob/main/docs/install/digitalocean.md)
3. [OpenClaw Docker compose operations](https://github.com/openclaw/openclaw/blob/main/docs/install/docker/compose-operations.md)
4. [Cyera: Claw Chain](https://www.cyera.com/blog/claw-chain-cyera-research-unveil-four-chainable-vulnerabilities-in-openclaw)
5. [OpenClaw Fleet storage and container layout](https://github.com/openclaw/openclaw/blob/main/docs/cli/fleet.md)
6. [OpenClaw setup wizard daemon install](https://github.com/openclaw/openclaw/blob/main/docs/start/wizard-cli-reference.md)
7. [OpenClaw SECURITY.md](https://github.com/openclaw/openclaw/blob/main/SECURITY.md)
8. [OpenClaw elevated mode](https://github.com/openclaw/openclaw/blob/main/docs/tools/elevated.md) and [what gets sandboxed](https://github.com/openclaw/openclaw/blob/main/docs/gateway/sandboxing/what-gets-sandboxed.md)
9. [OpenClaw exposure runbook](https://github.com/openclaw/openclaw/blob/main/docs/gateway/security/exposure-runbook.md)
10. [OpenClaw troubleshooting](https://github.com/openclaw/openclaw/blob/main/docs/help/troubleshooting.md)
11. [OpenClaw Ansible install](https://github.com/openclaw/openclaw/blob/main/docs/install/ansible.md)
12. [ProArch: CVE-2026-25253](https://www.proarch.com/blog/threats-vulnerabilities/openclaw-rce-vulnerability-cve-2026-25253)
13. [SentinelOne: CVE-2026-32048](https://www.sentinelone.com/vulnerability-database/cve-2026-32048/)
14. [The Hacker Wire: CVE-2026-41329](https://www.thehackerwire.com/openclaw-sandbox-bypass-leads-to-privilege-escalation-cve-2026-41329/)
15. [ARMO: CVE-2026-32922](https://www.armosec.io/blog/cve-2026-32922-openclaw-privilege-escalation-cloud-security/)
16. [OpenClaw security CLI](https://github.com/openclaw/openclaw/blob/main/docs/cli/security.md)
