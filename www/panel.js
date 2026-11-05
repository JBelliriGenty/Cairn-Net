// cairn-net front end. The server (src/main.rs) reads the switches (RouterOS
// API, read-only) and returns raw rows; this page derives everything else:
// rates from byte counters, the front panel, where each device is plugged.
// The server CSP forbids inline styles: widths go through data-w (paint()).
"use strict";

const FAST_MS = 3000;   // resources, health, interface counters
const SLOW_MS = 20000;  // links, VLANs, neighbours, MAC tables, leases
const HIST = 60;        // sparkline points (3 min at 3 s)
const ENTER_MS = matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 650;
const VIEWS = { overview: "Overview", ports: "Ports", clients: "Devices", dhcp: "DHCP", vlans: "VLANs", infra: "Infrastructure", settings: "Settings" };

let CFG = { switches: [] };
let D = null, CHANGES = [], PREVIEW = null; // /api/dhcp, pending reservation changes, their preview
let F = null, S = null, prevF = null, VIEW = null, SW = null, SEL = null, animUntil = 0;
const rates = {};   // "switch|iface" -> {rx, tx} in bit/s
const hist = {};    // switch -> {rx: [], tx: []}

/* ---------- helpers (as in Cairn) ---------- */

const $ = id => document.getElementById(id);
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const nf = (n, d = 1) => Number(n).toLocaleString(LOCALE(), { maximumFractionDigits: d, minimumFractionDigits: d });
const clamp = (v, a = 0, b = 100) => Math.max(a, Math.min(b, v));
const push = (arr, v) => { arr.push(v); if (arr.length > HIST) arr.shift(); };
const last = arr => (arr && arr.length ? arr[arr.length - 1] : 0);
const num = v => Number(v) || 0;
const yes = v => v === "true" || v === "yes";
const list = v => (v ? String(v).split(",").map(x => x.trim()).filter(Boolean) : []);
const natural = (a, b) => a.localeCompare(b, undefined, { numeric: true });
const short = n => String(n).replace(/^sfp-sfpplus/, "sfp").replace(/^qsfpplus/, "qsfp");

function bytes(n) {
  const u = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  while (Math.abs(n) >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return nf(n, i ? 1 : 0) + " " + t(u[i]);
}
function bits(bps) {
  const u = ["b/s", "Kb/s", "Mb/s", "Gb/s"];
  let i = 0;
  while (bps >= 1000 && i < u.length - 1) { bps /= 1000; i++; }
  return nf(bps, i ? 1 : 0) + " " + t(u[i]);
}
function duration(s) {
  const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60);
  return (d ? t("{d} d", { d }) + " " : "") + h + " h " + String(m).padStart(2, "0");
}
// RouterOS durations: "1w2d3h4m5s".
function rosSeconds(s) {
  const k = { w: 604800, d: 86400, h: 3600, m: 60, s: 1 };
  return [...String(s || "").matchAll(/(\d+)([wdhms])/g)].reduce((a, [, n, u]) => a + n * k[u], 0);
}
// RouterOS health names -> words: cpu-temperature, fan2-speed, psu1-state…
function healthLabel(n) {
  const m = String(n).match(/^(cpu|switch|phy|sfp|board)?-?temperature$/);
  if (m) return t("Temperature ({what})", { what: t(m[1] || "board") });
  let k = String(n).match(/^fan(\d+)-speed$/);
  if (k) return t("Fan {n}", { n: k[1] });
  k = String(n).match(/^psu(\d+)-state$/);
  if (k) return t("Power supply {n}", { n: k[1] });
  return n === "fan-state" ? t("Fans") : n;
}
const worst = lvls => (lvls.includes("bad") ? "bad" : lvls.includes("warn") ? "warn" : "ok");

function spark(series, floor = 1, area = false) {
  const max = Math.max(floor, ...series.flat());
  const pts = s => s.map((v, i) => (s.length > 1 ? i * 100 / (s.length - 1) : 100).toFixed(1) + "," + (30 - v / max * 28).toFixed(1));
  return '<svg class="spark' + (area ? " area" : "") + '" viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">' +
    series.map((s, i) => {
      const p = pts(s);
      if (!p.length) return "";
      const line = "M" + p.join("L");
      return (area ? '<path class="fill s' + i + '" d="' + line + "L100,30L" + p[0].split(",")[0] + ',30Z"/>' : "") +
        '<path class="s' + i + '" d="' + line + '"/>';
    }).join("") + "</svg>";
}

function paint(root = document) {
  root.querySelectorAll("[data-w]").forEach(e => { e.style.width = clamp(Number(e.dataset.w)) + "%"; });
  root.querySelectorAll("table").forEach(tb => {
    const head = [...tb.querySelectorAll("thead th")].map(th => th.textContent);
    if (head.length) tb.querySelectorAll("tbody tr").forEach(r => [...r.cells].forEach((c, i) => { if (head[i]) c.dataset.label = head[i]; }));
  });
}

function banner(msg, kind = "") {
  $("banner").hidden = !msg;
  $("banner").textContent = msg || "";
  $("banner").dataset.kind = kind;
  // an answer to a click must be seen even when the page is scrolled (not the periodic read errors)
  if (msg && kind !== "read") $("banner").scrollIntoView({ block: "nearest", behavior: "smooth" });
}
const readOk = () => { if ($("banner").dataset.kind === "read") banner(""); };

const cssMs = name => parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
async function post(path, payload) {
  const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload), redirect: "manual" });
  if (r.type === "opaqueredirect") location.reload();
  const res = await r.json().catch(() => ({ error: r.status + " " + r.statusText }));
  if (!r.ok) throw new Error(res.error || r.status);
  return res;
}

// Dialog (as in Cairn): resolves with the form data, or null when cancelled.
function ask(title, bodyHtml, okLabel = t("OK"), setup) {
  const d = $("dlg");
  $("dlg-title").textContent = title;
  $("dlg-body").innerHTML = bodyHtml;
  if (setup) setup(d);
  $("dlg-ok").textContent = okLabel;
  d.classList.remove("is-closing");
  d.showModal();
  void d.offsetWidth;
  d.classList.add("is-open");
  return new Promise(res => d.addEventListener("close", () =>
    res(d.returnValue === "ok" ? new FormData(d.querySelector("form")) : null), { once: true }));
}
function closeDialog(value) {
  const d = $("dlg");
  if (!d.classList.contains("is-open")) return;
  d.classList.replace("is-open", "is-closing");
  setTimeout(() => { d.classList.remove("is-closing"); d.close(value); }, cssMs("--modal-close-dur"));
}
$("dlg").querySelector("form").addEventListener("submit", e => { e.preventDefault(); closeDialog(e.submitter ? e.submitter.value : "ok"); });
$("dlg").addEventListener("cancel", e => { e.preventDefault(); closeDialog("cancel"); });
// Cancel is not a submit button: Enter in a field submits with OK, the default button.
$("dlg-cancel").addEventListener("click", () => closeDialog("cancel"));

async function get(path) {
  const r = await fetch(path, { cache: "no-store", redirect: "manual" });
  // session expired: back to the proxy's portal, or to the built-in sign-in form
  if (r.type === "opaqueredirect" || (r.status === 401 && CFG.user)) location.reload();
  if (!r.ok) throw Object.assign(new Error(r.status + " " + (await r.text())), { status: r.status });
  return r.json();
}

/* ---------- model: one object per switch, from the raw rows ---------- */

const isPort = i => i.type === "ether";
// Front-panel order: copper, combo, then cages; numbers in natural order.
const GROUPS = ["ether", "combo", "sfp", "qsfp"];
const groupOf = n => { const g = n.replace(/\d+$/, "").replace(/-$/, ""); return g.startsWith("sfp") ? "sfp" : g.startsWith("qsfp") ? "qsfp" : g; };

function model() {
  const all = CFG.switches.map(c => {
    const f = (F.switches.find(x => x.name === c.name)) || {}, s = (S && S.switches.find(x => x.name === c.name)) || {};
    const ignore = new Set(c.ignore || []);
    const health = (f.health || []).filter(h => !ignore.has(h.name));
    const ifaces = f.ifaces || [];
    const mon = Object.fromEntries((s.monitor || []).map(m => [m.name, m]));
    const bridgePort = Object.fromEntries((s.ports || []).map(p => [p.interface, p]));
    const bondOf = {};
    for (const b of s.bonds || []) for (const sl of list(b.slaves)) bondOf[sl] = b.name;
    // Inter-switch links: interfaces where a MikroTik neighbour answers (port, its bond).
    const uplinks = new Set();
    const neighborOf = {};
    for (const n of s.neighbors || []) {
      const [port, ...rest] = list(n.interface);
      if (!port) continue;
      neighborOf[port] = n;
      if (n.platform === "MikroTik") { uplinks.add(port); rest.filter(x => x !== "bridge").forEach(x => uplinks.add(x)); }
    }
    const vlans = (s.vlans || []).map(v => ({ ids: v["vlan-ids"], comment: v.comment, dynamic: yes(v.dynamic),
      tagged: list(v["current-tagged"]).filter(x => x !== "bridge"), untagged: list(v["current-untagged"]).filter(x => x !== "bridge") }));
    const ports = ifaces.filter(isPort).sort((a, b) => GROUPS.indexOf(groupOf(a.name)) - GROUPS.indexOf(groupOf(b.name)) || natural(a.name, b.name)).map(i => {
      const via = bondOf[i.name] || i.name; // a bond member takes its VLANs from the bond
      const bp = bridgePort[via];
      return {
        name: i.name, comment: i.comment, running: yes(i.running), disabled: yes(i.disabled), linkDowns: num(i["link-downs"]),
        rate: (mon[i.name] || {}).rate, mon: mon[i.name] || {}, bond: bondOf[i.name], uplink: uplinks.has(i.name) || uplinks.has(via),
        pvid: bp && bp.pvid, tagged: vlans.filter(v => v.tagged.includes(via)).map(v => v.ids), neighbor: neighborOf[i.name],
        rx: (rates[c.name + "|" + i.name] || {}).rx || 0, tx: (rates[c.name + "|" + i.name] || {}).tx || 0,
      };
    });
    const r = f.resource || {};
    return { name: c.name, url: c.url, error: f.error || s.error, r, rb: s.routerboard || {}, health, ports, uplinks, vlans,
      bonds: (s.bonds || []).map(b => ({ name: b.name, comment: b.comment, running: yes(b.running), slaves: list(b.slaves) })),
      hosts: s.hosts || [], leases: s.leases || [], arp: s.arp || [], neighbors: s.neighbors || [], ms: f.ms };
  });
  return all;
}

// Health rows -> problems (thresholds for the switch chips, not for SFP modules).
function issuesOf(sw) {
  const out = [];
  if (sw.error) return [["bad", t("{name} unreachable: {err}", { name: sw.name, err: sw.error })]];
  for (const h of sw.health) {
    if (/state$/.test(h.name) && h.value !== "ok") out.push([h.name.startsWith("psu") ? "warn" : "bad", t("{name}: {what} {value}", { name: sw.name, what: healthLabel(h.name), value: h.value })]);
    if (h.type === "C" && num(h.value) >= 75) out.push([num(h.value) >= 90 ? "bad" : "warn", t("{name}: {what} {value} °C", { name: sw.name, what: healthLabel(h.name), value: h.value })]);
  }
  if (num(sw.r["cpu-load"]) >= 85) out.push(["warn", t("{name}: CPU {pct} %", { name: sw.name, pct: sw.r["cpu-load"] })]);
  for (const b of sw.bonds) {
    const up = b.slaves.filter(n => (sw.ports.find(p => p.name === n) || {}).running).length;
    if (!b.running) out.push(["bad", t("{name}: {bond} down", { name: sw.name, bond: b.name })]);
    else if (up < b.slaves.length) out.push(["warn", t("{name}: {bond} {up} / {total} links", { name: sw.name, bond: b.name, up, total: b.slaves.length })]);
  }
  return out;
}

// MAC -> {ip, name}, from leases, the switches' ARP tables and this host's own.
function identities(sws) {
  const ipOf = {}, nameOf = {};
  for (const [ip, mac] of S.arp || []) ipOf[mac] = ip;
  for (const sw of sws) for (const a of sw.arp) if (a["mac-address"]) ipOf[a["mac-address"]] = a.address;
  for (const sw of sws) for (const l of sw.leases) {
    const mac = l["mac-address"];
    if (!mac) continue;
    if (l.status === "bound" || !ipOf[mac]) ipOf[mac] = l.address;
    const n = l["host-name"] || String(l.comment || "").replace(/^ansible:\s*/, "");
    if (n) nameOf[mac] = n;
  }
  return mac => { const ip = ipOf[mac]; return { ip, name: nameOf[mac] || (ip && S.names[ip]) || "" }; };
}

// Each MAC where it is plugged: the entry that is not on an inter-switch link.
function devices(sws) {
  const who = identities(sws), seen = new Map();
  for (const sw of sws) for (const h of sw.hosts) {
    const port = h["on-interface"];
    if (!port || sw.uplinks.has(port)) continue;
    const key = h["mac-address"] + "|" + sw.name + "|" + port;
    const d = seen.get(key) || Object.assign({ mac: h["mac-address"], sw: sw.name, port, vids: [] }, who(h["mac-address"]));
    if (h.vid && !d.vids.includes(h.vid)) d.vids.push(h.vid);
    seen.set(key, d);
  }
  return [...seen.values()].sort((a, b) => natural(a.sw, b.sw) || natural(a.port, b.port) || natural(a.name || "~", b.name || "~"));
}

/* ---------- settings: the switches, their accounts and how each is managed ---------- */

let SET = null; // /api/settings
const MODE_LABEL = { read: "Read only", direct: "Direct", git: "Git" };
const DNS_LABEL = { none: "No", hosts: "Hosts file over SSH (Pi-hole, dnsmasq)", routeros: "Static DNS of this switch (write account)" };
const MODE_HELP = {
  read: "Cairn Net only reads this switch.",
  direct: "Cairn Net writes to the switch through its API, after showing the difference.",
  git: "Cairn Net edits a file in a repository; your own tool (Ansible, CI) applies it.",
};

async function loadSettings() {
  SET = await get("/api/settings");
  renderSettings();
}

function renderSettings() {
  if (!SET) return;
  const live = n => (F && F.switches.find(x => x.name === n)) || {};
  $("set-switches").innerHTML = SET.switches.map(s => {
    const f = live(s.name), res = f.resource || {};
    const st = !F ? ["", t("…")] : f.error ? ["bad", f.error] : ["ok", [res["board-name"], res.version && "RouterOS " + res.version].filter(Boolean).join(" · ") || t("connected")];
    const acct = (label, u, has) => "<li>" + esc(t(label)) + " : " + (u ? "<b>" + esc(u) + "</b>" + (has ? "" : " · " + esc(t("no password"))) : "—") + "</li>";
    return '<article class="card srv ' + (st[0] || "") + '"><div class="card-head"><h3>' + esc(s.name) + '</h3><span class="pill">' + esc(t(MODE_LABEL[s.mode] || s.mode)) + "</span></div>" +
      '<p class="sub">' + esc(s.host + ":" + s.port) + "</p>" +
      '<p class="sub"><span class="dot ' + st[0] + '"></span> ' + esc(st[1]) + "</p>" +
      '<ul class="gives">' + acct("Read account", s.user, s.has_password) + acct("Write account", s.write_user, s.has_write_password) +
      (s.mode === "git" && s.git_url ? '<li class="mono fp">' + esc(s.git_url + " · " + s.git_branch + " · " + s.git_path) + "</li>" : "") +
      '<li class="mono fp" title="' + esc(t("Certificate fingerprint (sha256)")) + '">' + esc(s.sha256) + "</li></ul>" +
      '<div class="card-head foot"><span class="sub">' + esc(t(MODE_HELP[s.mode] || "")) + '</span><span class="actions"><button data-act="sw-edit" data-name="' + esc(s.name) + '">' +
      esc(t("Edit")) + '</button><button class="danger" data-act="sw-del" data-name="' + esc(s.name) + '">' + esc(t("Delete")) + "</button></span></div></article>";
  }).join("") || '<p class="sub">' + esc(t("No switch yet: add the first one.")) + "</p>";
  renderPrep();
  $("git-key").textContent = SET.git_key || t("No key: ssh-keygen is missing in the image.");
  $("dns-line").textContent = 'command="/usr/local/bin/cairn-net-hosts /etc/pihole/hosts/cairn-net.list /etc/pihole/hosts/custom.list",restrict ' + (SET.git_key || "ssh-ed25519 …");
}

// Settings changed: the rest of the page reads the new switch list.
async function settingsSaved(res) {
  SET = res.settings;
  CFG = await get("/api/config");
  F = S = prevF = null;
  renderSettings();
  slowTick();
  fastTick();
}

async function swForm(s, typed, err) {
  const v = Object.assign({ name: "", host: "", port: 8729, user: "", write_user: "", mode: "read", url: "", ignore: [], sha256: "", git_url: "", git_branch: "main", git_path: "", dns: "none", dns_targets: [], dns_servers: [] }, s || {}, typed || {});
  const inp = (k, label, val, more = "") => "<label>" + esc(t(label)) + '<input name="' + k + '" value="' + esc(val) + '"' + more + "></label>";
  const pw = (k, label, has) => "<label>" + esc(t(label)) + '<input type="password" name="' + k + '" autocomplete="new-password"' +
    (has ? ' placeholder="' + esc(t("unchanged")) + '"' : "") + "></label>";
  const fd = await ask(s ? t("Edit {name}", { name: s.name }) : t("New switch"),
    (err ? '<p class="banner">' + esc(err) + "</p>" : "") +
    '<div class="cols2">' + inp("name", "Name", v.name, ' required maxlength="63" pattern="[A-Za-z0-9]([A-Za-z0-9\\-]*[A-Za-z0-9])?"') +
    inp("host", "Address", v.host, ' required placeholder="192.168.88.1"') + "</div>" +
    '<div class="cols2">' + inp("port", "API port (api-ssl)", v.port, ' type="number" min="1" max="65535" required') +
    inp("url", "Its web interface (optional)", v.url, ' placeholder="http://192.168.88.1"') + "</div>" +
    "<label>" + esc(t("How this switch is managed")) + '<select name="mode">' + SET.modes.map(m => '<option value="' + m + '"' + (m === v.mode ? " selected" : "") + ">" +
      esc(t(MODE_LABEL[m]) + " — " + t(MODE_HELP[m])) + "</option>").join("") + "</select></label>" +
    '<div class="cols2">' + inp("user", "Read account", v.user, " required") + pw("password", "Its password", s && s.has_password) +
      inp("write_user", "Write account (direct mode)", v.write_user) + pw("write_password", "Its password", s && s.has_write_password) + "</div>" +
    '<div class="cols2">' + inp("git_url", "Repository (git mode)", v.git_url, ' placeholder="ssh://git@git.example.com/network.git"') +
      inp("git_path", "DHCP file in it", v.git_path, ' placeholder="network/dhcp.yml"') + inp("git_branch", "Branch", v.git_branch) + "</div>" +
    "<label>" + esc(t("Publish the names of the leases (DNS)")) + '<select name="dns">' + Object.keys(DNS_LABEL).map(k => '<option value="' + k + '"' +
      (k === v.dns ? " selected" : "") + ">" + esc(t(DNS_LABEL[k])) + "</option>").join("") + "</select></label>" +
    '<div class="cols2">' + inp("dns_targets", "Hosts targets (user@host[:port], comma separated)", (v.dns_targets || []).join(", "), ' placeholder="root@192.168.88.2"') +
      inp("dns_servers", "Of these DHCP servers only (comma separated, empty = all)", (v.dns_servers || []).join(", ")) + "</div>" +
    inp("ignore", "Health entries to ignore (comma separated, e.g. psu2-state)", (v.ignore || []).join(", ")) +
    '<p class="sub">' + esc(t("Saving first reads the switch's certificate: you compare its fingerprint, then the accounts are tested.")) + "</p>",
    t("Save"));
  if (!fd) return;
  const g = k => String(fd.get(k) || "").trim();
  const sw = { name: g("name"), host: g("host"), port: +g("port") || 8729, url: g("url"), mode: g("mode"), user: g("user"), password: String(fd.get("password") || ""),
    write_user: g("write_user"), write_password: String(fd.get("write_password") || ""), ignore: g("ignore").split(/[\s,;]+/).filter(Boolean),
    git_url: g("git_url"), git_branch: g("git_branch") || "main", git_path: g("git_path"),
    dns: g("dns") || "none", dns_targets: g("dns_targets").split(/[\s,;]+/).filter(Boolean), dns_servers: g("dns_servers").split(/[\s,;]+/).filter(Boolean),
    sha256: s && s.host === g("host") && s.port === (+g("port") || 8729) ? s.sha256 : "" };
  const again = msg => swForm(s, sw, msg);
  const orig = s ? s.name : "";
  try {
    let pr = await post("/api/settings/probe", { orig, switch: sw });
    if (!pr.ok) return again(t("Cannot reach {host}: {err}", { host: sw.host, err: pr.error }));
    if (pr.sha256 !== sw.sha256) {
      const ok = await ask(t("Trust this certificate?"),
        "<p>" + esc(t(sw.sha256 ? "The certificate of {host} CHANGED. New fingerprint (sha256):" : "Fingerprint (sha256) of the certificate of {host}:", { host: sw.host })) + "</p>" +
        '<p class="mono fp big-fp">' + esc(pr.sha256) + "</p><p class=\"sub\">" + esc(t("Compare it with the switch's own: /certificate print detail (fingerprint), or the last line of the preparation script.")) + "</p>",
        t("Trust"));
      if (!ok) return;
      sw.sha256 = pr.sha256;
      pr = await post("/api/settings/probe", { orig, switch: sw });
      if (!pr.ok) return again(t("Cannot reach {host}: {err}", { host: sw.host, err: pr.error }));
    }
    if (pr.read && pr.read.error) return again(t("Read account refused: {err}", { err: pr.read.error }));
    if (pr.write && pr.write.error) return again(t("Write account refused: {err}", { err: pr.write.error }));
    if (pr.git && pr.git.error) return again(t("Repository refused: {err}. Is the deploy key added with write access?", { err: pr.git.error }));
    const res = await post("/api/settings", { op: "save", orig, switch: sw });
    if (!res.ok) return again(problemText(res.error));
    await settingsSaved(res);
  } catch (e) { return again(e.message); }
}

async function swDelete(s) {
  if (!(await ask(t("Delete {name}?", { name: s.name }), "<p>" + esc(t("Cairn Net forgets this switch and its accounts. Nothing changes on the switch.")) + "</p>", t("Delete")))) return;
  const res = await post("/api/settings", { op: "delete", name: s.name });
  if (!res.ok) return banner(problemText(res.error));
  await settingsSaved(res);
}

// RouterOS script that prepares a switch for Cairn Net (pasted in its terminal).
function prepScript(ip) {
  const a = ip || "CAIRN-NET-ADDRESS";
  return [
    "# Cairn Net: read account, optional write account, API over TLS only",
    "# Replace both passwords. Paste in the switch terminal (WebFig > Terminal or ssh).",
    "/user group add name=cairn-read policy=read,api",
    "/user add name=cairn-read group=cairn-read address=" + a + "/32 password=\"CHANGE-ME-READ\"",
    "# Direct mode only (Cairn Net writes): no policy, password, reboot or sensitive rights",
    "/user group add name=cairn-write policy=read,write,api",
    "/user add name=cairn-write group=cairn-write address=" + a + "/32 password=\"CHANGE-ME-WRITE\"",
    "# Self-signed certificate for api-ssl (ten years)",
    "/certificate add name=cairn-api common-name=cairn-api days-valid=3650 key-size=2048 key-usage=key-cert-sign,digital-signature,key-encipherment,tls-server",
    "/certificate sign cairn-api",
    "/ip service set api-ssl certificate=cairn-api address=" + a + "/32 disabled=no",
    "# Plain API: turn it off unless another tool still uses it",
    "# /ip service set api disabled=yes",
    "# Fingerprint to compare with the one Cairn Net shows",
    ":put [/certificate get cairn-api fingerprint]",
  ].join("\n");
}
function renderPrep() {
  const ip = $("prep-ip").value.trim();
  $("prep-script").textContent = prepScript(/^[0-9.]+$/.test(ip) ? ip : "");
}

/* ---------- loops ---------- */

async function fastTick() {
  try {
    const f = await get("/api/fast");
    if (prevF) {
      const dt = (f.t - prevF.t) / 1000;
      for (const s of f.switches) {
        const p = prevF.switches.find(x => x.name === s.name);
        if (!p || !s.ifaces || !p.ifaces || dt <= 0) continue;
        let rx = 0, tx = 0;
        for (const i of s.ifaces) {
          const o = p.ifaces.find(x => x.name === i.name);
          if (!o) continue;
          const r = { rx: Math.max(0, num(i["rx-byte"]) - num(o["rx-byte"])) * 8 / dt, tx: Math.max(0, num(i["tx-byte"]) - num(o["tx-byte"])) * 8 / dt };
          rates[s.name + "|" + i.name] = r;
          if (isPort(i)) { rx += r.rx; tx += r.tx; }
        }
        hist[s.name] = hist[s.name] || { rx: [], tx: [] };
        push(hist[s.name].rx, rx); push(hist[s.name].tx, tx);
      }
    }
    prevF = F = f;
    readOk();
    render();
  } catch (e) { banner(t("Read failed: {err}", { err: e.message }), "read"); }
}

async function slowTick() {
  try {
    S = await get("/api/slow");
    if (CFG.dhcp) D = await get("/api/dhcp?switch=" + encodeURIComponent(CFG.dhcp.switch));
    if (VIEW === "infra") loadInfra();
    readOk(); render(true);
  } catch (e) { banner(t("Read failed: {err}", { err: e.message }), "read"); }
}

/* ---------- views ---------- */

function show(view) {
  const [v, sw] = String(view || "").split("/");
  if (sw) SW = decodeURIComponent(sw);
  view = VIEWS[v] ? v : "overview";
  document.querySelectorAll(".view").forEach(s => { const on = s.dataset.view === view; s.hidden = !on; s.classList.toggle("enter", on); });
  document.querySelectorAll("#nav button").forEach(b => b.setAttribute("aria-current", b.dataset.view === view ? "page" : "false"));
  $("view-title").textContent = t(VIEWS[view]);
  VIEW = view;
  const hash = view + ((view === "ports" || view === "infra") && SW ? "/" + encodeURIComponent(SW) : "");
  if (location.hash.slice(1) !== hash) history.replaceState(null, "", "#" + hash);
  if (view === "settings") loadSettings().catch(e => banner(e.message));
  if (view === "infra" && (!INF || INF.name !== SW)) { INF = null; loadInfra(); }
  render(true);
  animUntil = Date.now() + ENTER_MS;
  clearTimeout(show.timer);
  show.timer = setTimeout(() => document.querySelectorAll(".view.enter").forEach(s => s.classList.remove("enter")), ENTER_MS);
}

function render(force = false) {
  if (VIEW === "settings") renderSettings();
  if (!F || !S) return;
  if (!force && Date.now() < animUntil) return;
  const sws = model();
  if (!SW || !sws.some(s => s.name === SW)) SW = sws[0] && sws[0].name;
  renderChrome(sws);
  if (VIEW === "overview") renderOverview(sws);
  if (VIEW === "ports") renderPorts(sws);
  if (VIEW === "clients") renderClients(sws);
  if (VIEW === "vlans") renderVlans(sws);
  if (VIEW === "dhcp") renderDhcp();
  if (VIEW === "infra") renderInfra();
  paint();
}

function renderChrome(sws) {
  $("pills").innerHTML = sws.map(s => {
    const lvl = worst(issuesOf(s).map(([l]) => l));
    return '<button class="pill ' + lvl + '" data-goto="ports/' + esc(s.name) + '">' + esc(s.name) + "</button>";
  }).join("");
  const v = [...new Set(sws.map(s => s.r.version).filter(Boolean))];
  $("meta").textContent = [tp(sws.length, "{n} switch", "{n} switches"), v.length ? "RouterOS " + v.join(", ") : "",
    t("refreshed {time}", { time: new Date().toLocaleTimeString(LOCALE()) })].filter(Boolean).join(" · ");
}

/* overview */

function tile(view, title, value, lvl, extra) {
  return '<button class="card tile ' + (lvl || "") + '" data-goto="' + esc(view) + '"><span class="label">' + esc(title) + "</span><strong>" +
    esc(value) + "</strong>" + extra + "</button>";
}

// One entry per pair of switches seen by LLDP / MNDP (both ends report it: keep one).
function links(sws) {
  const out = [], done = new Set();
  for (const s of sws) {
    const by = {};
    for (const n of s.neighbors.filter(n => n.platform === "MikroTik")) {
      const [port, bond] = list(n.interface);
      const key = n.identity + "|" + (bond && bond !== "bridge" ? bond : port);
      (by[key] = by[key] || { peer: n.identity, via: bond && bond !== "bridge" ? bond : port, ports: [] }).ports.push(port);
    }
    for (const l of Object.values(by)) {
      const pair = [s.name, l.peer].sort().join("|");
      if (done.has(pair)) continue;
      done.add(pair);
      const b = s.bonds.find(x => x.name === l.via);
      const members = b ? b.slaves : l.ports;
      const upn = members.filter(n => (s.ports.find(p => p.name === n) || {}).running).length;
      const speed = members.map(n => (s.ports.find(p => p.name === n) || {}).rate).filter(Boolean);
      out.push({ s, l, b, members, upn, speed, lv: upn === members.length ? "ok" : upn ? "warn" : "bad" });
    }
  }
  return out;
}

function renderOverview(sws) {
  const issues = sws.flatMap(issuesOf);
  const lvl = worst(issues.map(([l]) => l));
  const ports = sws.flatMap(s => s.ports), up = ports.filter(p => p.running).length;
  const lk = links(sws), devs = devices(sws);
  if (!sws.length) {
    $("verdict").className = "verdict warn";
    $("verdict").innerHTML = '<span class="dot warn"></span>' + esc(t("No switch yet: add one in Settings."));
    $("tiles").innerHTML = "";
    return;
  }
  $("verdict").className = "verdict " + lvl;
  $("verdict").innerHTML = '<span class="dot ' + lvl + '"></span>' + esc(issues.length ? issues.map(([, m]) => m).join(" · ") :
    t("All good: {sw}, {ports}, {links}, {devs}", { sw: tp(sws.length, "{n} switch reachable", "{n} switches reachable"),
      ports: tp(up, "{n} active port", "{n} active ports"), links: tp(lk.filter(x => x.lv === "ok").length, "{n} link between switches up", "{n} links between switches up"),
      devs: tp(devs.length, "{n} device", "{n} devices") }));

  $("tiles").innerHTML = sws.map(s => {
    const l = worst(issuesOf(s).map(([x]) => x));
    if (s.error) return tile("ports/" + s.name, s.name, t("unreachable"), "bad", '<p class="sub">' + esc(s.error) + "</p>");
    const temps = s.health.filter(h => h.type === "C").map(h => num(h.value));
    const h = hist[s.name] || { rx: [], tx: [] };
    const sp = s.ports.filter(p => p.running).length;
    return tile("ports/" + s.name, s.name + " · " + (s.rb.model || s.r["board-name"] || ""), t("{up} / {total} ports up", { up: sp, total: s.ports.length }), l,
      '<div class="front mini">' + frontPanel(s, true) + "</div>" +
      '<div class="rates"><span class="rx">↓ ' + bits(last(h.rx)) + '</span><span class="tx">↑ ' + bits(last(h.tx)) + "</span></div>" +
      spark([h.rx, h.tx], 8000, true) +
      '<p class="sub">' + esc([t("CPU {pct} %", { pct: num(s.r["cpu-load"]) }), temps.length ? nf(Math.max(...temps), 0) + " °C" : "",
        t("up for {d}", { d: duration(rosSeconds(s.r.uptime)) })].filter(Boolean).join(" · ")) + "</p>");
  }).join("");

  const rows = links(sws).map(({ s, l, b, members, upn, speed, lv }) =>
      "<tr><td><b>" + esc(s.name) + "</b> ⇄ <b>" + esc(l.peer) + "</b></td><td>" + esc(b ? t("{bond} (LACP)", { bond: b.name }) : short(l.via)) +
        "</td><td>" + esc(members.map(short).join(", ")) + "</td><td>" + esc(speed.length ? speed.length + " × " + speed[0] : "") +
        '</td><td><span class="pill ' + lv + '">' + t("{up} / {total} up", { up: upn, total: members.length }) + "</span></td>" +
        '<td class="num">' + bits(members.reduce((a, n) => a + ((rates[s.name + "|" + n] || {}).rx || 0) + ((rates[s.name + "|" + n] || {}).tx || 0), 0)) + "</td></tr>");

  $("links").innerHTML = "<thead><tr><th>" + t("Link") + "</th><th>" + t("Via") + "</th><th>" + t("Ports") + "</th><th>" + t("Speed") + "</th><th>" +
    t("State") + "</th><th>" + t("Traffic") + "</th></tr></thead><tbody>" + (rows.join("") || '<tr><td colspan="6" class="sub">' + t("No link between switches seen (LLDP / MNDP).") + "</td></tr>") + "</tbody>";

  const vids = new Set(sws.flatMap(s => s.vlans.map(v => v.ids)));
  const upgrades = sws.filter(s => s.rb["upgrade-firmware"] && s.rb["current-firmware"] && s.rb["upgrade-firmware"] !== s.rb["current-firmware"]);
  $("facts").innerHTML = [
    ["clients", tp(devs.length, "{n} device seen", "{n} devices seen"), ""],
    ["vlans", tp(vids.size, "{n} VLAN", "{n} VLANs"), ""],
    ...upgrades.map(s => ["ports/" + s.name, t("{name}: firmware {v} available", { name: s.name, v: s.rb["upgrade-firmware"] }), ""]),
  ].map(([v, txt, c]) => '<button class="chip' + c + '" data-goto="' + esc(v) + '">' + esc(txt) + "</button>").join("");
}

/* ports */

function portState(p) { return p.disabled ? "off" : p.running ? "ok" : "down"; }
function portTitle(p) {
  return [p.name, p.comment, p.disabled ? t("disabled") : p.running ? p.rate || t("up") : t("no link"),
    p.neighbor ? "→ " + p.neighbor.identity : ""].filter(Boolean).join(" · ");
}
const rateShort = r => String(r || "").replace(/bps$/, ""); // 10Gbps -> 10G

// Copper and cages in groups; a group of more than 8 ports goes on two rows, odd on top (as on the device).
function frontPanel(sw, mini = false) {
  const groups = {};
  for (const p of sw.ports) (groups[groupOf(p.name)] = groups[groupOf(p.name)] || []).push(p);
  return Object.entries(groups).map(([g, ps]) => {
    const rows = ps.length > 8 ? [ps.filter((_, i) => i % 2 === 0), ps.filter((_, i) => i % 2 === 1)] : [ps];
    return '<div class="pgroup ' + esc(g) + '">' + rows.map(r => '<div class="prow">' + r.map(p => {
      const n = (p.name.match(/(\d+)$/) || [, p.name])[1];
      const sel = !mini && SEL === p.name ? " sel" : "";
      return (mini ? '<span class="port ' : '<button class="port ') + portState(p) + (p.uplink ? " uplink" : "") + sel + '" title="' + esc(portTitle(p)) + '"' +
        (mini ? "" : ' data-port="' + esc(p.name) + '"') + "><b>" + esc(n) + "</b>" + (mini ? "" : "<small>" + esc(p.running ? rateShort(p.rate) : "") + "</small>") +
        (mini ? "</span>" : "</button>");
    }).join("") + "</div>").join("") + "</div>";
  }).join("");
}

function linkCell(sw, p, devs) {
  if (p.neighbor) {
    const remote = String(p.neighbor["interface-name"] || "").split("/").pop();
    return esc(p.neighbor.identity + (remote ? " · " + short(remote) : ""));
  }
  const here = devs.filter(d => d.sw === sw.name && d.port === (p.bond || p.name));
  if (!here.length) return "";
  if (here.length === 1) return esc(here[0].name || here[0].ip || here[0].mac);
  return '<button class="chip" data-goto="clients" data-q="' + esc(sw.name + " " + (p.bond || p.name)) + '">' + tp(here.length, "{n} device", "{n} devices") + "</button>";
}

function renderPorts(sws) {
  $("sw-tabs").innerHTML = sws.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="ports/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const sw = sws.find(s => s.name === SW);
  if (!sw) return;
  if (sw.error) {
    $("device").innerHTML = '<p class="verdict bad"><span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: sw.name, err: sw.error })) + "</p>";
    $("ports").innerHTML = "";
    return;
  }
  const iss = issuesOf(sw), lvl = worst(iss.map(([l]) => l));
  const kv = [
    [t("Model"), sw.rb.model || sw.r["board-name"]], [t("RouterOS"), sw.r.version],
    [t("Firmware"), sw.rb["current-firmware"] + (sw.rb["upgrade-firmware"] && sw.rb["upgrade-firmware"] !== sw.rb["current-firmware"] ? " → " + sw.rb["upgrade-firmware"] : "")],
    [t("Serial number"), sw.rb["serial-number"]], [t("Up for"), duration(rosSeconds(sw.r.uptime))],
    [t("CPU"), num(sw.r["cpu-load"]) + " %"], [t("Memory"), bytes(num(sw.r["total-memory"]) - num(sw.r["free-memory"])) + " / " + bytes(num(sw.r["total-memory"]))],
    ...sw.health.map(h => [healthLabel(h.name), h.value + (h.type ? " " + h.type.replace(/^C$/, "°C") : ""),
      /state$/.test(h.name) ? (h.value === "ok" ? "ok" : h.name.startsWith("psu") ? "warn" : "bad") : h.type === "C" && num(h.value) >= 75 ? "warn" : ""]),
  ];
  const h = hist[sw.name] || { rx: [], tx: [] };
  $("device").innerHTML = '<div class="card-head"><h3>' + esc(sw.name) + '</h3><span class="pill ' + lvl + '">' +
    esc(iss.length ? tp(iss.length, "{n} problem", "{n} problems") : t("healthy")) + "</span>" +
    (sw.url ? '<a class="chip" href="' + esc(sw.url) + '" target="_blank" rel="noopener">WebFig ↗</a>' : "") + "</div>" +
    '<div class="front">' + frontPanel(sw) + "</div>" +
    '<div class="dev-grid"><dl class="kv">' + kv.map(([k, v, c]) => "<dt>" + esc(k) + '</dt><dd class="' + (c || "") + '">' + esc(v) + "</dd>").join("") + "</dl>" +
    '<div><div class="rates"><span class="rx">↓ ' + bits(last(h.rx)) + '</span><span class="tx">↑ ' + bits(last(h.tx)) + "</span></div>" +
    '<div class="chart">' + spark([h.rx, h.tx], 8000, true) + "</div></div></div>";

  const devs = devices(sws);
  const vlanCell = p => [p.pvid, ...p.tagged.filter(v => v !== p.pvid).map(v => "T" + v)].filter(Boolean).join(" · ");
  $("ports").innerHTML = "<thead><tr><th>" + [t("Port"), t("Name"), t("State"), t("Speed"), t("VLAN"), t("Connected to"), "↓", "↑", t("Link drops")].join("</th><th>") +
    "</th></tr></thead><tbody>" + sw.ports.map(p => {
      const st = portState(p);
      const m = p.mon, sfp = yes(m["sfp-module-present"]) ? [m["sfp-vendor-name"], m["sfp-vendor-part-number"]].filter(Boolean).join(" ") : "";
      return '<tr class="' + (SEL === p.name ? "sel" : "") + (st === "ok" ? "" : " dim") + '" data-row="' + esc(p.name) + '"><td><b>' + esc(p.name) + "</b>" +
        (p.bond ? ' <span class="pill">' + esc(p.bond) + "</span>" : "") + (sfp ? '<br><small class="sub">' + esc(sfp) + "</small>" : "") +
        "</td><td>" + esc(p.comment || "") + '</td><td><span class="pill ' + (st === "ok" ? "ok" : "") + '">' + esc(st === "ok" ? t("up") : st === "off" ? t("disabled") : t("no link")) +
        "</span></td><td>" + esc(p.running ? p.rate || "" : "") + "</td><td>" + esc(vlanCell(p)) + "</td><td>" + linkCell(sw, p, devs) +
        '</td><td class="num rx">' + (p.running ? bits(p.rx) : "") + '</td><td class="num tx">' + (p.running ? bits(p.tx) : "") +
        '</td><td class="num">' + p.linkDowns + "</td></tr>";
    }).join("") + "</tbody>";
}

/* devices */

function renderClients(sws) {
  const q = $("q").value.trim().toLowerCase();
  const all = devices(sws);
  const rows = all.filter(d => !q || q.split(/\s+/).every(w => [d.name, d.ip, d.mac, d.sw, d.port, short(d.port), ...d.vids].join(" ").toLowerCase().includes(w)));
  $("clients").innerHTML = "<thead><tr><th>" + [t("Device"), t("IP address"), t("MAC address"), t("Switch"), t("Port"), t("VLAN")].join("</th><th>") + "</th></tr></thead><tbody>" +
    (rows.map(d => "<tr><td>" + esc(d.name || "—") + "</td><td>" + esc(d.ip || "") + '</td><td class="mono">' + esc(d.mac) + "</td><td>" + esc(d.sw) +
      '</td><td><button class="chip" data-goto="ports/' + esc(d.sw) + '" data-sel="' + esc(d.port) + '">' + esc(d.port) + "</button></td><td>" + esc(d.vids.join(", ")) + "</td></tr>").join("") ||
      '<tr><td colspan="6" class="sub">' + t("No device matches.") + "</td></tr>") + "</tbody>";
  $("view-title").textContent = t(VIEWS.clients) + " · " + (q ? rows.length + " / " : "") + all.length;
}

/* DHCP: reservations from the file (repository) vs the router, or the router itself (direct mode) */

// Line diff (LCS): [" ", line] kept, ["-", line] removed, ["+", line] added.
function diffLines(a, b) {
  const n = a.length, m = b.length, L = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out = [];
  let i = 0, j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) { out.push([" ", a[i]]); i++; j++; }
    else if (j < m && (i === n || L[i][j + 1] >= L[i + 1][j])) out.push(["+", b[j++]]);
    else out.push(["-", a[i++]]);
  }
  return out;
}

const rawRouter = () => S.switches.find(x => x.name === CFG.dhcp.switch) || {};
const lanLeases = () => (rawRouter().leases || []).filter(l => !CFG.dhcp.server || l.server === CFG.dhcp.server);
const macUp = l => String(l["mac-address"] || "").toUpperCase();

async function queue(change) {
  CHANGES.push(change);
  await refreshPreview();
}
// The server rebuilds dhcp.yml from the pending changes (nothing is written).
// Only the latest call counts: a slower, older answer is dropped.
let previewSeq = 0;
async function refreshPreview() {
  const seq = ++previewSeq, sent = CHANGES.slice();
  let p = null;
  if (sent.length) {
    try { p = await post("/api/dhcp/preview", { switch: CFG.dhcp.switch, changes: sent }); } catch (e) { p = { ok: false, error: e.message }; }
  }
  if (seq !== previewSeq) return;
  if (p && !p.ok) { // drop the change that broke it (the last one sent), keep the rest
    const bad = sent[sent.length - 1];
    CHANGES = CHANGES.filter(c => c !== bad);
    await refreshPreview();
    return banner(t("Refused: {err}", { err: problemText(p.error) }));
  }
  PREVIEW = p;
  banner("");
  render(true);
}

const direct = () => CFG.dhcp && CFG.dhcp.mode === "direct";
const gitMode = () => CFG.dhcp && CFG.dhcp.mode === "git" && D && D.can_apply;

// Another switch in the DHCP view: its own data, nothing pending carried over.
async function pickDhcp(name) {
  if (CHANGES.length && !(await ask(t("Discard the pending changes?"), "", t("Discard")))) return;
  CFG.dhcp = CFG.dhcp_targets.find(x => x.switch === name) || CFG.dhcp;
  try { localStorage.setItem("cairn-dhcp", CFG.dhcp.switch); } catch { /* not remembered */ }
  CHANGES = []; PREVIEW = null; D = null;
  render(true);
  slowTick();
}

// Direct mode: the switch runs the commands of the preview, then is read back.
async function applyDhcp() {
  const n = CFG.dhcp.switch;
  const git = gitMode();
  if (!(await (git ? ask(t("Commit to the repository?"), "<p>" + esc(t("One commit under your name with these changes.")) + "</p>", t("Commit"))
    : ask(t("Apply on {name}?", { name: n }), '<pre class="diff">' + esc((PREVIEW.commands || []).join("\n")) + "</pre>", t("Apply"))))) return;
  let r;
  try { r = await post("/api/dhcp/apply", { switch: n, changes: CHANGES }); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.ok) { CHANGES = []; PREVIEW = null; banner(""); }
  else banner(git ? t("Commit refused: {err}", { err: problemText(r.error || "") }) : r.done !== undefined
    ? t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name: n }) + (r.error ? " · " + r.error : "") + (r.verified ? "" : " · " + t("the switch does not read back as expected"))
    : t("Refused: {err}", { err: problemText(r.error || "") }));
  await slowTick();
  if (r.ok) banner(git ? t("Committed ({c}). Your own tool (Ansible, CI) applies it to {name}.", { c: r.commit, name: n })
    : t("Applied on {name}: {n} commands, read back as expected.", { name: n, n: r.commands.length }), "ok");
}

async function resForm(entry, lease) {
  const e = entry || { mac: lease ? macUp(lease) : "", ip: lease ? lease.address : "", name: lease ? String(lease["host-name"] || "").replace(/[^A-Za-z0-9-]/g, "") : "" };
  const fd = await ask(entry ? t("Edit {name}", { name: entry.name }) : t("New reservation"),
    "<label>" + t("MAC address") + '<input name="mac" required pattern="([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}" value="' + esc(e.mac) + '" autocapitalize="characters"></label>' +
    "<label>" + t("IP address") + '<input name="ip" required pattern="(\\d{1,3}\\.){3}\\d{1,3}" value="' + esc(e.ip) + '" placeholder="192.168.1.x"></label>' +
    "<label>" + t("Name") + '<input name="name" required pattern="[A-Za-z0-9]([A-Za-z0-9\\-]*[A-Za-z0-9])?" maxlength="63" value="' + esc(e.name) + '"></label>' +
    (entry ? "" : "<label>" + t("Comment (optional, one line in the file)") + '<input name="comment" maxlength="200"></label>'), entry ? t("Edit") : t("Add"));
  if (!fd) return;
  const v = k => String(fd.get(k) || "").trim();
  return queue(entry ? { op: "set", old_mac: entry.mac, mac: v("mac"), ip: v("ip"), name: v("name") } : { op: "add", mac: v("mac"), ip: v("ip"), name: v("name"), comment: v("comment") });
}

// Server checks (src/dhcp.rs, English) -> page language.
const PROBLEMS = [
  [/^(.*): invalid MAC address (.*)$/, "{who}: invalid MAC address {v}"], [/^(.*): invalid IP address (.*)$/, "{who}: invalid IP address {v}"],
  [/^(.*): (\S+) is outside (.*)$/, "{who}: {v} is outside {net}"], [/^(.*): invalid name .*$/, "{who}: invalid name (letters, digits, dashes)"],
  [/^(.*): same (MAC|IP|name) as (.*)$/, "{who}: same {v} as {net}"],
  [/^(\S+) is already reserved$/, "{who} is already reserved"], [/^(\S+) is not reserved$/, "{who} is not reserved"],
  [/^(\S+) is already a server$/, "{who} is already a server"], [/^(\S+) is not a server$/, "{who} is not a server"],
  // src/infra.rs
  [/^(\S+) would take the network of the management path \((\S+)\)$/, "{v} would take the network of the management path ({net})", 1],
  [/^(\S+) overlaps (\S+), already on the switch$/, "{v} overlaps {net}, already on the switch", 1],
  [/^(\S+) is dynamic \(DHCP client\): it cannot be edited$/, "{v} is dynamic (DHCP client): it cannot be edited", 1],
  [/^(\S+) is not in direct mode: nothing is written to it$/, "{who} is not in direct mode: nothing is written to it"],
];
function problemText(p) {
  for (const [re, k, raw] of PROBLEMS) {
    const m = p.match(re);
    if (m) return raw ? t(k, { v: m[1], net: m[2] }) : t(k, { who: m[1], v: t(m[2] || ""), net: m[3] });
  }
  const m = p.match(/^([^:\s]+): (.*)$/); // "<server>: <problem>" (src/servers.rs)
  return m ? t("{who}: " + m[2], { who: m[1] }) : t(p);
}

// IPv4 without bit operators (they are 32-bit signed in JS).
const ip4 = s => String(s).split(".").reduce((a, o) => a * 256 + +o, 0);
const inNet = (ip, net) => { const [a, l] = String(net).split("/"), k = 2 ** (32 - +l); return Math.floor(ip4(ip) / k) === Math.floor(ip4(a) / k); };
// Option 121 (RFC 3442): prefix length, significant octets, gateway, repeated.
function routes121(v) {
  const b = (String(v || "").replace(/^'|'$/g, "").replace(/^0x/i, "").match(/../g) || []).map(x => parseInt(x, 16)), out = [];
  for (let i = 0; i < b.length;) {
    const l = b[i++], n = Math.ceil(l / 8), net = b.slice(i, i + n).concat([0, 0, 0, 0]).slice(0, 4).join(".");
    i += n;
    out.push(net + "/" + l + " → " + b.slice(i, i + 4).join("."));
    i += 4;
  }
  return out;
}
// Pool ranges "a-b,c" -> [[from, to]] as numbers.
const ranges = v => list(v).map(r => { const [a, b] = r.split("-"); return [ip4(a), ip4(b || a)]; });
const netOf = a => { const [ip, l] = a.split("/"), k = 2 ** (32 - +l), n = Math.floor(ip4(ip) / k) * k; return [24, 16, 8, 0].map(s => Math.floor(n / 2 ** s) % 256).join(".") + "/" + l; };
// What the switch runs for one server, in the shape of a dhcp.yml entry.
function liveServer(s, srv) {
  const addr = (s.addresses || []).find(a => a.interface === srv.interface && !yes(a.disabled));
  const ip = addr ? addr.address.split("/")[0] : "";
  const net = (s.dhcp_networks || []).find(n => ip && inNet(ip, n.address)) || {};
  const pool = (s.pools || []).find(p => p.name === srv["address-pool"]);
  const opts = list(net["dhcp-option"]).map(name => (s.dhcp_options || []).find(x => x.name === name) || { name });
  const r121 = opts.find(o => o.code === "121");
  // RouterOS rogue DHCP alert on the interface: what it saw (MACs, kept for its alert-timeout)
  const al = (s.dhcp_alerts || []).find(a => a.interface === srv.interface && !yes(a.disabled));
  const unknown = al ? list(al["unknown-server"]).map(m => m.toUpperCase()) : [];
  return { opts, unknown, v: { enabled: !yes(srv.disabled), interface: srv.interface, network: net.address || "", range: (pool && pool.ranges) || "",
    gateway: net.gateway || "", dns: list(net["dns-server"]), domain: net.domain || "", ntp: list(net["ntp-server"]), lease_time: srv["lease-time"] || "",
    routes: r121 ? routes121(r121.value).map(r => r.replace(" → ", ",")) : [],
    options: opts.filter(o => o.code && o.code !== "121").map(o => o.code + "," + o.value),
    alert: !!al, alert_valid: al ? list(al["valid-server"]).map(m => m.toUpperCase()) : [] } };
}
const SRV_FIELDS = ["enabled", "interface", "network", "range", "gateway", "dns", "domain", "ntp", "lease_time", "routes", "options", "alert", "alert_valid"];
// One option line "code value" -> "code,value" in RouterOS syntax: a bare
// value becomes text (s'…'), an address '…' (4 bytes), 0x… and numbers stay.
function optionValue(line) {
  const m = line.trim().match(/^(\d+)[\s,]+(.*)$/);
  if (!m) return line.trim();
  const v = m[2].trim();
  return m[1] + "," + (/^(0x[0-9a-f]*|\d+|s?'.*')$/i.test(v) ? v : /^(\d{1,3}\.){3}\d{1,3}$/.test(v) ? "'" + v + "'" : "s'" + v + "'");
}
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// Pie of a pool, used share in the middle. SVG attributes, not inline styles
// (the CSP forbids those): a circle of circumference 100, one dash per part.
function pie(parts, size, label) {
  let off = 0;
  const arcs = parts.filter(([, n]) => n > 0 && size).map(([cls, n]) => {
    const p = Math.min(100, 100 * n / size), a = '<circle class="' + cls + '" r="15.915" cx="18" cy="18" stroke-dasharray="' + p.toFixed(3) + " " +
      (100 - p).toFixed(3) + '" stroke-dashoffset="' + (-off).toFixed(3) + '"></circle>';
    off += p;
    return a;
  });
  const used = size ? Math.round(off) + " %" : "—";
  return '<svg viewBox="-1.5 -1.5 39 39" class="pie" role="img" aria-label="' + esc(label) + '"><g transform="rotate(-90 18 18)"><circle class="track" r="15.915" cx="18" cy="18"></circle>' +
    arcs.join("") + '</g><text x="18" y="18">' + esc(used) + "</text></svg>";
}

// Every DHCP server: what the router runs, next to what dhcp.yml asks for.
function renderServers() {
  const r = rawRouter(), file = D.servers || [], cards = [];
  const names = [...new Set((r.dhcp_servers || []).map(x => x.name).concat(file.map(f => f.name)))];
  for (const name of names) {
    const srv = (r.dhcp_servers || []).find(x => x.name === name), f = file.find(x => x.name === name);
    const live = srv && liveServer(r, srv), v = live ? live.v : f;
    const vl = (r.vlan_ifaces || []).find(x => x.name === v.interface);
    const rg = ranges(v.range), size = rg.reduce((n, [a, b]) => n + b - a + 1, 0);
    const inPool = x => rg.some(([a, b]) => ip4(x) >= a && ip4(x) <= b);
    const mine = (r.leases || []).filter(l => l.server === name);
    const leased = mine.filter(l => yes(l.dynamic) && l.status === "bound").length;
    const stat = mine.filter(l => !yes(l.dynamic)), resIn = stat.filter(l => inPool(l.address)).length, resOut = stat.length - resIn;
    const free = Math.max(0, size - leased - resIn);
    const extra = live ? live.opts.filter(o => o.code !== "121").map(o => o.name + (o.code ? " (" + o.code + ") = " + o.value : "")) : [];
    const gives = [v.gateway && t("gateway {gw}", { gw: v.gateway }), v.dns.length && "DNS " + v.dns.join(", "), v.domain && t("domain {d}", { d: v.domain }),
      v.ntp.length && "NTP " + v.ntp.join(", "), v.lease_time && t("lease {d}", { d: v.lease_time })].filter(Boolean)
      .concat(v.routes.map(x => t("route {r}", { r: x.replace(",", " → ") })), extra,
        v.alert ? [t("watches for other DHCP servers") + (v.alert_valid.length ? " (" + t("expected: {m}", { m: v.alert_valid.join(", ") }) + ")" : "")] : []);
    const rogue = live ? live.unknown : [];
    const st = !srv ? ["warn", t("not on the switch")] : yes(srv.invalid) ? ["bad", t("invalid")] : v.enabled ? ["ok", t("on")] : ["warn", t("off")];
    const diff = f && live ? SRV_FIELDS.filter(k => !same(f[k], live.v[k])) : [];
    const lg = (cls, n, label) => '<span class="legend-item"><span class="sw ' + cls + '"></span>' + esc(label) + " <b>" + n + "</b></span>";
    cards.push('<article class="card srv ' + st[0] + '"><div class="card-head"><h3>' + esc(name) + ' <span class="sub">' + esc(CFG.dhcp.switch) + "</span></h3>" +
      '<span class="pill ' + st[0] + '">' + esc(st[1]) + "</span>" +
      (rogue.length ? '<span class="pill bad">' + esc(t("other DHCP server")) + "</span>" : "") +
      (diff.length ? '<span class="pill warn" title="' + esc(diff.join(", ")) + '">' + esc(t("differs from the file")) + "</span>" : "") + "</div>" +
      '<p class="sub">' + esc([v.interface + (vl ? " · " + t("VLAN {id}", { id: vl["vlan-id"] }) : ""), v.network, v.range].filter(Boolean).join(" · ")) + "</p>" +
      '<div class="pool">' + pie([["leased", leased], ["reserved", resIn]], size, t("{a} leased, {b} reserved, {c} free of {n}", { a: leased, b: resIn, c: free, n: size })) +
      '<div class="legend">' + lg("leased", leased, t("leased")) + lg("reserved", resIn, t("reserved")) + lg("free", free, t("free")) +
      '<span class="legend-item sub">' + esc(t("of {n} in the range", { n: size })) + "</span></div></div>" +
      (resOut ? '<p class="sub">' + esc(tp(resOut, "+ {n} reservation outside the range", "+ {n} reservations outside the range")) + "</p>" : "") +
      (rogue.length ? '<p class="banner bad-note">' + esc(tp(rogue.length, "Another DHCP server answers on {iface}: {macs}. Two servers on one network hand out conflicting addresses.",
        "{n} other DHCP servers answer on {iface}: {macs}. Two servers on one network hand out conflicting addresses.", { iface: v.interface, macs: rogue.join(", ") })) + "</p>" : "") +
      '<ul class="gives">' + gives.map(g => "<li>" + esc(g) + "</li>").join("") + "</ul>" +
      (diff.length ? '<p class="sub">' + esc(t("The file asks for another value ({what}): your tool sets it back at its next run.", { what: diff.join(", ") })) + "</p>" : "") +
      '<div class="card-head foot"><span class="sub">' + esc(t("Managed by {who}", { who: direct() ? "Cairn Net" : f ? t("the file") : t("hand (RouterOS)") })) + "</span>" +
      (f ? '<span class="actions"><button data-act="srv-toggle" data-name="' + esc(name) + '">' + esc(f.enabled ? t("Turn off") : t("Turn on")) +
        '</button><button data-act="srv-edit" data-name="' + esc(name) + '">' + esc(t("Edit")) + "</button>" +
        (name === CFG.dhcp.server ? "" : '<button class="danger" data-act="srv-del" data-name="' + esc(name) + '">' + esc(t("Delete")) + "</button>") + "</span>" : "") + "</div></article>");
  }
  // an alert on an interface without a server of this switch: shown on its own
  const own = new Set((r.dhcp_servers || []).map(x => x.interface));
  const loose = (r.dhcp_alerts || []).filter(a => !own.has(a.interface) && !yes(a.disabled) && list(a["unknown-server"]).length).map(a =>
    '<p class="banner bad-note">' + esc(t("Another DHCP server answers on {iface}: {macs}.", { iface: a.interface, macs: list(a["unknown-server"]).join(", ") })) + "</p>");
  $("dhcp-srv").innerHTML = loose.join("") + (cards.join("") || '<p class="sub">' + t("No DHCP server on the switches.") + "</p>");
}

// The router's interfaces that have an address: where a server can listen.
function ifaces() {
  const r = rawRouter();
  return (r.addresses || []).filter(a => !yes(a.disabled)).map(a => {
    const vl = (r.vlan_ifaces || []).find(x => x.name === a.interface);
    return { name: a.interface, ip: a.address.split("/")[0], net: netOf(a.address), label: a.interface + (vl ? " · VLAN " + vl["vlan-id"] : "") + " · " + a.address };
  });
}
const OTHER_DHCP_WARN = "Another DHCP server may already serve this network: turn it off first, never two DHCP servers on one network.";
async function srvForm(f) {
  const v = f || { name: "", enabled: true, interface: "", network: "", range: "", gateway: "", dns: [], domain: "", ntp: [], lease_time: "1d", routes: [], options: [], alert: true, alert_valid: [] };
  const ifs = ifaces(), opts = ifs.some(i => i.name === v.interface) || !v.interface ? ifs : ifs.concat([{ name: v.interface, label: v.interface }]);
  const inp = (k, label, val, more = "") => "<label>" + esc(t(label)) + '<input name="' + k + '" value="' + esc(val) + '"' + more + "></label>";
  const lan = f && f.name === CFG.dhcp.server;
  const fd = await ask(f ? t("Edit server {name}", { name: f.name }) : t("New DHCP server"),
    // the server named in the configuration keeps its name
    inp("name", "Name", v.name, ' required maxlength="63" pattern="[A-Za-z0-9]([A-Za-z0-9\\-]*[A-Za-z0-9])?"' + (lan ? " readonly" : "")) +
    '<label class="check"><input type="checkbox" name="enabled"' + (v.enabled ? " checked" : "") + ">" + esc(t("Server on")) + "</label>" +
    (lan ? '<p class="banner warn-note">' + esc(t(OTHER_DHCP_WARN)) + "</p>" : "") +
    "<label>" + esc(t("Interface")) + '<select name="interface" required>' + (v.interface ? "" : '<option value="">' + esc(t("Choose…")) + "</option>") +
    opts.map(i => '<option value="' + esc(i.name) + '"' + (i.name === v.interface ? " selected" : "") + ">" + esc(i.label) + "</option>").join("") + "</select></label>" +
    '<div class="cols2">' + inp("network", "Network", v.network, ' required placeholder="192.168.30.0/24"') + inp("gateway", "Gateway", v.gateway, " required") +
    inp("range", "Range", v.range, ' required placeholder="192.168.30.100-192.168.30.199"') + inp("lease_time", "Lease time", v.lease_time, ' required placeholder="1d, 12h, 30m"') +
    inp("dns", "DNS servers (comma separated)", v.dns.join(", ")) + inp("ntp", "NTP servers (comma separated)", v.ntp.join(", ")) + "</div>" +
    inp("domain", "Domain", v.domain) +
    '<label class="check"><input type="checkbox" name="alert"' + (v.alert ? " checked" : "") + ">" + esc(t("Watch for other DHCP servers on this interface")) + "</label>" +
    inp("alert_valid", "Other DHCP servers that are expected (MAC addresses, comma separated)", (v.alert_valid || []).join(", ")) +
    '<p class="sub">' + esc(t("The switch listens for DHCP answers and sends a discovery once a minute; any other server shows on this card.")) + "</p>" +
    "<label>" + esc(t("Routes (option 121), one per line: network gateway")) + '<textarea name="routes" rows="4" placeholder="0.0.0.0/0 192.168.30.1">' +
    esc(v.routes.map(x => x.replace(",", " ")).join("\n")) + "</textarea></label>" +
    '<p class="sub">' + esc(t("A client that gets routes ignores the gateway: put the default route (0.0.0.0/0) in them.")) + "</p>" +
    "<label>" + esc(t("Other DHCP options, one per line: code value")) + '<textarea name="options" rows="3" placeholder="66 tftp.example">' +
    esc((v.options || []).map(x => x.replace(",", " ")).join("\n")) + "</textarea></label>" +
    '<p class="sub">' + esc(t("e.g. 66 TFTP server, 67 boot file, 150 TFTP address, 252 proxy (WPAD) URL. A plain value is sent as text, an address as 4 bytes; 0x… is raw hex.")) + "</p>" +
    (f ? "" : "<label>" + esc(t("Comment (optional, one line in the file)")) + '<input name="comment" maxlength="200"></label>'),
    f ? t("Edit") : t("Add"), dlg => {
      // picking an interface fills the network and the gateway with its own
      dlg.querySelector('[name="interface"]').addEventListener("change", e => {
        const i = ifs.find(x => x.name === e.target.value);
        if (!i) return;
        dlg.querySelector('[name="network"]').value = i.net;
        dlg.querySelector('[name="gateway"]').value = i.ip;
      });
    });
  if (!fd) return;
  const g = k => String(fd.get(k) || "").trim(), many = k => g(k).split(/[\s,;]+/).filter(Boolean);
  const server = { name: g("name") || (f && f.name), enabled: fd.get("enabled") === "on", interface: g("interface"), network: g("network"), range: g("range").replace(/\s+/g, ""),
    gateway: g("gateway"), dns: many("dns"), domain: g("domain"), ntp: many("ntp"), lease_time: g("lease_time"),
    routes: g("routes").split("\n").map(l => l.trim().split(/[\s,→>]+/).filter(Boolean)).filter(p => p.length).map(p => p.join(",")),
    options: g("options").split("\n").filter(l => l.trim()).map(optionValue),
    alert: fd.get("alert") === "on", alert_valid: many("alert_valid").map(m => m.toUpperCase()) };
  return queue(f ? { op: "srv_set", name: f.name, server } : { op: "srv_add", server, comment: g("comment") });
}
// The reservations of its network go with it (they would be outside every server).
async function srvDelete(f) {
  const res = D.entries.filter(e => f.network && inNet(e.ip, f.network));
  if (!(await ask(t("Delete server {name}?", { name: f.name }), "<p>" + esc(t("Its network, pool and options go too, unless another server uses them.")) + "</p>" +
    (res.length ? "<p>" + esc(tp(res.length, "Its {n} reservation is deleted too.", "Its {n} reservations are deleted too.")) + "</p>" : ""), t("Delete")))) return;
  CHANGES.push(...res.map(e => ({ op: "del", mac: e.mac })));
  return queue({ op: "srv_del", name: f.name });
}
async function srvToggle(f) {
  const r = rawRouter(), srv = (r.dhcp_servers || []).find(x => x.name === f.name);
  const rogue = srv ? liveServer(r, srv).unknown : [];
  const warn = rogue.length ? t("Another DHCP server answers on {iface}: {macs}.", { iface: f.interface, macs: rogue.join(", ") }) + " " + t(OTHER_DHCP_WARN)
    : f.name === CFG.dhcp.server ? t(OTHER_DHCP_WARN) : "";
  if (!f.enabled && warn && !(await ask(t("Turn on {name}?", { name: f.name }), "<p>" + esc(warn) + "</p>", t("Turn on")))) return;
  return queue({ op: "srv_set", name: f.name, server: Object.assign({}, f, { enabled: !f.enabled }) });
}

function renderDhcp() {
  const tg = CFG.dhcp_targets || [];
  $("dhcp-tabs").hidden = tg.length < 2;
  $("dhcp-tabs").innerHTML = tg.map(x => '<button class="chip' + (CFG.dhcp && x.switch === CFG.dhcp.switch ? " on" : "") + '" data-act="dhcp-sw" data-name="' + esc(x.switch) + '">' +
    esc(x.switch) + "</button>").join("");
  if (!CFG.dhcp || !D) return;
  // A switch in read-only mode: no edit button anywhere in this view.
  document.body.classList.toggle("ro-dhcp", ((CFG.switches.find(x => x.name === CFG.dhcp.switch) || {}).mode || "read") === "read");
  renderServers();
  const r = rawRouter(), srv = (r.dhcp_servers || []).find(x => x.name === CFG.dhcp.server);
  const pool = srv && (r.pools || []).find(p => p.name === srv["address-pool"]);
  const leases = lanLeases(), stat = leases.filter(l => !yes(l.dynamic)), byMac = Object.fromEntries(stat.map(l => [macUp(l), l]));
  const on = srv && !yes(srv.disabled);
  const nSrv = (r.dhcp_servers || []).length, nOn = (r.dhcp_servers || []).filter(x => !yes(x.disabled)).length;
  const st = r.error || D.router_error ? ["bad", t("{name} unreachable: {err}", { name: CFG.dhcp.switch, err: r.error || D.router_error })]
    : direct() ? ["ok", t("{name} is managed directly: changes are written to the switch through its API. {on} of {n} DHCP servers on.", { name: CFG.dhcp.switch, on: nOn, n: nSrv })]
    : !srv ? ["warn", t("No DHCP server “{srv}” on {name}.", { srv: CFG.dhcp.server, name: CFG.dhcp.switch })]
    : on ? ["ok", t("DHCP server “{srv}” of {name} is on · pool {pool} · leases {time}", { srv: srv.name, name: CFG.dhcp.switch, pool: pool ? pool.ranges : "?", time: srv["lease-time"] })]
    : ["warn", t("DHCP server “{srv}” of {name} is off. Its reservations are already on the switch, ready when it is turned on.", { srv: srv.name, name: CFG.dhcp.switch })];
  $("dhcp-state").className = "verdict " + st[0];
  $("dhcp-state").innerHTML = '<span class="dot ' + st[0] + '"></span>' + esc(st[1]);
  const probs = (PREVIEW ? PREVIEW.problems : D.problems) || [];
  $("dhcp-problems").hidden = !probs.length;
  $("dhcp-problems").innerHTML = probs.map(p => "<li>" + esc(problemText(p)) + "</li>").join("");

  // Pending changes: the dhcp.yml diff, with 2 lines of context.
  $("dhcp-pending").hidden = !PREVIEW;
  if (PREVIEW) {
    const d = diffLines(PREVIEW.old.split("\n"), PREVIEW.new.split("\n"));
    const near = i => d.slice(Math.max(0, i - 2), i + 3).some(y => y[0] !== " ");
    let gap = false;
    const pre = d.map((x, i) => {
      if (!near(i)) { const g = gap ? "" : '<span class="gap">…</span>'; gap = true; return g; }
      gap = false;
      return '<span class="' + (x[0] === "+" ? "add" : x[0] === "-" ? "del" : "") + '">' + esc(x[0] + " " + x[1]) + "</span>";
    }).join("");
    const why = t("Git mode without a repository: set it in Settings.");
    const blocked = (PREVIEW.problems || []).length > 0, cmds = PREVIEW.commands || [];
    $("dhcp-pending").innerHTML = '<div class="card-head"><h3>' + tp(CHANGES.length, "{n} pending change", "{n} pending changes") + "</h3>" +
      '<span class="pill warn">' + t("not applied") + "</span></div>" +
      '<pre class="diff">' + pre + "</pre>" +
      (direct()
        ? "<h3>" + esc(t("Commands for {name}", { name: CFG.dhcp.switch })) + '</h3><pre class="diff">' + (cmds.map(esc).join("\n") || esc(t("none: the switch already runs this"))) + "</pre>" +
          '<div class="toolbar"><button class="primary" data-act="dhcp-apply"' + (blocked || !cmds.length ? " disabled" : "") + ">" + t("Apply") + '</button><button data-act="res-discard">' + t("Discard") + "</button></div>" +
          (blocked ? '<p class="sub">' + esc(t("Fix the problems above first.")) + "</p>" : "")
        : gitMode()
        ? '<p class="sub">' + esc(t("Committing writes the file in the repository under your name; your own tool (Ansible, CI) then applies it to {name}.", { name: CFG.dhcp.switch })) + "</p>" +
          '<div class="toolbar"><button class="primary" data-act="dhcp-apply"' + (blocked ? " disabled" : "") + ">" + t("Commit") + '</button><button data-act="res-discard">' + t("Discard") + "</button></div>" +
          (blocked ? '<p class="sub">' + esc(t("Fix the problems above first.")) + "</p>" : "")
        : '<p class="sub">' + esc(t("Read-only copy of the file: set a repository (git mode) in Settings to change it from here.", { name: CFG.dhcp.switch })) + "</p>" +
          '<div class="toolbar"><button class="primary" disabled title="' + esc(why) + '">' + t("Apply") + '</button><button data-act="res-discard">' + t("Discard") + "</button></div>" +
          '<p class="sub">' + esc(why) + "</p>");
  }

  const inFile = new Set(D.entries.map(e => e.mac));
  const extra = stat.filter(l => !inFile.has(macUp(l)));
  const leaseCell = l => (!l ? "" : esc(l.status === "bound" ? t("bound, expires in {d}", { d: duration(rosSeconds(l["expires-after"])) })
    : !l["last-seen"] || l["last-seen"] === "never" ? t("never seen") : t("seen {d} ago", { d: duration(rosSeconds(l["last-seen"])) })));
  $("dhcp-res").innerHTML = "<thead><tr><th>" + [t("Name"), t("IP address"), t("MAC address"), CFG.dhcp.switch, t("Lease"), t("Note"), ""].join("</th><th>") + "</th></tr></thead><tbody>" +
    D.entries.map(e => {
      const l = byMac[e.mac];
      const s = !l ? ["warn", t("missing")] : l.address !== e.ip ? ["warn", t("has {ip}", { ip: l.address })] : ["ok", t("up to date")];
      return "<tr><td><b>" + esc(e.name) + "</b></td><td>" + esc(e.ip) + '</td><td class="mono">' + esc(e.mac) + '</td><td><span class="pill ' + s[0] + '">' + esc(s[1]) +
        "</span></td><td>" + leaseCell(l) + '</td><td class="wrap sub note">' + esc(e.comment) + '</td><td class="actions"><button data-act="res-edit" data-mac="' + esc(e.mac) + '">' + t("Edit") +
        '</button><button class="danger" data-act="res-del" data-mac="' + esc(e.mac) + '">' + t("Delete") + "</button></td></tr>";
    }).join("") +
    extra.map(l => '<tr class="dim"><td>' + esc(String(l.comment || "").replace(/^ansible:\s*/, "") || "—") + "</td><td>" + esc(l.address) + '</td><td class="mono">' + esc(macUp(l)) +
      '</td><td><span class="pill warn">' + esc(/^ansible/.test(l.comment || "") ? t("not in the file: will be removed") : t("added by hand on the switch")) +
      "</span></td><td>" + leaseCell(l) + "</td><td></td><td></td></tr>").join("") + "</tbody>";

  const dyn = leases.filter(l => yes(l.dynamic));
  $("dhcp-dyn").innerHTML = "<thead><tr><th>" + [t("Host name"), t("IP address"), t("MAC address"), t("Lease"), ""].join("</th><th>") + "</th></tr></thead><tbody>" +
    (dyn.map(l => "<tr><td>" + esc(l["host-name"] || "—") + "</td><td>" + esc(l.address) + '</td><td class="mono">' + esc(macUp(l)) + "</td><td>" + leaseCell(l) +
      '</td><td class="actions">' + (inFile.has(macUp(l)) ? "" : '<button data-act="res-from" data-mac="' + esc(l["mac-address"]) + '">' + t("Reserve") + "</button>") + "</td></tr>").join("") ||
      '<tr><td colspan="5" class="sub">' + (on ? t("No dynamic lease.") : t("None while the server is off.")) + "</td></tr>") + "</tbody>";

  // DNS integration of this switch: where the names go and how the last push went
  const dn = D.dns, st2 = dn && dn.status;
  $("dhcp-names-box").hidden = !CFG.names_note && !dn;
  $("dhcp-names-note").textContent = !dn ? CFG.names_note || ""
    : t("Published to {where} every 2 minutes.", { where: dn.kind === "routeros" ? t("the static DNS of {name}", { name: CFG.dhcp.switch }) : dn.targets.join(", ") }) + " " +
      (!st2 ? t("Not sent yet.") : (st2.results || []).map(r => r.target + " : " + (r.ok ? (r.message === "unchanged" ? t("up to date") : r.message) : t("failed ({err})", { err: r.message }))).join(" · ") +
        " · " + t("last sent {d} ago", { d: duration(Math.round((Date.now() - st2.t) / 1000)) }) +
        (st2.skipped ? " · " + t("switch unreachable at the last pass: the previous names stay") : ""));
  $("dhcp-names").innerHTML = "<thead><tr><th>" + t("Name") + "</th><th>" + t("IP address") + "</th></tr></thead><tbody>" +
    (D.names.map(([ip, n]) => "<tr><td>" + esc(n) + "</td><td>" + esc(ip) + "</td></tr>").join("") ||
      '<tr><td colspan="2" class="sub">' + t("No bound lease with a usable name yet.") + "</td></tr>") + "</tbody>";
}

/* Infrastructure: one switch as WebFig's Quick Set shows it (src/infra.rs).
   Direct mode: identity, DNS, one address, gateway; every change is rolled
   back by the switch itself unless kept within rollback_s seconds. */

let INF = null, INF_UNTIL = 0; // /api/infra of SW, when the switch rolls back (ms)

async function loadInfra() {
  const name = SW;
  if (!name) return;
  try { const v = await get("/api/infra?switch=" + encodeURIComponent(name)); if (name === SW) INF = v; }
  catch (e) { if (name === SW) INF = { name, error: e.message }; }
  if (VIEW === "infra") render(true);
}

const defaultRoutes = v => (v.routes || []).filter(r => r["dst-address"] === "0.0.0.0/0" && !yes(r.dynamic) && (!r["routing-table"] || r["routing-table"] === "main"));
const dnsOf = v => list((v.dns || {}).servers);
const vlanOf = (v, iface) => (v.vlan_ifaces || []).find(x => x.name === iface);

function infraPending() {
  const box = $("infra-pending"), v = INF;
  const left = Math.max(0, Math.ceil((INF_UNTIL - Date.now()) / 1000));
  const waiting = v && !v.error && (left > 0 || v.rollback);
  box.hidden = !waiting;
  if (!waiting) return;
  box.innerHTML = '<div class="card-head"><h3>' + esc(t("Keep these settings on {name}?", { name: v.name })) + '</h3><span class="pill warn">' +
    esc(left > 0 ? t("rollback in {n} s", { n: left }) : t("rollback at {time}", { time: (v.rollback || {})["next-run"] || "?" })) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net read the switch back as asked. Without your confirmation, the switch puts the old settings back by itself.")) + "</p>" +
    '<div class="toolbar"><button class="primary" data-act="infra-keep">' + esc(t("Keep")) + "</button></div>";
}

function renderInfra() {
  $("infra-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="infra/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const v = INF && INF.name === SW ? INF : null;
  infraPending();
  if (!v || v.error) {
    $("infra-cards").innerHTML = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
    for (const id of ["infra-addr", "infra-br", "infra-nat"]) $(id).innerHTML = "";
    return;
  }
  const r = v.resource || {}, rb = v.routerboard || {}, up = v.update || {}, dns = v.dns || {};
  const edit = v.mode === "direct" ? '<button data-act="infra-edit">' + esc(t("Edit")) + "</button>" : '<span class="pill">' + esc(t(MODE_LABEL[v.mode] || v.mode)) + "</span>";
  // the switch's own check if it ran one, else what MikroTik publishes for its channel
  const latest = up["latest-version"] || v.newest, inst = up["installed-version"] || r.version;
  const upd = latest && latest !== inst ? [t("{v} available", { v: latest }), "warn"] : latest ? [t("up to date"), "ok"] : [t("not checked"), ""];
  const fw = rb["current-firmware"] ? rb["current-firmware"] + (rb["upgrade-firmware"] && rb["upgrade-firmware"] !== rb["current-firmware"] ? " → " + rb["upgrade-firmware"] : "") : "—";
  const kv = rows => '<dl class="kv">' + rows.map(([k, val, c]) => "<dt>" + esc(k) + '</dt><dd class="' + (c || "") + '">' + esc(val) + "</dd>").join("") + "</dl>";
  const gws = defaultRoutes(v), dynGw = (v.routes || []).filter(x => yes(x.dynamic) && x["dst-address"] === "0.0.0.0/0");
  const nat = (v.nat || []).filter(x => !yes(x.dynamic));
  $("infra-cards").innerHTML =
    '<article class="card srv"><div class="card-head"><h3>' + esc(t("System")) + "</h3>" + edit + "</div>" + kv([
      [t("Identity"), v.identity || "—"], [t("Model"), rb.model || r["board-name"] || "—"], [t("Architecture"), r["architecture-name"] || "—"],
      [t("RouterOS"), (inst || "—") + (up.channel ? " (" + up.channel + ")" : "")], [t("Update"), upd[0], upd[1]], [t("Firmware"), fw], [t("Up for"), duration(rosSeconds(r.uptime))],
    ]) + "</article>" +
    '<article class="card srv"><div class="card-head"><h3>' + esc(t("Routing and DNS")) + "</h3></div>" + kv([
      ...(gws.length ? gws.map(g => [t("Gateway"), g.gateway + (yes(g.active) ? "" : " · " + t("inactive")), yes(g.active) ? "" : "warn"]) : [[t("Gateway"), t("none")]]),
      ...dynGw.map(g => [t("Gateway (DHCP client)"), g.gateway]),
      [t("DNS servers"), dnsOf(v).join(", ") || "—"], ...(dns["dynamic-servers"] ? [[t("DNS from DHCP"), list(dns["dynamic-servers"]).join(", ")]] : []),
      [t("Answers DNS for the network"), yes(dns["allow-remote-requests"]) ? t("yes") : t("no")],
      [t("NAT"), nat.length ? tp(nat.length, "{n} rule", "{n} rules") : t("none")],
    ]) + "</article>";

  const addrs = (v.addresses || []).slice().sort((a, b) => natural(a.interface, b.interface));
  $("infra-addr").innerHTML = "<thead><tr><th>" + [t("Interface"), t("Address"), t("Network"), t("Origin"), t("State")].join("</th><th>") + "</th></tr></thead><tbody>" +
    (addrs.map(a => {
      const vl = vlanOf(v, a.interface), st = yes(a.disabled) ? ["", t("disabled")] : yes(a.invalid) ? ["warn", t("invalid")] : ["ok", t("active")];
      return "<tr><td><b>" + esc(a.interface) + "</b>" + (vl ? ' <span class="sub">' + esc(t("VLAN {id} on {iface}", { id: vl["vlan-id"], iface: vl.interface })) + "</span>" : "") +
        (a.interface === v.mgmt_iface ? ' <span class="pill">' + esc(t("Cairn Net's path")) + "</span>" : "") + "</td><td>" + esc(a.address) + "</td><td>" + esc(a.network) +
        "</td><td>" + esc(yes(a.dynamic) ? t("DHCP client") : t("static")) + '</td><td><span class="pill ' + st[0] + '">' + esc(st[1]) + "</span></td></tr>";
    }).join("") || '<tr><td colspan="5" class="sub">' + esc(t("No IP address.")) + "</td></tr>") + "</tbody>";

  const ports = v.ports || [], inBridge = new Set(ports.map(p => p.interface));
  const loose = (v.ifaces || []).filter(i => ["ether", "bond"].includes(i.type) && !yes(i.slave) && !inBridge.has(i.name)).map(i => i.name).sort(natural);
  $("infra-br").innerHTML = "<thead><tr><th>" + [t("Bridge"), t("Ports"), t("VLAN filtering"), t("Protocol"), t("Comment")].join("</th><th>") + "</th></tr></thead><tbody>" +
    (v.bridges || []).map(b => {
      const ps = ports.filter(p => p.bridge === b.name).map(p => short(p.interface) + (p.pvid && p.pvid !== "1" ? " (" + p.pvid + ")" : "") + (yes(p.disabled) ? " ✕" : ""));
      return "<tr><td><b>" + esc(b.name) + '</b></td><td class="wrap">' + esc(ps.join(", ") || t("no port")) + "</td><td>" + esc(yes(b["vlan-filtering"]) ? t("yes") : t("no")) +
        "</td><td>" + esc(b["protocol-mode"] || "") + '</td><td class="wrap sub">' + esc(b.comment || "") + "</td></tr>";
    }).join("") +
    (loose.length ? '<tr class="dim"><td>' + esc(t("outside any bridge")) + '</td><td class="wrap">' + esc(loose.map(short).join(", ")) + "</td><td></td><td></td><td></td></tr>" : "") + "</tbody>";

  const match = x => [x.protocol && x.protocol + (x["dst-port"] ? ":" + x["dst-port"] : ""), x["src-address"] && t("from {v}", { v: x["src-address"] }),
    x["dst-address"] && t("to {v}", { v: x["dst-address"] }), x["in-interface"] && t("in {v}", { v: x["in-interface"] }), x["out-interface"] && t("out {v}", { v: x["out-interface"] })].filter(Boolean).join(" · ");
  $("infra-nat").innerHTML = "<thead><tr><th>" + [t("Chain"), t("Action"), t("Match"), t("To"), t("Comment")].join("</th><th>") + "</th></tr></thead><tbody>" +
    ((v.nat || []).map(x => '<tr class="' + (yes(x.disabled) ? "dim" : "") + '"><td>' + esc(x.chain) + "</td><td><b>" + esc(x.action) + "</b>" + (yes(x.disabled) ? " · " + esc(t("disabled")) : "") +
      "</td><td>" + esc(match(x) || t("everything")) + "</td><td>" + esc([x["to-addresses"], x["to-ports"]].filter(Boolean).join(":")) + '</td><td class="wrap sub">' + esc(x.comment || "") + "</td></tr>").join("") ||
      '<tr><td colspan="5" class="sub">' + esc(t("No NAT rule.")) + "</td></tr>") + "</tbody>";
}

const MASKS = Array.from({ length: 32 }, (_, i) => i + 1);
const maskText = l => { const m = 2 ** 32 - 2 ** (32 - l); return [24, 16, 8, 0].map(s => Math.floor(m / 2 ** s) % 256).join("."); };

async function infraForm() {
  const v = INF, gw = (defaultRoutes(v)[0] || {}).gateway || "";
  const addrs = (v.addresses || []).filter(a => !yes(a.dynamic) && a.interface !== v.mgmt_iface);
  const fd = await ask(t("Edit {name}", { name: v.name }),
    "<label>" + esc(t("Identity")) + '<input name="identity" required maxlength="64" value="' + esc(v.identity || "") + '"></label>' +
    "<label>" + esc(t("DNS servers, one per line")) + '<textarea name="dns" rows="3">' + esc(dnsOf(v).join("\n")) + "</textarea></label>" +
    "<label>" + esc(t("Address to change")) + '<select name="addr"><option value="">' + esc(t("none")) + "</option>" +
      addrs.map(a => '<option value="' + esc(a[".id"]) + '">' + esc(a.interface + " · " + a.address) + "</option>").join("") + "</select></label>" +
    '<div class="cols2"><label>' + esc(t("New address")) + '<input name="ip" pattern="(\\d{1,3}\\.){3}\\d{1,3}" placeholder="10.0.0.1"></label>' +
    "<label>" + esc(t("Netmask")) + '<select name="len">' + MASKS.map(l => '<option value="' + l + '"' + (l === 24 ? " selected" : "") + ">/" + l + " (" + maskText(l) + ")</option>").join("") + "</select></label></div>" +
    "<label>" + esc(t("Gateway (default route)")) + '<input name="gateway" pattern="(\\d{1,3}\\.){3}\\d{1,3}" value="' + esc(gw) + '"></label>' +
    '<p class="sub">' + esc(v.mgmt_iface ? t("{iface} is how Cairn Net reaches the switch: it is not offered here.", { iface: v.mgmt_iface }) : "") + " " +
      esc(t("Nothing is written yet: the next step shows the commands.")) + "</p>",
    t("Preview"), d => {
      // picking an address fills its current value
      const sel = d.querySelector('[name="addr"]');
      sel.addEventListener("change", () => {
        const a = addrs.find(x => x[".id"] === sel.value);
        if (!a) return;
        const [ip, l] = a.address.split("/");
        d.querySelector('[name="ip"]').value = ip;
        d.querySelector('[name="len"]').value = l;
      });
    });
  if (!fd) return;
  const g = k => String(fd.get(k) || "").trim();
  const want = {};
  if (g("identity") !== (v.identity || "")) want.identity = g("identity");
  const dns = g("dns").split(/[\s,;]+/).filter(Boolean);
  if (dns.join(",") !== dnsOf(v).join(",")) want.dns = dns;
  if (g("addr") && g("ip")) want.address = { id: g("addr"), address: g("ip") + "/" + g("len") };
  if (g("gateway") && g("gateway") !== gw) want.gateway = g("gateway");
  if (!Object.keys(want).length) return banner(t("Nothing to change."), "ok");
  let p;
  try { p = await post("/api/infra/preview", { switch: v.name, want }); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: problemText(p.error || "") }));
  if (!p.commands.length) return banner(t("Nothing to change."), "ok");
  banner("");
  if (!(await ask(t("Apply on {name}?", { name: v.name }),
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>",
    t("Apply")))) return;
  let r;
  try { r = await post("/api/infra/apply", { switch: v.name, want }); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.verified) { INF_UNTIL = Date.now() + r.rollback_s * 1000; banner(""); }
  else banner(r.pending ? t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name: v.name }) + (r.error ? " · " + problemText(r.error) : "") +
    " · " + t("the switch puts the old settings back within {n} s", { n: r.rollback_s }) : t("Refused: {err}", { err: problemText(r.error || "") }));
  await loadInfra();
}

async function infraKeep() {
  let r;
  try { r = await post("/api/infra/confirm", { switch: INF.name }); } catch (e) { r = { ok: false, error: e.message }; }
  INF_UNTIL = 0;
  banner(r.ok ? t("Kept on {name}.", { name: INF.name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadInfra();
}

// The countdown; at zero the switch has rolled back: read it again.
setInterval(() => {
  if (!INF_UNTIL || VIEW !== "infra") return;
  if (Date.now() >= INF_UNTIL + 3000) { INF_UNTIL = 0; loadInfra(); }
  infraPending();
}, 1000);

/* VLANs */

function renderVlans(sws) {
  const ids = [...new Set(sws.flatMap(s => s.vlans.map(v => v.ids)))].sort(natural);
  const ok = sws.filter(s => !s.error);
  $("vlans").innerHTML = "<thead><tr><th>VLAN</th><th>" + t("Name") + "</th>" + ok.map(s => "<th>" + esc(s.name) + "</th>").join("") + "</tr></thead><tbody>" +
    ids.map(id => {
      const name = ok.map(s => (s.vlans.find(v => v.ids === id && v.comment && !v.dynamic) || {}).comment).find(Boolean) || "";
      return "<tr><td><b>" + esc(id) + "</b></td><td>" + esc(name) + "</td>" + ok.map(s => {
        const v = s.vlans.find(x => x.ids === id);
        if (!v) return "<td></td>";
        return '<td class="wrap">' + [v.untagged.length ? "U " + v.untagged.map(short).join(", ") : "", v.tagged.length ? "T " + v.tagged.map(short).join(", ") : ""].filter(Boolean).map(esc).join("<br>") + "</td>";
      }).join("") + "</tr>";
    }).join("") + "</tbody>";
}

/* ---------- events ---------- */

const THEMES = { "": "auto", light: "light", dark: "dark" };
function setTheme(th) {
  if (!(th in THEMES)) th = "";
  if (th) document.documentElement.dataset.theme = th; else delete document.documentElement.dataset.theme;
  $("theme").textContent = t("Theme: {name}", { name: t(THEMES[th]) });
  try { localStorage.setItem("theme", th); } catch { /* not remembered */ }
}
try { setTheme(localStorage.getItem("theme") || ""); } catch { setTheme(""); }

function setLang(l) {
  LANG = LANGS[l] ? l : "en";
  applyStatic();
  $("lang").textContent = t("Language: {name}", { name: LANGS[LANG].name });
  setTheme(document.documentElement.dataset.theme || "");
  if (VIEW) $("view-title").textContent = t(VIEWS[VIEW]);
  render(true);
}

document.addEventListener("click", e => {
  const b = e.target.closest("button, a");
  if (!b) return;
  const d = b.dataset;
  if (d.view) return show(d.view);
  if (d.goto) {
    if (d.q !== undefined) $("q").value = d.q;
    if (d.sel) SEL = d.sel;
    show(d.goto);
    if (d.sel) document.querySelector('tr[data-row="' + CSS.escape(d.sel) + '"]')?.scrollIntoView({ block: "center" });
    return;
  }
  if (d.port) {
    SEL = SEL === d.port ? null : d.port;
    render(true);
    if (SEL) document.querySelector('tr[data-row="' + CSS.escape(SEL) + '"]')?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    return;
  }
  if (d.act === "dhcp-sw") return pickDhcp(d.name);
  if (INF && !INF.error) switch (d.act) {
    case "infra-edit": return infraForm();
    case "infra-keep": return infraKeep();
  }
  if (D) switch (d.act) {
    case "res-new": return resForm();
    case "res-edit": return resForm(D.entries.find(e => e.mac === d.mac));
    case "res-from": return resForm(null, lanLeases().find(l => l["mac-address"] === d.mac));
    case "res-del": return queue({ op: "del", mac: d.mac });
    case "res-discard": CHANGES = []; return refreshPreview();
    case "dhcp-apply": return applyDhcp();
    case "srv-new": return srvForm();
    case "srv-edit": return srvForm(D.servers.find(x => x.name === d.name));
    case "srv-toggle": return srvToggle(D.servers.find(x => x.name === d.name));
    case "srv-del": return srvDelete(D.servers.find(x => x.name === d.name));
  }
  if (SET) switch (d.act) {
    case "sw-new": return swForm();
    case "sw-edit": return swForm(SET.switches.find(x => x.name === d.name));
    case "sw-del": return swDelete(SET.switches.find(x => x.name === d.name));
    case "dns-copy": return navigator.clipboard.writeText($("dns-line").textContent).then(() => { b.textContent = t("Copied"); });
    case "key-copy": return navigator.clipboard.writeText($("git-key").textContent).then(() => { b.textContent = t("Copied"); });
    case "prep-copy": return navigator.clipboard.writeText($("prep-script").textContent).then(() => { b.textContent = t("Copied"); });
  }
  switch (d.act) {
    case "logout": return post("/api/logout", {}).finally(() => location.reload());
    case "theme": { const k = Object.keys(THEMES); return setTheme(k[(k.indexOf(document.documentElement.dataset.theme || "") + 1) % k.length]); }
    case "lang": {
      const k = Object.keys(LANGS), l = k[(k.indexOf(LANG) + 1) % k.length];
      try { localStorage.setItem("cairn-lang", l); } catch { /* not remembered */ }
      return setLang(l);
    }
  }
});
$("q").addEventListener("input", () => render(true));
$("prep-ip").addEventListener("input", renderPrep);
window.addEventListener("hashchange", () => show(location.hash.slice(1)));
document.addEventListener("pointerdown", () => { animUntil = Math.max(animUntil, Date.now() + 500); });

/* ---------- start ---------- */

// Built-in sign-in: /api/config answers 401 until signed in.
async function signIn() {
  let err = ""; // shown in the form: the page banner sits behind the modal
  for (;;) {
    const fd = await ask(t("Sign-in"), (err ? '<p class="banner">' + esc(err) + "</p>" : "") + "<label>" + t("Username") + '<input name="user" required autocomplete="username" autocapitalize="off"></label>' +
      "<label>" + t("Password") + '<input type="password" name="password" required autocomplete="current-password"></label>', t("Sign in"));
    if (!fd) continue;
    try {
      await post("/api/login", { user: fd.get("user").trim(), password: fd.get("password") });
      return location.reload();
    } catch (e) { err = t(e.message).replace(/^too many attempts: try again in (\d+) s$/, (_, n) => t("Too many attempts: try again in {n} s", { n })); }
  }
}

$("meta").textContent = t("Loading…");
setLang(LANG);
get("/api/config").then(cfg => {
  CFG = cfg;
  $("logout").hidden = !CFG.login;
  document.title = CFG.title + " · Cairn Net";
  $("title").textContent = CFG.title;
  $("subtitle").textContent = CFG.subtitle || "";
  $("user").textContent = CFG.user;
  setLang(pickLang(CFG.lang));
  let last = null;
  try { last = localStorage.getItem("cairn-dhcp"); } catch { /* none */ }
  const tg = CFG.dhcp_targets || [];
  CFG.dhcp = tg.find(x => x.switch === last) || tg[0] || null;
  document.querySelector('#nav [data-view="dhcp"]').hidden = !CFG.dhcp;
  show(location.hash.slice(1));
  Promise.all([slowTick(), fastTick()]).then(() => setTimeout(fastTick, 1000)); // second read: first rates
  setInterval(fastTick, FAST_MS);
  setInterval(slowTick, SLOW_MS);
}).catch(e => e.status === 401 ? signIn() : banner(t("Unreadable configuration: {err}", { err: e.message })));
