// cairn-net: System page (src/system.rs), loaded after panel.js and uses its helpers.
// One switch at a time, in parts (chips): overview (resources, clock, NTP, CPU
// profile), logs, users, scheduler and scripts, files and backups, packages,
// certificates and SNMP. Reads work in every mode; anything that needs the
// write account (changes, downloads, export, profile) is for direct mode only.
// A change is one operation: previewed as RouterOS commands, applied, read back.
"use strict";

let SYS = null, SYS_LOG = null, SYS_PART = "overview", SYS_PROF = null;
const SYS_PARTS = { overview: "Resources and clock", log: "Logs", users: "Users", scripts: "Scheduler and scripts", files: "Files and backups",
  packages: "Packages", certs: "Certificates and SNMP" };
try { SYS_PART = SYS_PARTS[localStorage.getItem("cairn-sys-part")] ? localStorage.getItem("cairn-sys-part") : SYS_PART; } catch { /* default */ }

async function loadSys() {
  const name = SW;
  if (!name) return;
  try { const v = await get("/api/system?switch=" + encodeURIComponent(name)); if (name === SW) SYS = v; }
  catch (e) { if (name === SW) SYS = { name, error: e.message }; }
  if (SYS_PART === "log") await loadSysLog();
  if (VIEW === "system") render(true);
}
async function loadSysLog() {
  const name = SW;
  try { const v = await get("/api/system/log?switch=" + encodeURIComponent(name)); if (name === SW) SYS_LOG = Object.assign(v, { name }); }
  catch (e) { if (name === SW) SYS_LOG = { name, error: e.message }; }
}

const sysDirect = () => SYS && SYS.mode === "direct";
// what the write account may do; buttons it can't use are off
const sysMay = p => sysDirect() && (!SYS.write_policy || SYS.write_policy.includes(p));
const sysWhy = p => !sysDirect() ? t("Direct mode only (Settings)") : t("The write account's group lacks the {p} right (Settings, preparation script)", { p });
const sysBtn = (act, label, p, attrs = "", cls = "") => '<button class="' + cls + '" data-act="' + act + '"' + attrs + (sysMay(p) ? "" : ' disabled title="' + esc(sysWhy(p)) + '"') + ">" + esc(t(label)) + "</button>";
const kvl = rows => '<dl class="kv">' + rows.map(([k, val, c]) => "<dt>" + esc(k) + '</dt><dd class="' + (c || "") + '">' + esc(val) + "</dd>").join("") + "</dl>";
const thead = cols => "<thead><tr><th>" + cols.map(c => esc(t(c))).join("</th><th>") + "</th></tr></thead>";
const none = (n, msg) => '<tr><td colspan="' + n + '" class="sub">' + esc(t(msg)) + "</td></tr>";
const idx = i => ' data-i="' + i + '"';

function renderSys() {
  $("sys-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="system/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  $("sys-parts").innerHTML = Object.entries(SYS_PARTS).map(([k, l]) => '<button class="chip' + (k === SYS_PART ? " on" : "") + '" data-act="sys-part" data-part="' + k + '">' + esc(t(l)) + "</button>").join("");
  const v = SYS && SYS.name === SW ? SYS : null, body = $("sys-body");
  if (!v || v.error) {
    body.dataset.key = "";
    body.innerHTML = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
    return;
  }
  // the log's filters live outside what is redrawn, so typing in them is not lost
  const key = SW + "|" + SYS_PART + "|" + LANG;
  if (body.dataset.key !== key) {
    body.dataset.key = key;
    body.innerHTML = (SYS_PART === "log" ? sysLogShell(v) : "") + '<div id="sys-content"></div>' + (sysDirect() ? "" : '<p class="sub">' + esc(t("Read only: changes, downloads, export and the CPU profile use the write account, in direct mode (Settings).")) + "</p>");
  }
  // redrawn only when it changed: the 3 s refresh must not reset scrolling or steal clicks
  const html = ({ overview: sysOverview, log: sysLogTable, users: sysUsers, scripts: sysScripts, files: sysFiles, packages: sysPackages, certs: sysCerts })[SYS_PART](v);
  const box = $("sys-content");
  if (box.dataset.html !== html) { box.innerHTML = html; box.dataset.html = html; }
}

/* resources, clock, NTP, profile */

function sysOverview(v) {
  const r = v.resource || {}, c = v.clock || {}, n = v.ntp || {};
  const mem = num(r["total-memory"]), memUsed = mem - num(r["free-memory"]), hdd = num(r["total-hdd-space"]), hddUsed = hdd - num(r["free-hdd-space"]);
  const pct = (a, b) => (b ? Math.round(a * 100 / b) : 0), lvl = p => (p >= 90 ? "bad" : p >= 75 ? "warn" : "ok");
  const meter = (label, p, text) => '<div class="meter ' + lvl(p) + '"><span>' + esc(label) + '</span><div class="bar"><i data-w="' + p + '"></i></div><b>' + esc(text) + "</b></div>";
  const ntpOk = /synchronized|started/.test(n.status || "") && yes(n.enabled);
  const prof = SYS_PROF && SYS_PROF.name === v.name ? SYS_PROF : null;
  return '<div class="grid srvs">' +
    '<article class="card srv sys"><div class="card-head"><h3>' + esc(t("Resources")) + "</h3></div>" +
      meter(t("CPU"), num(r["cpu-load"]), num(r["cpu-load"]) + " %") +
      (v.cpu || []).map(x => meter(t("Core {n}", { n: String(x.cpu || "").replace(/^cpu/, "") }), num(x.load), num(x.load) + " %")).join("") +
      meter(t("Memory"), pct(memUsed, mem), pct(memUsed, mem) + " %") +
      meter(t("Disk"), pct(hddUsed, hdd), pct(hddUsed, hdd) + " %") +
      kvl([[t("CPU"), [r.cpu, r["cpu-count"] && tp(num(r["cpu-count"]), "{n} core", "{n} cores"), r["cpu-frequency"] && r["cpu-frequency"] + " MHz"].filter(Boolean).join(" · ")],
        [t("Memory"), bytes(memUsed) + " / " + bytes(mem)], [t("Disk"), bytes(hddUsed) + " / " + bytes(hdd)], [t("Board"), r["board-name"] || "—"], [t("RouterOS"), r.version || "—"], [t("Up for"), duration(rosSeconds(r.uptime))],
        [t("Disk writes since restart"), r["write-sect-since-reboot"] ? tp(num(r["write-sect-since-reboot"]), "{n} sector", "{n} sectors") : "—"]]) + "</article>" +
    '<article class="card srv"><div class="card-head"><h3>' + esc(t("Clock and NTP")) + "</h3>" + '<span class="actions">' + sysBtn("sys-clock", "Time zone", "write") + sysBtn("sys-ntp", "NTP", "write") + "</span></div>" +
      kvl([[t("Date and time"), (c.date || "") + " " + (c.time || "")], [t("Time zone"), (c["time-zone-name"] || "—") + (c["gmt-offset"] ? " (" + c["gmt-offset"] + ")" : "")],
        [t("Time zone from the IP address"), yes(c["time-zone-autodetect"]) ? t("yes") : t("no")],
        [t("NTP client"), yes(n.enabled) ? t("on") : t("off"), yes(n.enabled) ? "" : "warn"], [t("NTP servers"), list(n.servers).join(", ") || "—"],
        [t("Synchronisation"), n.status || "—", ntpOk ? "ok" : yes(n.enabled) ? "warn" : ""], ...(n["synced-server"] ? [[t("Synchronised with"), n["synced-server"]]] : []),
        ...(n["system-offset"] ? [[t("Offset"), n["system-offset"] + " ms"]] : [])]) + "</article>" +
    '<article class="card srv"><div class="card-head"><h3>' + esc(t("CPU profile")) + "</h3>" + sysBtn("sys-profile", "Measure 5 s", "test") + "</div>" +
      (prof ? (prof.error ? '<p class="sub">' + esc(prof.error) + "</p>" : prof.busy ? '<p class="sub">' + esc(t("Measuring…")) + "</p>" :
        '<table class="mini">' + prof.rows.slice(0, 12).map(x => "<tr><td>" + esc(x.name) + (x.cpu && x.cpu !== "all" ? ' <span class="sub">' + esc(x.cpu) + "</span>" : "") + '</td><td class="num">' + nf(x.usage) + " %</td></tr>").join("") + "</table>")
        : '<p class="sub">' + esc(t("What uses the CPU, process by process (/tool profile).")) + "</p>") + "</article>" +
    "</div>";
}

/* logs */

const LEVELS = ["debug", "info", "warning", "error", "critical"];
function sysLogShell(v) {
  return '<div class="toolbar log-filters"><input type="search" id="sys-q" placeholder="' + esc(t("Search the log")) + '">' +
    '<select id="sys-topic"><option value="">' + esc(t("All topics")) + "</option></select>" +
    '<select id="sys-level"><option value="">' + esc(t("All levels")) + "</option>" + LEVELS.map(l => '<option value="' + l + '">' + esc(t(l)) + "</option>").join("") + "</select>" +
    '<label class="check"><input type="checkbox" id="sys-live" checked> ' + esc(t("Live")) + "</label></div>";
}
function sysLogTable(v) {
  const L = SYS_LOG && SYS_LOG.name === v.name ? SYS_LOG : null;
  const rules = '<h2>' + esc(t("Logging rules")) + '</h2><div class="toolbar">' + sysBtn("sys-rule", "New rule", "write") + sysBtn("sys-remote", "New remote syslog", "write") + "</div>" +
    '<div class="card table-wrap"><table>' + thead(["Topics", "Action", "Prefix", "State", ""]) + "<tbody>" +
    (v.log_rules || []).map((x, i) => '<tr class="' + (yes(x.disabled) ? "dim" : "") + '"><td><b>' + esc(x.topics) + "</b></td><td>" + esc(x.action) + "</td><td>" + esc(x.prefix || "") + "</td><td>" +
      esc(yes(x.disabled) ? t("disabled") : yes(x.invalid) ? t("invalid") : t("active")) + '</td><td class="actions">' + sysBtn("sys-rule", "Edit", "write", idx(i)) +
      sysBtn("sys-rule-toggle", yes(x.disabled) ? "Turn on" : "Turn off", "write", idx(i)) + (yes(x.default) ? "" : sysBtn("sys-rule-del", "Delete", "write", idx(i), "danger")) + "</td></tr>").join("") + "</tbody></table></div>" +
    '<h2>' + esc(t("Log destinations")) + '</h2><div class="card table-wrap"><table>' + thead(["Name", "Kind", "Where", ""]) + "<tbody>" +
    (v.log_actions || []).map((x, i) => "<tr><td><b>" + esc(x.name) + "</b></td><td>" + esc(x.target) + "</td><td>" +
      esc(x.target === "remote" ? (x.remote || "") + ":" + (x["remote-port"] || "514") + " " + (x["remote-protocol"] || "udp") + (x["remote-log-format"] ? " · " + x["remote-log-format"] : "")
        : x.target === "memory" ? tp(num(x["memory-lines"]), "{n} line", "{n} lines") : x.target === "disk" ? (x["disk-file-name"] || "") + " · " + tp(num(x["disk-file-count"]), "{n} file", "{n} files") : "") +
      '</td><td class="actions">' + (x.target === "remote" ? sysBtn("sys-remote", "Edit", "write", idx(i)) + (yes(x.default) ? "" : sysBtn("sys-action-del", "Delete", "write", idx(i), "danger")) : "") + "</td></tr>").join("") +
    "</tbody></table></div>";
  if (!L) return '<p class="sub">' + esc(t("Loading…")) + "</p>" + rules;
  if (L.error) return '<p class="verdict bad"><span class="dot bad"></span>' + esc(L.error) + "</p>" + rules;
  const lines = L.log || [], topics = [...new Set(lines.flatMap(x => list(x.topics)))].filter(x => !LEVELS.includes(x)).sort();
  const sel = $("sys-topic");
  if (sel && sel.options.length - 1 !== topics.length) {
    const cur = sel.value;
    sel.innerHTML = '<option value="">' + esc(t("All topics")) + "</option>" + topics.map(x => '<option' + (x === cur ? " selected" : "") + ">" + esc(x) + "</option>").join("");
  }
  const q = ($("sys-q") || {}).value || "", topic = ($("sys-topic") || {}).value || "", level = ($("sys-level") || {}).value || "";
  const shownRows = lines.filter(x => {
    const tps = list(x.topics);
    return (!topic || tps.includes(topic)) && (!level || tps.includes(level)) && (!q || (x.message + " " + x.topics).toLowerCase().includes(q.toLowerCase()));
  }).reverse().slice(0, 500);
  const cls = x => { const tps = list(x.topics); return tps.includes("critical") || tps.includes("error") ? "bad" : tps.includes("warning") ? "warn" : ""; };
  return '<p class="sub">' + esc(t("{n} of {m} lines, newest first (the switch keeps them in memory).", { n: shownRows.length, m: lines.length })) + "</p>" +
    '<div class="card table-wrap"><table class="log">' + thead(["Time", "Topics", "Message"]) + "<tbody>" +
    (shownRows.map(x => '<tr class="' + cls(x) + '"><td class="mono">' + esc(x.time) + '</td><td class="sub">' + esc(x.topics) + '</td><td class="wrap">' + esc(x.message) + "</td></tr>").join("") || none(3, "Nothing matches.")) +
    "</tbody></table></div>" + rules;
}

/* users */

function sysUsers(v) {
  const mine = v.mine || [];
  const fullN = (v.users || []).filter(u => u.group === "full" && !yes(u.disabled) && !mine.includes(u.name)).length;
  return '<div class="toolbar">' + sysBtn("sys-user-new", "New user", "policy") + "</div>" +
    '<div class="card table-wrap"><table>' + thead(["Name", "Group", "Allowed from", "Last sign-in", "Comment", ""]) + "<tbody>" +
    (v.users || []).map((u, i) => {
      const ours = mine.includes(u.name), last = u.group === "full" && !yes(u.disabled) && fullN <= 1;
      return '<tr class="' + (yes(u.disabled) ? "dim" : "") + '"><td><b>' + esc(u.name) + "</b>" + (ours ? ' <span class="pill">Cairn Net</span>' : "") + (yes(u.disabled) ? " · " + esc(t("disabled")) : "") +
        "</td><td>" + esc(u.group) + "</td><td>" + esc(u.address || t("everywhere")) + "</td><td>" + esc(u["last-logged-in"] || "—") + '</td><td class="wrap sub">' + esc(u.comment || "") +
        '</td><td class="actions">' + (ours ? "" : sysBtn("sys-user-addr", "Allowed from", "policy", idx(i)) + (last ? "" : sysBtn("sys-user-del", "Delete", "policy", idx(i), "danger"))) + "</td></tr>";
    }).join("") + "</tbody></table></div>" +
    "<h2>" + esc(t("Groups")) + '</h2><div class="card table-wrap"><table>' + thead(["Name", "Rights"]) + "<tbody>" +
    (v.groups || []).map(g => "<tr><td><b>" + esc(g.name) + '</b></td><td class="wrap sub">' + esc(String(g.policy || "").split(",").filter(p => p && !p.startsWith("!")).join(", ")) + "</td></tr>").join("") + "</tbody></table></div>" +
    "<h2>" + esc(t("Active sessions")) + '</h2><div class="card table-wrap"><table>' + thead(["Name", "From", "Through", "Since"]) + "<tbody>" +
    ((v.active || []).map(a => "<tr><td><b>" + esc(a.name) + "</b></td><td>" + esc(a.address) + "</td><td>" + esc(a.via) + "</td><td>" + esc(a.when) + "</td></tr>").join("") || none(4, "Nobody.")) + "</tbody></table></div>";
}

/* scheduler and scripts */

const firstLine = s => { const l = String(s || "").trim().split("\n"); return (l[0].length > 60 ? l[0].slice(0, 60) + "…" : l[0]) + (l.length > 1 ? " (+" + (l.length - 1) + ")" : ""); };
function sysScripts(v) {
  const rb = v.rollback_name;
  return "<h2>" + esc(t("Scheduler")) + '</h2><div class="toolbar">' + sysBtn("sys-sched", "New task", "write") + "</div>" +
    '<div class="card table-wrap"><table>' + thead(["Name", "Start", "Every", "Next run", "Runs", "Event", ""]) + "<tbody>" +
    ((v.scheds || []).map((x, i) => '<tr class="' + (yes(x.disabled) ? "dim" : "") + '"><td><b>' + esc(x.name) + "</b>" + (x.name === rb ? ' <span class="pill warn">' + esc(t("Cairn Net rollback")) + "</span>" : "") +
      "</td><td>" + esc([x["start-date"], x["start-time"]].filter(Boolean).join(" ")) + "</td><td>" + esc(x.interval || "") + "</td><td>" + esc(x["next-run"] || "—") + "</td><td>" + esc(x["run-count"] || "0") +
      '</td><td class="mono sub" title="' + esc(x["on-event"] || "") + '">' + esc(firstLine(x["on-event"])) + '</td><td class="actions">' +
      (x.name === rb ? "" : sysBtn("sys-sched", "Edit", "write", idx(i)) + sysBtn("sys-sched-del", "Delete", "write", idx(i), "danger")) + "</td></tr>").join("") || none(7, "No task.")) +
    "</tbody></table></div>" +
    "<h2>" + esc(t("Scripts")) + '</h2><div class="toolbar">' + sysBtn("sys-script", "New script", "write") + "</div>" +
    '<div class="card table-wrap"><table>' + thead(["Name", "Last run", "Runs", "Source", ""]) + "<tbody>" +
    ((v.scripts || []).map((x, i) => "<tr><td><b>" + esc(x.name) + "</b>" + (x.comment ? ' <span class="sub">' + esc(x.comment) + "</span>" : "") + "</td><td>" + esc(x["last-started"] || "—") +
      "</td><td>" + esc(x["run-count"] || "0") + '</td><td class="mono sub" title="' + esc(x.source || "") + '">' + esc(firstLine(x.source)) + '</td><td class="actions">' +
      sysBtn("sys-script-run", "Run…", "write", idx(i)) + sysBtn("sys-script", "Edit", "write", idx(i)) + sysBtn("sys-script-del", "Delete", "write", idx(i), "danger") + "</td></tr>").join("") || none(5, "No script.")) +
    "</tbody></table></div>";
}

/* files, export, backup */

function sysFiles(v) {
  const href = f => "/api/system/file?switch=" + encodeURIComponent(v.name) + "&name=" + encodeURIComponent(f.name);
  const can = sysMay("ftp");
  return '<div class="grid srvs"><article class="card srv"><div class="card-head"><h3>' + esc(t("Backup and export")) + "</h3></div>" +
    '<p class="sub">' + esc(t("A backup (.backup) is the whole switch, to put back on this same switch; an export (.rsc) is the configuration as commands, readable and comparable.")) + "</p>" +
    '<div class="toolbar">' + sysBtn("sys-backup", "Create a backup…", "sensitive") + sysBtn("sys-export", "Download the export", "write") + sysBtn("sys-export-s", "Export with secrets…", "sensitive") + "</div></article></div>" +
    '<div class="toolbar">' + sysBtn("sys-upload", "Send a file…", "ftp") + "</div>" +
    '<div class="card table-wrap"><table>' + thead(["Name", "Kind", "Size", "Changed", ""]) + "<tbody>" +
    ((v.files || []).map((f, i) => "<tr><td><b>" + esc(f.name) + "</b></td><td>" + esc(f.type || "") + '</td><td class="num">' + (f.size != null ? esc(bytes(num(f.size))) : "") + "</td><td>" + esc(f["last-modified"] || "") +
      '</td><td class="actions">' + (f.type === "directory" ? "" : (can ? '<a class="button" href="' + esc(href(f)) + '" download>' + esc(t("Download")) + "</a>" : sysBtn("", "Download", "ftp")) +
        (/\.backup$/.test(f.name) ? sysBtn("sys-restore", "Restore…", "policy", idx(i)) : "") + sysBtn("sys-file-del", "Delete", "ftp", idx(i), "danger")) + "</td></tr>").join("") || none(5, "No file.")) +
    "</tbody></table></div>";
}

/* packages, firmware */

function sysPackages(v) {
  const rb = v.routerboard;
  const state = p => p.scheduled ? [p.scheduled, "warn"] : yes(p.available) ? [t("available"), ""] : yes(p.disabled) ? [t("disabled"), ""] : [t("enabled"), "ok"];
  return (rb && rb.model ? '<div class="grid srvs"><article class="card srv"><div class="card-head"><h3>' + esc(t("RouterBOARD firmware")) + "</h3>" +
      sysBtn("sys-rb", "Upgrade…", "write", rb["upgrade-firmware"] && rb["upgrade-firmware"] !== rb["current-firmware"] ? "" : " hidden") + "</div>" +
      kvl([[t("Model"), rb.model], [t("Firmware"), rb["current-firmware"] || "—"], [t("Available"), rb["upgrade-firmware"] || "—", rb["upgrade-firmware"] !== rb["current-firmware"] ? "warn" : "ok"]]) +
      '<p class="sub">' + esc(t("The new firmware is installed at the next restart (Infrastructure page).")) + "</p></article></div>" : "") +
    '<p class="sub">' + esc(t("Enabling, disabling or uninstalling waits for the next restart; Apply the changes downloads what is needed and restarts now. The routeros package itself is left alone.")) + "</p>" +
    ((v.packages || []).some(p => p.scheduled) ? '<div class="toolbar">' + sysBtn("sys-pkg-apply", "Apply the changes…", "write", "", "primary") + "</div>" : "") +
    '<div class="card table-wrap"><table>' + thead(["Package", "Version", "State", ""]) + "<tbody>" +
    (v.packages || []).map((p, i) => {
      const [st, c] = state(p), on = !yes(p.disabled) && !yes(p.available);
      return "<tr><td><b>" + esc(p.name) + "</b></td><td>" + esc(p.version || "—") + '</td><td><span class="pill ' + c + '">' + esc(st) + '</span></td><td class="actions">' +
        (p.scheduled ? sysBtn("sys-pkg", "Cancel", "write", idx(i) + ' data-do="unschedule"') : p.name === "routeros" ? "" :
          (on ? sysBtn("sys-pkg", "Disable…", "write", idx(i) + ' data-do="disable"') + sysBtn("sys-pkg", "Uninstall…", "write", idx(i) + ' data-do="uninstall"', "danger")
            : sysBtn("sys-pkg", "Enable…", "write", idx(i) + ' data-do="enable"'))) + "</td></tr>";
    }).join("") + "</tbody></table></div>";
}

/* certificates, SNMP */

function sysCerts(v) {
  const sn = v.snmp || {};
  const left = c => { const s = rosSeconds(c["expires-after"]); return yes(c.expired) ? [t("expired"), "bad"] : !c["expires-after"] ? ["—", ""] : [duration(s).replace(/ \d+ h \d+$/, "") || duration(s), s < 30 * 86400 ? "bad" : s < 90 * 86400 ? "warn" : "ok"]; };
  return '<div class="card table-wrap"><table>' + thead(["Certificate", "Common name", "Valid until", "Left", "Key"]) + "<tbody>" +
    ((v.certs || []).map(c => { const [l, cl] = left(c); return "<tr><td><b>" + esc(c.name) + "</b>" + (yes(c.trusted) ? ' <span class="pill">' + esc(t("trusted")) + "</span>" : "") + "</td><td>" + esc(c["common-name"] || "") + "</td><td>" +
      esc(c["invalid-after"] || "") + '</td><td><span class="pill ' + cl + '">' + esc(l) + "</span></td><td>" + esc(yes(c["private-key"]) ? t("yes") : t("no")) + "</td></tr>"; }).join("") || none(5, "No certificate.")) +
    "</tbody></table></div>" +
    '<div class="grid srvs"><article class="card srv"><div class="card-head"><h3>SNMP</h3>' + sysBtn("sys-snmp", "Edit", "write") + "</div>" +
      kvl([[t("SNMP"), yes(sn.enabled) ? t("on") : t("off"), yes(sn.enabled) ? "ok" : ""], [t("Contact"), sn.contact || "—"], [t("Location"), sn.location || "—"],
        [t("Traps to"), sn["trap-target"] || "—"]]) + "</article></div>" +
    '<div class="card table-wrap"><table>' + thead(["Community", "Allowed from", "Security", "Access", ""]) + "<tbody>" +
    (v.communities || []).map((c, i) => '<tr class="' + (yes(c.disabled) ? "dim" : "") + '"><td><b>' + esc(c.name) + "</b></td><td>" + esc(c.addresses || "") + "</td><td>" + esc(c.security || "") + "</td><td>" +
      esc([yes(c["read-access"]) && t("read"), yes(c["write-access"]) && t("write")].filter(Boolean).join(", ") || "—") + '</td><td class="actions">' + sysBtn("sys-community", "Edit", "write", idx(i)) + "</td></tr>").join("") +
    "</tbody></table></div>";
}

/* changes */

// Preview, confirmation(s), apply, read back.
async function sysOp(op, what) {
  const v = SYS;
  let p;
  try { p = await post("/api/system/preview", { switch: v.name, op }); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: t(p.error || "") }));
  banner("");
  if (!(await ask(t("Apply on {name}?", { name: v.name }), (what ? "<p>" + esc(what) + "</p>" : "") + "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>",
    t(p.confirm_name ? "Continue" : "Apply")))) return;
  let confirm = "";
  if (p.confirm_name) {
    const fd = await ask(t("Type {name} to confirm", { name: v.name }),
      "<label>" + esc(t("Switch name")) + '<input name="confirm" required autocomplete="off" pattern="' + esc(v.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")) + '"></label>', t("Apply"));
    if (!fd) return;
    confirm = String(fd.get("confirm") || "");
  }
  let r;
  try { r = await post("/api/system/apply", { switch: v.name, op, confirm }); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.ok && r.restarting) { banner(""); return show("infra/" + encodeURIComponent(v.name)); } // its restart card follows the switch
  banner(r.ok ? (r.verified === false ? t("Applied, but the switch reads back differently.") : t("Done on {name}.", { name: v.name })) : t("Refused: {err}", { err: t(r.error || "") }), r.ok ? "ok" : "");
  await loadSys();
}

// Form fields: [name, label, value, kind] -> html; kind: text, area, check, select:<a|b|c>, password.
function sysFields(fields) {
  return fields.map(([n, label, val, kind = "text", extra = ""]) => {
    if (kind === "check") return '<label class="check"><input type="checkbox" name="' + n + '"' + (yes(val) ? " checked" : "") + "> " + esc(label) + "</label>";
    if (kind === "area") return "<label>" + esc(label) + '<textarea name="' + n + '" rows="8" spellcheck="false">' + esc(val || "") + "</textarea></label>";
    if (kind.startsWith("select:")) return "<label>" + esc(label) + '<select name="' + n + '">' + kind.slice(7).split("|").map(o => "<option" + (o === val ? " selected" : "") + ">" + esc(o) + "</option>").join("") + "</select></label>";
    return "<label>" + esc(label) + '<input name="' + n + '" type="' + (kind === "password" ? "password" : "text") + '" value="' + esc(val || "") + '" autocomplete="' + (kind === "password" ? "new-password" : "off") + '"' + extra + "></label>";
  }).join("");
}
// The fields that differ from row (all of them for a new row); checkboxes -> yes/no.
function sysDiff(fd, fields, row) {
  const set = {};
  for (const [n, , , kind = "text"] of fields) {
    const val = kind === "check" ? (fd.get(n) === "on" ? "yes" : "no") : String(fd.get(n) ?? "").replace(/\r\n/g, "\n");
    const cur = row ? (kind === "check" ? (yes(row[n]) ? "yes" : "no") : String(row[n] ?? "")) : null;
    if (kind === "password" && !val) continue;
    if (cur === null ? val !== "" || kind === "check" : val !== cur) set[n] = val;
  }
  return set;
}
async function sysForm(title, t_, fields, row, help = "") {
  const fd = await ask(title, sysFields(fields) + (help ? '<p class="sub">' + esc(help) + "</p>" : ""), t("Preview"));
  if (!fd) return;
  const set = sysDiff(fd, fields, row);
  if (!Object.keys(set).length) return banner(t("Nothing to change."), "ok");
  return sysOp(Object.assign({ t: t_, set }, row && row[".id"] ? { id: row[".id"] } : {}));
}
const sysDel = async (t_, row, label) => {
  if (!(await ask(t("Delete {what}?", { what: label }), "<p>" + esc(t("On {name}. This cannot be undone from Cairn Net.", { name: SYS.name })) + "</p>", t("Continue")))) return;
  return sysOp({ t: t_, id: row[".id"], del: true });
};

async function sysUserNew() {
  const v = SYS;
  const fd = await ask(t("New user on {name}", { name: v.name }), sysFields([["name", t("Name")], ["group", t("Group"), "read", "select:" + (v.groups || []).map(g => g.name).join("|")],
    ["password", t("Password"), "", "password", " required"], ["password2", t("Password again"), "", "password", " required"], ["address", t("Allowed from (empty: everywhere)")], ["comment", t("Comment")]]) +
    '<p class="sub">' + esc(t("The password goes to the switch only: Cairn Net neither keeps nor logs it.")) + "</p>", t("Preview"));
  if (!fd) return;
  if (fd.get("password") !== fd.get("password2")) return banner(t("The two passwords differ."));
  const set = { name: String(fd.get("name")).trim(), group: String(fd.get("group")), password: String(fd.get("password")) };
  for (const k of ["address", "comment"]) if (String(fd.get(k)).trim()) set[k] = String(fd.get(k)).trim();
  return sysOp({ t: "user", set });
}

async function sysExport(sensitive) {
  const v = SYS;
  if (sensitive && !(await ask(t("Export with secrets?"), "<p>" + esc(t("The file will hold the switch's passwords and keys in clear. Keep it somewhere safe.")) + "</p>", t("Download")))) return;
  banner(t("Exporting {name}…", { name: v.name }), "ok");
  let r;
  try { r = await post("/api/system/export", { switch: v.name, sensitive }); } catch (e) { r = { ok: false, error: e.message }; }
  if (!r.ok) return banner(t("Refused: {err}", { err: t(r.error || "") }));
  sysSave(new Blob([r.text], { type: "text/plain" }), v.name + "-" + new Date().toISOString().slice(0, 10) + (sensitive ? "-secrets" : "") + ".rsc");
  banner("");
}
function sysSave(blob, name) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
}

async function sysUpload() {
  const v = SYS;
  const fd = await ask(t("Send a file to {name}", { name: v.name }), '<label>' + esc(t("File (60 KB at most: the switch takes no bigger file through its API)")) + '<input type="file" name="file" required></label>' +
    "<label>" + esc(t("Name on the switch (empty: the file's name)")) + '<input name="name" autocomplete="off"></label>', t("Send"));
  if (!fd) return;
  const f = fd.get("file");
  if (!f || !f.size) return;
  if (f.size > 60000) return banner(t("Larger than 60 KB: send it over SFTP (SSH)."));
  const buf = new Uint8Array(await f.arrayBuffer());
  let bin = "";
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
  banner(t("Sending {file}…", { file: f.name }), "ok");
  let r;
  try { r = await post("/api/system/upload", { switch: v.name, name: String(fd.get("name") || "").trim() || f.name, data: btoa(bin) }); } catch (e) { r = { ok: false, error: e.message }; }
  banner(r.ok ? t("Sent: {n} read back on the switch.", { n: bytes(r.size) }) : t("Refused: {err}", { err: t(r.error || (r.verified === false ? "size read back differs" : "")) }), r.ok ? "ok" : "");
  await loadSys();
}

async function sysRestore(f) {
  const v = SYS;
  if (!(await ask(t("Restore {file} on {name}?", { file: f.name, name: v.name }),
    "<p>" + esc(t("Cairn Net first saves the current state as cairn-net-before-restore.backup, then the switch loads this backup and restarts. Its accounts, addresses and passwords become those of the backup: if they differ from what Settings hold, Cairn Net may lose its way in.")) + "</p>", t("Continue")))) return;
  const fd = await ask(t("Type {name} to confirm", { name: v.name }),
    "<label>" + esc(t("Backup password (empty if not encrypted)")) + '<input type="password" name="password" autocomplete="off"></label>' +
    "<label>" + esc(t("Switch name")) + '<input name="confirm" required autocomplete="off" pattern="' + esc(v.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")) + '"></label>', t("Restore and restart"));
  if (!fd) return;
  banner(t("Restoring {file} on {name}…", { file: f.name, name: v.name }), "ok");
  let r;
  try { r = await post("/api/system/restore", { switch: v.name, name: f.name, password: String(fd.get("password") || ""), confirm: String(fd.get("confirm") || "") }); } catch (e) { r = { ok: false, error: e.message }; }
  if (!r.ok) return banner(t("Refused: {err}", { err: t(r.error || "") }));
  banner(t("{name} restarts with {file}; the state before is in {before}.", { name: v.name, file: f.name, before: r.before }), "ok");
  show("infra/" + encodeURIComponent(v.name)); // its restart card follows the switch until it answers again
}

async function sysProfile() {
  const v = SYS;
  SYS_PROF = { name: v.name, busy: true };
  render(true);
  let r;
  try { r = await post("/api/system/profile", { switch: v.name, seconds: 5 }); } catch (e) { r = { ok: false, error: e.message }; }
  SYS_PROF = r.ok ? { name: v.name, rows: r.profile } : { name: v.name, error: t("Refused: {err}", { err: t(r.error || "") }) };
  render(true);
}

function sysAct(d) {
  const v = SYS, i = +d.i;
  const RULE = [["topics", t("Topics (comma-separated, ! to leave out)"), "", "text"], ["action", t("Destination"), "memory", "select:" + (v.log_actions || []).map(a => a.name).join("|")], ["prefix", t("Prefix")]];
  const REMOTE = [["name", t("Name"), "remote"], ["remote", t("Syslog server")], ["remote-port", t("Port"), "514"], ["remote-protocol", t("Protocol"), "udp", "select:udp|tcp"],
    ["remote-log-format", t("Format"), "default", "select:default|bsd-syslog|syslog"]];
  const SCHED = [["name", t("Name")], ["start-date", t("Start date")], ["start-time", t("Start time"), "startup"], ["interval", t("Every (e.g. 1d, 10m; empty: once)")],
    ["on-event", t("Commands"), "", "area"], ["comment", t("Comment")], ["disabled", t("Disabled"), "no", "check"]];
  const SCRIPT = [["name", t("Name")], ["source", t("Source"), "", "area"], ["comment", t("Comment")]];
  switch (d.act) {
    case "sys-part": SYS_PART = d.part; try { localStorage.setItem("cairn-sys-part", SYS_PART); } catch { /* not kept */ } if (SYS_PART === "log") loadSysLog().then(() => render(true)); return render(true);
    case "sys-clock": return sysForm(t("Time zone of {name}", { name: v.name }), "clock", [["time-zone-name", t("Time zone (e.g. Europe/Paris)"), (v.clock || {})["time-zone-name"]],
      ["time-zone-autodetect", t("Guess it from the IP address"), (v.clock || {})["time-zone-autodetect"], "check"]], v.clock);
    case "sys-ntp": return sysForm(t("NTP client of {name}", { name: v.name }), "ntp", [["enabled", t("On"), (v.ntp || {}).enabled, "check"],
      ["servers", t("Servers (names or addresses, comma-separated)"), (v.ntp || {}).servers]], v.ntp);
    case "sys-profile": return sysProfile();
    case "sys-rule": { const r = d.i != null ? v.log_rules[i] : null; return sysForm(t(r ? "Logging rule" : "New logging rule"), "log-rule", RULE.map(f => [f[0], f[1], r ? r[f[0]] : f[2], f[3]]), r); }
    case "sys-rule-toggle": { const r = v.log_rules[i]; return sysOp({ t: "log-rule", id: r[".id"], set: { disabled: yes(r.disabled) ? "no" : "yes" } }); }
    case "sys-rule-del": return sysDel("log-rule", v.log_rules[i], v.log_rules[i].topics);
    case "sys-remote": {
      const r = d.i != null ? v.log_actions[i] : null;
      const f = REMOTE.map(x => [x[0], x[1], r ? r[x[0]] : x[2], x[3]]);
      if (r) return sysForm(t("Remote syslog"), "log-action", f, r);
      return ask(t("New remote syslog"), sysFields(f) + '<p class="sub">' + esc(t("Then add a logging rule that sends topics to it.")) + "</p>", t("Preview")).then(fd => fd &&
        sysOp({ t: "log-action", set: Object.assign({ target: "remote" }, sysDiff(fd, f, null)) }));
    }
    case "sys-action-del": return sysDel("log-action", v.log_actions[i], v.log_actions[i].name);
    case "sys-user-new": return sysUserNew();
    case "sys-user-addr": { const u = v.users[i]; return sysForm(t("{user} on {name}", { user: u.name, name: v.name }), "user", [["address", t("Allowed from (empty: everywhere)"), u.address], ["comment", t("Comment"), u.comment]], u); }
    case "sys-user-del": return sysDel("user", v.users[i], v.users[i].name);
    case "sys-sched": { const r = d.i != null ? v.scheds[i] : null; return sysForm(t(r ? "Task {n}" : "New task", { n: r && r.name }), "sched", SCHED.map(f => [f[0], f[1], r ? r[f[0]] : f[2], f[3]]), r); }
    case "sys-sched-del": return sysDel("sched", v.scheds[i], v.scheds[i].name);
    case "sys-script": { const r = d.i != null ? v.scripts[i] : null; return sysForm(t(r ? "Script {n}" : "New script", { n: r && r.name }), "script", SCRIPT.map(f => [f[0], f[1], r ? r[f[0]] : f[2], f[3]]), r); }
    case "sys-script-del": return sysDel("script", v.scripts[i], v.scripts[i].name);
    case "sys-script-run": { const r = v.scripts[i]; return sysOp({ t: "script", id: r[".id"], run: true }, t("The script runs on the switch with its own rights; Cairn Net cannot undo what it does.")); }
    case "sys-backup": return ask(t("Backup of {name}", { name: v.name }), sysFields([["name", t("File name"), v.name + "-" + new Date().toISOString().slice(0, 10)], ["password", t("Password (empty: not encrypted)"), "", "password"]]) +
      '<p class="sub">' + esc(t("Kept on the switch, in Files: download it from there.")) + "</p>", t("Preview")).then(fd => fd && sysOp({ t: "backup", name: String(fd.get("name")).trim(), password: String(fd.get("password") || "") }));
    case "sys-export": return sysExport(false);
    case "sys-export-s": return sysExport(true);
    case "sys-upload": return sysUpload();
    case "sys-file-del": return ask(t("Delete {what}?", { what: v.files[i].name }), "<p>" + esc(t("On {name}. This cannot be undone from Cairn Net.", { name: v.name })) + "</p>", t("Continue"))
      .then(ok => ok && sysOp({ t: "file", name: v.files[i].name, del: true }));
    case "sys-restore": return sysRestore(v.files[i]);
    case "sys-pkg": {
      const p = v.packages[i], what = { enable: "{p} is enabled at the next restart.", disable: "{p} is disabled at the next restart: what it provides stops working.",
        uninstall: "{p} is removed at the next restart, with its settings.", unschedule: "What was planned for {p} at the next restart is cancelled." }[d.do];
      return sysOp({ t: "pkg", name: p.name, do: d.do }, t(what, { p: p.name }));
    }
    case "sys-pkg-apply": return sysOp({ t: "pkg-apply" }, t("The switch downloads what is needed, then restarts to install the package changes. Everything behind it is cut while it restarts; there is no rollback."));
    case "sys-rb": return sysOp({ t: "rb-upgrade" }, t("The firmware is written now and used from the next restart. Never cut the power while the switch restarts."));
    case "sys-snmp": return sysForm(t("SNMP of {name}", { name: v.name }), "snmp", [["enabled", t("On"), (v.snmp || {}).enabled, "check"], ["contact", t("Contact"), (v.snmp || {}).contact],
      ["location", t("Location"), (v.snmp || {}).location], ["trap-target", t("Traps to"), (v.snmp || {})["trap-target"]]], v.snmp);
    case "sys-community": { const c = v.communities[i]; return sysForm(t("Community {n}", { n: c.name }), "snmp-community", [["addresses", t("Allowed from (e.g. 192.168.1.0/24)"), c.addresses],
      ["disabled", t("Disabled"), c.disabled, "check"]], c); }
  }
}

document.addEventListener("click", e => {
  const b = e.target.closest("button[data-act]");
  if (!b || !String(b.dataset.act).startsWith("sys-") || !SYS || SYS.error) return;
  sysAct(b.dataset);
});
document.addEventListener("input", e => { if (e.target.closest && e.target.closest(".log-filters") && VIEW === "system") render(true); });
// live log: every 3 s while it is shown
setInterval(() => {
  if (VIEW !== "system" || SYS_PART !== "log" || !($("sys-live") || {}).checked || document.hidden) return;
  loadSysLog().then(() => { if (VIEW === "system") renderSys(); });
}, 3000);
