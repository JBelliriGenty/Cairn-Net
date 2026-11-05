// Interfaces and bridge (src/ifaces.rs): ethernet ports, bonds, bridges,
// bridge ports with their STP state, the bridge VLAN table, VLAN interfaces
// and the host table of one switch; SFP diagnostics in the Ports view.
// Direct mode edits one object at a time: preview, apply, read back; the
// switch rolls anything but a comment back by itself unless it is kept.
"use strict";

let IF = null, IF_UNTIL = 0; // /api/ifaces of SW; when the switch rolls back (ms)

async function loadIfaces() {
  const name = SW;
  if (!name) return;
  try { const v = await get("/api/ifaces?switch=" + encodeURIComponent(name)); if (name === SW) IF = v; }
  catch (e) { if (name === SW) IF = { name, error: e.message }; }
  if (VIEW === "ifaces") render(true);
}

const BOOL = ["yes", "no"];
// [property, label, kind of input, choices]; kinds: text, num, bool, sel, multi (checkboxes)
const IF_FIELDS = {
  ether: [["disabled", "Disabled", "bool"], ["comment", "Comment", "text"], ["mtu", "MTU", "num"], ["l2mtu", "L2MTU", "num"],
    ["auto-negotiation", "Auto-negotiation", "bool"], ["speed", "Speed (without auto-negotiation)", "text", ["10M-baseT-half", "10M-baseT-full", "100M-baseT-half", "100M-baseT-full", "1G-baseT-full", "1G-baseX", "2.5G-baseT", "2.5G-baseX", "5G-baseT", "10G-baseT", "10G-baseSR-LR", "10G-baseCR", "25G-baseSR-LR", "25G-baseCR"]],
    ["advertise", "Advertised speeds (comma list)", "text"], ["rx-flow-control", "Flow control (receive)", "sel", ["off", "on", "auto"]], ["tx-flow-control", "Flow control (send)", "sel", ["off", "on", "auto"]]],
  bond: [["name", "Name", "text"], ["slaves", "Members", "multi"], ["mode", "Mode", "sel", ["802.3ad", "active-backup", "balance-rr", "balance-xor", "balance-tlb", "balance-alb", "broadcast"]],
    ["transmit-hash-policy", "Hash (802.3ad, balance-xor)", "sel", ["layer-2", "layer-2-and-3", "layer-3-and-4", "encap2+3", "encap3+4"]], ["lacp-rate", "LACP rate", "sel", ["30secs", "1sec"]],
    ["min-links", "Minimum links", "num"], ["mtu", "MTU", "num"], ["disabled", "Disabled", "bool"], ["comment", "Comment", "text"]],
  bridge: [["vlan-filtering", "VLAN filtering", "bool"], ["protocol-mode", "Protocol", "sel", ["none", "stp", "rstp", "mstp"]], ["priority", "Priority (0x0000-0xF000)", "text"], ["comment", "Comment", "text"]],
  bport: [["bridge", "Bridge", "sel"], ["interface", "Interface", "sel"], ["pvid", "PVID", "num"], ["frame-types", "Frame types", "sel", ["admit-all", "admit-only-untagged-and-priority-tagged", "admit-only-vlan-tagged"]],
    ["ingress-filtering", "Ingress filtering", "bool"], ["horizon", "Horizon (none or a number)", "text"], ["disabled", "Disabled", "bool"], ["comment", "Comment", "text"]],
  bvlan: [["bridge", "Bridge", "sel"], ["vlan-ids", "VLAN IDs (10 or 10,20 or 100-200)", "text"], ["tagged", "Tagged", "multi"], ["untagged", "Untagged", "multi"], ["disabled", "Disabled", "bool"], ["comment", "Comment", "text"]],
  vlan: [["name", "Name", "text"], ["vlan-id", "VLAN ID", "num"], ["interface", "Interface", "sel"], ["mtu", "MTU", "num"], ["disabled", "Disabled", "bool"], ["comment", "Comment", "text"]],
};
const IF_KIND = { ether: "port", bond: "bond", bridge: "bridge", bport: "bridge port", bvlan: "VLAN row", vlan: "VLAN interface" };
const IF_ROWS = { ether: "ether", bond: "bonds", bridge: "bridges", bport: "bports", bvlan: "bvlans", vlan: "vlans" };

const ifNorm = v => { v = String(v ?? "").trim(); v = v === "yes" ? "true" : v === "no" ? "false" : v;
  if (/^0x[0-9a-f]+$/i.test(v)) return String(parseInt(v, 16)); return v.split(",").map(x => x.trim()).filter(Boolean).sort().join(","); };
const ifCan = v => v.mode === "direct" && !ifWaiting(v);
const ifWaiting = v => !!(v.rollback || IF_UNTIL > Date.now());
const onPath = (v, n) => !!v.mgmt && ((v.mgmt.ifaces || []).includes(n) || (!v.mgmt.port && v.mgmt.bridge && (v.bports || []).some(p => p.bridge === v.mgmt.bridge && p.interface === n)));
const pathPill = (v, n) => onPath(v, n) ? ' <span class="pill">' + esc(t("Cairn Net's path")) + "</span>" : "";
const ifBtn = (act, label, kind, id, cls = "") => '<button class="' + cls + '" data-act="' + act + '" data-kind="' + kind + '"' + (id ? ' data-id="' + esc(id) + '"' : "") + ">" + esc(t(label)) + "</button>";
const acts = (v, kind, x, del = true) => ifCan(v) && !yes(x.dynamic) ? '<td class="actions">' + ifBtn("if-edit", "Edit", kind, x[".id"]) + (del ? ifBtn("if-del", "Delete", kind, x[".id"], "danger") : "") + "</td>" : (v.mode === "direct" ? "<td></td>" : "");
const head = (v, title, kind) => '<div class="card-head"><h2>' + esc(t(title)) + "</h2>" + (kind && ifCan(v) ? '<div class="toolbar">' + ifBtn("if-add", "Add", kind) + "</div>" : "") + "</div>";
const tableHtml = (cols, rows, v, empty) => "<thead><tr><th>" + cols.map(c => esc(t(c))).join("</th><th>") + (v.mode === "direct" ? "</th><th>" : "") + "</th></tr></thead><tbody>" +
  (rows.join("") || '<tr><td colspan="' + (cols.length + 1) + '" class="sub">' + esc(t(empty)) + "</td></tr>") + "</tbody>";
const stateOf = (v, n) => { const i = (v.ifaces || []).find(x => x.name === n) || {}; return yes(i.disabled) ? ["", t("disabled")] : yes(i.running) ? ["ok", t("up")] : ["", t("no link")]; };
const statePill = (v, n) => { const [c, l] = stateOf(v, n); return '<span class="pill ' + c + '">' + esc(l) + "</span>"; };

function ifPending() {
  const box = $("if-pending"), v = IF;
  const left = Math.max(0, Math.ceil((IF_UNTIL - Date.now()) / 1000));
  const waiting = v && !v.error && (left > 0 || v.rollback);
  box.hidden = !waiting;
  if (!waiting) return;
  box.innerHTML = '<div class="card-head"><h3>' + esc(t("Keep these settings on {name}?", { name: v.name })) + '</h3><span class="pill warn">' +
    esc(left > 0 ? t("rollback in {n} s", { n: left }) : t("rollback at {time}", { time: (v.rollback || {})["next-run"] || "?" })) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net read the switch back as asked. Without your confirmation, the switch puts the old settings back by itself.")) + "</p>" +
    '<div class="toolbar"><button class="primary" data-act="if-keep">' + esc(t("Keep")) + "</button></div>";
}

function renderIfaces() {
  $("if-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="ifaces/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const v = IF && IF.name === SW ? IF : null;
  ifPending();
  if (!v || v.error) {
    $("if-body").innerHTML = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
    return;
  }
  const m = v.mgmt || {};
  const mon = n => (v.monitor || []).find(x => x.name === n) || {};
  const path = m.bridge ? t("{ifaces}; packets arrive on {port}", { ifaces: (m.ifaces || []).join(", "), port: m.port || t("an unknown port (every port of {b} is protected)", { b: m.bridge }) }) + (m.vid ? " · " + t("VLAN {id}", { id: m.vid }) : "") : (m.ifaces || []).join(", ");
  const note = '<article class="card"><div class="card-head"><h3>' + esc(t("Interfaces and bridge")) + '</h3><span class="pill">' + esc(t(MODE_LABEL[v.mode] || v.mode)) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net's path ({app}): {path}. A change touching it is refused.", { app: m.app || "?", path: path || "?" })) + " " +
    esc(v.mode === "direct" ? t("Each change is previewed, then read back; anything but a comment is rolled back by the switch after {n} s unless you keep it.", { n: v.rollback_s }) : t("Read only: only a switch in direct mode is edited here.")) + "</p></article>";

  const ports = (v.ether || []).slice().sort((a, b) => natural(a.name, b.name)).map(x => {
    const mo = mon(x.name), fc = [x["rx-flow-control"], x["tx-flow-control"]].filter(Boolean);
    const speed = yes(x["auto-negotiation"]) ? t("auto") + (mo.rate ? " · " + mo.rate : "") : (x.speed || "") + (mo.rate && mo.rate !== x.speed ? " · " + mo.rate : "");
    return '<tr class="' + (yes(x.disabled) ? "dim" : "") + '"><td><b>' + esc(x.name) + "</b>" + pathPill(v, x.name) + '</td><td class="wrap sub">' + esc(x.comment || "") +
      "</td><td>" + statePill(v, x.name) + "</td><td>" + esc([x.mtu, x.l2mtu].filter(Boolean).join(" / ")) + "</td><td>" + esc(speed) + "</td><td>" + esc(fc.length ? "rx " + fc[0] + " · tx " + (fc[1] || "") : "") + "</td>" + acts(v, "ether", x, false) + "</tr>";
  });

  const bonds = (v.bonds || []).map(x => "<tr><td><b>" + esc(x.name) + "</b>" + pathPill(v, x.name) + '</td><td class="wrap">' + esc(list(x.slaves).map(short).join(", ")) + "</td><td>" + esc(x.mode || "") +
    "</td><td>" + esc(x["transmit-hash-policy"] || "") + "</td><td>" + statePill(v, x.name) + '</td><td class="wrap sub">' + esc(x.comment || "") + "</td>" + acts(v, "bond", x) + "</tr>");

  const bridges = (v.bridges || []).map(x => "<tr><td><b>" + esc(x.name) + "</b>" + pathPill(v, x.name) + "</td><td>" + esc(yes(x["vlan-filtering"]) ? t("yes") : t("no")) + "</td><td>" + esc(x["protocol-mode"] || "") +
    "</td><td>" + esc(x.priority || "") + '</td><td class="wrap sub">' + esc(x.comment || "") + "</td>" + acts(v, "bridge", x, false) + "</tr>");

  const bports = (v.bports || []).slice().sort((a, b) => natural(a.bridge, b.bridge) || natural(a.interface, b.interface)).map(x => {
    const stp = [x.role && x.role.replace(/-port$/, ""), x.status !== "in-bridge" && x.status, x.forwarding != null && x.status === "in-bridge" && (yes(x.forwarding) ? t("forwarding") : t("blocked")), yes(x.edge) && "edge"].filter(Boolean);
    return '<tr class="' + (yes(x.disabled) || yes(x.inactive) ? "dim" : "") + '"><td><b>' + esc(x.interface) + "</b>" + pathPill(v, x.interface) + (yes(x.dynamic) ? ' <span class="pill">D</span>' : "") + "</td><td>" + esc(x.bridge) +
      "</td><td>" + esc(x.pvid || "") + '</td><td class="wrap">' + esc(x["frame-types"] || "") + "</td><td>" + esc(yes(x["ingress-filtering"]) ? t("yes") : t("no")) + "</td><td>" + esc(x.horizon || "") +
      '</td><td class="wrap">' + esc(stp.join(" · ")) + "</td>" + acts(v, "bport", x) + "</tr>";
  });

  const bvlans = (v.bvlans || []).slice().sort((a, b) => natural(a.bridge, b.bridge) || natural(a["vlan-ids"], b["vlan-ids"])).map(x => {
    // what RouterOS runs, when it differs from the row (dynamic members, inactive ports)
    const cur = [["T", "tagged"], ["U", "untagged"]].filter(([, k]) => ifNorm(x["current-" + k]) !== ifNorm(x[k])).map(([l, k]) => l + " " + (list(x["current-" + k]).map(short).join(", ") || "—"));
    return '<tr class="' + (yes(x.disabled) ? "dim" : "") + '"><td>' + esc(x.bridge) + "</td><td><b>" + esc(x["vlan-ids"]) + "</b>" + (yes(x.dynamic) ? ' <span class="pill">D</span>' : "") +
      (m.vid && x.bridge === m.bridge && String(x["vlan-ids"]).split(",").some(r => { const [a, b] = r.split("-"); return +a <= m.vid && m.vid <= +(b || a); }) ? ' <span class="pill">' + esc(t("Cairn Net's path")) + "</span>" : "") +
      '</td><td class="wrap">' + esc(list(x.tagged).map(short).join(", ")) + '</td><td class="wrap">' + esc(list(x.untagged).map(short).join(", ")) + '</td><td class="wrap sub">' + esc(cur.join(" · ")) +
      '</td><td class="wrap sub">' + esc(x.comment || "") + "</td>" + acts(v, "bvlan", x) + "</tr>";
  });

  const vlans = (v.vlans || []).slice().sort((a, b) => natural(a.name, b.name)).map(x => "<tr><td><b>" + esc(x.name) + "</b>" + pathPill(v, x.name) + "</td><td>" + esc(x["vlan-id"]) + "</td><td>" + esc(x.interface) +
    "</td><td>" + statePill(v, x.name) + '</td><td class="wrap sub">' + esc(x.comment || "") + "</td>" + acts(v, "vlan", x) + "</tr>");

  const q = (($("if-q") || {}).value || "").trim().toLowerCase();
  const hostsAll = v.hosts || [], hosts = hostsAll.filter(h => !q || [h["mac-address"], h["on-interface"], h.bridge, h.vid].some(x => String(x || "").toLowerCase().includes(q)));
  const flags = h => [yes(h.local) && t("local"), yes(h.dynamic) ? t("learned") : t("static"), yes(h.external) && t("external")].filter(Boolean).join(" · ");
  const hostRows = hosts.slice(0, 300).map(h => '<tr><td class="mono">' + esc(h["mac-address"]) + "</td><td>" + esc(h["on-interface"]) + "</td><td>" + esc(h.vid || "") + "</td><td>" + esc(h.bridge) + "</td><td>" + esc(flags(h)) + "</td></tr>");

  const ro = { mode: "read" }; // tables without the action column
  const focus = document.activeElement && document.activeElement.id === "if-q";
  $("if-body").innerHTML = note +
    "<h2>" + esc(t("Ports")) + '</h2><div class="card table-wrap"><table>' + tableHtml(["Port", "Comment", "State", "MTU / L2MTU", "Speed", "Flow control"], ports, v, "No ethernet port.") + "</table></div>" +
    head(v, "Bonds", "bond") + '<div class="card table-wrap"><table>' + tableHtml(["Bond", "Members", "Mode", "Hash", "State", "Comment"], bonds, v, "No bond.") + "</table></div>" +
    head(v, "Bridges") + '<div class="card table-wrap"><table>' + tableHtml(["Bridge", "VLAN filtering", "Protocol", "Priority", "Comment"], bridges, v, "No bridge.") + "</table></div>" +
    head(v, "Bridge ports", "bport") + '<div class="card table-wrap"><table>' + tableHtml(["Interface", "Bridge", "PVID", "Frame types", "Ingress filtering", "Horizon", "STP"], bports, v, "No bridge port.") + "</table></div>" +
    head(v, "Bridge VLAN table", "bvlan") + '<div class="card table-wrap"><table>' + tableHtml(["Bridge", "VLAN", "Tagged", "Untagged", "Currently", "Comment"], bvlans, v, "No VLAN row.") + "</table></div>" +
    head(v, "VLAN interfaces", "vlan") + '<div class="card table-wrap"><table>' + tableHtml(["Name", "VLAN ID", "On", "State", "Comment"], vlans, v, "No VLAN interface.") + "</table></div>" +
    "<h2>" + esc(t("Host table")) + ' <span class="sub">' + esc(hosts.length < hostsAll.length || hosts.length > 300 ? t("{n} of {total}", { n: Math.min(hosts.length, 300), total: hostsAll.length }) : String(hostsAll.length)) + "</span></h2>" +
    '<div class="toolbar"><input type="search" id="if-q" placeholder="' + esc(t("Filter: MAC, port, VLAN…")) + '" value="' + esc(q) + '" autocomplete="off"></div>' +
    '<div class="card table-wrap"><table>' + tableHtml(["MAC", "Port", "VLAN", "Bridge", "Entry"], hostRows, ro, "No host.") + "</table></div>";
  if (focus) { const i = $("if-q"); i.focus(); i.setSelectionRange(i.value.length, i.value.length); }
}

/* the form of one object; only what changed is sent for an edit */

function ifChoices(v, kind, prop) {
  const names = rows => (rows || []).map(x => x.name);
  if (prop === "bridge") return names(v.bridges);
  if (kind === "bport" && prop === "interface") return (v.ifaces || []).filter(i => ["ether", "bond", "vlan", "eoip", "vxlan", "veth", "wlan"].includes(i.type) && !yes(i.slave)).map(i => i.name).sort(natural);
  if (kind === "vlan" && prop === "interface") return (v.ifaces || []).filter(i => i.type !== "loopback").map(i => i.name).sort(natural);
  if (prop === "slaves") return (v.ifaces || []).filter(i => ["ether", "eoip", "vxlan", "veth"].includes(i.type)).map(i => i.name).sort(natural);
  if (prop === "tagged" || prop === "untagged") return [...names(v.bridges), ...(v.bports || []).map(p => p.interface)].sort(natural);
  return [];
}

function fieldHtml(v, kind, [k, label, type, choices], val) {
  const id = "f-" + k;
  if (type === "multi") {
    const on = new Set(list(val));
    const all = [...new Set([...ifChoices(v, kind, k), ...on])];
    return "<fieldset><legend>" + esc(t(label)) + '</legend><div class="cols2">' + all.map(n => '<label class="check"><input type="checkbox" name="' + id + '" value="' + esc(n) + '"' + (on.has(n) ? " checked" : "") + "> " + esc(n) + "</label>").join("") + "</div></fieldset>";
  }
  if (type === "bool") return "<label>" + esc(t(label)) + '<select name="' + id + '">' + BOOL.map(b => '<option value="' + b + '"' + (ifNorm(val) === ifNorm(b) ? " selected" : "") + ">" + esc(t(b)) + "</option>").join("") + "</select></label>";
  if (type === "sel") {
    const c = choices || ifChoices(v, kind, k);
    return "<label>" + esc(t(label)) + '<select name="' + id + '">' + [...new Set([...(val ? [val] : [""]), ...c])].map(o => '<option value="' + esc(o) + '"' + (o === val ? " selected" : "") + ">" + esc(o || "—") + "</option>").join("") + "</select></label>";
  }
  const dl = choices ? ' list="dl-' + id + '"' : "";
  return "<label>" + esc(t(label)) + '<input name="' + id + '"' + (type === "num" ? ' inputmode="numeric"' : "") + dl + ' value="' + esc(val || "") + '">' + (choices ? opts("dl-" + id, choices) : "") + "</label>";
}

async function ifForm(kind, id) {
  const v = IF, row = id ? (v[IF_ROWS[kind]] || []).find(x => x[".id"] === id) : null;
  const fields = IF_FIELDS[kind];
  const def = { bond: { mode: "802.3ad", "transmit-hash-policy": "layer-2-and-3" }, bport: { pvid: "1", "frame-types": "admit-all", "ingress-filtering": "yes", horizon: "none" }, bvlan: {}, vlan: {} }[kind] || {};
  const cur = row || Object.assign({ bridge: (v.bridges || [])[0] && v.bridges[0].name, disabled: "no" }, def);
  const title = row ? t("Edit {what} {name}", { what: t(IF_KIND[kind]), name: row.name || row.interface || row["vlan-ids"] || "" }) : t("Add a {what}", { what: t(IF_KIND[kind]) });
  const fd = await ask(title, fields.map(f => fieldHtml(v, kind, f, cur[f[0]])).join("") +
    '<p class="sub">' + esc(t("Nothing is written yet: the next step shows the commands.")) + "</p>", t("Preview"));
  if (!fd) return;
  const props = {};
  for (const [k, , type] of fields) {
    const val = type === "multi" ? fd.getAll("f-" + k).join(",") : String(fd.get("f-" + k) ?? "").trim();
    if (row ? ifNorm(val) !== ifNorm(row[k]) : val !== "") props[k] = val;
  }
  if (!Object.keys(props).length) return banner(t("Nothing to change."), "ok");
  return ifSend({ kind, op: row ? "set" : "add", id: id || "", props });
}

async function ifDelete(kind, id) {
  const row = (IF[IF_ROWS[kind]] || []).find(x => x[".id"] === id);
  if (row) return ifSend({ kind, op: "remove", id, props: {} });
}

async function ifSend(change) {
  const v = IF, body = Object.assign({ switch: v.name }, change);
  let p;
  try { p = await post("/api/ifaces/preview", body); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: problemText(p.error || "") }));
  if (!p.commands.length) return banner(t("Nothing to change."), "ok");
  banner("");
  if (!(await ask(t("Apply on {name}?", { name: v.name }),
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    (p.guarded ? "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>"
      : '<p class="sub">' + esc(t("A comment only: no rollback, the switch is read back.")) + "</p>"), t("Apply")))) return;
  let r;
  try { r = await post("/api/ifaces/apply", body); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.pending && r.verified) { IF_UNTIL = Date.now() + r.rollback_s * 1000; banner(""); }
  else if (r.pending) {
    IF_UNTIL = Date.now() + r.rollback_s * 1000;
    banner(t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name: v.name }) + (r.error ? " · " + problemText(r.error) : " · " + t("read back differs")) +
      " · " + t("the switch puts the old settings back within {n} s", { n: r.rollback_s }));
  } else banner(r.ok ? t("Applied on {name}.", { name: v.name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadIfaces();
}

async function ifKeep() {
  let r;
  try { r = await post("/api/infra/confirm", { switch: IF.name }); } catch (e) { r = { ok: false, error: e.message }; }
  IF_UNTIL = 0;
  banner(r.ok ? t("Kept on {name}.", { name: IF.name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadIfaces();
}

document.addEventListener("click", e => {
  const b = e.target.closest("button");
  if (!b || !IF || IF.error || !String(b.dataset.act || "").startsWith("if-")) return;
  const d = b.dataset;
  if (d.act === "if-keep") return ifKeep();
  if (d.act === "if-add") return ifForm(d.kind);
  if (d.act === "if-edit") return ifForm(d.kind, d.id);
  if (d.act === "if-del") return ifDelete(d.kind, d.id);
});
document.addEventListener("input", e => { if (e.target.id === "if-q") renderIfaces(); });

// The countdown; at zero the switch has rolled back: read it again.
setInterval(() => {
  if (!IF_UNTIL || VIEW !== "ifaces") return;
  if (Date.now() >= IF_UNTIL + 3000) { IF_UNTIL = 0; loadIfaces(); }
  ifPending();
}, 1000);

/* SFP diagnostics, under the Ports view's table (renderPorts) */
function renderSfp(sw) {
  const box = $("sfp");
  if (!box) return;
  const mods = (sw.ports || []).filter(p => p.mon && yes(p.mon["sfp-module-present"]));
  box.hidden = !mods.length;
  if (!mods.length) return;
  // measures (DDM) only when a module reports them: copper DAC cables do not
  const MEAS = [["sfp-temperature", "Temperature", "°C"], ["sfp-rx-power", "Rx power", "dBm"], ["sfp-tx-power", "Tx power", "dBm"], ["sfp-supply-voltage", "Voltage", "V"], ["sfp-tx-bias-current", "Bias", "mA"]]
    .filter(([k]) => mods.some(p => p.mon[k] != null && p.mon[k] !== ""));
  $("sfp-t").innerHTML = "<thead><tr><th>" + ["Port", "Vendor", "Part number", "Serial", "Type", ...MEAS.map(x => x[1])].map(c => esc(t(c))).join("</th><th>") + "</th></tr></thead><tbody>" +
    mods.map(p => { const m = p.mon;
      return "<tr><td><b>" + esc(p.name) + "</b></td><td>" + esc(m["sfp-vendor-name"] || "") + "</td><td>" + esc(m["sfp-vendor-part-number"] || "") + '</td><td class="mono">' + esc(m["sfp-vendor-serial"] || "") +
        '</td><td class="wrap">' + esc([m["sfp-type"], m["sfp-connector-type"], m["sfp-wavelength"] && m["sfp-wavelength"] + " nm"].filter(Boolean).join(" · ")) + "</td>" +
        MEAS.map(([k, , unit]) => '<td class="num' + (k === "sfp-temperature" && num(m[k]) >= 70 ? " warn" : "") + '">' + esc(m[k] == null || m[k] === "" ? "" : m[k] + " " + unit) + "</td>").join("") + "</tr>"; }).join("") + "</tbody>";
  $("sfp-note").textContent = MEAS.length ? "" : t("No module reports diagnostics (temperature, power): copper DAC cables have none.");
}

// src/ifaces.rs messages -> page language (problemText in panel.js)
PROBLEMS.push(
  [/^(.+) carries Cairn Net's path to the switch \((.*)\): refused, .*$/, "{who} carries Cairn Net's path to the switch ({v}): refused, it could cut the app off. Change it from another way in (SSH, MAC-WinBox)"],
  [/^(\S+) is in use \(bridge port, VLAN or address\): remove that first$/, "{who} is in use (bridge port, VLAN or address): remove that first"],
  [/^(\S+) has an IP address: a bridge port loses it, move it first$/, "{who} has an IP address: a bridge port loses it, move it first"],
  [/^(\S+) is needed$/, "{who} is needed"],
);

Object.assign(DICT.fr, {
  "{who} carries Cairn Net's path to the switch ({v}): refused, it could cut the app off. Change it from another way in (SSH, MAC-WinBox)":
    "{who} porte le chemin de Cairn Net vers le switch ({v}) : refusé, Cairn Net pourrait perdre le switch. Faites-le par un autre accès (SSH, MAC-WinBox).",
  "an interface of the path": "une interface du chemin", "a member would move": "un membre changerait d'agrégat", "the bridge of the path": "le pont du chemin",
  "the port Cairn Net's packets arrive on": "le port par lequel arrivent les paquets de Cairn Net", "a member of the path would change": "un membre du chemin changerait",
  "{who} is in use (bridge port, VLAN or address): remove that first": "{who} est utilisé (port de pont, VLAN ou adresse) : retirez-le d'abord",
  "{who} has an IP address: a bridge port loses it, move it first": "{who} a une adresse IP : un port de pont la perd, déplacez-la d'abord",
  "{who} is needed": "{who} est obligatoire", "a dynamic entry is made by RouterOS: it is not edited": "une entrée dynamique est faite par RouterOS : elle ne se modifie pas",
  "the object is gone: reload": "l'objet n'existe plus : rechargez",
  "Interfaces": "Interfaces", "Interfaces and bridge": "Interfaces et pont", "Bonds": "Agrégats (bonds)", "Bridges": "Ponts", "Bridge ports": "Ports des ponts",
  "Bridge VLAN table": "Table des VLAN du pont", "VLAN interfaces": "Interfaces VLAN", "Host table": "Table des hôtes",
  "Cairn Net's path ({app}): {path}. A change touching it is refused.": "Chemin de Cairn Net ({app}) : {path}. Une modification qui le touche est refusée.",
  "{ifaces}; packets arrive on {port}": "{ifaces} ; les paquets arrivent par {port}", "an unknown port (every port of {b} is protected)": "un port inconnu (tous les ports de {b} sont protégés)",
  "Each change is previewed, then read back; anything but a comment is rolled back by the switch after {n} s unless you keep it.": "Chaque modification passe par un aperçu puis une relecture ; tout sauf un commentaire est remis par le switch au bout de {n} s si vous ne le gardez pas.",
  "Read only: only a switch in direct mode is edited here.": "Lecture seule : seul un switch en mode direct se modifie ici.",
  "MTU / L2MTU": "MTU / L2MTU", "Flow control": "Contrôle de flux", "No ethernet port.": "Aucun port Ethernet.", "Members": "Membres", "Mode": "Mode", "Hash": "Hachage",
  "No bond.": "Aucun agrégat.", "VLAN filtering": "Filtrage VLAN", "Protocol": "Protocole", "Priority": "Priorité", "No bridge.": "Aucun pont.",
  "PVID": "PVID", "Frame types": "Types de trames", "Ingress filtering": "Filtrage en entrée", "Horizon": "Horizon", "STP": "STP", "No bridge port.": "Aucun port de pont.",
  "Tagged": "Étiqueté", "Untagged": "Non étiqueté", "Currently": "En vigueur", "No VLAN row.": "Aucune ligne VLAN.", "VLAN ID": "ID VLAN", "On": "Sur", "No VLAN interface.": "Aucune interface VLAN.",
  "MAC": "MAC", "Entry": "Entrée", "No host.": "Aucun hôte.", "{n} of {total}": "{n} sur {total}", "Filter: MAC, port, VLAN…": "Filtrer : MAC, port, VLAN…",
  "local": "locale", "learned": "apprise", "static": "statique", "external": "externe", "forwarding": "transmet", "blocked": "bloqué",
  "Add": "Ajouter", "port": "port", "bond": "agrégat", "bridge": "pont", "bridge port": "port de pont", "VLAN row": "ligne VLAN", "VLAN interface": "interface VLAN",
  "Edit {what} {name}": "Modifier {what} {name}", "Add a {what}": "Ajouter : {what}", "Disabled": "Désactivé", "Comment": "Commentaire", "MTU": "MTU", "L2MTU": "L2MTU",
  "Auto-negotiation": "Auto-négociation", "Speed (without auto-negotiation)": "Vitesse (sans auto-négociation)", "Advertised speeds (comma list)": "Vitesses annoncées (liste séparée par des virgules)",
  "Flow control (receive)": "Contrôle de flux (réception)", "Flow control (send)": "Contrôle de flux (émission)", "Hash (802.3ad, balance-xor)": "Hachage (802.3ad, balance-xor)",
  "LACP rate": "Cadence LACP", "Minimum links": "Liens minimum", "Priority (0x0000-0xF000)": "Priorité (0x0000-0xF000)", "Bridge": "Pont", "Interface": "Interface",
  "Horizon (none or a number)": "Horizon (none ou un nombre)", "VLAN IDs (10 or 10,20 or 100-200)": "ID VLAN (10, 10,20 ou 100-200)",
  "A comment only: no rollback, the switch is read back.": "Un commentaire seulement : pas de retour automatique, le switch est relu.",
  "read back differs": "la relecture diffère", "Applied on {name}.": "Appliqué sur {name}.",
  "Vendor": "Fournisseur", "Part number": "Référence", "Serial": "N° de série", "Type": "Type", "Temperature": "Température", "Rx power": "Puissance Rx", "Tx power": "Puissance Tx",
  "Voltage": "Tension", "Bias": "Courant laser", "SFP modules": "Modules SFP",
  "No module reports diagnostics (temperature, power): copper DAC cables have none.": "Aucun module ne donne de mesures (température, puissance) : les câbles DAC cuivre n'en ont pas.",
});
