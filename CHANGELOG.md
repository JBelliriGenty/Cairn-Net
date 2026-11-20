# Changelog

## 0.10.2 (2026-11-20)

- **Restart** button under the state of each radio on the access point
  cards (direct mode with a write account): the radio stops three seconds
  and starts again with its SSIDs; clients join again by themselves. For a
  radio that stops taking new clients while the connected ones stay (seen
  on RouterOS 7.25.1: new devices associate and drop at once, in a loop,
  with nothing in the info log). One background job on the access point
  turns the radio back on, even if Cairn Net's connection drops.
- New `POST /api/wifi/restart` (`switch`, `iface`); refused for an SSID
  interface, an unknown one or a disabled radio.

## 0.10.1 (2026-11-20)

- Drift notices no longer fire at a schedule boundary: where Cairn Net runs
  the schedules (direct mode with a write account), the drift check judges
  an access point as the schedules leave it within the minute. Before, a
  check falling in the minute after a boundary sent "differs from the
  Wi-Fi state", then "in line again" ten minutes later.

## 0.10.0 (2026-11-20)

- SSID schedules: in **Edit the SSIDs**, an SSID may be on air only from
  one hour to another on the days ticked (from 22:00 until 07:00 runs past
  midnight; hours left empty: always). The hours are those of each access
  point's own clock (its time zone), read with the rest. Outside them the
  SSID counts as off: the access point cards and drift notices agree with
  the schedule, and taking the state again from an access point keeps it.
- Cairn Net runs the schedules itself (access points without a scheduler,
  such as hAP in device-mode `home`): every minute, on access points in
  direct mode with a write account, it turns the interfaces of scheduled
  SSIDs on or off, and nothing else of the state, with the usual rollback,
  kept by itself once a new connection reads them back. A change waiting
  for **Keep** delays it by a minute; a failure is notified once. The read
  account looks first: the write account logs in only when there is
  something to switch.
- A schedule fits an SSID of its own (guests, children): the main SSID of
  the radios cannot go off without the SSIDs it carries, and is refused.
- `POST /api/wifi/state` op `ssids` takes `schedule` (`days` 1 = Monday to
  7, `from`, `to` as `HH:MM`) per SSID, or none.

## 0.9.0 (2026-11-20)

- Roaming, per SSID of the Wi-Fi state (**Edit the SSIDs**): fast roaming
  (802.11r, written where the SSID's security lives), roaming help
  (802.11k neighbour reports and 802.11v transition requests) and dual-band
  devices steered to 5 GHz (2.4 GHz probe answers delayed), both written
  on the SSID's interfaces. Taken from an access point like the rest of
  the state, shown on the SSIDs (differences between access points too).
  802.11r needs a password; the preview warns that some older devices
  cannot join with it. RouterOS fills the 802.11k neighbour list of each
  access point by itself (its own radios).
- Clients: **Disconnect** (now, on the access point it is on, direct mode;
  it usually comes back at once) and **Block**: the device goes into the
  Wi-Fi state, and applying it on each access point adds a reject row of
  the access list (comment `cairn-net: blocked`, first in the list) and
  disconnects it if it is there. **Unblock** removes the row. Blocking the
  device you use cuts you off: the change is then not kept, and the
  access point undoes it by itself.
- Guests: a **QR code** button per SSID of the state shows the code phones
  read to join, with the password (`POST /api/wifi/share`, never cached,
  like every answer). **New password** in it draws a random one guests can
  type (`xxxx-xxxx-xxxx`), saves it in the state and applies the state on
  each access point in direct mode, previewed and kept as usual.
- `POST /api/wifi/state` takes `block`, `unblock` (`mac`, `name`) and
  `rotate` (`ssid`); new `POST /api/wifi/kick` (`switch`, `mac`) and
  `/api/wifi/share` (`ssid`). New dependency: `qrcode` (no default
  features, no dependency of its own).

## 0.8.1 (2026-11-20)

- Adopting an access point whose write account refuses Cairn Net's key
  now says so ("cairn-write does not accept Cairn Net's key: run the two
  RouterOS lines first", or "the first-setup script did not run there"
  for a new one) instead of the raw `Permission denied` of ssh.
- The **Adopt** dialog puts the two RouterOS lines first, and gives them
  as one `ssh <account>@<address> '…'` command to copy (the account is
  remembered by the browser, `admin` by default).

## 0.8.0 (2026-11-19)

- Wi-Fi state (`data/wifi.json`, mode 600): the SSIDs common to every
  access point (name, VLAN, security, password, client isolation, hidden,
  bands) and each access point's radios (channel, width, power). Taken
  once from an access point (**Take it from …**: its SSIDs, their
  passwords in direct mode, its radios), then edited on the page (**Edit
  the SSIDs**: add, rename, remove, VLAN, security, bands; an empty
  password keeps the old one). Changing the state writes nothing to the
  access points, and the page never receives a password.
- Each access point's card says whether it is in line with the state, or
  lists what applying it would change (passwords aside: the read account
  cannot see them). **Apply the state** previews, applies with the write
  account, reads back until nothing is left to change, and is undone by
  the access point unless kept. **Set** now saves that access point's
  radios in the state, then previews the state there. The per-SSID form
  of 0.7.0 is gone: the state replaces it.
- Applying the state creates the SSIDs an access point lacks (on the
  radio's own interface when it is free, else one secondary interface per
  SSID), turns them off on the bands they leave, removes those the state
  no longer has, and adds their VLANs on its uplink. A VLAN new to an
  access point must already pass through the switch port across (link of
  the topology, that switch's bridge VLANs): otherwise refused, naming the
  port to fix. Nothing is written to the switch.
- Drift: every 10 minutes the access points are compared with the state
  (passwords too in direct mode); a change is notified once.
- New access points: **Add an access point** writes a first-setup script
  without any secret (bridge and uplink, management VLAN and address,
  radios off, accounts with random passwords nobody keeps, Cairn Net's
  public SSH key on the write account `cairn-write`, services closed but
  api-ssl and SSH, then the lines of `data/ap-extra.rsc` if there is one,
  VLAN filtering last). **Adopt** logs in over SSH with that key, gives
  the accounts passwords only Cairn Net knows, matches the api-ssl
  certificate with the fingerprint shown over SSH, and saves the access
  point in Settings, direct mode. An access point already in Settings is
  adopted the same way once its write account has the key (two RouterOS
  lines shown on its card).
- `GET /api/wifi` carries `state` (`has_passphrase`, never the password),
  `app` (Cairn Net's address as the switches see it) and, per access
  point, `plan`, `plan_warn`, `plan_error`, `has_write`. New:
  `POST /api/wifi/state` (`import`, `ssids`, `radios`),
  `/api/wifi/bootstrap`, `/api/wifi/adopt`. `/api/wifi/preview` and
  `/apply` take `switch` only.

## 0.7.0 (2026-11-19)

- Wi-Fi page: an access point in direct mode is set from its card
  (**Settings**): channel (the radio's legal channels for its country,
  radar channels marked), width and transmit power per radio, automatic
  or fixed; each SSID on or off per radio (a radio off takes its secondary
  SSIDs with it); client isolation and a new password per SSID. Previewed
  with warnings (radar channel, automatic 5 GHz channel, clients cut or
  reconnecting, new password to give out), read back, undone by the access
  point unless kept. Isolation and password go to the SSID's own
  datapath or security profile (refused when another SSID shares it), or
  on its interfaces when it has none.
- Rollback without scheduler: where the device-mode forbids
  `/system/scheduler` (hAP access points leave the factory in mode
  `home`), the rollback is a background `:execute` job watching a global
  variable; "Keep" and the countdown restart go through that variable.
  Every page's guarded change uses it on such a device.
- Passwords never shown: commands, rollback scripts and errors mask
  `password` and `passphrase` values everywhere.
- `POST /api/wifi/preview`, `POST /api/wifi/apply`; "Keep" is
  `/api/infra/confirm`.

## 0.6.0 (2026-11-19)

- Wi-Fi page (read only): every switch with Wi-Fi radios (RouterOS 7
  `wifi` package) is an access point, and all of them show as one network.
  Access points with their radios (band, channel, width, transmit power,
  state, clients); SSIDs merged by name (VLAN, security, client isolation,
  where each one exists) with the differences between access points
  (VLAN, security, isolation, hidden, enabled, bands, missing on an access
  point); the channel map per band (DFS range, overlapping blocks of two
  radios); the clients of every access point with their name and IP (the
  Devices names: DHCP leases, names given by hand, DNS), SSID, VLAN, band,
  signal, link rates, traffic and time connected; connections and roaming
  read from the access points' logs (between radios of one access point,
  and from one access point to another).
- `GET /api/wifi` (5 s cache shared by viewers).
- Overview: an access point's tile counts its Wi-Fi clients and opens the
  Wi-Fi page; Topology: access points carry their icon and open it too;
  Devices: a Wi-Fi port shows its SSID.

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
