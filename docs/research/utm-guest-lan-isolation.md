# UTM guest network access and LAN isolation

Researched 2026-10-07. Nothing in this document was executed on a Mac; see
[Evidence status](#evidence-status).

## Conclusion

An Ubuntu guest in UTM on a macOS host, using the default **Shared Network**
mode, can open outbound connections to any device on the host's LAN, for example
a receiver at `192.168.0.101`. The host NATs the traffic, so the device sees the
Mac as the source. The guest cannot be discovered or reached by LAN devices, and
multicast or broadcast discovery (SSDP) does not cross the NAT.

UTM has no setting that keeps internet access while blocking the LAN. Shared
Network and Bridged allow both, and Host Only blocks both. "Isolate Guest from
Host" covers only the host, not other LAN devices. [1] See
[Network modes](#network-modes) for every mode by backend.

To allow the internet but block the LAN, the only approach with evidence is a
**host-side `pf` rule** on the Mac that blocks traffic from the guest subnet to
private address ranges. [2] It must be enforced on the host because a guest-side
rule can be removed by root in the guest. It also **fails open**: if macOS
flushes the rules, the guest silently regains LAN access. A fail-closed
alternative is Bridged mode onto a guest VLAN or guest Wi-Fi with client
isolation, enforced on the network gear.

## Scope and example addressing

This is background for running Linux guests against a real receiver; it does not
change any crate or delivery package. Addresses used below are examples:

| Device | Address |
| --- | --- |
| macOS host, LAN side | `192.168.0.100` |
| Receiver / other LAN device | `192.168.0.101` |
| vmnet bridge (guest gateway, DNS, DHCP) | `192.168.64.1` |
| Guest, Shared Network | `192.168.64.0/24` |

The guest range is a vmnet default and UTM can change it. Confirm the real
subnet and bridge with `ifconfig` on the host while the VM runs.

## UTM and Shared Network

UTM is a virtual machine host for macOS and iOS built on QEMU. [3] It has two
backends:

- **QEMU** (default) virtualizes or emulates. Virtualization requires the guest
  architecture to match the host: `aarch64` on Apple Silicon, `x86_64` on Intel.
  A mismatched architecture is emulated and much slower. [4]
- **Apple Virtualization** only virtualizes, is less mature, and is the only way
  to run macOS guests on Apple Silicon. [5]

Shared Network is the default for new VMs. [1] Per the UTM docs the host routes
the traffic and the guest shares a VLAN with the host, so services on the guest
and the host can see each other without extra configuration. [1] Beyond the docs
(from general knowledge of UTM, not verified here): it is built on Apple's vmnet
NAT, the guest gets a `192.168.64.x` address by DHCP, and the host appears as
`192.168.64.1`. The UTM scripting example shows a guest address of
`192.168.64.9`. [6]

The **emulated network card** is a separate setting. `virtio-net-pci` is a
paravirtualized card that the docs recommend but say may need guest drivers. [1]
Ubuntu ships virtio drivers, so it works without setup. The card does not change
what the guest can reach; the network mode does.

## Network modes

The modes available depend on the backend.

### QEMU backend

| Mode | Behavior | LAN | Internet |
| --- | --- | --- | --- |
| Emulated VLAN | QEMU's own NAT. Requests appear to come from the UTM process. Port forwarding is available only in this mode. [1][13] | not specified | yes |
| Shared Network | The host routes the traffic and the guest shares a VLAN with the host. Recommended default. [1] | yes | yes |
| Host Only | The guest is isolated from the host network and from other VMs. No DHCP is provided, so IP settings are manual. [1] | no | no |
| Bridged | Layer 2 bridge to a chosen interface. Advanced option. [1] | yes | yes, if that interface has it |

Host Only can use named **host networks** created in UTM's app settings. VMs on
the same host network can talk to each other and are isolated from the host and
other networks. This is supported only by the QEMU backend, and host networks can
be imported from VMware Fusion. [14]

### Apple Virtualization backend

| Mode | Behavior |
| --- | --- |
| Shared Network | Same description as above. Recommended default. [15] |
| Bridged | The host creates a layer 2 bridge with the specified interface. "For advanced users." [15] |

This backend offers only these two. It has no Emulated VLAN, no Host Only, and
no host networks.

### Related settings

- **Isolate Guest from Host** blocks the guest from connecting to the host. It
  does not block other LAN devices. [1]
- **Emulated Network Card** (`virtio-net-pci` recommended) sets the virtual
  hardware, not the network mode. [1]

## The bridge

In Shared Network mode, macOS creates a virtual switch on the host when the VM
starts, commonly named `bridge100`, with the host at `192.168.64.1` on it. The
bridge name and subnet come from a search-result summary of a UTM discussion and
were not confirmed against Apple documentation. [16]

```text
guest -> bridge100 (host side 192.168.64.1) -> host -> router -> internet / LAN
```

- The guest uses `192.168.64.1` as its gateway. From general knowledge, not
  verified here, it is also the guest's DNS and DHCP server, which is why the
  `pf` rule set allows those two services before blocking.
- Every guest packet crosses this bridge before the host sends it on. That is why
  a host-side `pf` rule can filter it regardless of what root in the guest does.
- The bridge number is not fixed and can change when other vmnet users run.
  Confirm it with `ifconfig` while the VM is up.
- Do not confuse it with UTM's **Bridged** mode. The virtual switch belongs to
  Shared Network and keeps the guest behind NAT. Bridged mode puts the guest on
  the real LAN with its own `192.168.0.x` address.

## Reachability from the guest

Guest to LAN device, outbound:

```text
guest 192.168.64.x -> host 192.168.64.1 / 192.168.0.100 (NAT) -> 192.168.0.101
```

- Outbound TCP and UDP unicast work. The device replies to `192.168.0.100` and
  the host maps replies back.
- The two subnets do not overlap, so there is no routing conflict.
- LAN devices cannot initiate connections to the guest.
- SSDP and other multicast or broadcast discovery do not cross the NAT. Direct
  connections by IP address work.
- If the host can reach the device but the guest cannot, check **System Settings
  > Privacy & Security > Local Network** for UTM. This is from memory and is not
  confirmed against the UTM docs.

Bridged mode gives the guest its own LAN address, which restores inbound
connections and discovery.

## Blocking access

### Guest-side (bypassable)

These stop an ordinary app but not a guest user with root.

- **systemd:** `sudo systemd-run --pty --wait --uid="$USER" -p
  IPAddressDeny=192.168.0.101 /path/to/app`. `systemd-run -p` sets properties on
  the transient unit and `IPAddressDeny=` is a documented unit property, but the
  docs reviewed show no combined example. Test it. [7]
- **nftables:** an output chain rule such as `ip daddr 192.168.0.101 meta skuid
  appuser reject`, with the app run as a dedicated user. Chain and rule syntax
  follow the nftables wiki; the `skuid` match is from general knowledge. [8]

### Host-side `pf`

The apple/container discussion covers an Apple vmnet NAT network on the same
`192.168.64.0/24` subnet. It reports that blocking must be on the **`in`**
direction with the guest subnet as the source, e.g. `block in quick from
192.168.64.0/24 to <rfc1918>`, and that `block out` and guest-side rules did not
filter reliably. [2] It was not tested with UTM.

That example blocks `192.168.0.0/16`, which includes the gateway `192.168.64.1`.
In Shared mode the gateway is also the guest's DNS and DHCP server, so the rule
set must allow those first:

```text
guest = "192.168.64.0/24"
gw    = "192.168.64.1"
table <lan> const { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 100.64.0.0/10 }

pass  in quick proto { udp tcp } from $guest to $gw port 53
pass  in quick proto udp         from $guest to $gw port 67
block in quick from $guest to <lan>
```

- Every rule uses `quick` because `pf` is last-match-wins otherwise.
- `<lan>` covers the receiver and the host's LAN address. It also covers
  link-local and carrier-grade NAT space (the latter is used by Tailscale).
- Everything else on the gateway address is blocked.

To load it without editing the main ruleset, try a sub-anchor under Apple's
`com.apple/*` wildcard. This is untested:

```text
sudo pfctl -E
sudo pfctl -a com.apple/utm-lanblock -f utm-lanblock.pf
sudo pfctl -sA
sudo pfctl -a com.apple/utm-lanblock -vsr
```

Do not run `pfctl -f /etc/pf.conf` while a VM is running. One Apple forum report
says a custom `rdr` rule on `bridge100` broke all traffic on that bridge until
reboot. [9]

### Known gaps

- **IPv6.** The table is IPv4 only. LAN devices can have IPv6 addresses it never
  matches. Check `ip -6 addr` and `ip -6 route` in the guest. A blanket `inet6`
  block would also block the host's IPv6, so it needs an interface match such as
  `on bridgeN`. Whether `pf` accepts a rule for a bridge that does not exist yet
  is unverified.
- **Spoofed source.** Matching on the guest subnet means a guest root user who
  sets a static address outside it might not match. Matching on the bridge
  closes this but shares the interface-name problem above.
- **Gateway services.** The apple/container thread notes the host gateway stays
  reachable from the guest network when host services bind to `0.0.0.0`. [2]

### Why it fails open

- Apple states that Packet Filter is not API (TN3165) and that it does not
  support custom `pf.conf` rules. [10]
- One forum report says macOS flushes the main ruleset on network changes such
  as toggling Wi-Fi, and that the rules had to be reloaded by hand. It is not
  known whether a sub-anchor survives. [11]
- Surviving reboot takes a LaunchDaemon in `/Library/LaunchDaemons` that runs
  `pfctl` at boot. [12] A `StartInterval` that re-applies the rules is a
  possible mitigation but is untested.
- If the rules disappear, nothing tells the guest or the user.

### Fail-closed alternative

Router-side blocking cannot work in Shared mode, because the router sees all
guest traffic as coming from `192.168.0.100`. Use Bridged mode onto a guest VLAN
or guest Wi-Fi with client isolation. The block then lives on the network gear
and does not depend on macOS. This requires a router or access point that
supports it.

## Related

[OpenClaw user and root risk in a VM](openclaw-vm-user-and-root-risk.md) covers
why guest-side rules are not enough when an agent runs in the guest: root in the
guest removes them.

## Validation plan

Record the macOS version, UTM version, backend, network mode, guest subnet, and
bridge name. Run after setup, after every reboot, after toggling Wi-Fi, and
after a macOS update.

These must **fail** in the guest:

```text
ping -c2 192.168.0.101
nc -vz -w3 192.168.0.101 23
nc -vz -w3 192.168.0.100 22
nc -vz -w3 192.168.64.1 22
```

Also test an IPv6 LAN address if the guest has one.

These must **succeed**:

```text
resolvectl query example.com
curl -I https://example.com
```

On the host, `pfctl -a com.apple/utm-lanblock -vsr` should show hit counters
rising during the failing tests. Mark each check `pass` or `fail`, and do not
claim isolation unless every check, including the post-reboot and post-Wi-Fi
runs, passed.

## Evidence status

| Claim | Basis |
| --- | --- |
| UTM mode behavior, Host Only blocks LAN and internet | UTM docs [1] |
| Apple backend offers only Shared and Bridged; host networks are QEMU only | UTM docs [14][15] |
| Bridge named `bridge100` on `192.168.64.0/24` | search-result summary of a UTM discussion [16], not confirmed |
| `pf` `block in quick from <guest subnet>` filters vmnet traffic | apple/container discussion, not UTM [2] |
| `pf` is unsupported and rules can be flushed | Apple DTS and forum reports [9][10][11] |
| Shared Network uses vmnet NAT, `192.168.64.1` gateway with DNS and DHCP, Local Network permission | general knowledge, not verified |
| Anchor loading under `com.apple/*`, interface-name behavior, `StartInterval` mitigation | untested suggestions |

## References

1. [UTM network settings (QEMU)](https://docs.getutm.app/settings-qemu/devices/network/network/)
2. [apple/container discussion #719: filtering vmnet traffic with pf](https://github.com/apple/container/discussions/719)
3. [What is UTM?](https://docs.getutm.app/index)
4. [UTM QEMU system settings: architecture](https://docs.getutm.app/settings-qemu/system)
5. [UTM Apple Virtualization backend](https://docs.getutm.app/settings-apple/settings-apple)
6. [UTM scripting cheat sheet](https://docs.getutm.app/scripting/cheat-sheet)
7. [systemd documentation](https://github.com/systemd/systemd)
8. [nftables wiki: configuring chains and rejecting traffic](https://wiki.nftables.org/)
9. [Apple Developer Forums: pf.conf thread 776492](https://developer.apple.com/forums/thread/776492)
10. [Apple Developer Forums: DTS on pf, thread 776505](https://developer.apple.com/forums/thread/776505)
11. [Apple Developer Forums: pf rules flushed on network changes, thread 774555](https://developer.apple.com/forums/thread/774555)
12. [macOS pf LaunchDaemon guide](https://inventivehq.com/knowledge-base/macos/how-to-configure-macos-firewall-pf)
13. [UTM port forwarding (QEMU, Emulated VLAN only)](https://docs.getutm.app/settings-qemu/devices/network/port-forwarding)
14. [UTM app settings: host networks](https://docs.getutm.app/preferences/macos)
15. [UTM network settings (Apple Virtualization)](https://docs.getutm.app/settings-apple/devices/network/)
16. [UTM discussion #7344: connecting to the guest in Shared Network mode](https://github.com/utmapp/UTM/discussions/7344)
