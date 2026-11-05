// cairn-net: Switch chip page (loaded after panel.js, uses its helpers):
// the switch chip (CRS3xx: mirror, rate limits, storm control, hardware ACL
// rules, port isolation, counters, bridge offload), IGMP / MLD snooping,
// queues and PoE (src/chip.rs). Every change is a list of row operations
// {table, act: set|add|remove, id, values}, previewed, applied with the
// rollback armed on the switch, then kept (same rollback as Infrastructure).
// A table the switch lacks reads empty: its section stays hidden.
"use strict";

let CH = null, CH_UNTIL = 0, CH_HTML = ""; // /api/chip of SW, when the switch rolls back (ms), last markup

Object.assign(DICT.fr, {
  "Chip & QoS": "Puce et QoS", "Switch chip": "Puce du switch", "Hardware offload": "Déchargement matériel", "Through the CPU (own chip)": "Par le processeur (puce à part)",
  "Alone on its own switch chip: the CPU forwards for it. Normal for a management port.": "Seul sur sa propre puce : le processeur transmet pour lui. Normal pour un port de gestion.",
  "{n} of {m} bridge ports in hardware": "{n} ports du pont sur {m} en matériel", "lost on {v}": "perdu sur {v}",
  "Mirror": "Miroir", "Mirror target": "Cible du miroir", "Mirrored ports": "Ports copiés", "in": "entrée", "out": "sortie",
  "Mirror…": "Miroir…", "Capture on the target": "Capturer sur la cible",
  "No switch chip on this device: mirror, rate limits, storm control, hardware rules and port isolation do not exist here.":
    "Pas de puce de switch sur cet appareil : miroir, limites de débit, contrôle des tempêtes, règles matérielles et isolation de port n'existent pas ici.",
  "Received": "Reçu", "Sent": "Envoyé", "FCS errors": "Erreurs FCS", "Drops": "Pertes", "Pause": "Pause",
  "Limit in": "Limite entrée", "Limit out": "Limite sortie", "Storm control": "Tempêtes", "Isolation": "Isolation", "Offload": "Matériel",
  "Hardware counters and port settings": "Compteurs matériels et réglages des ports",
  "Hardware rules (ACL)": "Règles matérielles (ACL)",
  "forward": "transmettre", "drop": "jeter", "No hardware rule.": "Aucune règle matérielle.",
  "Multicast (IGMP / MLD snooping)": "Multicast (IGMP / MLD snooping)", "Snooping": "Snooping", "Querier": "Interrogateur",
  "Multicast groups": "Groupes multicast", "Group": "Groupe", "No multicast group learnt.": "Aucun groupe multicast appris.",
  "Simple queues": "Files simples", "Queue tree": "Arbre de files", "Queue types": "Types de file",
  "No queue.": "Aucune file.", "built-in": "intégré", "Target": "Cible", "Max limit": "Limite max", "Limit at": "Débit garanti", "Parent": "Parent", "Type": "Type",
  "Queues only see the traffic the CPU handles: what the switch chip forwards in hardware never reaches them.":
    "Les files ne voient que le trafic traité par le processeur : ce que la puce transmet en matériel ne les traverse jamais.",
  "PoE": "PoE", "Priority": "Priorité", "Packet mark": "Marque de paquet", "Direct mode only": "Mode direct seulement",
  "{table} on {name}": "{table} sur {name}", "Empty: RouterOS default. Fields left as they are are not sent.": "Vide : valeur par défaut de RouterOS. Les champs non modifiés ne sont pas envoyés.",
  "Hardware offload lost on {v}: their traffic now goes through the CPU. Let the rollback undo it, or keep it knowingly.":
    "Déchargement matériel perdu sur {v} : leur trafic passe maintenant par le processeur. Laissez le retour automatique annuler, ou gardez en connaissance de cause.",
  "Remove this row?": "Supprimer cette ligne ?", "Unchanged": "Inchangé",
  "Mirror on {name}": "Miroir sur {name}", "Target port (receives the copy)": "Port cible (reçoit la copie)",
});

const CH_BOOL = new Set(["mirror-ingress", "mirror-egress", "limit-broadcasts", "limit-unknown-multicasts", "limit-unknown-unicasts", "disabled",
  "redirect-to-cpu", "copy-to-cpu", "mirror", "igmp-snooping", "multicast-querier", "fast-leave"]);
const CH_TITLE = { port: "Port", isolation: "Isolation", rule: "Hardware rules (ACL)", bridge: "Multicast (IGMP / MLD snooping)", bport: "Port",
  simple: "Simple queues", tree: "Queue tree", type: "Queue types", poe: "PoE", switch: "Switch chip" };

function chipShow() { if (!CH || CH.name !== SW) { CH = null; loadChip(); } }

async function loadChip() {
  const name = SW;
  if (!name) return;
  try { const v = await get("/api/chip?switch=" + encodeURIComponent(name)); if (name === SW) CH = v; }
  catch (e) { if (name === SW) CH = { name, error: e.message }; }
  if (VIEW === "chip") render(true);
}

const chRows = (v, k) => (v && v[k]) || [];
const chRate = s => String(s || "").split("/").map(x => { const n = Number(x); return !x || isNaN(n) || n < 1000 ? x : n % 1e9 === 0 ? n / 1e9 + "G" : n % 1e6 === 0 ? n / 1e6 + "M" : n % 1e3 === 0 ? n / 1e3 + "k" : x; }).join("/");
const chMgmt = (v, n) => chRows(v, "mgmt").includes(n);
const chCan = v => v.mode === "direct";
function chBtn(v, act, label, extra = "", cls = "") {
  return '<button' + (cls ? ' class="' + cls + '"' : "") + ' data-act="' + act + '"' + extra + (chCan(v) ? "" : ' disabled title="' + esc(t("Direct mode only")) + '"') + ">" + esc(t(label)) + "</button>";
}
const chKv = rows => '<dl class="kv">' + rows.map(([k, val, c]) => "<dt>" + esc(k) + '</dt><dd class="' + (c || "") + '">' + esc(val) + "</dd>").join("") + "</dl>";
const chTable = (head, body, empty) => '<div class="card table-wrap"><table><thead><tr><th>' + head.map(h => esc(t(h))).join("</th><th>") + "</th></tr></thead><tbody>" +
  (body || '<tr><td colspan="' + head.length + '" class="sub">' + esc(t(empty)) + "</td></tr>") + "</tbody></table></div>";
const chRowActs = (v, table, r, remove = true) => '<td class="actions">' + chBtn(v, "ch-edit", "Edit", ' data-t="' + table + '" data-id="' + esc(r[".id"]) + '"') +
  (remove ? chBtn(v, "ch-del", "Delete", ' data-t="' + table + '" data-id="' + esc(r[".id"]) + '"', "danger") : "") + "</td>";

function chipPending() {
  const box = $("chip-pending"), v = CH;
  const left = Math.max(0, Math.ceil((CH_UNTIL - Date.now()) / 1000));
  const waiting = v && !v.error && (left > 0 || v.rollback);
  box.hidden = !waiting;
  if (!waiting) return;
  box.innerHTML = '<div class="card-head"><h3>' + esc(t("Keep these settings on {name}?", { name: v.name })) + '</h3><span class="pill warn">' +
    esc(left > 0 ? t("rollback in {n} s", { n: left }) : t("rollback at {time}", { time: (v.rollback || {})["next-run"] || "?" })) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net read the switch back as asked. Without your confirmation, the switch puts the old settings back by itself.")) + "</p>" +
    '<div class="toolbar"><button class="primary" data-act="ch-keep">' + esc(t("Keep")) + "</button></div>";
}

function renderChip() {
  $("chip-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="chip/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  chipPending();
  const v = CH && CH.name === SW ? CH : null;
  let html;
  if (!v || v.error) {
    html = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
  } else html = chipHtml(v);
  if (html !== CH_HTML) { $("chip-body").innerHTML = html; CH_HTML = html; paint($("chip-body")); }
}

function chipHtml(v) {
  const sws = chRows(v, "switch"), ports = chRows(v, "port"), bports = chRows(v, "bport"), off = chRows(v, "offload_off"), cpu = chRows(v, "cpu_ports");
  const hwPorts = bports.filter(p => yes(p.hw)), mirrored = ports.filter(p => yes(p["mirror-ingress"]) || yes(p["mirror-egress"]));
  const target = (sws.find(s => s["mirror-target"] && s["mirror-target"] !== "none") || {})["mirror-target"];
  let h = '<div class="grid srvs">';
  if (sws.length) {
    h += '<article class="card srv"><div class="card-head"><h3>' + esc(t("Switch chip")) + "</h3>" + chBtn(v, "ch-mirror", "Mirror…") + "</div>" + chKv([
      ...sws.map(s => [s.name, s.type || "—"]),
      [t("Hardware offload"), t("{n} of {m} bridge ports in hardware", { n: hwPorts.filter(p => !cpu.includes(p.interface)).length - off.length, m: bports.length - cpu.length }) + (off.length ? " · " + t("lost on {v}", { v: off.join(", ") }) : ""), off.length ? "warn" : "ok"],
      ...(cpu.length ? [[t("Through the CPU (own chip)"), cpu.join(", ")]] : []),
      [t("Mirror target"), target || t("none")],
      [t("Mirrored ports"), mirrored.map(p => p.name + " (" + [yes(p["mirror-ingress"]) && t("in"), yes(p["mirror-egress"]) && t("out")].filter(Boolean).join("+") + ")").join(", ") || t("none")],
    ]) + (target && typeof captureOn === "function" ? '<div class="toolbar"><button data-act="ch-capture" data-port="' + esc(target) + '">' + esc(t("Capture on the target")) + "</button></div>" : "") + "</article>";
  } else {
    h += '<article class="card srv"><div class="card-head"><h3>' + esc(t("Switch chip")) + '</h3></div><p class="sub">' +
      esc(t("No switch chip on this device: mirror, rate limits, storm control, hardware rules and port isolation do not exist here.")) + "</p></article>";
  }
  h += "</div>";

  if (sws.length) {
    const iso = n => (chRows(v, "isolation").find(x => x.name === n) || {})["forwarding-override"];
    const hw = n => bports.find(p => p.interface === n);
    const storm = p => (p["storm-rate"] || "100") + " %" + (["limit-broadcasts", "limit-unknown-multicasts", "limit-unknown-unicasts"].filter(k => yes(p[k]))
      .map(k => " " + { "limit-broadcasts": "B", "limit-unknown-multicasts": "M", "limit-unknown-unicasts": "U" }[k]).join(""));
    h += "<h2>" + esc(t("Hardware counters and port settings")) + "</h2>" + chTable(["Port", "Received", "Sent", "FCS errors", "Drops", "Pause", "Limit in", "Limit out", "Storm control", "Isolation", "Mirror", "Offload", ""],
      ports.slice().sort((a, b) => natural(a.name, b.name)).map(p => {
        const b = hw(p.name), errs = num(p["rx-fcs-error"]);
        return '<tr class="' + (yes(p.running) ? "" : "dim") + '"><td><b>' + esc(short(p.name)) + "</b>" + (chMgmt(v, p.name) ? ' <span class="pill">' + esc(t("Cairn Net's path")) + "</span>" : "") +
          "</td><td>" + esc(bytes(num(p["rx-bytes"]))) + "</td><td>" + esc(bytes(num(p["tx-bytes"]))) + '</td><td class="' + (errs ? "warn" : "") + '">' + esc(errs) +
          "</td><td>" + esc(num(p["tx-drop-packet"]) + num(p["rx-overflow"])) + "</td><td>" + esc(num(p["rx-pause"]) + num(p["tx-pause"])) +
          "</td><td>" + esc(chRate(p["ingress-rate"]) || "—") + "</td><td>" + esc(chRate(p["egress-rate"]) || "—") + "</td><td>" + esc(storm(p)) + '</td><td class="wrap">' + esc(iso(p.name) || "—") +
          "</td><td>" + esc([yes(p["mirror-ingress"]) && t("in"), yes(p["mirror-egress"]) && t("out")].filter(Boolean).join("+") || "—") + "</td><td>" +
          (b ? cpu.includes(p.name) ? '<span class="pill" title="' + esc(t("Alone on its own switch chip: the CPU forwards for it. Normal for a management port.")) + '">CPU</span>'
            : '<span class="pill ' + (yes(b["hw-offload"]) ? "ok" : yes(b.hw) ? "warn" : "") + '">' + esc(yes(b["hw-offload"]) ? t("yes") : t("no")) + "</span>" : "—") + "</td>" +
          chRowActs(v, "port", p, false) + "</tr>";
      }).join(""), "");
    const act = r => r["new-dst-ports"] === "" || r["new-dst-ports"] === "none" ? t("drop") : [r["new-dst-ports"] && t("to {v}", { v: r["new-dst-ports"] }),
      r["new-vlan-id"] && "VLAN " + r["new-vlan-id"], yes(r["redirect-to-cpu"]) && "CPU", yes(r["copy-to-cpu"]) && "+CPU", yes(r.mirror) && t("Mirror"), r.rate && r.rate].filter(Boolean).join(" · ") || t("forward");
    const match = r => ["mac-protocol", "src-mac-address", "dst-mac-address", "vlan-id", "protocol", "src-address", "dst-address", "src-port", "dst-port", "dscp"]
      .filter(k => r[k]).map(k => k + "=" + r[k]).join(" ");
    h += '<div class="card-head"><h2>' + esc(t("Hardware rules (ACL)")) + "</h2>" + chBtn(v, "ch-add", "Add a rule", ' data-t="rule"') + "</div>" +
      chTable(["#", "Ports", "Match", "Action", ""], chRows(v, "rule").map((r, i) => '<tr class="' + (yes(r.disabled) ? "dim" : "") + '"><td>' + (i + 1) + '</td><td class="wrap">' + esc(r.ports || "") +
        '</td><td class="wrap mono">' + esc(match(r) || "*") + "</td><td>" + esc(act(r)) + (r.comment ? '<br><span class="sub">' + esc(r.comment) + "</span>" : "") + "</td>" + chRowActs(v, "rule", r) + "</tr>").join(""), "No hardware rule.");
  }

  const onOff = x => '<span class="pill ' + (yes(x) ? "ok" : "") + '">' + esc(yes(x) ? t("on") : t("off")) + "</span>";
  h += "<h2>" + esc(t("Multicast (IGMP / MLD snooping)")) + "</h2>" + chTable(["Bridge", "Snooping", "IGMP", "MLD", "Querier", ""], chRows(v, "bridge").map(b => "<tr><td><b>" + esc(b.name) + "</b></td><td>" +
    onOff(b["igmp-snooping"]) + "</td><td>" + esc(b["igmp-version"] ? "v" + b["igmp-version"] : "—") + "</td><td>" + esc(b["mld-version"] ? "v" + b["mld-version"] : "—") + "</td><td>" +
    (b["multicast-querier"] != null ? onOff(b["multicast-querier"]) : "—") + "</td>" + chRowActs(v, "bridge", b, false) + "</tr>").join(""), "");
  if (chRows(v, "bridge").some(b => yes(b["igmp-snooping"])) || chRows(v, "mdb").length) {
    h += "<h2>" + esc(t("Multicast groups")) + "</h2>" + chTable(["Group", "VLAN", "Ports", "Bridge"], chRows(v, "mdb").map(m => "<tr><td><b>" + esc(m.group) + "</b></td><td>" + esc(m.vid || "—") +
      '</td><td class="wrap">' + esc(m["on-ports"] || m.ports || "") + "</td><td>" + esc(m.bridge || "") + "</td></tr>").join(""), "No multicast group learnt.");
  }

  const qhead = (title, table) => '<div class="card-head"><h2>' + esc(t(title)) + "</h2>" + chBtn(v, "ch-add", "Add", ' data-t="' + table + '"') + "</div>";
  h += qhead("Simple queues", "simple") + chTable(["Name", "Target", "Max limit", "Limit at", "Type", "Parent", ""], chRows(v, "simple").map(r => '<tr class="' + (yes(r.disabled) ? "dim" : "") + '"><td><b>' + esc(r.name) +
    '</b></td><td class="wrap">' + esc(r.target || "") + "</td><td>" + esc(chRate(r["max-limit"])) + "</td><td>" + esc(chRate(r["limit-at"])) + "</td><td>" + esc(r.queue || "") + "</td><td>" + esc(r.parent || "") + "</td>" +
    (yes(r.dynamic) ? '<td class="sub">' + esc(t("built-in")) + "</td>" : chRowActs(v, "simple", r)) + "</tr>").join(""), "No queue.");
  h += qhead("Queue tree", "tree") + chTable(["Name", "Parent", "Packet mark", "Max limit", "Limit at", "Type", ""], chRows(v, "tree").map(r => '<tr class="' + (yes(r.disabled) ? "dim" : "") + '"><td><b>' + esc(r.name) +
    "</b></td><td>" + esc(r.parent || "") + "</td><td>" + esc(r["packet-mark"] || "") + "</td><td>" + esc(chRate(r["max-limit"])) + "</td><td>" + esc(chRate(r["limit-at"])) + "</td><td>" + esc(r.queue || "") + "</td>" +
    (yes(r.dynamic) ? '<td class="sub">' + esc(t("built-in")) + "</td>" : chRowActs(v, "tree", r)) + "</tr>").join(""), "No queue.");
  h += qhead("Queue types", "type") + chTable(["Name", "Kind", ""], chRows(v, "type").map(r => "<tr><td><b>" + esc(r.name) + "</b></td><td>" + esc(r.kind) + "</td>" +
    (yes(r.default) ? '<td class="sub">' + esc(t("built-in")) + "</td>" : chRowActs(v, "type", r)) + "</tr>").join(""), "No queue.");
  if (sws.length) h += '<p class="sub">' + esc(t("Queues only see the traffic the CPU handles: what the switch chip forwards in hardware never reaches them.")) + "</p>";

  if (chRows(v, "poe").length) {
    h += "<h2>" + esc(t("PoE")) + "</h2>" + chTable(["Port", "PoE", "Priority", ""], chRows(v, "poe").map(r => "<tr><td><b>" + esc(r.name) + "</b>" +
      (chMgmt(v, r.name) ? ' <span class="pill">' + esc(t("Cairn Net's path")) + "</span>" : "") + "</td><td>" + esc(r["poe-out"] || "") + "</td><td>" + esc(r["poe-priority"] || "") + "</td>" + chRowActs(v, "poe", r, false) + "</tr>").join(""), "");
  }
  return h;
}

/* ---------- forms and apply ---------- */

function chField(k, val, names) {
  if (CH_BOOL.has(k)) {
    return "<label>" + esc(k) + '<select name="f-' + esc(k) + '">' + [["", t("Unchanged")], ["yes", t("yes")], ["no", t("no")]].map(([o, l]) =>
      '<option value="' + o + '"' + ((yes(val) ? "yes" : val === "false" || val === "no" ? "no" : "") === o ? " selected" : "") + ">" + esc(l) + "</option>").join("") + "</select></label>";
  }
  return "<label>" + esc(k) + '<input name="f-' + esc(k) + '" value="' + esc(val || "") + '"' + (names ? ' list="ch-names"' : "") + ' autocapitalize="off" spellcheck="false"></label>';
}

// One row: a field per editable property, prefilled; only what changed is sent.
async function chForm(table, r) {
  const v = CH, fields = ((v.fields || {})[table] || {}).fields || [];
  const names = [...chRows(v, "port").map(p => p.name), ...chRows(v, "bport").map(p => p.interface), ...chRows(v, "bridge").map(b => b.name)];
  const nameFields = new Set(["ports", "target", "parent", "new-dst-ports", "forwarding-override", "mirror-target"]);
  const cur = r || {};
  const extra = table === "port" ? chRows(v, "isolation").find(x => x.name === cur.name) : null;
  const fd = await ask(t("{table} on {name}", { table: t(CH_TITLE[table] || table) + (cur.name || cur.interface ? " " + (cur.name || cur.interface) : ""), name: v.name }),
    '<datalist id="ch-names">' + [...new Set(names)].map(n => '<option value="' + esc(n) + '">').join("") + "</datalist>" +
    fields.map(k => chField(k, cur[k], nameFields.has(k))).join("") + (extra ? chField("forwarding-override", extra["forwarding-override"], true) : "") +
    '<p class="sub">' + esc(t("Empty: RouterOS default. Fields left as they are are not sent.")) + "</p>", t("Preview"));
  if (!fd) return;
  const pick = (keys, row) => {
    const out = {};
    for (const k of keys) {
      let val = String(fd.get("f-" + k) ?? "").trim();
      const was = String(row[k] ?? "");
      if (CH_BOOL.has(k)) { if (!val || (val === "yes") === yes(was) && was !== "") continue; }
      else if (val === was) continue;
      out[k] = val;
    }
    return out;
  };
  const ops = [];
  const vals = pick(fields, cur);
  if (!r) { if (Object.keys(vals).length) ops.push({ table, act: "add", values: vals }); }
  else if (Object.keys(vals).length) ops.push({ table, act: "set", id: r[".id"], values: vals });
  if (extra) { const iv = pick(["forwarding-override"], extra); if (Object.keys(iv).length) ops.push({ table: "isolation", act: "set", id: extra[".id"], values: iv }); }
  if (!ops.length) return banner(t("Nothing to change."), "ok");
  return chipSend(ops);
}

// Mirror: the chip's target and, per port, what is copied to it.
async function chMirror() {
  const v = CH, sw = chRows(v, "switch")[0], ports = chRows(v, "port").slice().sort((a, b) => natural(a.name, b.name));
  const tgt = sw["mirror-target"] || "none";
  const fd = await ask(t("Mirror on {name}", { name: v.name }),
    "<label>" + esc(t("Target port (receives the copy)")) + '<select name="target">' + ["none", ...ports.map(p => p.name)].map(n => "<option" + (n === tgt ? " selected" : "") + ">" + esc(n) + "</option>").join("") + "</select></label>" +
    '<fieldset><legend>' + esc(t("Mirrored ports")) + "</legend>" + ports.map(p => '<div class="cols2"><span>' + esc(p.name) + '</span><span><label class="check"><input type="checkbox" name="in-' + esc(p[".id"]) + '"' +
      (yes(p["mirror-ingress"]) ? " checked" : "") + "> " + esc(t("in")) + '</label> <label class="check"><input type="checkbox" name="out-' + esc(p[".id"]) + '"' + (yes(p["mirror-egress"]) ? " checked" : "") + "> " + esc(t("out")) + "</label></span></div>").join("") + "</fieldset>",
    t("Preview"));
  if (!fd) return;
  const ops = [];
  if (fd.get("target") !== tgt) ops.push({ table: "switch", act: "set", id: sw[".id"], values: { "mirror-target": String(fd.get("target")) } });
  for (const p of ports) {
    const values = {};
    for (const [k, f] of [["mirror-ingress", "in-"], ["mirror-egress", "out-"]]) if ((fd.get(f + p[".id"]) === "on") !== yes(p[k])) values[k] = fd.get(f + p[".id"]) === "on" ? "yes" : "no";
    if (Object.keys(values).length) ops.push({ table: "port", act: "set", id: p[".id"], values });
  }
  if (!ops.length) return banner(t("Nothing to change."), "ok");
  return chipSend(ops);
}

async function chipSend(ops) {
  const v = CH;
  let p;
  try { p = await post("/api/chip/preview", { switch: v.name, ops }); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: problemText(p.error || "") }));
  if (!p.commands.length) return banner(t("Nothing to change."), "ok");
  banner("");
  if (!(await ask(t("Apply on {name}?", { name: v.name }),
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>",
    t("Apply")))) return;
  let r;
  try { r = await post("/api/chip/apply", { switch: v.name, ops }); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.verified) { CH_UNTIL = Date.now() + r.rollback_s * 1000; banner((r.offload_lost || []).length ? t("Hardware offload lost on {v}: their traffic now goes through the CPU. Let the rollback undo it, or keep it knowingly.", { v: r.offload_lost.join(", ") }) : ""); }
  else {
    if (r.pending) CH_UNTIL = Date.now() + r.rollback_s * 1000;
    banner(r.pending ? t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name: v.name }) + (r.error ? " · " + problemText(r.error) : "") +
      " · " + t("the switch puts the old settings back within {n} s", { n: r.rollback_s }) : t("Refused: {err}", { err: problemText(r.error || "") }));
  }
  await loadChip();
}

async function chKeep() {
  let r;
  try { r = await post("/api/infra/confirm", { switch: CH.name }); } catch (e) { r = { ok: false, error: e.message }; }
  CH_UNTIL = 0;
  banner(r.ok ? t("Kept on {name}.", { name: CH.name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadChip();
}

document.addEventListener("click", async e => {
  const b = e.target.closest("button[data-act^='ch-']");
  if (!b || !CH || CH.error) return;
  const d = b.dataset, row = () => chRows(CH, d.t).find(r => r[".id"] === d.id);
  switch (d.act) {
    case "ch-edit": return row() && chForm(d.t, row());
    case "ch-add": return chForm(d.t, null);
    case "ch-del": {
      const r = row();
      if (r && await ask(t("Remove this row?"), '<p class="mono">' + esc(r.name || r.ports || r[".id"]) + "</p>", t("Preview"))) return chipSend([{ table: d.t, act: "remove", id: r[".id"] }]);
      return;
    }
    case "ch-mirror": return chMirror();
    case "ch-keep": return chKeep();
    case "ch-capture": return captureOn({ switch: CH.name, iface: d.port });
  }
});

if (VIEW === "chip") chipShow(); // the page opened on this view before this file ran

// counters: read again every 20 s while the page is open; the countdown every second
setInterval(() => { if (VIEW === "chip" && !document.hidden) loadChip(); }, 20000);
setInterval(() => {
  if (!CH_UNTIL || VIEW !== "chip") return;
  if (Date.now() >= CH_UNTIL + 3000) { CH_UNTIL = 0; loadChip(); }
  chipPending();
}, 1000);
