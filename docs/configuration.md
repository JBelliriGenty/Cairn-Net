# Configuration reference

Everything Cairn Net reads or writes, key by key. The [README](../README.md)
explains what the panel does; this page is what you edit.

| File | Who writes it | |
|---|---|---|
| `config.json` (first argument) | you | sign-in, address, tokens, alerts |
| users file (`login.users_file`) | you, with `hash-password` | one `name:hash` line per user |
| token files (`summary_token_file`, `export_token_file`) | you | one secret per file, never in the config |
| `notify.headers_file` | you | header lines added to each alert |
| `<data_dir>/settings.json` | the Settings page | the switches, their accounts and modes |
| `<data_dir>/kinds.json` | the Topology and Devices pages | kinds and names set by hand |
| other files in `data_dir` | Cairn Net | deploy key, clones, Bonjour names, terminal log |

Paths are used as written. Give absolute paths: a relative one is read
from the folder Cairn Net was started in (`/` in the Docker image), not
from the config file's folder. With Docker, the config folder is mounted
at `/etc/cairn-net` and the data folder at `/var/lib/cairn-net`.

Check a config: there is no dry run. Check the JSON first, then start the
panel and read its first log line:

```sh
python3 -m json.tool config/config.json > /dev/null && echo "valid JSON"
docker compose up -d && docker compose logs cairn-net | head
```

`cairn-net listening on <address>` means the config was accepted. It
stops at once, with a message, on invalid JSON, a `login` without `tls`, a
token file it cannot read or shorter than 32 characters, a `dhcp` block
without `switch` or `network`, an invalid `settings.json`, or a data folder
it cannot write.

## config.json

| Key | Type | Default | |
|---|---|---|---|
| `listen` | `"host:port"` | `"127.0.0.1:9093"` | `"0.0.0.0:8443"` in Docker (see `examples/compose.yml`) |
| `title` | string | none | shown in the page header |
| `subtitle` | string | none | |
| `lang` | `"en"` \| `"fr"` | the browser's language, else `"en"` | each viewer can switch |
| `data_dir` | path | `"/var/lib/cairn-net"` | must be writable, see [Files written by Cairn Net](#files-written-by-cairn-net) |
| `login` | object | none | built-in sign-in, see [`login`](#login); needs `tls` |
| `tls` | `{"cert", "key"}` | none | PEM files; both set: HTTPS, else plain HTTP |
| `trusted_proxies` | list of IPs | `[]` | proxy mode: only these exact addresses (no CIDR) may connect |
| `user_header` | string | `"Remote-User"` | proxy mode: the signed-in user |
| `groups_header` | string | `"Remote-Groups"` | proxy mode: comma-separated groups |
| `require_groups` | list | `[]` | proxy mode: the user needs one of these groups; empty: any user |
| `switches` | list | `[]` | seeds `settings.json` **the first time only**, see [The settings file](#the-settings-file) |
| `user` | string | `"monitor"` | read account for a seeded switch that has no `user` |
| `password_file` | path | none | file holding the read password for a seeded switch that has no `password` |
| `notify` | `{"url", "headers_file"}` | none | alerts, see [`notify`](#notify) |
| `summary_token_file` | path | none | opens `GET /api/summary` to dashboards; none: closed |
| `export_token_file` | path | none | opens `GET /api/export` to read-only aggregators (Cairn Map); none: closed |
| `dhcp` | object | none | a DHCP file kept outside Git mode, see [`dhcp`](#dhcp) |

Unknown keys are ignored, so a typo does not stop the panel: it just has
no effect. If a setting seems ignored, check its spelling first.

**Who can connect.** With `login`, the built-in sign-in decides and the
proxy keys are not read. Without `login`, only requests from an address in
`trusted_proxies` that carry a user in `user_header` get in. An empty
`trusted_proxies` and no `login` let nobody in, this machine included.

The commit author of Git mode comes from the `Remote-Name` and
`Remote-Email` headers (names fixed, not configurable), else from the user
name. Behind a proxy, make sure it strips these headers from the client.

### `login`

| Key | Type | Default | |
|---|---|---|---|
| `users_file` | path | `"/etc/cairn-net/users"` | `name:hash` per line, `#` for comments; read at each sign-in, no restart needed |
| `session_minutes` | number | `30` | the session ends after this long without a request (1 at least) |

Make a hash with `docker run --rm -i ghcr.io/jbellirigenty/cairn-net
hash-password` (the password is read on stdin) or
`cairn-net hash-password` without Docker.

### `notify`

| Key | Type | Default | |
|---|---|---|---|
| `url` | URL | required | the message is POSTed as the body, with a `Title` header (an ntfy topic URL works as is) |
| `headers_file` | path | none | header lines added to the request (`Authorization: Bearer …`), passed to `curl -H @file` |

Alerts: a firewall table that no longer matches its Git file (or matches
again), and a switch that can no longer resolve a name. Without `notify`,
they are only shown in the page.

### Token files

`summary_token_file` and `export_token_file` each name a file holding one
token, 32 characters at least (`openssl rand -hex 32`). The client sends
`Authorization: Bearer <token>`. Each token opens its own endpoint only,
and nothing else.

- `GET /api/summary`: `switches` (`"2/3"` reachable), `switches_up`,
  `switches_total`, `devices`, `ports_up`, `temp_max_c`, `cpu_max`.
- `GET /api/export`: what the Topology and Devices pages read, plus each
  switch's live `resource`, `health` and `ifaces`.

### `dhcp`

Only for a router whose DHCP file is kept by something else than Git mode
(a copy updated by your own tool), or to restrict the reservations of a
Git-mode router to one network. Most setups do not need it: Direct and
Git modes are set per switch in Settings.

| Key | Type | Default | |
|---|---|---|---|
| `file` | path | required | the DHCP file (YAML, format in the README); without it, the whole block is ignored |
| `switch` | string | required | the switch name (as in Settings) that serves it |
| `network` | CIDR | required | reservations must be inside |
| `server` | string | `"lan"` | its DHCP server, whose leases give names |
| `names_note` | string | none | a line shown under the names of the leases (where your DNS publishes them) |

With a `file` and a switch in read-only mode, the DHCP page shows the file
read only. With a switch in Git mode, its repository file wins and only
`network` and `server` are used.

## The settings file

`<data_dir>/settings.json`, written by the Settings page (mode 600: it
holds the RouterOS passwords). When it does not exist, Cairn Net creates it
from the `switches` of `config.json`; after that, the config's `switches`
are never read again. To seed again, stop Cairn Net, remove the file, start
it.

```json
{
  "switches": [
    {"name": "core", "host": "10.0.10.2", "port": 8729,
     "sha256": "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89",
     "user": "monitor", "password": "…", "write_user": "", "write_password": "",
     "mode": "read", "ignore": [], "url": "",
     "git_url": "", "git_branch": "main", "git_path": "", "git_fw_path": "", "git_dns_path": "",
     "dns": "none", "dns_targets": [], "dns_servers": [], "infra_nat": false}
  ]
}
```

Each switch:

| Key | Type | Default | |
|---|---|---|---|
| `name` | string | required | letters, digits, dashes; unique |
| `host` | string | required | IP or name of the switch |
| `port` | number | `8729` | its `api-ssl` port |
| `sha256` | string | required | the certificate pin; colons and case do not matter |
| `user`, `password` | string | required | the read account |
| `write_user`, `write_password` | string | `""` | the write account; required by `direct` mode and by `dns: "routeros"` |
| `mode` | `"read"` \| `"direct"` \| `"git"` | `"read"` | see the README, How each switch is managed |
| `ignore` | list | `[]` | health entries not shown (`"psu2-state"` on a switch with one power supply) |
| `url` | URL | `""` | its web interface, linked from its page |
| `git_url` | string | `""` | Git mode: the repository (`ssh://…`) |
| `git_branch` | string | `"main"` | |
| `git_path` | path | `""` | the DHCP file, relative to the repository |
| `git_fw_path` | path | `""` | the firewall file (JSON); empty: firewall not managed from Git |
| `git_dns_path` | path | `""` | the static DNS names file (YAML) |
| `dns` | `"none"` \| `"hosts"` \| `"routeros"` | `"none"` | publish the names of the leases, see the README |
| `dns_targets` | list | `[]` | `hosts`: one or more `user@host[:port]` |
| `dns_servers` | list | `[]` | DHCP servers whose leases are published; empty: all |
| `infra_nat` | bool | `false` | allow the router/bridge switch of a port (prototype) |

With a `git_url`, at least one of the three paths is required, each
relative to the repository (no leading `/`, no `..`).

The same keys are accepted in the `switches` of `config.json`, where only
`name`, `host` and `sha256` are needed: the read account then comes from
`user` and `password_file` at the top of the config. A seeded switch is
written to the file as given; the page checks it (pin, accounts, paths)
the first time you save it.

You can edit the file by hand: stop Cairn Net, edit, start it again. The
page never sends the passwords back: an empty password field in the page
keeps the stored one.

## Files written by Cairn Net

All in `data_dir`:

| File | |
|---|---|
| `settings.json` | the switches, above |
| `kinds.json` | MAC (upper case) → `{"kind", "name"}` set by hand on the Topology or Devices page; a plain string is a kind |
| `bonjour.json` | MAC → Bonjour name, so a device asleep keeps its name |
| `git_key`, `git_key.pub` | the SSH key for Git mode and the DNS `hosts` integration, made at first start |
| `known_hosts` | SSH host keys of the Git and DNS servers, accepted on first use |
| `git/<switch>/` | the clone of each Git-mode switch's repository |
| `terminal.log` | every Terminal command, one JSON line each |

`kinds.json` by hand, same rules as the page (kinds: `switch`, `router`,
`ap`, `server`, `nas`, `pc`, `phone`, `printer`, `camera`, `iot`,
`unknown`; names of 63 characters at most):

```json
{"AA:BB:CC:DD:EE:01": {"kind": "nas", "name": "storage"}, "AA:BB:CC:DD:EE:02": "printer"}
```

Removing `git_key` makes a new key at the next start: add its public half
to the repositories and DNS servers again.

## Recipes

**Smallest useful config**: built-in sign-in over HTTPS, switches added
from Settings.

```json
{"listen": "0.0.0.0:8443",
 "login": {"users_file": "/etc/cairn-net/users"},
 "tls": {"cert": "/etc/cairn-net/cert.pem", "key": "/etc/cairn-net/key.pem"}}
```

**Behind Authelia (or any authenticating proxy)**: no `login`, answer only
the proxy, and keep the panel to one group.

```json
{"listen": "0.0.0.0:9093", "trusted_proxies": ["10.0.10.5"], "require_groups": ["network-admins"]}
```

**Seed the switches from the config** (a fresh install from Ansible, for
example): one read password for all of them; fingerprints as `openssl` or
RouterOS print them.

```json
{"listen": "0.0.0.0:8443",
 "login": {"users_file": "/etc/cairn-net/users"},
 "tls": {"cert": "/etc/cairn-net/cert.pem", "key": "/etc/cairn-net/key.pem"},
 "user": "monitor", "password_file": "/etc/cairn-net/monitor-password",
 "switches": [{"name": "core", "host": "10.0.10.2", "sha256": "AB:CD:…"},
              {"name": "access1", "host": "10.0.10.3", "sha256": "12:34:…"}]}
```

Only the first start reads them: later changes go through Settings.

**Alerts to ntfy**, with a token:

```json
{"notify": {"url": "https://ntfy.example.com/network", "headers_file": "/etc/cairn-net/ntfy-headers"}}
```

with `/etc/cairn-net/ntfy-headers` holding
`Authorization: Bearer tk_…`.

**Open the map's export to Cairn Map**:

```json
{"export_token_file": "/etc/cairn-net/export-token"}
```

then give Cairn Map's `cairn_net` module the same token in its own
`token_file`.
