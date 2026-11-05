// cairn-net: Tools page (src/tools.rs), loaded after panel.js (uses its helpers).
// A run (ping, traceroute, torch, bandwidth test, IP scan, capture) lives on
// the server; the page polls it every second, which also keeps it alive: once
// the page stops polling (tab closed) the server ends it on the switch.
"use strict";

VIEWS.tools = "Tools";
const TL_TOOLS = { ping: "Ping", traceroute: "Traceroute", torch: "Torch", btest: "Bandwidth test", scan: "IP scan", sniffer: "Packet capture", netwatch: "Netwatch", graphing: "Graphs" };
let TL = null, TL_TOOL = "ping", TL_KEY = "", RUN = null, RUN_ID = null, RUN_HIST = [], TL_PRESET = null;

// Opens the capture of a switch with its interface filled in (e.g. the
// mirror target of the switch-chip card, www/chip.js).
function captureOn({ switch: sw, iface }) {
  TL_TOOL = "sniffer"; TL_KEY = ""; TL_PRESET = ["interface", iface || ""];
  location.hash = "#tools/" + encodeURIComponent(sw);
}

async function loadTools() {
  const name = SW;
  if (!name) return;
  try {
    const v = await get("/api/tools?switch=" + encodeURIComponent(name));
    if (name !== SW) return;
    TL = v;
    const r = (v.runs || []).find(x => !x.done) || (v.runs || [])[0];
    if (r && !RUN_ID) { RUN_ID = r.id; TL_TOOL = r.tool; pollRun(); }
  } catch (e) { if (name === SW) TL = { name, error: e.message }; }
  TL_KEY = "";
  if (VIEW === "tools") renderTools();
}

/* ---------- forms ---------- */

const ifaceOpts = (req, none = "Automatic") => (req ? "" : '<option value="">' + esc(t(none)) + "</option>") +
  ((TL && TL.interfaces) || []).filter(i => !yes(i.disabled)).map(i => i.name).sort(natural).map(n => '<option value="' + esc(n) + '">' + esc(n) + "</option>").join("");
const field = (name, label, attrs = "", val = "") => "<label>" + esc(t(label)) + '<input name="' + name + '" value="' + esc(val) + '" ' + attrs + "></label>";
const numf = (name, label, def, min, max) => field(name, label, 'type="number" min="' + min + '" max="' + max + '"', def);
const sel = (name, label, opts) => "<label>" + esc(t(label)) + '<select name="' + name + '">' + opts + "</select></label>";
const tlOpts = o => Object.entries(o).map(([v, l]) => '<option value="' + v + '">' + esc(t(l)) + "</option>").join("");
// the first network of the switch, for the IP scan
const firstNet = () => {
  const a = ((TL && TL.addresses) || []).map(x => x.address).find(Boolean) || "";
  const m = a.match(/^(\d+)\.(\d+)\.(\d+)\.\d+\/(\d+)$/);
  return m && +m[4] >= 20 ? (+m[4] >= 24 ? m[1] + "." + m[2] + "." + m[3] + ".0/24" : a) : "";
};

function toolForm(tool) {
  const addr = field("address", "Address", 'required placeholder="192.168.1.1" autocapitalize="off"');
  switch (tool) {
    case "ping": return addr + numf("count", "Count", 5, 1, 100) + numf("size", "Size (bytes)", 56, 28, 10000) + sel("interface", "Source interface", ifaceOpts());
    case "traceroute": return addr + numf("count", "Probes per hop", 3, 1, 10) + numf("max_hops", "Max hops", 30, 1, 64) + sel("interface", "Source interface", ifaceOpts());
    case "torch": return sel("interface", "Interface", ifaceOpts(true)) + field("src-address", "Source address", 'placeholder="' + esc(t("empty: not split")) + '"', "0.0.0.0/0") +
      field("dst-address", "Destination address", 'placeholder="' + esc(t("empty: not split")) + '"', "0.0.0.0/0") + field("port", "Port", 'placeholder="any"', "any") +
      field("ip-protocol", "Protocol", 'placeholder="any"', "any") + numf("duration", "Duration (s)", 30, 1, 300);
    case "btest": return sel("target", "Target", tlOpts({ self: "The switch itself (local, no link loaded)" }) +
        (TL.peers || []).map(p => '<option value="switch:' + esc(p) + '">' + esc(t("Switch {name}", { name: p })) + "</option>").join("") + tlOpts({ address: "Another address…" })) +
      '<div class="tl-addr" hidden>' + field("address", "Address", 'placeholder="192.168.1.1"') + field("user", "User", 'autocomplete="off"') +
      field("password", "Password", 'type="password" autocomplete="new-password"') + "</div>" +
      sel("protocol", "Protocol", tlOpts({ tcp: "TCP", udp: "UDP" })) + sel("direction", "Direction", tlOpts({ both: "Both ways", transmit: "Send", receive: "Receive" })) +
      numf("speed", "Speed cap (Mb/s each way)", 100, 1, 1000) + numf("duration", "Duration (s)", 10, 1, 30);
    case "scan": return field("range", "Range", 'required placeholder="192.168.1.0/24"', firstNet()) + sel("interface", "Interface", ifaceOpts()) + numf("duration", "Duration (s)", 20, 1, 120);
    case "sniffer": return sel("interface", "Interface", ifaceOpts(false, "all interfaces")) + field("ip-address", "Address", 'placeholder="' + esc(t("any")) + '"') +
      field("port", "Port", 'placeholder="' + esc(t("any")) + '"') + field("ip-protocol", "Protocol", 'placeholder="' + esc(t("any")) + '"') +
      numf("duration", "Duration (s)", 10, 1, 60) + numf("limit", "File limit (KiB)", 1024, 16, 4096);
  }
  return "";
}

// What a tool needs, said in words.
function rightsText(r) {
  if (!r) return "";
  if (r.ok) return t(r.account === "read" ? "Runs with the read account" : "Runs with the write account");
  if (r.direct_needed) return t("Writes to the switch: only in direct mode (Settings)");
  const parts = [];
  if (r.read_lacks && r.read_lacks.length) parts.push(t("the read account lacks {p}", { p: r.read_lacks.join(", ") }));
  parts.push(r.write_lacks ? t("the write account lacks {p}", { p: r.write_lacks.join(", ") }) : t("there is no write account"));
  return t("Needs the right {p}: {why}", { p: r.need.join(", "), why: parts.join(", ") });
}

function renderTools() {
  $("tl-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="tools/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const v = TL && TL.name === SW ? TL : null;
  if (!v || v.error) {
    $("tl-picker").innerHTML = "";
    $("tl-main").innerHTML = '<p class="verdict ' + (v ? "bad" : "") + '">' + (v ? '<span class="dot bad"></span>' + esc(t("{name} unreachable: {err}", { name: SW, err: v.error })) : esc(t("Loading…"))) + "</p>";
    TL_KEY = "";
    return renderRun();
  }
  const key = SW + "|" + TL_TOOL + "|" + LANG;
  if (key !== TL_KEY) { // the form only when it changes: what is typed survives the refreshes
    TL_KEY = key;
    $("tl-picker").innerHTML = Object.entries(TL_TOOLS).map(([k, l]) => {
      const r = v.rights[k];
      return '<button class="chip' + (k === TL_TOOL ? " on" : "") + '" data-act="tl-pick" data-tool="' + k + '"' + (r && !r.ok ? ' title="' + esc(rightsText(r)) + '"' : "") + ">" +
        (r && !r.ok ? '<span class="dot warn"></span>' : "") + esc(t(l)) + "</button>";
    }).join("");
    const r = v.rights[TL_TOOL];
    const grant = r && !r.ok && (r.read_lacks || []).includes("test") ? ' <button data-act="tl-grant">' + esc(t("Give the read account the test right…")) + "</button>" : "";
    const note = '<p class="sub">' + (r && !r.ok ? '<span class="pill warn">' + esc(rightsText(r)) + "</span>" + grant : esc(rightsText(r))) + "</p>";
    if (TL_TOOL === "netwatch") $("tl-main").innerHTML = netwatchCard(v, r) + note;
    else if (TL_TOOL === "graphing") $("tl-main").innerHTML = graphCard(v) + note;
    else {
      $("tl-main").innerHTML = '<article class="card"><div class="card-head"><h3>' + esc(t(TL_TOOLS[TL_TOOL])) + "</h3></div>" + toolHelp(TL_TOOL) +
        '<form id="tl-form" class="tl-form" autocomplete="off">' + toolForm(TL_TOOL) + "</form>" + note +
        '<div class="toolbar"><button class="primary" data-act="tl-start"' + (r && r.ok ? "" : " disabled") + ">" + esc(t("Run the tool")) + '</button><button data-act="tl-stop" hidden>' +
        esc(t("Stop")) + "</button></div></article>";
      if (TL_PRESET) { const el = $("tl-form").elements[TL_PRESET[0]]; if (el) el.value = TL_PRESET[1]; TL_PRESET = null; }
      const tg = $("tl-form").elements.target;
      if (tg) tg.addEventListener("change", () => { document.querySelector(".tl-addr").hidden = tg.value !== "address"; });
    }
  }
  renderRun();
}

function toolHelp(tool) {
  const h = {
    torch: "Live traffic of one interface, split by the fields given (empty field: not split). Seen by the switch's CPU: on a hardware-switched port, only what reaches the CPU.",
    btest: "Saturates the path up to the cap. Towards the switch itself nothing leaves it; towards another device the links in between are loaded: the page asks twice.",
    scan: "Pings and ARP-asks each address of the range from the switch (4096 addresses at most).",
    sniffer: "What the switch's CPU sees (hardware-switched traffic is not captured). The file is downloaded, then removed from the switch; the sniffer settings are put back.",
  }[tool];
  return h ? '<p class="sub">' + esc(t(h)) + "</p>" : "";
}

/* ---------- results ---------- */

const tbl = (head, rows) => '<div class="table-wrap"><table><thead><tr>' + head.map(h => "<th>" + esc(t(h)) + "</th>").join("") + "</tr></thead><tbody>" +
  (rows.length ? rows.join("") : '<tr><td colspan="' + head.length + '" class="sub">' + esc(t("Nothing yet")) + "</td></tr>") + "</tbody></table></div>";
const tr = cells => "<tr>" + cells.map(c => "<td>" + c + "</td>").join("") + "</tr>";
const ms = v => String(v || "").replace(/us$/, " µs").replace(/ms$/, " ms");

function runBody(R) {
  const rows = R.rows || [];
  switch (R.tool) {
    case "ping": {
      const l = rows[rows.length - 1] || {};
      return (rows.length ? '<p class="sub">' + esc(t("{s} sent, {r} received, {l} % lost · min {min} · avg {avg} · max {max}", { s: l.sent, r: l.received, l: l["packet-loss"], min: ms(l["min-rtt"]) || "-", avg: ms(l["avg-rtt"]) || "-", max: ms(l["max-rtt"]) || "-" })) + "</p>" : "") +
        tbl(["#", "Host", "Size", "TTL", "Time", "Status"], rows.slice(-100).map(x => tr([esc(x.seq), esc(x.host), esc(x.size), esc(x.ttl), esc(ms(x.time)), esc(x.status || "")])));
    }
    case "traceroute": return tbl(["Hop", "Address", "Loss", "Sent", "Last", "Avg", "Best", "Worst", "Status"],
      rows.map((x, i) => tr([i + 1, esc(x.address), esc(x.loss) + " %", esc(x.sent), esc(x.last), esc(x.avg), esc(x.best), esc(x.worst), esc(x.status)])));
    case "torch": {
      const s = [...rows].sort((a, b) => num(b.tx) + num(b.rx) - num(a.tx) - num(a.rx));
      const tot = s.reduce((a, x) => [a[0] + num(x.tx), a[1] + num(x.rx)], [0, 0]);
      return '<p class="sub">' + esc(t("{n} flows · total sent {tx} · received {rx}", { n: s.length, tx: bits(tot[0]), rx: bits(tot[1]) })) + "</p>" +
        tbl(["Protocol", "Source", "Destination", "Sent", "Received", "Packets"], s.slice(0, 200).map(x => tr([esc(x["ip-protocol"] || x["mac-protocol"] || ""),
          esc((x["src-address"] || "") + (x["src-port"] ? ":" + x["src-port"] : "")), esc((x["dst-address"] || "") + (x["dst-port"] ? ":" + x["dst-port"] : "")),
          bits(num(x.tx)), bits(num(x.rx)), esc(num(x["tx-packets"]) + " / " + num(x["rx-packets"]))])));
    }
    case "btest": {
      const l = rows[rows.length - 1] || {};
      const series = [rows.map(x => num(x["tx-current"])), rows.map(x => num(x["rx-current"]))];
      return '<dl class="kv tl-kv"><dt>' + esc(t("Status")) + "</dt><dd>" + esc(l.status || "-") + "</dd><dt>" + esc(t("Sent now · 10 s · total")) + "</dt><dd>" +
        esc([l["tx-current"], l["tx-10-second-average"], l["tx-total-average"]].map(v => bits(num(v))).join(" · ")) + "</dd><dt>" + esc(t("Received now · 10 s · total")) + "</dt><dd>" +
        esc([l["rx-current"], l["rx-10-second-average"], l["rx-total-average"]].map(v => bits(num(v))).join(" · ")) + "</dd>" +
        (l["local-cpu-load"] ? "<dt>" + esc(t("CPU load (this switch · other end)")) + "</dt><dd>" + esc((l["local-cpu-load"] || "-") + " % · " + (l["remote-cpu-load"] || "-") + " %") + "</dd>" : "") +
        (l["lost-packets"] ? "<dt>" + esc(t("Lost packets")) + "</dt><dd>" + esc(l["lost-packets"]) + "</dd>" : "") + "</dl>" + (rows.length > 1 ? spark(series, 1, true) : "");
    }
    case "scan": return tbl(["Address", "MAC", "DNS", "SNMP", "NetBIOS", "Time"],
      [...rows].sort((a, b) => natural(a.address || "", b.address || "")).map(x => tr([esc(x.address), '<span class="mono">' + esc(x["mac-address"]) + "</span>", esc(x.dns), esc(x.snmp), esc(x.netbios), esc(ms(x.time))])));
    case "sniffer": {
      const l = rows[0] || {};
      return '<p class="sub">' + esc(t("{s} s captured · file {size}", { s: l.elapsed || 0, size: bytes(num(l.size)) })) + "</p>" +
        (R.pcap != null ? '<div class="toolbar"><a class="chip on" href="/api/tools/pcap?id=' + R.id + '" download>' + esc(t("Download the capture ({size})", { size: bytes(R.pcap) })) + "</a></div>" : "");
    }
  }
  return "";
}

function renderRun() {
  const box = $("tl-run"), R = RUN;
  const stop = document.querySelector('[data-act="tl-stop"]'), start = document.querySelector('[data-act="tl-start"]');
  const live = R && !R.done;
  if (stop) stop.hidden = !live;
  if (start && TL && TL.rights) start.disabled = live || !(TL.rights[TL_TOOL] || {}).ok;
  box.hidden = !R || R.switch !== SW || (R.done && R.tool !== TL_TOOL); // a finished run stays with its tool
  if (box.hidden) return;
  const state = R.error ? ["bad", t("Error: {err}", { err: R.error })] : !R.done ? ["warn", t("Running · {e} / {m} s", { e: R.elapsed, m: R.max })]
    : R.stopped ? ["", t(R.stopped === "stopped" ? "Stopped" : R.stopped === "page closed" ? "Stopped: page closed" : "Stopped: maximum duration")] : ["ok", t("Done")];
  box.innerHTML = '<div class="card-head"><h3>' + esc(t(TL_TOOLS[R.tool] || R.tool) + " · " + R.switch) + '</h3><span class="pill ' + state[0] + '">' + esc(state[1]) + "</span></div>" +
    '<details><summary>' + esc(t("Commands ({a} account)", { a: t(R.account) })) + '</summary><pre class="mono tl-cmd">' + esc((R.commands || []).join("\n")) + "</pre></details>" + runBody(R);
  paint(box);
}

async function pollRun() {
  clearTimeout(pollRun.timer);
  if (!RUN_ID) return;
  try {
    RUN = await get("/api/tools/run?id=" + RUN_ID);
    if (RUN.error && !RUN.tool) { RUN = null; RUN_ID = null; }
  } catch (e) { banner(t("Read failed: {err}", { err: e.message }), "read"); }
  if (VIEW === "tools") renderRun();
  if (RUN && !RUN.done) pollRun.timer = setTimeout(pollRun, 1000);
}

async function startTool() {
  const f = $("tl-form");
  if (!f.reportValidity()) return;
  const args = Object.fromEntries(new FormData(f));
  let confirm;
  if (TL_TOOL === "btest") {
    const tg = args.target;
    if (tg.startsWith("switch:")) { args.target = "switch"; args.peer = tg.slice(7); } else args.target = tg;
    if (args.target !== "self") {
      // links get loaded: say how much, then the switch's name typed
      const per = +args.speed, n = args.direction === "both" ? 2 : 1, mb = per * n * +args.duration / 8;
      const to = args.target === "switch" ? args.peer : args.address;
      const ok1 = await ask(t("Load the links?"), '<p class="banner warn-note">' + esc(t("Up to {s} Mb/s {dir} for {d} s between {a} and {b}: about {mb} MB through every link on the path, shared with real traffic.",
        { s: per, dir: t(n === 2 ? "each way" : args.direction === "transmit" ? "sent" : "received"), d: args.duration, a: SW, b: to, mb: nf(mb, 0) })) + "</p>" +
        '<label class="check"><input type="checkbox" name="ack" required>' + esc(t("I know the links will be loaded")) + "</label>", t("Continue"));
      if (!ok1) return;
      const ok2 = await ask(t("Confirm the bandwidth test"), "<p>" + esc(t("Type {name} to start.", { name: SW })) + '</p><label><input name="confirm" required autocomplete="off" autocapitalize="off"></label>', t("Run the tool"));
      if (!ok2) return;
      confirm = ok2.get("confirm").trim();
    }
  }
  try {
    const r = await post("/api/tools/start", { switch: SW, tool: TL_TOOL, args, confirm });
    if (!r.ok) return banner(r.error);
    banner("");
    RUN_ID = r.id; RUN = null; pollRun();
  } catch (e) { banner(e.message); }
}

const stopRun = () => RUN_ID && post("/api/tools/stop", { id: RUN_ID }).then(() => setTimeout(pollRun, 300)).catch(e => banner(e.message));
// tab closed: the server would stop the run after 15 s without polls; say it now
// netwatch: the probes' state as the switch sees it
setInterval(() => { if (VIEW === "tools" && TL_TOOL === "netwatch" && !document.hidden) loadTools(); }, 10000);
window.addEventListener("pagehide", () => {
  if (RUN && !RUN.done) navigator.sendBeacon("/api/tools/stop", new Blob([JSON.stringify({ id: RUN_ID })], { type: "application/json" }));
});

/* ---------- netwatch, graphs ---------- */

function netwatchCard(v, r) {
  const can = r && r.ok;
  const rows = (v.netwatch || []).map(x => tr([esc(x.host) + (x.port ? ":" + esc(x.port) : ""), esc(x.type || "simple"), esc(x.interval),
    '<span class="pill ' + (x.status === "up" ? "ok" : x.status === "down" ? "bad" : "") + '">' + esc(t(x.status || "unknown")) + "</span>", esc(x.since || ""),
    esc(x.comment || ""), x["down-script"] || x["up-script"] ? esc(t("yes")) : "", can ? '<button class="danger" data-act="tl-nw-del" data-id="' + esc(x[".id"]) + '" data-host="' + esc(x.host) + '">' + esc(t("Remove")) + "</button>" : ""]));
  return '<article class="card"><div class="card-head"><h3>' + esc(t("Netwatch")) + "</h3>" + (can ? '<button class="primary" data-act="tl-nw-add">' + esc(t("Add a probe…")) + "</button>" : "") + "</div>" +
    '<p class="sub">' + esc(t("The switch checks each host by itself and can call a URL (e.g. a ntfy topic) when it goes down or comes back.")) + "</p>" +
    tbl(["Host", "Type", "Interval", "Status", "Since", "Comment", "Alert", ""], rows) + "</article>";
}

async function netwatchSend(body) {
  try {
    const p = await post("/api/tools/netwatch/preview", Object.assign({ switch: SW }, body));
    if (!p.ok) return banner(p.error);
    const ok = await ask(t("Apply on {name}?", { name: SW }), '<pre class="mono tl-cmd">' + esc(p.commands.join("\n")) + "</pre>", t("Apply"));
    if (!ok) return;
    const r = await post("/api/tools/netwatch/apply", Object.assign({ switch: SW }, body));
    banner(r.ok ? t("Done, read back from the switch") : r.error || t("Not read back as asked"), r.ok ? "ok" : "");
    loadTools();
  } catch (e) { banner(e.message); }
}

// The read account gets the test right: applied when the write account may
// (direct mode, policy right), else the command to paste, then a re-check.
async function grantTest() {
  try {
    const p = await post("/api/tools/grant/preview", { switch: SW });
    if (!p.ok) return banner(problemText(p.error));
    if (p.done) return loadTools();
    const cmd = '<pre class="mono tl-cmd">' + esc(p.commands.join("\n")) + "</pre>";
    const help = '<p class="sub">' + esc(t("Ping, traceroute, torch, bandwidth test and IP scan then run with the read account. The right lets it run tests, not change the configuration.")) + "</p>";
    if (!p.can_apply) {
      if (await ask(t("Give the test right on {name}", { name: SW }), help + '<p class="sub">' + esc(t(p.why)) + "</p>" + cmd, t("Re-check"))) loadTools();
      return;
    }
    if (!await ask(t("Apply on {name}?", { name: SW }), help + cmd, t("Apply"))) return;
    const r = await post("/api/tools/grant/apply", { switch: SW });
    banner(r.ok ? t("Done, read back from the switch") : r.error || t("Not read back as asked"), r.ok ? "ok" : "");
    loadTools();
  } catch (e) { banner(e.message); }
}

async function netwatchAdd() {
  const fd = await ask(t("Add a probe on {name}", { name: SW }), '<div class="cols2">' + field("host", "Host", 'required placeholder="192.168.1.1"') +
    sel("type", "Type", tlOpts({ simple: "Simple (ping)", icmp: "ICMP (loss, latency)", "tcp-conn": "TCP connection" })) + numf("interval", "Interval (s)", 30, 5, 3600) +
    numf("port", "Port (TCP)", "", 1, 65535) + "</div>" + field("alert", "Alert URL (optional)", 'type="url" placeholder="https://ntfy.example.org/topic"') +
    '<p class="sub">' + esc(t("The switch POSTs \"<switch>: <host> down\" (or up) to this URL: nothing else is sent.")) + "</p>", t("Preview"));
  if (fd) netwatchSend(Object.assign({ op: "add" }, Object.fromEntries(fd)));
}

function graphCard(v) {
  const g = v.graphing || [], res = v.graphing_resource || [];
  const www = (v.www || []).filter(s => !yes(s.disabled)).sort((a, b) => (a.name === "www-ssl" ? -1 : 1));
  const link = www.length && (g.length || res.length) ? (www[0].name === "www-ssl" ? "https://" : "http://") + v.host + (["80", "443"].includes(www[0].port) ? "" : ":" + www[0].port) + "/graphs/" : "";
  return '<article class="card"><div class="card-head"><h3>' + esc(t("Graphs")) + "</h3>" + (link ? '<a class="chip on" href="' + esc(link) + '" target="_blank" rel="noopener">' + esc(t("Open the switch's graphs")) + "</a>" : "") + "</div>" +
    '<p class="sub">' + esc(!g.length && !res.length ? t("Graphing is not turned on for this switch (/tool graphing).") : link ? t("RouterOS draws them itself and serves them only on its web interface.")
      : t("RouterOS serves its graphs only on its web interface (www or www-ssl), which is off on this switch.")) + "</p>" +
    tbl(["Interface", "Allowed from", "Kept on disk", "Disabled"], g.map(x => tr([esc(x.interface), esc(x["allow-address"]), esc(t(yes(x["store-on-disk"]) ? "yes" : "no")), esc(t(yes(x.disabled) ? "yes" : "no"))]))
      .concat(res.map(x => tr([esc(t("Resources (CPU, memory, disk)")), esc(x["allow-address"]), esc(t(yes(x["store-on-disk"]) ? "yes" : "no")), esc(t(yes(x.disabled) ? "yes" : "no"))])))) + "</article>";
}

document.addEventListener("click", e => {
  const b = e.target.closest("button");
  if (!b || VIEW !== "tools") return;
  const d = b.dataset;
  switch (d.act) {
    case "tl-pick": TL_TOOL = d.tool; TL_KEY = ""; return renderTools();
    case "tl-start": return startTool();
    case "tl-stop": return stopRun();
    case "tl-nw-add": return netwatchAdd();
    case "tl-grant": return grantTest();
    case "tl-nw-del": return netwatchSend({ op: "remove", id: d.id, host: d.host });
  }
});

PROBLEMS.push(
  [/^(\S+) is in the built-in group (\S+): give it its own group first .*$/, "{v} is in the built-in group {net}: give it its own group first (Settings, preparation script)", 1],
  [/^the group (\S+) is shared with (.*): they would get the right too$/, "the group {v} is shared with {net}: they would get the right too", 1],
);

// added only where the other pages have none: they keep their wording
for (const [k, v] of Object.entries({
  "Give the read account the test right…": "Donner le droit test au compte de lecture…", "Give the test right on {name}": "Donner le droit test sur {name}", "Re-check": "Revérifier",
  "Ping, traceroute, torch, bandwidth test and IP scan then run with the read account. The right lets it run tests, not change the configuration.":
    "Ping, traceroute, torch, test de débit et scan IP passent alors par le compte de lecture. Ce droit permet de lancer des tests, pas de modifier la configuration.",
  "the switch is not in direct mode: paste the command in the switch's own terminal (SSH as admin); the Cairn Net Terminal page only writes in direct mode": "le switch n'est pas en mode direct : collez la commande dans le terminal du switch lui-même (SSH en admin) ; la page Terminal de Cairn Net n'écrit qu'en mode direct",
  "the write account lacks the policy right: paste the command in the switch's own terminal (SSH as admin)": "le compte d'écriture n'a pas le droit policy : collez la commande dans le terminal du switch lui-même (SSH en admin)",
  "{v} is in the built-in group {net}: give it its own group first (Settings, preparation script)": "{v} est dans le groupe intégré {net} : donnez-lui d'abord son propre groupe (Réglages, script de préparation)",
  "the group {v} is shared with {net}: they would get the right too": "le groupe {v} est partagé avec {net} : ils recevraient aussi le droit",
  "Tools": "Outils", "Ping": "Ping", "Traceroute": "Traceroute", "Torch": "Torch", "Bandwidth test": "Test de débit", "IP scan": "Scan IP", "Packet capture": "Capture de paquets",
  "Netwatch": "Netwatch", "Graphs": "Graphiques", "any": "tous", "Address": "Adresse", "Count": "Nombre", "Size (bytes)": "Taille (octets)", "Source interface": "Interface source",
  "Probes per hop": "Sondes par saut", "Max hops": "Sauts au plus", "Interface": "Interface", "Source address": "Adresse source", "Destination address": "Adresse de destination",
  "empty: not split": "vide : pas de découpage", "Port": "Port", "Protocol": "Protocole", "Duration (s)": "Durée (s)", "Target": "Cible",
  "The switch itself (local, no link loaded)": "Le switch lui-même (local, aucun lien chargé)", "Switch {name}": "Switch {name}", "Another address…": "Une autre adresse…",
  "User": "Utilisateur", "Password": "Mot de passe", "Both ways": "Dans les deux sens", "Send": "Envoi", "Receive": "Réception",
  "Speed cap (Mb/s each way)": "Débit plafond (Mb/s par sens)", "Range": "Plage", "File limit (KiB)": "Taille max du fichier (Kio)",
  "Runs with the read account": "Lancé avec le compte de lecture", "Runs with the write account": "Lancé avec le compte d'écriture",
  "Writes to the switch: only in direct mode (Settings)": "Écrit sur le switch : seulement en mode direct (Réglages)",
  "the read account lacks {p}": "le compte de lecture n'a pas {p}", "the write account lacks {p}": "le compte d'écriture n'a pas {p}", "there is no write account": "il n'y a pas de compte d'écriture",
  "Needs the right {p}: {why}": "Il faut le droit {p} : {why}", "Run the tool": "Lancer", "Stop": "Arrêter",
  "Live traffic of one interface, split by the fields given (empty field: not split). Seen by the switch's CPU: on a hardware-switched port, only what reaches the CPU.":
    "Trafic en direct d'une interface, découpé selon les champs remplis (champ vide : pas de découpage). Vu par le processeur du switch : sur un port commuté en matériel, seulement ce qui remonte au processeur.",
  "Saturates the path up to the cap. Towards the switch itself nothing leaves it; towards another device the links in between are loaded: the page asks twice.":
    "Sature le chemin jusqu'au plafond. Vers le switch lui-même rien ne sort ; vers un autre appareil les liens entre les deux sont chargés : la page demande deux confirmations.",
  "Pings and ARP-asks each address of the range from the switch (4096 addresses at most).": "Le switch interroge (ping, ARP) chaque adresse de la plage (4096 adresses au plus).",
  "What the switch's CPU sees (hardware-switched traffic is not captured). The file is downloaded, then removed from the switch; the sniffer settings are put back.":
    "Ce que voit le processeur du switch (le trafic commuté en matériel n'est pas capturé). Le fichier est téléchargé puis supprimé du switch ; les réglages du sniffer sont remis.",
  "Nothing yet": "Rien pour l'instant", "Host": "Hôte", "Size": "Taille", "TTL": "TTL", "Time": "Temps", "Status": "État", "Hop": "Saut", "Loss": "Perte", "Sent": "Envoyés",
  "Last": "Dernier", "Avg": "Moy.", "Best": "Meilleur", "Worst": "Pire", "Source": "Source", "Destination": "Destination", "Received": "Reçu", "Packets": "Paquets", "MAC": "MAC",
  "{s} sent, {r} received, {l} % lost · min {min} · avg {avg} · max {max}": "{s} envoyés, {r} reçus, {l} % perdus · min {min} · moy. {avg} · max {max}",
  "{n} flows · total sent {tx} · received {rx}": "{n} flux · envoyé en tout {tx} · reçu {rx}",
  "Sent now · 10 s · total": "Envoyé maintenant · 10 s · moyenne", "Received now · 10 s · total": "Reçu maintenant · 10 s · moyenne",
  "CPU load (this switch · other end)": "Charge processeur (ce switch · l'autre bout)", "Lost packets": "Paquets perdus",
  "{s} s captured · file {size}": "{s} s capturées · fichier {size}", "Download the capture ({size})": "Télécharger la capture ({size})",
  "Error: {err}": "Erreur : {err}", "Running · {e} / {m} s": "En cours · {e} / {m} s", "Stopped": "Arrêté", "Stopped: page closed": "Arrêté : page fermée",
  "Stopped: maximum duration": "Arrêté : durée maximale", "Done": "Terminé", "Commands ({a} account)": "Commandes (compte {a})", "read": "lecture", "write": "écriture",
  "Load the links?": "Charger les liens ?", "each way": "dans chaque sens", "sent": "en envoi", "received": "en réception",
  "Up to {s} Mb/s {dir} for {d} s between {a} and {b}: about {mb} MB through every link on the path, shared with real traffic.":
    "Jusqu'à {s} Mb/s {dir} pendant {d} s entre {a} et {b} : environ {mb} Mo sur chaque lien du chemin, partagés avec le trafic réel.",
  "I know the links will be loaded": "Je sais que les liens seront chargés", "Continue": "Continuer", "Confirm the bandwidth test": "Confirmer le test de débit",
  "Type {name} to start.": "Tapez {name} pour lancer.", "up": "en ligne", "down": "hors ligne", "unknown": "inconnu", "Since": "Depuis", "Comment": "Commentaire", "Alert": "Alerte",
  "Remove": "Retirer", "Add a probe…": "Ajouter une sonde…", "Interval": "Intervalle", "Type": "Type",
  "The switch checks each host by itself and can call a URL (e.g. a ntfy topic) when it goes down or comes back.":
    "Le switch surveille chaque hôte lui-même et peut appeler une URL (par exemple un sujet ntfy) quand il tombe ou revient.",
  "Apply on {name}?": "Appliquer sur {name} ?", "Apply": "Appliquer", "Done, read back from the switch": "Fait, relu sur le switch", "Not read back as asked": "Relecture différente de la demande",
  "Add a probe on {name}": "Ajouter une sonde sur {name}", "Simple (ping)": "Simple (ping)", "ICMP (loss, latency)": "ICMP (perte, latence)", "TCP connection": "Connexion TCP",
  "Interval (s)": "Intervalle (s)", "Port (TCP)": "Port (TCP)", "Alert URL (optional)": "URL d'alerte (facultative)", "Preview": "Aperçu",
  "The switch POSTs \"<switch>: <host> down\" (or up) to this URL: nothing else is sent.": "Le switch envoie en POST « <switch>: <hôte> down » (ou up) à cette URL : rien d'autre.",
  "Open the switch's graphs": "Ouvrir les graphiques du switch", "Graphing is not turned on for this switch (/tool graphing).": "Les graphiques ne sont pas activés sur ce switch (/tool graphing).",
  "RouterOS draws them itself and serves them only on its web interface.": "RouterOS les dessine lui-même et ne les sert que sur son interface web.",
  "RouterOS serves its graphs only on its web interface (www or www-ssl), which is off on this switch.": "RouterOS ne sert ses graphiques que sur son interface web (www ou www-ssl), coupée sur ce switch.",
  "all interfaces": "toutes les interfaces", "Allowed from": "Autorisé depuis", "Kept on disk": "Gardé sur disque", "Disabled": "Désactivé", "Resources (CPU, memory, disk)": "Ressources (processeur, mémoire, disque)", "yes": "oui", "no": "non",
})) if (!(k in DICT.fr)) DICT.fr[k] = v;
