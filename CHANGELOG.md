# Changelog

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
