// cairn-net: Wi-Fi page (src/wifi.rs), loaded after panel.js (uses its helpers):
// every access point as one network: radios, SSIDs merged by name with the
// differences between access points, the channel map, the clients of all
// access points with their name and IP, connections and roaming read from
// the access points' logs (lot Wi-Fi 1). An access point in direct mode is
// set from its card (lot Wi-Fi 2): channel, width and power per radio, SSIDs
// on or off per radio, client isolation and password per SSID; previewed,
// read back, undone by the access point unless kept.
"use strict";

VIEWS.wifi = "Wi-Fi";
let WF = null;
const WF_DRAWN = {}; // last markup per block: untouched blocks keep their scroll and selection
const WF_UNTIL = {}; // access point -> end of its rollback countdown (ms)

Object.assign(DICT.fr, {
  "Wi-Fi": "Wi-Fi", "Access points": "Bornes", "Channels": "Canaux", "SSIDs": "SSID", "Clients": "Clients", "Connections and roaming": "Connexions et itinérance",
  "Search: name, IP, MAC, SSID, access point…": "Recherche : nom, IP, MAC, SSID, borne…",
  "{n} Wi-Fi client": "{n} client Wi-Fi", "{n} Wi-Fi clients": "{n} clients Wi-Fi", "{n} client": "{n} client", "{n} clients": "{n} clients",
  "{n} access point": "{n} borne", "{n} access points": "{n} bornes", "{n} SSID": "{n} SSID", "{n} SSIDs": "{n} SSID",
  "No access point: the switches with Wi-Fi radios (RouterOS wifi package) show up here.": "Aucune borne : les appareils qui ont des radios Wi-Fi (paquet wifi de RouterOS) apparaissent ici.",
  "All good: {aps}, {clients}, {ssids}": "Tout va bien : {aps}, {clients}, {ssids}",
  "Band": "Bande", "Channel": "Canal", "Width": "Largeur", "Power": "Puissance", "max {n} dBm": "max {n} dBm",
  "on air": "en service", "radio disabled": "désactivée", "{ap} {iface}: {state}": "{ap} {iface} : {state}",
  "{a} and {b} on the same channel": "{a} et {b} sur le même canal", "{a} and {b} overlap": "{a} et {b} se chevauchent",
  "{ssid}: not the same on every access point": "{ssid} : pas pareil sur toutes les bornes",
  "ch {c}": "canal {c}", "No radio on air.": "Aucune radio en service.", "Wi-Fi channels, {band}": "Canaux Wi-Fi, {band}",
  "Differences": "Écarts", "open": "ouvert", "untagged": "non étiqueté",
  "isolated": "isolés", "hidden": "masqué", "SSID off": "coupé", "same everywhere": "pareil partout", "Hidden": "Masqué", "Enabled": "Activé", "Bands": "Bandes",
  "missing on {where}": "absent de {where}", "{what}: {values}": "{what} : {values}",
  "Access point": "Borne", "Signal": "Signal", "Rate ↓ / ↑": "Débit ↓ / ↑", "Now ↓ / ↑": "Trafic ↓ / ↑", "Connected for": "Connecté depuis", "Roams": "Itinérances",
  "not authorized": "non autorisé", "No client matches.": "Aucun client ne correspond.", "No client.": "Aucun client.",
  "When": "Heure", "What happened": "Événement", "disconnected": "déconnecté", "associated": "associé", "disassociated": "dissocié",
  "roamed from {ap}": "arrivé de {ap}", "roamed {a} → {b}": "passé de {a} à {b}", "Nothing in the logs.": "Rien dans les journaux.",
  "Roaming of {name}": "Itinérance de {name}",
  "An access point in direct mode is set from its card, change previewed, read back, and undone by the access point unless kept. Connections and roaming come from the access points' logs, kept in their memory: lost when they restart.":
    "Une borne en mode direct se règle depuis sa carte : aperçu, relecture, et retour arrière par la borne sauf si vous gardez. Connexions et itinérance viennent des journaux des bornes, gardés en mémoire : perdus à leur redémarrage.",
  "Set": "Régler", "Settings of {name}": "Réglages de {name}", "automatic": "automatique", "up to {n} MHz": "jusqu'à {n} MHz",
  "Transmit power (dBm)": "Puissance d'émission (dBm)", "automatic, max {n}": "automatique, max {n}", "radar": "radar",
  "On {band} ({iface})": "En {band} ({iface})", "Clients isolated from each other": "Clients isolés entre eux", "New password": "Nouveau mot de passe",
  "Only what you change is sent. A new password is sent once, never shown again; the read account cannot read it.":
    "Seul ce que vous changez est envoyé. Un nouveau mot de passe est envoyé une fois, jamais réaffiché ; le compte de lecture ne peut pas le lire.",
  "Cairn Net read the access point back as asked. Without your confirmation, it puts the old settings back by itself.":
    "Cairn Net a relu la borne comme demandé. Sans votre confirmation, elle remet d'elle-même les anciens réglages.",
  "rollback pending": "retour arrière en attente",
  "{who}: its clients reconnect": "{who} : ses clients se reconnectent",
  "{who}: off, its clients are cut": "{who} : coupé, ses clients sont déconnectés",
  "{who}: radar (DFS) channel: the radio listens 1 to 10 min before sending, its clients are cut meanwhile":
    "{who} : canal radar (DFS) : la radio écoute 1 à 10 min avant d'émettre, ses clients sont coupés pendant ce temps",
  "{who}: automatic channel: the radio may pick a radar (DFS) channel": "{who} : canal automatique : la radio peut prendre un canal radar (DFS)",
  "{who}: its clients see each other again": "{who} : ses clients se voient de nouveau",
  "{who}: every device on it needs the new password": "{who} : chaque appareil doit recevoir le nouveau mot de passe",
  "a Wi-Fi password is 8 to 63 printable ASCII characters": "un mot de passe Wi-Fi fait 8 à 63 caractères ASCII imprimables",
  "too late: the switch is putting the old settings back": "trop tard : l'appareil remet les anciens réglages",
});

async function loadWifi() {
  try { WF = await get("/api/wifi"); } catch (e) { WF = { error: e.message }; }
  if (VIEW === "wifi") render(true);
}

const wfBand = b => (b ? b + " GHz" : "—");
// "5ghz-ax" -> "5 GHz · Wi-Fi 6"
const WF_GEN = { be: "Wi-Fi 7", ax: "Wi-Fi 6", ac: "Wi-Fi 5", n: "Wi-Fi 4" };
function wfPhy(b) {
  const m = String(b || "").match(/^(\d+)ghz-(\w+)$/);
  return m ? wfBand(m[1] === "2" ? "2.4" : m[1]) + " · " + (WF_GEN[m[2]] || m[2]) : b || "—";
}
// "wpa2-psk,wpa3-psk" -> "WPA2 / WPA3"
const wfSec = v => { const k = list(v); return k.length ? k.map(x => x.toUpperCase().replace(/-PSK$/, "").replace(/-EAP$/, " Enterprise")).join(" / ") : t("open"); };
const wfSignal = s => (s == null ? "" : s >= -65 ? "ok" : s >= -75 ? "" : s >= -82 ? "warn" : "bad");
const wfState = s => (s === "running" ? t("on air") : s === "disabled" ? t("radio disabled") : s);
const wfTh = head => "<thead><tr><th>" + head.map(esc).join("</th><th>") + "</th></tr></thead>";
function wfSet(id, html) {
  if (WF_DRAWN[id] === html) return;
  $(id).innerHTML = html;
  WF_DRAWN[id] = html;
  paint($(id));
}

function wfIssues(v) {
  const out = [];
  for (const a of v.aps) {
    if (a.error) { out.push(["bad", t("{name} unreachable: {err}", { name: a.name, err: a.error })]); continue; }
    for (const r of a.radios) {
      if (r.state !== "running" && r.state !== "disabled") out.push(["warn", t("{ap} {iface}: {state}", { ap: a.name, iface: r.iface, state: r.state })]);
      // each overlapping pair once
      for (const o of r.overlap || []) if (natural(a.name + " " + r.iface, o.ap + " " + o.iface) < 0)
        out.push(["warn", t(o.same ? "{a} and {b} on the same channel" : "{a} and {b} overlap", { a: a.name + " " + r.iface, b: o.ap + " " + o.iface })]);
    }
  }
  for (const s of v.ssids) if (s.drift.length) out.push(["warn", t("{ssid}: not the same on every access point", { ssid: s.ssid })]);
  return out;
}

function renderWifi() {
  const v = WF, vd = $("wf-verdict");
  wfPending(v);
  if (!v || v.error || !v.aps.length) {
    vd.className = "verdict" + (v && v.error ? " bad" : v ? " warn" : "");
    vd.innerHTML = v ? '<span class="dot ' + (v.error ? "bad" : "warn") + '"></span>' + esc(v.error ? t("Read failed: {err}", { err: v.error }) :
      t("No access point: the switches with Wi-Fi radios (RouterOS wifi package) show up here.")) : esc(t("Loading…"));
    for (const id of ["wf-aps", "wf-map", "wf-ssids", "wf-clients", "wf-events"]) wfSet(id, "");
    return;
  }
  const iss = wfIssues(v), lvl = worst(iss.map(([l]) => l));
  vd.className = "verdict " + lvl;
  vd.innerHTML = '<span class="dot ' + lvl + '"></span>' + esc(iss.length ? iss.map(([, m]) => m).join(" · ") :
    t("All good: {aps}, {clients}, {ssids}", { aps: tp(v.aps.length, "{n} access point", "{n} access points"),
      clients: tp(v.clients.length, "{n} Wi-Fi client", "{n} Wi-Fi clients"), ssids: tp(v.ssids.length, "{n} SSID", "{n} SSIDs") }));
  wfSet("wf-aps", wfAps(v));
  wfSet("wf-map", wfMap(v));
  wfSet("wf-ssids", wfSsids(v));
  wfSet("wf-clients", wfClients(v));
  wfSet("wf-events", wfEvents(v));
  $("view-title").textContent = t(VIEWS.wifi) + " · " + tp(v.clients.length, "{n} client", "{n} clients");
}

/* access points and their radios */

// control channel, and the channels a wider block covers: "36 (36–40)"
const wfChanOf = f => (f === 2484 ? 14 : f < 3000 ? (f - 2407) / 5 : f < 5925 ? (f - 5000) / 5 : (f - 5950) / 5);
const wfChanCell = r => (r.channel == null ? "—" : r.width > 20 ? r.channel + " (" + wfChanOf(r.lo + 10) + "–" + wfChanOf(r.hi - 10) + ")" : String(r.channel));

function wfAps(v) {
  return '<div class="grid wf-aps">' + v.aps.map(a => {
    if (a.error) return '<article class="card srv bad"><div class="card-head"><h3>' + esc(a.name) + '</h3><span class="pill bad">' + esc(t("unreachable")) +
      '</span></div><p class="sub">' + esc(a.error) + "</p></article>";
    const lv = a.radios.some(r => r.state !== "running" && r.state !== "disabled") ? "warn" : "ok";
    return '<article class="card srv ' + lv + '"><div class="card-head"><h3>' + esc(a.name) + '</h3><span class="pill">' + esc(tp(a.clients, "{n} client", "{n} clients")) +
      '</span></div><p class="sub">' + esc(a.model) + '</p><div class="table-wrap"><table>' + wfTh([t("Band"), t("Channel"), t("Width"), t("Power"), t("State"), t("Clients")]) + "<tbody>" +
      a.radios.map(r => '<tr class="' + (r.state === "running" ? "" : "dim") + '"><td><b>' + esc(wfBand(r.band)) + '</b><br><small class="sub">' + esc([r.iface, ...r.ssids].join(" · ")) + "</small></td>" +
        "<td>" + esc(wfChanCell(r)) + (r.auto ? ' <span class="pill">' + esc(t("auto")) + "</span>" : "") + "</td>" +
        '<td title="' + esc(r.width_cfg) + '">' + esc(r.width ? r.width + " MHz" : "—") + "</td>" +
        "<td>" + esc(r.tx_power != null ? r.tx_power + " dBm" : "—") + (r.max_power != null ? '<br><small class="sub">' + esc(t("max {n} dBm", { n: r.max_power })) + "</small>" : "") + "</td>" +
        '<td><span class="pill ' + (r.state === "running" ? "ok" : r.state === "disabled" ? "" : "warn") + '">' + esc(wfState(r.state)) + '</span></td><td class="num">' + esc(r.clients) + "</td></tr>").join("") +
      '</tbody></table></div><div class="toolbar"><button data-act="wf-edit" data-ap="' + esc(a.name) + '"' +
      (a.mode === "direct" ? "" : ' disabled title="' + esc(t("Direct mode only")) + '"') + ">" + esc(t("Set")) + "</button></div></article>";
  }).join("") + "</div>";
}

/* settings of one access point (direct mode): form, preview, apply, keep */

const wfLeft = name => Math.max(0, Math.ceil(((WF_UNTIL[name] || 0) - Date.now()) / 1000));
function wfPending(v) {
  const box = $("wf-pending");
  const aps = v && !v.error ? v.aps.filter(a => !a.error && (a.pending || wfLeft(a.name) > 0)) : [];
  box.hidden = !aps.length;
  box.innerHTML = aps.map(a => '<div class="card-head"><h3>' + esc(t("Keep these settings on {name}?", { name: a.name })) + '</h3><span class="pill warn">' +
    esc(wfLeft(a.name) > 0 ? t("rollback in {n} s", { n: wfLeft(a.name) }) : t("rollback pending")) + "</span></div>" +
    '<p class="sub">' + esc(t("Cairn Net read the access point back as asked. Without your confirmation, it puts the old settings back by itself.")) + "</p>" +
    '<div class="toolbar"><button class="primary" data-act="wf-keep" data-ap="' + esc(a.name) + '">' + esc(t("Keep")) + "</button></div>").join("");
}

const WF_WIDTH = { 20: "20mhz", 40: "20/40mhz", 80: "20/40/80mhz", 160: "20/40/80/160mhz", 320: "20/40/80/160/320mhz" };
const wfOpt = (val, label, cur) => '<option value="' + esc(val) + '"' + (val === cur ? " selected" : "") + ">" + esc(label) + "</option>";

function wfRadioForm(r) {
  const n = esc(r.iface), ws = [...new Set(r.channels.flatMap(c => c.w.map(([w]) => w)))].sort((x, y) => x - y);
  const radar = c => (c.w.find(([w]) => w === 20) || [0, false])[1];
  const fo = [wfOpt("", t("automatic"), r.freq_cfg), ...r.channels.map(c => wfOpt(String(c.f), t("ch {c}", { c: c.ch }) + " · " + c.f + " MHz" + (radar(c) ? " · " + t("radar") : ""), r.freq_cfg))];
  if (r.freq_cfg && !r.channels.some(c => String(c.f) === r.freq_cfg)) fo.push(wfOpt(r.freq_cfg, r.freq_cfg, r.freq_cfg)); // a list or range set by hand
  const wo = [wfOpt("", t("automatic"), r.width_cfg), ...ws.map(w => wfOpt(WF_WIDTH[w], t("up to {n} MHz", { n: w }), r.width_cfg))];
  if (r.width_cfg && !ws.some(w => WF_WIDTH[w] === r.width_cfg)) wo.push(wfOpt(r.width_cfg, r.width_cfg, r.width_cfg));
  return '<fieldset class="rows"><legend>' + esc(r.iface + " · " + wfBand(r.band)) + '</legend><div class="cols2">' +
    "<label>" + esc(t("Channel")) + '<select name="f-' + n + '">' + fo.join("") + "</select></label>" +
    "<label>" + esc(t("Width")) + '<select name="w-' + n + '">' + wo.join("") + "</select></label></div>" +
    "<label>" + esc(t("Transmit power (dBm)")) + '<input type="number" name="p-' + n + '" min="1" max="' + esc(r.max_power || 30) + '" step="1" value="' + esc(r.tx_cfg) +
    '" placeholder="' + esc(t("automatic, max {n}", { n: r.max_power || "?" })) + '"></label></fieldset>';
}

function wfSsidForm({ s, on }, i) {
  return '<fieldset class="rows"><legend>' + esc(s.ssid + " · VLAN " + (s.vlan || t("untagged"))) + "</legend>" +
    on.map(e => '<label class="check"><input type="checkbox" name="on-' + esc(e.iface) + '"' + (e.enabled ? " checked" : "") + "> " +
      esc(t("On {band} ({iface})", { band: wfBand(e.band), iface: e.iface })) + "</label>").join("") +
    '<label class="check"><input type="checkbox" name="iso-' + i + '"' + (on.every(e => e.isolation) ? " checked" : "") + "> " + esc(t("Clients isolated from each other")) + "</label>" +
    (list(s.security).length ? "<label>" + esc(t("New password")) + '<input type="text" class="mono" name="pw-' + i +
      '" minlength="8" maxlength="64" autocomplete="off" spellcheck="false" placeholder="' + esc(t("unchanged")) + '"></label>' : "") + "</fieldset>";
}

async function wfEdit(name) {
  const a = (WF.aps || []).find(x => x.name === name);
  if (!a || a.error) return;
  const mine = WF.ssids.map(s => ({ s, on: s.on.filter(e => e.ap === name) })).filter(x => x.on.length);
  const fd = await ask(t("Settings of {name}", { name }), a.radios.map(wfRadioForm).join("") + mine.map(wfSsidForm).join("") +
    '<p class="sub">' + esc(t("Only what you change is sent. A new password is sent once, never shown again; the read account cannot read it.")) + "</p>", t("Preview"));
  $("dlg-body").innerHTML = ""; // a typed password leaves the page
  if (!fd) return;
  const ifaces = {}, ssids = [];
  const put = (n, k, val) => ((ifaces[n] = ifaces[n] || { name: n })[k] = val);
  const field = k => String(fd.get(k) ?? "").trim();
  for (const r of a.radios) {
    if (field("f-" + r.iface) !== r.freq_cfg) put(r.iface, "frequency", field("f-" + r.iface));
    if (field("w-" + r.iface) !== r.width_cfg) put(r.iface, "width", field("w-" + r.iface));
    if (field("p-" + r.iface) !== r.tx_cfg) put(r.iface, "tx_power", field("p-" + r.iface));
  }
  mine.forEach(({ s, on }, i) => {
    for (const e of on) if ((fd.get("on-" + e.iface) === "on") !== e.enabled) put(e.iface, "disabled", e.enabled);
    const x = { ssid: s.ssid }, iso = fd.get("iso-" + i) === "on", pw = String(fd.get("pw-" + i) ?? "");
    if (iso !== on.every(e => e.isolation)) x.isolation = iso;
    if (pw) x.passphrase = pw;
    if (Object.keys(x).length > 1) ssids.push(x);
  });
  const want = { ifaces: Object.values(ifaces), ssids };
  if (!want.ifaces.length && !ssids.length) return banner(t("Nothing to change."), "ok");
  return wfSend(name, want);
}

async function wfSend(name, want) {
  let p;
  try { p = await post("/api/wifi/preview", { switch: name, want }); } catch (e) { p = { ok: false, error: e.message }; }
  if (!p.ok) return banner(t("Refused: {err}", { err: problemText(p.error || "") }));
  if (!p.commands.length) return banner(t("Nothing to change."), "ok");
  banner("");
  if (!(await ask(t("Apply on {name}?", { name }),
    p.warnings.map(x => '<p class="banner">' + esc(problemText(x)) + "</p>").join("") +
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>",
    t("Apply")))) return;
  let r;
  try { r = await post("/api/wifi/apply", { switch: name, want }); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.pending) WF_UNTIL[name] = Date.now() + r.rollback_s * 1000;
  banner(r.pending && r.verified ? "" : r.pending ? t("{done} of {n} commands run on {name}", { done: r.done, n: r.commands.length, name }) + (r.error ? " · " + problemText(r.error) : "") +
    " · " + t("the switch puts the old settings back within {n} s", { n: r.rollback_s }) : r.ok ? t("Nothing to change.") : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadWifi();
}

async function wfKeep(name) {
  let r;
  try { r = await post("/api/infra/confirm", { switch: name }); } catch (e) { r = { ok: false, error: e.message }; }
  delete WF_UNTIL[name];
  banner(r.ok ? t("Kept on {name}.", { name }) : t("Refused: {err}", { err: problemText(r.error || "") }), r.ok ? "ok" : "");
  await loadWifi();
}

/* channel map: one strip per band, a bar per radio over the channels it covers */

const WF_RANGE = { "2.4": [2400, 2484], "5": [5170, 5895], "6": [5945, 7125] };
const wfSeq = (a, b, k) => Array.from({ length: Math.floor((b - a) / k) + 1 }, (_, i) => a + i * k);
const WF_TICKS = { "2.4": wfSeq(1, 13, 1), "5": [...wfSeq(36, 64, 4), ...wfSeq(100, 144, 4), ...wfSeq(149, 177, 4)], "6": wfSeq(1, 233, 16) };
const wfFreq = (b, c) => (b === "2.4" ? 2407 + 5 * c : b === "5" ? 5000 + 5 * c : 5950 + 5 * c);

function wfMap(v) {
  const radios = v.aps.flatMap(a => a.radios).filter(r => r.lo != null && WF_RANGE[r.band]);
  const bands = ["2.4", "5", "6"].filter(b => radios.some(r => r.band === b));
  if (!bands.length) return '<p class="sub">' + esc(t("No radio on air.")) + "</p>";
  return bands.map(b => {
    const [lo, hi] = WF_RANGE[b], rs = radios.filter(r => r.band === b).sort((x, y) => x.lo - y.lo || natural(x.ap, y.ap));
    const W = 800, L = 28, top = 6, base = top + rs.length * L + 4, H = base + 22;
    const X = f => (f - lo) / (hi - lo) * W;
    let g = "";
    if (b === "5") g += '<rect class="dfs" x="' + X(5250).toFixed(1) + '" y="0" width="' + (X(5730) - X(5250)).toFixed(1) + '" height="' + base + '"/>' +
      '<text class="dfs-l" x="' + X(5490).toFixed(1) + '" y="' + (base - 4) + '">DFS</text>';
    for (const c of WF_TICKS[b]) {
      const x = X(wfFreq(b, c)).toFixed(1);
      g += '<line class="tick-l" x1="' + x + '" x2="' + x + '" y1="0" y2="' + base + '"/><text class="tick" x="' + x + '" y="' + (H - 6) + '">' + c + "</text>";
    }
    rs.forEach((r, i) => {
      const y = top + i * L, cls = (r.overlap || []).some(o => o.same) ? "bad" : (r.overlap || []).length ? "warn" : "ok";
      const label = r.ap + " · " + r.iface + " · " + t("ch {c}", { c: wfChanCell(r) }) + " · " + r.width + " MHz";
      const right = X(r.hi) + 8 + label.length * 6.6 < W; // label after the bar, else before it
      g += '<g class="bar ' + cls + '"><title>' + esc(label + (r.overlap || []).map(o => " · " + o.ap + " " + o.iface).join("")) + "</title>" +
        '<rect x="' + X(r.lo).toFixed(1) + '" y="' + (y + 3) + '" width="' + (X(r.hi) - X(r.lo)).toFixed(1) + '" height="' + (L - 8) + '" rx="4"/>' +
        '<text class="lbl" x="' + (right ? X(r.hi) + 8 : X(r.lo) - 8).toFixed(1) + '" y="' + (y + L / 2 + 3) + '"' + (right ? "" : ' text-anchor="end"') + ">" + esc(label) + "</text></g>";
    });
    return '<h3 class="wf-band">' + esc(wfBand(b)) + '</h3><svg class="wf-map" viewBox="0 0 ' + W + " " + H + '" role="img" aria-label="' +
      esc(t("Wi-Fi channels, {band}", { band: wfBand(b) })) + '">' + g + "</svg>";
  }).join("");
}

/* SSIDs merged by name */

const WF_WHAT = { vlan: "VLAN", security: "Security", isolation: "Isolation", hidden: "Hidden", enabled: "Enabled", bands: "Bands" };
function wfVal(what, v) {
  if (what === "vlan") return v || t("untagged");
  if (what === "security") return wfSec(v);
  if (what === "bands") return String(v).split("+").map(wfBand).join(" + ");
  return typeof v === "boolean" ? t(v ? "yes" : "no") : String(v);
}
const wfDrift = d => (d.what === "missing" ? t("missing on {where}", { where: d.where.join(", ") }) :
  t("{what}: {values}", { what: t(WF_WHAT[d.what] || d.what), values: d.by.map(b => wfVal(d.what, b.value) + " (" + b.where.join(", ") + ")").join(" ≠ ") }));

function wfSsids(v) {
  const many = v.aps.filter(a => !a.error).length > 1;
  return "<table>" + wfTh(["SSID", "VLAN", t("Security"), t("Isolation"), t("Access points"), t("Clients"), t("Differences")]) + "<tbody>" + v.ssids.map(s => {
    const per = {};
    for (const e of s.on) (per[e.ap] = per[e.ap] || []).push(e);
    const where = Object.entries(per).map(([ap, es]) => ap + " · " + [...new Set(es.filter(e => e.enabled).map(e => e.band))].sort().map(wfBand).join(" + ") +
      (es.some(e => !e.enabled) ? " (" + t("SSID off") + ": " + es.filter(e => !e.enabled).map(e => e.iface).join(", ") + ")" : ""));
    return '<tr class="' + (s.on.some(e => e.enabled) ? "" : "dim") + '"><td><b>' + esc(s.ssid) + "</b>" + (s.hidden ? ' <span class="pill">' + esc(t("hidden")) + "</span>" : "") +
      "</td><td>" + esc(s.vlan || t("untagged")) + "</td><td>" + esc(wfSec(s.security)) + "</td><td>" + (s.isolation ? '<span class="pill">' + esc(t("isolated")) + "</span>" : "—") +
      '</td><td class="wrap">' + where.map(esc).join("<br>") + '</td><td class="num">' + esc(s.clients) + '</td><td class="wrap">' +
      (s.drift.length ? s.drift.map(d => '<span class="pill warn">' + esc(wfDrift(d)) + "</span>").join("<br>") : many ? '<span class="pill ok">' + esc(t("same everywhere")) + "</span>" : "—") + "</td></tr>";
  }).join("") + "</tbody></table>";
}

/* clients of all access points */

function wfClients(v) {
  const q = $("wf-q").value.trim().toLowerCase();
  const rows = v.clients.filter(c => !q || q.split(/\s+/).every(w => [c.name, c.ip, c.mac, c.ssid, c.vlan, c.ap, c.iface, wfPhy(c.band)].join(" ").toLowerCase().includes(w)));
  return "<table>" + wfTh([t("Device"), t("IP address"), "SSID", t("Access point"), t("Signal"), t("Rate ↓ / ↑"), t("Connected for"), t("Roams")]) + "<tbody>" +
    (rows.map(c => "<tr><td>" + '<button class="chip" data-goto="clients" data-q="' + esc(c.mac) + '">' + kindSvg(c.kind || "unknown") + esc(c.name || c.mac) + "</button>" +
      (c.name ? '<br><small class="sub mono">' + esc(c.mac) + "</small>" : "") + (c.authorized ? "" : ' <span class="pill warn">' + esc(t("not authorized")) + "</span>") +
      "</td><td>" + esc(c.ip) + "</td><td>" + esc(c.ssid) + (c.vlan ? '<br><small class="sub">VLAN ' + esc(c.vlan) + "</small>" : "") +
      "</td><td>" + esc(c.ap + " · " + c.iface) + '<br><small class="sub">' + esc(wfPhy(c.band)) + "</small>" +
      '</td><td><span class="pill ' + wfSignal(c.signal) + '">' + esc(c.signal != null ? c.signal + " dBm" : "—") + "</span></td>" +
      '<td class="num">' + esc(bits(num(c.tx_rate)) + " / " + bits(num(c.rx_rate))) + '<br><small class="sub" title="' + esc(t("Now ↓ / ↑")) + '">' +
      esc(bits(num(c.tx_bps)) + " / " + bits(num(c.rx_bps))) + "</small></td><td>" + esc(duration(rosSeconds(c.uptime))) + '</td><td class="num">' +
      (c.roams ? '<button class="chip" data-act="wf-hist" data-mac="' + esc(c.mac) + '" title="' + esc(c.last_roam) + '">' + esc(c.roams) + "</button>" : "0") + "</td></tr>").join("") ||
      '<tr><td colspan="8" class="sub">' + esc(t(v.clients.length ? "No client matches." : "No client.")) + "</td></tr>") + "</tbody></table>";
}

/* connections and roaming, newest first */

function wfWhat(e) {
  if (e.what === "connected") return esc(e.from ? t("roamed from {ap}", { ap: e.from }) : t("connected"));
  if (e.what === "roamed") return esc(t("roamed {a} → {b}", { a: e.iface, b: e.to }));
  return esc(t(e.what)) + (e.why ? ' <span class="pill' + (/wrong|fail|timeout/.test(e.why) ? " warn" : "") + '">' + esc(e.why) + "</span>" : "");
}
const wfEvRow = e => '<tr><td class="mono">' + esc(String(e.t).replace(/^\d{4}-/, "")) + "</td><td>" + esc(e.name || e.mac) + "</td><td>" + wfWhat(e) +
  "</td><td>" + esc(e.ap + " · " + e.iface) + '<br><small class="sub">' + esc(e.ssid) + "</small></td><td>" + esc(e.signal != null ? e.signal + " dBm" : "") + "</td></tr>";
const wfEvTable = evs => "<table>" + wfTh([t("When"), t("Device"), t("What happened"), t("Access point"), t("Signal")]) + "<tbody>" +
  (evs.map(wfEvRow).join("") || '<tr><td colspan="5" class="sub">' + esc(t("Nothing in the logs.")) + "</td></tr>") + "</tbody></table>";
const wfEvents = v => wfEvTable(v.events.slice(0, 60));

document.addEventListener("click", e => {
  const b = e.target.closest("button[data-act='wf-edit'], button[data-act='wf-keep']");
  if (!b || !WF || WF.error) return;
  if (b.dataset.act === "wf-edit") wfEdit(b.dataset.ap);
  else wfKeep(b.dataset.ap);
});
document.addEventListener("click", e => {
  const b = e.target.closest("button[data-act='wf-hist']");
  if (!b || !WF || !WF.events) return;
  const c = WF.clients.find(x => x.mac === b.dataset.mac) || { mac: b.dataset.mac };
  ask(t("Roaming of {name}", { name: c.name || c.mac }), '<div class="table-wrap">' + wfEvTable(WF.events.filter(x => x.mac === c.mac)) + "</div>", t("OK"));
});
$("wf-q").addEventListener("input", () => render(true));
