<p align="center"><img src="www/icon.svg" width="96" alt=""></p>

# Cairn Net (`cairn-net`)

A web panel for a few **MikroTik RouterOS** switches and routers: what is
plugged where, link state and traffic, VLANs, and the DHCP servers and
reservations of the router. One Rust binary with the page embedded, in a
small Docker image. It talks to the switches through the RouterOS API over
TLS (`api-ssl`), with a read-only account by default.

> Interface in English and French. Early days: it runs every day on its
> author's network (three CRS switches), on RouterOS 7. Issues and pull
> requests are welcome. Sister project:
> [Cairn](https://github.com/JBelliriGenty/Cairn-NAS), the same panel for a
> Debian + ZFS NAS.

## What it does

- **Overview**: every switch at a glance (ports up, traffic, CPU,
  temperature, uptime, health sensors) and the links between switches.
- **Ports**: front panel per switch, speed, traffic, what is behind each
  port.
- **Devices**: every MAC address the switches see, with its port, VLAN, IP
  address and name (DHCP leases, ARP, reverse DNS).
- **VLANs**: which ports carry which VLAN, tagged or untagged.
- **DHCP**: servers (pool use, options, static routes) and reservations of
  the router, with an editor that shows the difference before anything is
  written.
- **Settings**: switches are added from the page. Cairn Net reads the
  certificate of the switch first, you compare its fingerprint, then the
  accounts are tried. A RouterOS script to prepare a switch is shown there.

## How each switch is managed

Per switch, in Settings:

| Mode | What Cairn Net does |
|---|---|
| **Read only** (default) | Reads. Nothing is ever written. |
| **Direct** | Writes the DHCP servers and reservations to the switch through its API, with a second account that can write. It reads the switch back afterwards and says whether it matches. |
| **Git** | Edits a DHCP file in a Git repository and pushes one commit per change, under the name of the person who applies it. Your own tool (Ansible, CI…) applies the file to the switch. |

Direct mode suits a router managed by hand; Git mode suits one managed as
code. Both use the same editor and checks: reservations inside the network,
no duplicate MAC, IP or name, ranges and gateways that make sense.

## Install

With Docker, on any machine that can reach the switches' `api-ssl` port
(8729):

```sh
mkdir -p cairn-net/config cairn-net/data && cd cairn-net
curl -fsSLO https://raw.githubusercontent.com/JBelliriGenty/Cairn-Net/main/examples/compose.yml
curl -fsSL https://raw.githubusercontent.com/JBelliriGenty/Cairn-Net/main/examples/config.json -o config/config.json

# HTTPS certificate (or use your own)
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=cairn-net \
  -keyout config/key.pem -out config/cert.pem

# a user: the password is read on stdin
printf 'admin:' > config/users
docker run --rm -i ghcr.io/jbellirigenty/cairn-net hash-password >> config/users

# the container runs as nobody (65534)
chown -R 65534:65534 config data && chmod 600 config/key.pem config/users

docker compose up -d
```

Then open `https://<address>:8443`, sign in, and add your switches in
**Settings**.

The data directory keeps the switches and their accounts
(`settings.json`, readable by the container only), the Git deploy key and
the clones of the repositories.

### Preparing a switch

Settings shows a script to paste in the switch terminal (WebFig → Terminal,
or SSH), filled with the address of Cairn Net. It creates:

1. a **read** account (`read,api`);
2. optionally a **write** account for direct mode (`read,write,api`, without
   `policy`, `password`, `reboot` or `sensitive`);
3. a self-signed certificate for `api-ssl`, with the service limited to the
   address of Cairn Net;

then prints the certificate fingerprint, to compare with the one Cairn Net
shows when you add the switch. A different fingerprint later (certificate
regenerated, or someone in the middle) means the connection is refused
before any password is sent.

## Sign-in

Two ways, set in `config.json`:

- **Built-in** (`login` + `tls`): users from the users file, one
  `name:hash` line each, hashes made by `hash-password` (PBKDF2-SHA256). The
  file is read at each sign-in: no restart needed after editing it. Session
  cookie, expired after `session_minutes` of inactivity; failed attempts
  double the wait for that address. HTTPS is required.
- **Behind an authenticating reverse proxy** (Authelia, Authentik, oauth2-proxy…):
  no `login`; Cairn Net only answers requests from `trusted_proxies` that
  carry the user in `user_header` (default `Remote-User`) and, if
  `require_groups` is set, one of those groups in `groups_header` (default
  `Remote-Groups`). Commits are then signed with `Remote-Name` and
  `Remote-Email` when the proxy sends them. See `examples/config-proxy.json`.

## Configuration

`config.json`:

| Key | Default | |
|---|---|---|
| `listen` | `127.0.0.1:9093` | address and port |
| `title`, `subtitle` | | shown in the page |
| `lang` | `en` | default language of the page (`en`, `fr`); each viewer can switch |
| `data_dir` | `/var/lib/cairn-net` | |
| `login` | | `{"users_file": "/etc/cairn-net/users", "session_minutes": 30}` |
| `tls` | | `{"cert": "…/cert.pem", "key": "…/key.pem"}`, PEM files |
| `trusted_proxies`, `user_header`, `groups_header`, `require_groups` | | proxy mode |
| `switches` | | seeds Settings the first time only (`name`, `host`, `sha256`, `user`, `mode`…); then Settings is the reference |

## Git mode

Cairn Net creates one SSH deploy key at first start and shows its public
half in Settings. Add it to the repository **with write access**, then set,
for the switch: the repository (`ssh://…`), the branch and the path of the
DHCP file. On each change, Cairn Net starts from the tip of the branch,
replays the changes, refuses if a check fails, commits and pushes. A commit
pushed by someone else in between is kept.

The DHCP file is YAML, edited as text: comments and layout outside the
touched entry stay as they are.

```yaml
dhcp_reservations:
  # comment lines above an entry belong to it
  - "AA:BB:CC:DD:EE:01,192.168.10.20,printer"
  - "AA:BB:CC:DD:EE:02,192.168.10.21,nas"

dhcp_servers:
  - name: lan
    enabled: true
    interface: bridge
    network: 192.168.10.0/24
    range: 192.168.10.100-192.168.10.199
    gateway: 192.168.10.1
    dns: [192.168.10.2]
    domain: home.arpa
    ntp: []
    lease_time: 1d
    routes:
      - "192.168.20.0/24,192.168.10.254"
```

Applying it is up to you: for example an Ansible play that turns each
reservation into a static lease and each server into
`/ip dhcp-server`, `/ip pool` and `/ip dhcp-server network` (routes as
option 121).

## Security notes

- Read-only accounts by default; the write account has no `policy`,
  `password`, `reboot` or `sensitive` rights.
- The switch certificate is pinned (SHA-256 fingerprint).
- Passwords of the switch accounts are kept in `data/settings.json`
  (mode 600, owner of the container), never sent back to the page.
- Every write is shown first as a difference; direct mode reads the switch
  back after writing.
- No third-party JavaScript, a strict Content-Security-Policy, POSTs in
  JSON only.

## Build

```sh
docker build -t cairn-net .
```

The build runs the tests (`cargo test`). Without Docker: Rust 1.85 or
later, `libssl-dev`, `cargo build --release --locked`; at run time `git`
and `ssh` for Git mode.

## License

MIT
