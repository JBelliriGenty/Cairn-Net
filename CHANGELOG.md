# Changelog

## 0.5.3 (2026-11-10)

- Devices: every interface MAC of the switches is left out, not only the one
  each announces. A port blocked by STP is still learned by the switch
  across, and showed up as an unknown device.

## 0.5.2 (2026-11-10)

- Links between switches: a neighbour heard through a VLAN interface (an
  `mgmt99` announcing itself out of an access port) is no longer a link of
  its own; only cables with an Ethernet port at both ends are kept.
- An access link (one untagged VLAN, nothing tagged at both ends, such as
  the switches' management cables) is labelled "access, VLAN n" with the
  VLAN's comment.

## 0.5.1 (2026-11-10)

- MSTP: each switch's port states in the extra instances are read
  (`/interface/bridge/port/mst-override/monitor`, `msti` in the state). A link
  blocked for the common tree but forwarding in its own instance (management
  VLAN on dedicated links) is shown in service, no longer "standby (STP)".

## 0.5.0 (2026-11-07)

- `GET /api/export` also carries each switch's live state, as `/api/fast`
  reads it: `resource` (CPU, memory, uptime), `health` (temperatures,
  voltages) and `ifaces` (`running`, `disabled`, counters, link downs), for
  the up/down dots of Cairn Map.

## 0.4.0 (2026-11-07)

- `export_token_file`: a second bearer token that opens `GET /api/export` only
  (switches, links, devices as the Topology page reads them), for read-only
  aggregators such as Cairn Map. Separate from the dashboards' summary token.
- Switch DNS check: every 10 minutes each switch is asked to resolve
  `upgrade.mikrotik.com` with the read account; shown on the
  Infrastructure page ("Resolves names") and notified when it changes.
  A stale `/ip dns servers` breaks NTP by name and the update check while
  the clients, served by DHCP, see nothing.

## 0.3.4 (2026-11-07)

- Bonjour names are kept in `data_dir/bonjour.json`: a device asleep keeps
  its name across restarts.

## 0.3.3 (2026-11-07)

- Bonjour names: every 2 minutes a unicast mDNS reverse query goes to each
  bound lease; the answer ("MacBook-Pro-de-Alex") is used for DNS and the
  Devices page, after a name set by hand or a reservation, before the
  DHCP host-name. Kept per MAC while the app runs.
- Two leases with the same name: the higher IP gets -2, -3... instead of
  being left out.

## 0.3.2 (2026-11-07)

- Git mode: a `.git/index.lock` left by a restart in the middle of a Git
  command no longer blocks the switch's repository ("file unreadable").

## 0.3.1 (2026-11-07)

- DNS names of the leases: a name set by hand on the Devices page now wins,
  then the reservation name, then the client's host-name; names with spaces
  or accents become valid labels ("Meross Smart Plug" -> meross-smart-plug)
  instead of being dropped.

## 0.3.0 (2026-11-07)

- `GET /api/summary` for dashboards such as Homepage, opened by a bearer
  token (`summary_token_file`): switches reachable, devices, Ethernet ports
  up, hottest sensor, busiest CPU.

## 0.2.0 (2026-11-05)

- Topology view: switches, links (LACP, STP-blocked backups) with live
  traffic, devices under their port, device kind and name set by hand.
- Interfaces: ports, bonds, bridge ports and VLANs, VLAN interfaces, SFP
  diagnostics.
- IP and routing: addresses, routes, ARP, pools, DHCP client, IPv6.
- System: clock, log, users, scheduler and scripts, files, export, backup
  and restore, packages, certificates, SNMP.
- Tools: ping, traceroute, torch, bandwidth test, IP scan, netwatch,
  packet capture.
- Chip & QoS: switch chip settings of CRS3xx, IGMP/MLD snooping, queues.
- Terminal: RouterOS commands per switch, logged.
- Infrastructure: restart, management address move, admin access card
  (`/ip service`, MAC-WinBox), write account rights shown.
- Ports: the "Connected to" value opens that device or the switch behind.
- Own favicon colours; overview ports no longer clipped in the corners.

## 0.1.1 (2026-11-05)

- Generic ISP box words instead of a brand in the router name guess.

## 0.1.0 (2026-11-05)

- First public release: overview, ports, devices, VLANs, DHCP (direct and
  Git mode), DNS integrations, infrastructure page, firewall editor with
  automatic rollback, settings, built-in sign-in.
