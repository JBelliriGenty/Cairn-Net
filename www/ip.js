// cairn-net: IP and routing view (src/ip.rs), loaded after panel.js, uses its helpers.
// One switch at a time: addresses, routes and routing tables, ARP, pools,
// DHCP client, IPv6 and the IP settings. Direct mode edits one menu at a
// time like the firewall page: rows are changed in the page, the preview
// shows the commands and the rollback, the switch rolls back by itself
// unless the change is kept (same rollback as the Infrastructure page).
"use strict";

let IPV = null, IPE = null, IP_UNTIL = 0; // /api/ip of SW; the menu being edited {menu, rows}; when the switch rolls back (ms)

async function loadIp() {
  const name = SW;
  if (!name) return;
  try { const v = await get("/api/ip?switch=" + encodeURIComponent(name)); if (name === SW) IPV = v; }
  catch (e) { if (name === SW) IPV = { name, error: e.message }; }
  if (VIEW === "ip") render(true);
}

const YN = ["yes", "no"];
const ipRows = (v, m) => ((v.menus || {})[m] || []);
const ipStatic = (v, m) => ipRows(v, m).filter(x => !yes(x.dynamic)).map(x => Object.assign({}, x));
const ipWaiting = v => !!(v.rollback || IP_UNTIL > Date.now());
const flag = v => v == null || v === "" ? "" : yes(v) ? t("yes") : v === "false" || v === "no" ? t("no") : String(v);
const origin = x => yes(x.dynamic) ? t("dynamic") : t("static");
const pill = (txt, cls = "") => ' <span class="pill ' + cls + '">' + esc(txt) + "</span>";

// Columns shown, and the fields of the edit form: [name, label, kind] with
// kind "text", "num", a list of choices (select), or a datalist id ("@id").
const IP_MENUS = {
  address: {
    title: "IPv4 addresses", add: "Add an address",
    cols: [["Address", x => "<b>" + esc(x.address) + "</b>"], ["Network", x => esc(x.network)], ["Interface", x => esc(x.interface)], ["Origin", origin],
      ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["address", "Address (a.b.c.d/len)", "text", "10.0.0.1/24"], ["interface", "Interface", "@ip-ifaces"], ["comment", "Comment", "text"]],
  },
  route: {
    title: "IPv4 routes", add: "Add a route",
    cols: [["Destination", x => "<b>" + esc(x["dst-address"]) + "</b>"], ["Gateway", x => esc(x.gateway) + (x["immediate-gw"] && x["immediate-gw"] !== x.gateway ? '<br><span class="sub">' + esc(x["immediate-gw"]) + "</span>" : "")],
      ["Distance", x => esc(x.distance)], ["Table", x => esc(x["routing-table"] || "main")], ["Check", x => esc(x["check-gateway"] || "")],
      ["State", x => yes(x.active) ? pill(t("active"), "ok") : yes(x.disabled) ? "" : pill(t("inactive"), "warn")], ["Origin", x => yes(x.connect) ? t("connected") : origin(x)],
      ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["dst-address", "Destination (a.b.c.d/len)", "text", "0.0.0.0/0"], ["gateway", "Gateway (address or interface)", "text", "192.168.1.1"],
      ["distance", "Distance", "num"], ["routing-table", "Routing table", "@ip-tables"], ["check-gateway", "Gateway check", ["", "ping", "arp", "bfd"]], ["comment", "Comment", "text"]],
  },
  table: {
    title: "Routing tables", add: "Add a table",
    cols: [["Name", x => "<b>" + esc(x.name) + "</b>"], ["FIB", x => flag(x.fib) || (x.fib === "" ? t("yes") : "")], ["Origin", origin], ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["name", "Name", "text"], ["fib", "Used for forwarding (FIB)", ["", "yes"]], ["comment", "Comment", "text"]],
  },
  arp: {
    title: "ARP", add: "Add a static entry",
    cols: [["Address", x => "<b>" + esc(x.address) + "</b>"], ["MAC", x => '<span class="mono">' + esc(x["mac-address"] || "") + "</span>"], ["Interface", x => esc(x.interface)],
      ["Status", x => esc(x.status || (yes(x.complete) ? t("complete") : ""))], ["Origin", origin], ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["address", "Address", "text"], ["mac-address", "MAC address", "text", "AA:BB:CC:DD:EE:FF"], ["interface", "Interface", "@ip-ifaces"],
      ["published", "Published (proxy ARP for this address)", YN], ["comment", "Comment", "text"]],
  },
  pool: {
    title: "IP pools", add: "Add a pool", dynamicless: true,
    cols: [["Name", x => "<b>" + esc(x.name) + "</b>"], ["Ranges", x => '<span class="mono">' + esc(list(x.ranges).join(", ")) + "</span>"], ["Next pool", x => esc(x["next-pool"] || "")],
      ["Used", x => x.total ? esc(num(x.used) + " / " + num(x.total)) : ""], ["Used by", x => ipPoolUsers(IPV, x.name).map(esc).join("<br>")],
      ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["name", "Name", "text"], ["ranges", "Ranges (a-b, comma separated)", "text", "10.0.0.100-10.0.0.199"], ["next-pool", "Next pool", "@ip-pools"], ["comment", "Comment", "text"]],
  },
  "dhcp-client": {
    title: "DHCP clients", add: "Add a DHCP client",
    cols: [["Interface", x => "<b>" + esc(x.interface) + "</b>"], ["Status", x => esc(x.status || "")], ["Address", x => esc(x.address || "")], ["Gateway", x => esc(x.gateway || "")],
      ["Default route", x => esc(flag(x["add-default-route"])) + (x["default-route-distance"] ? " (" + esc(x["default-route-distance"]) + ")" : "")], ["Peer DNS", x => esc(flag(x["use-peer-dns"]))],
      ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["interface", "Interface", "@ip-ifaces"], ["add-default-route", "Add a default route", ["yes", "no", "special-classless"]], ["default-route-distance", "Default route distance", "num"],
      ["use-peer-dns", "Use the server's DNS", YN], ["use-peer-ntp", "Use the server's NTP", YN], ["comment", "Comment", "text"]],
  },
  "ipv6-address": {
    title: "IPv6 addresses", add: "Add an address",
    cols: [["Address", x => "<b>" + esc(x.address) + "</b>"], ["Interface", x => esc(x.interface)], ["From pool", x => esc(x["from-pool"] || "")], ["Advertise", x => esc(flag(x.advertise))],
      ["Origin", x => yes(x["link-local"]) ? t("link-local") : origin(x)], ["Comment", x => '<span class="sub">' + esc(x.comment || "") + "</span>"]],
    fields: [["address", "Address (prefix/len)", "text", "2001:db8::1/64"], ["from-pool", "From pool", "text"], ["interface", "Interface", "@ip-ifaces"],
      ["advertise", "Advertise (RA)", YN], ["eui-64", "EUI-64", YN], ["no-dad", "No duplicate address detection", YN], ["comment", "Comment", "text"]],
  },
  "ipv6-route": {
    title: "IPv6 routes", add: "Add a route",
    cols: [["Destination", x => "<b>" + esc(x["dst-address"]) + "</b>"], ["Gateway", x => esc(x.gateway)], ["Distance", x => esc(x.distance)], ["Table", x => esc(x["routing-table"] || "main")],
      ["State", x => yes(x.active) ? pill(t("active"), "ok") : yes(x.disabled) ? "" : pill(t("inactive"), "warn")], ["Origin", x => yes(x.connect) ? t("connected") : origin(x)]],
    fields: [["dst-address", "Destination (prefix/len)", "text", "::/0"], ["gateway", "Gateway (address or interface)", "text"], ["distance", "Distance", "num"],
      ["routing-table", "Routing table", "@ip-tables"], ["check-gateway", "Gateway check", ["", "ping", "arp", "bfd"]], ["comment", "Comment", "text"]],
  },
  nd: {
    title: "IPv6 neighbour discovery", add: "Add an interface",
    cols: [["Interface", x => "<b>" + esc(x.interface) + "</b>" + (yes(x.default) ? pill(t("default")) : "")], ["RA interval", x => esc(x["ra-interval"] || "")], ["RA lifetime", x => esc(x["ra-lifetime"] || "")],
      ["DNS", x => esc(flag(x["advertise-dns"]))], ["Managed", x => esc(flag(x["managed-address-configuration"]))], ["Other", x => esc(flag(x["other-configuration"]))]],
    fields: [["interface", "Interface", "@ip-ifaces"], ["ra-interval", "RA interval", "text", "3m20s-10m"], ["ra-lifetime", "RA lifetime", "text", "30m"],
      ["advertise-dns", "Advertise DNS", YN], ["advertise-mac-address", "Advertise MAC address", YN], ["managed-address-configuration", "Managed (DHCPv6 addresses)", YN],
      ["other-configuration", "Other configuration (DHCPv6)", YN]],
  },
};
const IP_SETTINGS = {
  "ip-settings": { title: "IPv4 settings", fields: [["ip-forward", "Routing (ip-forward)", YN], ["rp-filter", "Reverse path filter", ["no", "loose", "strict"]],
    ["send-redirects", "Send redirects", YN], ["accept-redirects", "Accept redirects", YN], ["secure-redirects", "Secure redirects", YN], ["accept-source-route", "Accept source route", YN],
    ["allow-fast-path", "Fast path", YN], ["tcp-syncookies", "TCP SYN cookies", YN], ["arp-timeout", "ARP timeout", "text"], ["icmp-rate-limit", "ICMP rate limit", "num"]] },
  "ipv6-settings": { title: "IPv6 settings", fields: [["disable-ipv6", "IPv6 off", YN], ["forward", "Routing (forward)", YN],
    ["accept-redirects", "Accept redirects", ["yes", "no", "yes-if-forwarding-disabled"]], ["accept-router-advertisements", "Accept router advertisements", ["yes", "no", "yes-if-forwarding-disabled"]],
    ["max-neighbor-entries", "Neighbour entries (max)", "num"]] },
};

function ipPoolUsers(v, name) {
  if (!v) return [];
  return (v.dhcp_servers || []).filter(d => d["address-pool"] === name).map(d => t("DHCP server {name}", { name: d.name }))
    .concat(ipRows(v, "pool").filter(p => p["next-pool"] === name).map(p => t("pool {name}", { name: p.name })));
}

// Rows the page never offers to edit (Cairn Net's path), with why.
function ipLocked(v, m, x) {
  if (m === "address" && x.interface === v.mgmt_iface) return String(x.address).split("/")[0] === v.switch_ip ? "move" : t("Cairn Net's path");
  if (m === "dhcp-client" && x.interface === v.mgmt_iface) return t("Cairn Net's path");
  if (m === "arp" && (x.address === v.app || x.address === v.switch_ip)) return t("Cairn Net's path");
  if (m === "pool" && (v.dhcp_servers || []).some(d => d["address-pool"] === x.name)) return t("DHCP page");
  return "";
}

function ipPending() {
  const box = $("ip-pending"), v = IPV;
  const left = Math.max(0, Math.ceil((IP_UNTIL - Date.now()) / 1000));
  const waiting = v && !v.error && (left > 0 || v.rollback);
  box.hidden = !waiting;
  if (!waiting) return;
  box.innerHTML = '<div class="card-head"><h3>' + esc(t("Keep these settings on {name}?", { name: v.name })) + '</h3><span class="pill warn">' +
    esc(left > 0 ? t("rollback in {n} s", { n: left }) : t("rollback at {time}", { time: (v.rollback || {})["next-run"] || "?" })) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net read the switch back as asked. Without your confirmation, the switch puts the old settings back by itself.")) + "</p>" +
    '<div class="toolbar"><button class="primary" data-act="ip-keep">' + esc(t("Keep")) + "</button></div>";
}

const ipBtn = (act, label, extra = "", cls = "") => '<button class="' + cls + '" data-act="' + act + '"' + extra + ">" + esc(t(label)) + "</button>";

function ipSection(v, m) {
  const M = IP_MENUS[m], editing = IPE && IPE.menu === m;
  const rows = editing ? IPE.rows : ipRows(v, m).slice().sort((a, b) => (yes(a.dynamic) - yes(b.dynamic)));
  if (!editing && !rows.length && v.mode !== "direct") return "";
  const acts = editing ? ipBtn("ip-add", M.add) + ipBtn("ip-preview", "Preview", "", "primary") + ipBtn("ip-cancel", "Cancel")
    : v.mode === "direct" && !IPE && !ipWaiting(v) ? ipBtn("ip-edit", "Edit", ' data-menu="' + esc(m) + '"') : "";
  const n = M.cols.length + (editing ? 1 : 0);
  return '<div class="card-head"><h2>' + esc(t(M.title)) + "</h2>" + (acts ? '<div class="toolbar">' + acts + "</div>" : "") + "</div>" +
    '<div class="card table-wrap"><table><thead><tr><th>' + M.cols.map(c => esc(t(c[0]))).join("</th><th>") + (editing ? "</th><th>" : "") + "</th></tr></thead><tbody>" +
    (rows.map((x, i) => {
      const off = yes(x.disabled), lock = ipLocked(v, m, x), fresh = editing && !x[".id"];
      const tags = (off ? pill(t("disabled")) : "") + (yes(x.invalid) ? pill(t("invalid"), "warn") : "") + (fresh ? pill(t("new"), "ok") : "") + (lock ? pill(lock === "move" ? t("Cairn Net's address") : lock) : "");
      const actions = !editing ? "" : '<td class="actions">' + (lock === "move" ? ipBtn("ip-row", "Edit", ' data-i="' + i + '"') : lock ? "" :
        ipBtn("ip-row", "Edit", ' data-i="' + i + '"') + (M.dynamicless && m === "pool" ? "" : ipBtn("ip-toggle", off ? "Enable" : "Disable", ' data-i="' + i + '"')) +
        ipBtn("ip-del", "Delete", ' data-i="' + i + '"', "danger")) + "</td>";
      return '<tr class="' + (off || yes(x.dynamic) ? "dim" : "") + '">' + M.cols.map((c, j) => '<td class="wrap">' + c[1](x) + (j === 0 ? tags : "") + "</td>").join("") + actions + "</tr>";
    }).join("") || '<tr><td colspan="' + n + '" class="sub">' + esc(t("None.")) + "</td></tr>") + "</tbody></table></div>";
}

function ipSettingsCard(v, m) {
  const S6 = IP_SETTINGS[m], x = ipRows(v, m)[0];
  if (!x) return "";
  const busy = IPE || ipWaiting(v) ? " disabled" : "";
  return '<article class="card srv"><div class="card-head"><h3>' + esc(t(S6.title)) + "</h3>" +
    (v.mode === "direct" ? ipBtn("ip-settings", "Edit", ' data-menu="' + m + '"' + busy) : "") + "</div>" +
    '<dl class="kv">' + S6.fields.filter(f => x[f[0]] != null).map(f => "<dt>" + esc(t(f[1])) + "</dt><dd>" + esc(flag(x[f[0]])) + "</dd>").join("") + "</dl></article>";
}

function renderIp() {
  $("ip-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="ip/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const v = IPV && IPV.name === SW ? IPV : null;
  ipPending();
  if (!v || v.error) {
    $("ip-cards").innerHTML = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
    $("ip-menus").innerHTML = "";
    return;
  }
  const routes = ipRows(v, "route"), def = routes.filter(r => r["dst-address"] === "0.0.0.0/0" && yes(r.active))[0];
  $("ip-cards").innerHTML =
    '<article class="card srv"><div class="card-head"><h3>' + esc(t("Summary")) + '</h3><span class="pill">' + esc(t(MODE_LABEL[v.mode] || v.mode)) + "</span></div>" +
      '<dl class="kv">' + [[t("Addresses"), String(ipRows(v, "address").length)], [t("Routes"), tp(routes.length, "{n} route", "{n} routes") + " · " + t("{n} active", { n: routes.filter(r => yes(r.active)).length })],
        [t("Default route"), def ? def.gateway : t("none")], [t("ARP entries"), String(ipRows(v, "arp").length)]].map(([k, val]) => "<dt>" + esc(k) + "</dt><dd>" + esc(val) + "</dd>").join("") + "</dl>" +
      (v.mode === "direct" ? '<p class="sub">' + esc(t("Cairn Net reaches this switch at {ip} through {iface}: that interface, its route and its ARP entries are not edited here. Its address moves from the Addresses table, Cairn Net follows it.",
        { ip: v.switch_ip || "?", iface: v.mgmt_iface || "?" })) + "</p>" : "") + "</article>" +
    ipSettingsCard(v, "ip-settings") + ipSettingsCard(v, "ipv6-settings");
  // the refresh redraws the tables: keep their horizontal scroll, or the edit buttons jump out of reach
  const sx = [...$("ip-menus").querySelectorAll(".table-wrap")].map(w => w.scrollLeft);
  $("ip-menus").innerHTML = Object.keys(IP_MENUS).map(m => ipSection(v, m)).join("") +
    opts("ip-ifaces", (F && (F.switches.find(s => s.name === v.name) || {}).ifaces || []).map(i => i.name).concat(ipRows(v, "address").map(a => a.interface))) +
    opts("ip-tables", ["main"].concat(ipRows(v, "table").map(x => x.name))) + opts("ip-pools", ipRows(v, "pool").map(x => x.name));
  $("ip-menus").querySelectorAll(".table-wrap").forEach((w, i) => { w.scrollLeft = sx[i] || 0; });
}

function ipField([k, label, kind, ph], val) {
  const v = val == null ? "" : String(val);
  if (Array.isArray(kind)) {
    const cur = kind.includes(v) ? v : yes(v) ? "yes" : v === "false" ? "no" : v;
    const all = kind.includes(cur) ? kind : kind.concat([cur]);
    return "<label>" + esc(t(label)) + '<select name="' + esc(k) + '">' + all.map(o => '<option value="' + esc(o) + '"' + (o === cur ? " selected" : "") + ">" + esc(o === "" ? t("(none)") : o) + "</option>").join("") + "</select></label>";
  }
  return "<label>" + esc(t(label)) + '<input name="' + esc(k) + '" value="' + esc(v) + '" autocomplete="off"' + (kind === "num" ? ' inputmode="numeric"' : "") +
    (kind[0] === "@" ? ' list="' + kind.slice(1) + '"' : "") + (ph ? ' placeholder="' + esc(ph) + '"' : "") + "></label>";
}

/* One row of the menu being edited (i: its index, none: a new row). */
async function ipRowForm(i) {
  const m = IPE.menu, M = IP_MENUS[m], x = i == null ? {} : IPE.rows[i];
  const move = i != null && ipLocked(IPV, m, x) === "move";
  const fields = move ? M.fields.filter(f => f[0] === "address") : M.fields;
  const fd = await ask(t(i == null ? M.add : "Edit"),
    (move ? '<p class="sub">' + esc(t("Cairn Net reaches the switch at this address: only the address changes here, and Cairn Net follows it to the new one.")) + "</p>" : "") +
    fields.map(f => ipField(f, f[0] === "fib" ? (x.fib != null && x.fib !== "false" ? "yes" : "") : x[f[0]])).join("") +
    (move ? "" : '<label class="check"><input type="checkbox" name="disabled"' + (yes(x.disabled) ? " checked" : "") + "> " + esc(t("Disabled")) + "</label>") +
    '<p class="sub">' + esc(t("Nothing is written yet: the next step shows the commands.")) + "</p>", t("OK"));
  if (!fd) return;
  const row = Object.assign({}, x);
  for (const [k] of fields) { const val = String(fd.get(k) || "").trim(); if (val) row[k] = val; else delete row[k]; }
  if (!move) { if (fd.get("disabled")) row.disabled = "true"; else delete row.disabled; }
  if (i == null) IPE.rows.push(row); else IPE.rows[i] = row;
  render(true);
}

async function ipSend(menu, rows) {
  const v = IPV, w = { switch: v.name, menu, rows };
  let p;
  try { p = await post("/api/ip/preview", w); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: problemText(p.error || "") }));
  if (!p.commands.length) { IPE = null; render(true); return banner(t("Nothing to change."), "ok"); }
  banner("");
  if (!(await ask(t("Apply on {name}?", { name: v.name }),
    (p.moves ? '<p class="banner">' + esc(t("Cairn Net reaches the switch at this address. After the change it reaches it at {ip}, reads it back and keeps the change by itself, then saves {ip} in Settings. If it cannot reach it, the switch takes its old address back after {n} s.", { ip: p.moves, n: p.rollback_s })) + "</p>" : "") +
    (p.warnings || []).map(x => '<p class="banner">' + esc(problemText(x)) + "</p>").join("") +
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>",
    t("Apply")))) return;
  if (p.moves) banner(t("Moving {name} to {ip}…", { name: v.name, ip: p.moves }), "ok");
  let r;
  try { r = await post("/api/ip/apply", w); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.moved) { IPE = null; banner(t("{name} now answers at {ip}: kept, and Settings updated.", { name: v.name, ip: r.moved }), "ok"); CFG = await get("/api/config"); }
  else if (r.verified) { IPE = null; IP_UNTIL = Date.now() + r.rollback_s * 1000; banner(""); }
  else if (r.pending) {
    IPE = null; IP_UNTIL = Date.now() + (r.left_s != null ? r.left_s : r.rollback_s) * 1000;
    banner(t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name: v.name }) + (r.error ? " · " + problemText(r.error) : "") +
      " · " + t("the switch puts the old settings back within {n} s", { n: r.rollback_s }));
  } else banner(r.ok ? t("Nothing to change.") : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadIp();
}

async function ipSettingsForm(m) {
  const S6 = IP_SETTINGS[m], x = ipRows(IPV, m)[0] || {};
  const fields = S6.fields.filter(f => x[f[0]] != null);
  const fd = await ask(t(S6.title), fields.map(f => ipField(f, x[f[0]])).join("") +
    '<p class="sub">' + esc(t("Nothing is written yet: the next step shows the commands.")) + "</p>", t("OK"));
  if (!fd) return;
  const row = {};
  for (const [k] of fields) { const val = String(fd.get(k) || "").trim(); if (val) row[k] = val; }
  return ipSend(m, [row]);
}

async function ipKeep() {
  let r;
  try { r = await post("/api/infra/confirm", { switch: IPV.name }); } catch (e) { r = { ok: false, error: e.message }; }
  IP_UNTIL = 0;
  banner(r.ok ? t("Kept on {name}.", { name: IPV.name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadIp();
}

function ipAct(d) {
  const i = d.i == null ? null : Number(d.i), rows = IPE && IPE.rows;
  switch (d.act) {
    case "ip-edit": IPE = { menu: d.menu, rows: ipStatic(IPV, d.menu) }; break;
    case "ip-cancel": IPE = null; break;
    case "ip-add": return ipRowForm(null);
    case "ip-row": return ipRowForm(i);
    case "ip-toggle": if (yes(rows[i].disabled)) delete rows[i].disabled; else rows[i].disabled = "true"; break;
    case "ip-del": rows.splice(i, 1); break;
    case "ip-preview": return ipSend(IPE.menu, IPE.rows);
    case "ip-settings": return ipSettingsForm(d.menu);
    case "ip-keep": return ipKeep();
    default: return;
  }
  render(true);
}

// The countdown; at zero the switch has rolled back: read it again.
setInterval(() => {
  if (!IP_UNTIL || VIEW !== "ip") return;
  if (Date.now() >= IP_UNTIL + 3000) { IP_UNTIL = 0; loadIp(); }
  ipPending();
}, 1000);

// Server checks (src/ip.rs) -> page language.
PROBLEMS.push(
  [/^(\S+): Cairn Net reaches the switch through this interface, its DHCP client is not edited from here$/, "{v}: Cairn Net reaches the switch through this interface, its DHCP client is not edited from here", 1],
  [/^(\S+): Cairn Net reaches the switch through this interface, it is not edited from here$/, "{v}: Cairn Net reaches the switch through this interface, it is not edited from here", 1],
  [/^no route would be left to Cairn Net \((\S+)\)$/, "no route would be left to Cairn Net ({v})", 1],
  [/^(\S+): Cairn Net would be sent to a blackhole$/, "{v}: Cairn Net would be sent to a blackhole", 1],
  [/^Cairn Net \((\S+)\) is on a network of the switch: .*$/, "Cairn Net ({v}) is on a network of the switch: a route would take its replies elsewhere", 1],
  [/^(\S+): the gateway towards Cairn Net \((\S+)\) is on no network of the switch$/, "{v}: the gateway towards Cairn Net ({net}) is on no network of the switch", 1],
  [/^the switch answers Cairn Net \((\S+)\) through (\S+) after the change$/, "the switch answers Cairn Net ({v}) through {net} after the change", 1],
  [/^(\S+): on Cairn Net's path to the switch, its ARP entry is not edited from here$/, "{v}: on Cairn Net's path to the switch, its ARP entry is not edited from here", 1],
  [/^(\S+): a default route from this DHCP client may change how the switch answers Cairn Net$/, "{v}: a default route from this DHCP client may change how the switch answers Cairn Net", 1],
  [/^(\S+): used by DHCP server (\S+), edited on the DHCP page$/, "{v}: used by DHCP server {net}, edited on the DHCP page", 1],
  [/^(\S+): used by pool (\S+), edited on the DHCP page$/, "{v}: used by pool {net}, edited on the DHCP page", 1],
  [/^(\S+): invalid destination \(a\.b\.c\.d\/len\)$/, "{v}: invalid destination (a.b.c.d/len)", 1],
  [/^(\S+): invalid address \(a\.b\.c\.d\/len\)$/, "{v}: invalid address (a.b.c.d/len)", 1],
  [/^(\S+): line breaks and control characters are not allowed$/, "{v}: line breaks and control characters are not allowed", 1],
);

// only what the shared dictionary does not already say (other pages keep their words)
for (const [k, fr] of Object.entries({
  "IP and routing": "IP et routage",
  "IPv4 addresses": "Adresses IPv4", "IPv4 routes": "Routes IPv4", "Routing tables": "Tables de routage", "ARP": "ARP", "IP pools": "Pools IP", "DHCP clients": "Clients DHCP",
  "IPv6 addresses": "Adresses IPv6", "IPv6 routes": "Routes IPv6", "IPv6 neighbour discovery": "Découverte des voisins IPv6", "IPv4 settings": "Réglages IPv4", "IPv6 settings": "Réglages IPv6",
  "Add an address": "Ajouter une adresse", "Add a route": "Ajouter une route", "Add a table": "Ajouter une table", "Add a static entry": "Ajouter une entrée statique",
  "Add a pool": "Ajouter un pool", "Add a DHCP client": "Ajouter un client DHCP", "Add an interface": "Ajouter une interface",
  "Destination": "Destination", "Gateway": "Passerelle", "Distance": "Distance", "Table": "Table", "Check": "Vérification", "State": "État", "Origin": "Origine",
  "active": "active", "inactive": "inactive", "connected": "connectée", "dynamic": "dynamique", "static": "statique", "link-local": "lien local", "default": "par défaut",
  "Name": "Nom", "FIB": "FIB", "MAC": "MAC", "Status": "État", "complete": "complète", "Ranges": "Plages", "Next pool": "Pool suivant", "Used": "Utilisées", "Used by": "Utilisé par",
  "Default route": "Route par défaut", "Peer DNS": "DNS du serveur", "From pool": "Depuis le pool", "Advertise": "Annonce",
  "RA interval": "Intervalle RA", "RA lifetime": "Durée de vie RA", "DNS": "DNS", "Managed": "Géré", "Other": "Autre",
  "Address (a.b.c.d/len)": "Adresse (a.b.c.d/long.)", "Interface": "Interface", "Comment": "Commentaire", "Destination (a.b.c.d/len)": "Destination (a.b.c.d/long.)",
  "Gateway (address or interface)": "Passerelle (adresse ou interface)", "Routing table": "Table de routage", "Gateway check": "Vérification de la passerelle",
  "Used for forwarding (FIB)": "Utilisée pour l'acheminement (FIB)", "MAC address": "Adresse MAC", "Published (proxy ARP for this address)": "Publiée (proxy ARP pour cette adresse)",
  "Ranges (a-b, comma separated)": "Plages (a-b, séparées par des virgules)", "Add a default route": "Ajouter une route par défaut", "Default route distance": "Distance de la route par défaut",
  "Use the server's DNS": "Utiliser le DNS du serveur", "Use the server's NTP": "Utiliser le NTP du serveur", "Address (prefix/len)": "Adresse (préfixe/long.)",
  "Advertise (RA)": "Annoncer (RA)", "EUI-64": "EUI-64", "No duplicate address detection": "Sans détection d'adresse en double", "Destination (prefix/len)": "Destination (préfixe/long.)",
  "Advertise DNS": "Annoncer le DNS", "Advertise MAC address": "Annoncer l'adresse MAC", "Managed (DHCPv6 addresses)": "Géré (adresses DHCPv6)", "Other configuration (DHCPv6)": "Autre configuration (DHCPv6)",
  "Routing (ip-forward)": "Routage (ip-forward)", "Reverse path filter": "Filtre du chemin de retour", "Send redirects": "Envoyer les redirections", "Accept redirects": "Accepter les redirections",
  "Secure redirects": "Redirections sûres", "Accept source route": "Accepter le routage par la source", "Fast path": "Fast path", "TCP SYN cookies": "Cookies TCP SYN",
  "ARP timeout": "Expiration ARP", "ICMP rate limit": "Limite de débit ICMP", "IPv6 off": "IPv6 désactivé", "Routing (forward)": "Routage (forward)",
  "Accept router advertisements": "Accepter les annonces de routeur", "Neighbour entries (max)": "Entrées de voisins (max.)",
  "DHCP server {name}": "serveur DHCP {name}", "pool {name}": "pool {name}", "Cairn Net's path": "chemin de Cairn Net", "Cairn Net's address": "adresse de Cairn Net", "DHCP page": "page DHCP",
  "Summary": "Synthèse", "Addresses": "Adresses", "Routes": "Routes", "{n} route": "{n} route", "{n} routes": "{n} routes", "{n} active": "{n} actives", "ARP entries": "Entrées ARP", "None.": "Aucune.",
  "(none)": "(aucune)", "yes": "oui", "no": "non",
  "Cairn Net reaches this switch at {ip} through {iface}: that interface, its route and its ARP entries are not edited here. Its address moves from the Addresses table, Cairn Net follows it.":
    "Cairn Net joint ce switch en {ip} par {iface} : cette interface, sa route et ses entrées ARP ne se modifient pas ici. Son adresse se déplace depuis le tableau des adresses, Cairn Net la suit.",
  "Cairn Net reaches the switch at this address: only the address changes here, and Cairn Net follows it to the new one.":
    "Cairn Net joint le switch à cette adresse : seule l'adresse change ici, et Cairn Net la suit vers la nouvelle.",
  "{v}: Cairn Net reaches the switch through this interface, its DHCP client is not edited from here": "{v} : Cairn Net joint le switch par cette interface, son client DHCP ne se modifie pas d'ici",
  "{v}: Cairn Net reaches the switch through this interface, it is not edited from here": "{v} : Cairn Net joint le switch par cette interface, elle ne se modifie pas d'ici",
  "no route would be left to Cairn Net ({v})": "il ne resterait aucune route vers Cairn Net ({v})",
  "{v}: Cairn Net would be sent to a blackhole": "{v} : Cairn Net serait envoyé dans un trou noir",
  "Cairn Net ({v}) is on a network of the switch: a route would take its replies elsewhere": "Cairn Net ({v}) est sur un réseau du switch : une route enverrait ses réponses ailleurs",
  "{v}: the gateway towards Cairn Net ({net}) is on no network of the switch": "{v} : la passerelle vers Cairn Net ({net}) n'est sur aucun réseau du switch",
  "the switch answers Cairn Net ({v}) through {net} after the change": "après le changement, le switch répond à Cairn Net ({v}) par {net}",
  "{v}: on Cairn Net's path to the switch, its ARP entry is not edited from here": "{v} : sur le chemin de Cairn Net vers le switch, son entrée ARP ne se modifie pas d'ici",
  "{v}: a default route from this DHCP client may change how the switch answers Cairn Net": "{v} : une route par défaut de ce client DHCP peut changer la façon dont le switch répond à Cairn Net",
  "{v}: used by DHCP server {net}, edited on the DHCP page": "{v} : utilisé par le serveur DHCP {net}, se modifie sur la page DHCP",
  "{v}: used by pool {net}, edited on the DHCP page": "{v} : utilisé par le pool {net}, se modifie sur la page DHCP",
  "{v}: invalid destination (a.b.c.d/len)": "{v} : destination invalide (a.b.c.d/long.)", "{v}: invalid address (a.b.c.d/len)": "{v} : adresse invalide (a.b.c.d/long.)",
  "{v}: line breaks and control characters are not allowed": "{v} : retours à la ligne et caractères de contrôle interdits",
  "rp-filter strict: Cairn Net's packets come in through another interface than the switch answers them by":
    "rp-filter strict : les paquets de Cairn Net arrivent par une autre interface que celle par laquelle le switch leur répond",
  "rp-filter loose: the switch has no route to Cairn Net": "rp-filter loose : le switch n'a aucune route vers Cairn Net",
  "ip-forward off: the switch stops routing between its networks; devices reached through it may be cut":
    "ip-forward coupé : le switch ne route plus entre ses réseaux ; les appareils joints à travers lui peuvent être coupés",
  "this row is gone from the switch: reload": "cette ligne a disparu du switch : rechargez",
})) if (!(k in DICT.fr)) DICT.fr[k] = fr;
