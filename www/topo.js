// cairn-net: Topology view, hand-drawn SVG (loaded after panel.js, uses its helpers).
// Switches in a row, links between them as arcs above (one per LACP bond or
// cable, port names at both ends), devices hanging below their switch, one
// row per port. The layout depends only on the sorted structure (src/topo.rs),
// so nothing moves between refreshes; the live part (traffic) only changes
// classes and texts. Icons: the #k-<kind> symbols of index.html.
"use strict";

const COL = 250, GAP = 60, SWH = 96, ROW = 52, SUB = 42, PAD = 24;
const KIND_LABEL = { switch: "Switch", router: "Router", ap: "Wi-Fi access point", server: "Server or hypervisor", nas: "NAS", pc: "Computer",
  phone: "Phone", printer: "Printer", camera: "Camera", iot: "Connected device", unknown: "Unknown" };
const TOPO = { vb: null, markup: "", size: [0, 0], open: {}, flows: [] };

// Name and kind set by hand are applied by the server (src/topo.rs, data_dir/kinds.json);
// d.manual says what was set, d.guess what Cairn Net found by itself.
const kindOf = d => d.kind || "unknown";
function whyOf(d, w = d.why || {}) {
  return w.by === "hand" ? t("set by hand") : w.by === "announce" ? t("announced as {v}", { v: w.v }) : w.by === "name" ? t("from its name ({v})", { v: w.v })
    : w.by === "vendor" ? t("vendor: {v}", { v: w.v }) : w.by === "vms" ? tp(w.v, "hosts {n} virtual machine", "hosts {n} virtual machines")
    : w.by === "random" ? t("random (private) address") : t("nothing to go by");
}
const kindSvg = k => '<svg class="kicon-s" viewBox="0 0 24 24" aria-hidden="true"><use href="#k-' + k + '"/></svg>';
function kindButton(d) {
  const k = kindOf(d);
  return '<button class="chip kind" data-act="kind-edit" data-mac="' + esc(d.mac) + '" title="' + esc(whyOf(d)) + '">' + kindSvg(k) + esc(t(KIND_LABEL[k])) + "</button>";
}
// Devices page, name column: the name, or a button to give one.
const nameCell = d => (d.name ? esc(d.name) : '<button class="chip" data-act="kind-edit" data-mac="' + esc(d.mac) + '">' + esc(t("Name it")) + "</button>");

async function kindForm(mac) {
  const d = devices().find(x => x.mac === mac);
  if (!d) return;
  const m = d.manual || {}, g = d.guess || d;
  const opt = (v, label, k) => '<label class="swatch"><input type="radio" name="kind" value="' + v + '"' + (v === (m.kind || "") ? " checked" : "") + ">" + kindSvg(k) + esc(label) + "</label>";
  const fd = await ask(t("Device {name}", { name: d.name || d.ip || d.mac }),
    '<p class="sub mono">' + esc([d.mac, d.ip, d.sw + " · " + d.port].filter(Boolean).join(" · ")) + "</p>" +
    "<label>" + esc(t("Name")) + '<input name="name" maxlength="63" value="' + esc(m.name || "") + '" placeholder="' + esc(g.name || t("none found (DHCP, DNS, LLDP)")) + '"></label>' +
    '<fieldset class="swatches kinds"><legend>' + esc(t("Kind · guess: {kind}, {why}", { kind: t(KIND_LABEL[g.kind] || g.kind), why: whyOf(d, g.why || {}) })) + "</legend>" +
    opt("", t("Automatic"), g.kind) + Object.keys(KIND_LABEL).map(k => opt(k, t(KIND_LABEL[k]), k)).join("") + "</fieldset>" +
    '<p class="sub">' + esc(t("Kept by Cairn Net for this MAC address; nothing is written to the switches. An empty name goes back to the one found.")) + "</p>", t("Save"));
  if (!fd) return;
  let r;
  try { r = await post("/api/kinds", { mac, kind: String(fd.get("kind") || ""), name: String(fd.get("name") || "").trim() }); } catch (e) { r = { ok: false, error: e.message }; }
  if (!r.ok) return banner(t("Refused: {err}", { err: t(r.error || "") }));
  TOPO.markup = "";
  await slowTick(); // the server applies it: read again
}

/* ---------- live traffic ---------- */

const rateOf = (sw, ifs) => ifs.reduce((a, n) => { const r = rates[sw + "|" + n] || {}; return { rx: a.rx + (r.rx || 0), tx: a.tx + (r.tx || 0) }; }, { rx: 0, tx: 0 });
const level = bps => (bps < 1e3 ? 0 : bps < 1e6 ? 1 : bps < 1e8 ? 2 : bps < 1e9 ? 3 : 4);

// Paths are drawn from the switch outwards: traffic the switch sends (tx) flows forward.
function updateFlows() {
  for (const f of TOPO.flows) {
    const r = rateOf(f.sw, f.ifs), tot = r.rx + r.tx;
    const p = document.querySelector('#topo [data-flow="' + f.id + '"]');
    if (p) p.setAttribute("class", "flow l" + level(tot) + (r.rx > r.tx ? " rev" : "") + (f.down ? " down" : ""));
    const tx = document.querySelector('#topo [data-rate="' + f.id + '"]');
    if (tx) tx.textContent = f.down ? "" : (f.arrows ? "↑ " + bits(r.tx) + "  ↓ " + bits(r.rx) : bits(tot));
  }
  for (const e of document.querySelectorAll("#topo [data-swrate]")) {
    const h = hist[e.dataset.swrate] || { rx: [], tx: [] };
    e.textContent = "↓ " + bits(last(h.rx)) + "  ↑ " + bits(last(h.tx));
  }
}

/* ---------- layout ---------- */

const cut = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
const svgText = (x, y, txt, cls, more = "") => '<text x="' + x + '" y="' + y + '" class="' + cls + '"' + more + ">" + esc(txt) + "</text>";
const use = (k, x, y, s, cls = "") => '<use href="#k-' + k + '" x="' + x + '" y="' + y + '" width="' + s + '" height="' + s + '" class="kicon ' + cls + '"/>';
const link = (attrs, inner, label) => '<a ' + attrs + ' tabindex="0" role="link" aria-label="' + esc(label) + '">' + inner + "</a>";

// The switch's front: ports as small squares, green when up (same groups as the Ports view).
function miniFront(sw, x, y) {
  const groups = {};
  for (const p of sw.ports) (groups[groupOf(p.name)] = groups[groupOf(p.name)] || []).push(p);
  let out = "", gx = x;
  for (const ps of Object.values(groups)) {
    const two = ps.length > 8;
    ps.forEach((p, i) => {
      const cx = gx + (two ? Math.floor(i / 2) : i) * 10, cy = y + (two ? (i % 2) * 9 : 4);
      out += '<rect x="' + cx + '" y="' + cy + '" width="8" height="7" rx="1.5" class="fp ' + portState(p) + (p.uplink ? " up" : "") + '"><title>' + esc(portTitle(p)) + "</title></rect>";
    });
    gx += (two ? Math.ceil(ps.length / 2) : ps.length) * 10 + 8;
  }
  return out;
}

function layout(sws) {
  const lk = links(sws);
  const ok = sws.filter(s => s.name);
  // root: the switch with the most links; the others around it, in Settings order
  const deg = n => lk.filter(l => l.s.name === n || l.o.name === n).length;
  const root = ok.slice().sort((a, b) => deg(b.name) - deg(a.name))[0];
  const rest = ok.filter(s => s !== root);
  const order = root ? [...rest.slice(0, Math.floor(rest.length / 2)).reverse(), root, ...rest.slice(Math.floor(rest.length / 2))] : [];
  const col = Object.fromEntries(order.map((s, i) => [s.name, i]));
  const X = n => PAD + col[n] * (COL + GAP);

  // link ends along the top of each switch, sorted by the column of the peer
  const ends = {};
  lk.forEach((l, i) => {
    (ends[l.s.name] = ends[l.s.name] || []).push({ i, peer: col[l.o.name] });
    (ends[l.o.name] = ends[l.o.name] || []).push({ i, peer: col[l.s.name] });
  });
  const anchor = {};
  for (const [n, es] of Object.entries(ends)) {
    es.sort((a, b) => a.peer - b.peer || a.i - b.i);
    es.forEach((e, k) => { anchor[n + "|" + e.i] = X(n) + 34 + (k + 1) * (COL - 68) / (es.length + 1); });
  }
  // heights: shortest arcs lowest; arcs that overlap side by side get another level (labels never collide)
  const arcs = lk.map((l, i) => {
    const xs = [anchor[l.s.name + "|" + i], anchor[l.o.name + "|" + i]];
    return { l, i, lo: Math.min(...xs) - 70, hi: Math.max(...xs) + 70 };
  }).sort((a, b) => (a.hi - a.lo) - (b.hi - b.lo) || a.lo - b.lo);
  arcs.forEach((a, k) => {
    let lv = 0;
    while (arcs.slice(0, k).some(o => o.lv === lv && o.lo < a.hi && a.lo < o.hi)) lv++;
    a.lv = lv;
    a.h = 50 + 44 * lv;
  });
  const T = PAD + 44 + Math.max(0, ...arcs.map(a => a.h)) * 0.75 + 20; // top of the switches
  let markup = "", H = T + SWH;
  const flows = [];

  for (const { l, i, h } of arcs) {
    const x1 = anchor[l.s.name + "|" + i], x2 = anchor[l.o.name + "|" + i];
    const d = "M" + x1 + "," + T + " C" + x1 + "," + (T - h) + " " + x2 + "," + (T - h) + " " + x2 + "," + T;
    const id = "L" + i, mx = (x1 + x2) / 2, my = T - 0.75 * h;
    const speed = l.speed.length ? (l.speed.length > 1 ? l.speed.length + " × " : "") + rateShort(l.speed[0]) : "";
    const what = [l.b ? "LACP" : "", speed, l.lv === "standby" ? t("blocked by STP") : l.lv === "bad" ? t("down") : l.lv === "warn" ? t("{up} / {total} up", { up: l.upn, total: l.members.length }) : ""].filter(Boolean).join(" · ");
    flows.push({ id, sw: l.s.name, ifs: l.members, down: l.lv === "bad" || l.lv === "standby", arrows: true });
    const end = (x, ports) => svgText(x - 3, T - 8, ports.map(short).join("+"), "t-port", ' transform="rotate(-90 ' + (x - 3) + " " + (T - 8) + ')"');
    markup += link('data-goto="ports/' + esc(l.s.name) + '" data-sel="' + esc(l.members[0]) + '" class="t-link ' + l.lv + '"',
      '<path class="wire" d="' + d + '"/><path class="flow" data-flow="' + id + '" d="' + d + '"/><path class="hit" d="' + d + '"/>' +
      end(x1, l.members) + end(x2, l.remote) +
      svgText(mx, my - 8, what, "t-what") + svgText(mx, my + 7, "", "t-rate", ' data-rate="' + id + '"'),
      l.s.name + " ⇄ " + l.o.name + " · " + what);
  }

  const devs = devices();
  for (const sw of order) {
    const x = X(sw.name);
    const lv = worst(issuesOf(sw).map(([v]) => v));
    markup += link('data-goto="infra/' + esc(sw.name) + '" class="t-sw ' + lv + '"',
      '<rect x="' + x + '" y="' + T + '" width="' + COL + '" height="' + SWH + '" rx="10" class="box"/>' +
      (sw.error ? svgText(x + 14, T + 30, t("unreachable"), "t-sub bad") : miniFront(sw, x + 14, T + 12)) +
      svgText(x + 14, T + 52, sw.name, "t-name big") +
      svgText(x + 14, T + 69, cut([cut(sw.rb.model || sw.r["board-name"] || "", 18), sw.error ? "" : t("{up} / {total} ports up", { up: sw.ports.filter(p => p.running).length, total: sw.ports.length })].filter(Boolean).join(" · "), 40), "t-sub") +
      svgText(x + 14, T + 85, "", "t-sub rate", ' data-swrate="' + esc(sw.name) + '"'),
      sw.name);

    // its devices, one row per port (several on a port: one group, folded when many)
    const byPort = {};
    for (const d of devs.filter(d => d.sw === sw.name)) (byPort[d.port] = byPort[d.port] || []).push(d);
    const ports = Object.keys(byPort).sort(natural);
    let y = T + SWH + 34, lastRow = 0;
    const busX = x + 16;
    for (const port of ports) {
      const ds = byPort[port], key = sw.name + "|" + port, id = "P" + flows.length;
      lastRow = y;
      const p = sw.ports.find(q => q.name === port) || {};
      const bond = sw.bonds.find(b => b.name === port);
      const ifs = bond ? bond.slaves : [port];
      const down = p.name ? !p.running : false;
      flows.push({ id, sw: sw.name, ifs: [port], down });
      const wire = "M" + busX + "," + y + " H" + (x + 62);
      const head = ds.find(d => d.announce) || ds.find(d => d.why && d.why.by === "vms") || ds[0];
      const many = ds.length > 1, open = TOPO.open[key] ?? ds.length <= 3;
      const sel = 'data-goto="ports/' + esc(sw.name) + '" data-sel="' + esc(ifs[0]) + '"';
      markup += '<path class="wire dev" d="' + wire + '"/><path class="flow" data-flow="' + id + '" d="' + wire + '"/>' +
        link(sel + ' class="t-portlink"', svgText(busX + 6, y - 6, short(port), "t-port"), sw.name + " " + port) +
        svgText(busX + 4, y + 14, "", "t-sub rate wire-rate", ' data-rate="' + id + '"');
      const label = d => d.name || d.ip || d.mac;
      const row = (d, cx, cy, s, n) => link('data-goto="clients" data-q="' + esc(d.mac) + '" class="t-dev"',
        '<rect x="' + (cx - 4) + '" y="' + (cy - s / 2 - 6) + '" width="' + (x + COL - cx) + '" height="' + (s + 12) + '" class="hit"/>' +
        use(kindOf(d), cx, cy - s / 2, s, kindOf(d)) + svgText(cx + s + 8, cy - 2, cut(label(d), n), "t-name") +
        svgText(cx + s + 8, cy + 12, cut([d.name ? d.ip : "", t(KIND_LABEL[kindOf(d)])].filter(Boolean).join(" · "), n + 4), "t-sub"),
        label(d) + " · " + t(KIND_LABEL[kindOf(d)]));
      if (!many) {
        markup += row(ds[0], x + 64, y, 28, 20);
        y += ROW;
        continue;
      }
      // the group: its head (the announced device, or the host of the VMs) and a count
      markup += '<a data-act="topo-toggle" data-key="' + esc(key) + '" class="t-group" tabindex="0" role="button" aria-expanded="' + open + '">' +
        '<rect x="' + (x + 60) + '" y="' + (y - 20) + '" width="' + (COL - 60) + '" height="40" class="hit"/>' +
        use(kindOf(head), x + 68, y - 11, 28, "ghost " + kindOf(head)) + use(kindOf(head), x + 64, y - 14, 28, kindOf(head)) +
        svgText(x + 102, y - 2, (open ? "▾ " : "▸ ") + cut(head.announce || (head.why && head.why.by === "vms") ? label(head) : tp(ds.length, "{n} device", "{n} devices"), 18), "t-name") +
        svgText(x + 102, y + 12, tp(ds.length, "{n} device on this port", "{n} devices on this port"), "t-sub") + "</a>";
      y += ROW;
      if (!open) continue;
      const top = y - ROW + 20;
      for (const d of ds) {
        markup += '<path class="wire sub" d="M' + (x + 78) + "," + top + " V" + y + " H" + (x + 92) + '"/>' + row(d, x + 94, y, 22, 18);
        y += SUB;
      }
      y += 6;
    }
    if (ports.length) markup = '<path class="wire bus" d="M' + busX + "," + (T + SWH) + " V" + lastRow + '"/>' + markup;
    H = Math.max(H, y);
  }
  const W = order.length ? X(order[order.length - 1].name) + COL + PAD : 0;
  return { markup, flows, W, H: H + PAD, rootX: root ? X(root.name) + COL / 2 : 0 };
}

/* ---------- view: zoom and pan (viewBox), stable across refreshes ---------- */

const svgEl = () => $("topo");
function applyVb() {
  const v = TOPO.vb;
  if (v) svgEl().setAttribute("viewBox", [v.x, v.y, v.w, v.h].map(n => n.toFixed(1)).join(" "));
}
// Whole drawing in view; on a narrow screen, readable text first (the root switch, from the top).
function fit() {
  const r = svgEl().getBoundingClientRect();
  if (!r.width || !TOPO.size[0]) return;
  let k = Math.min(r.width / TOPO.size[0], r.height / TOPO.size[1], 1.2);
  let x = 0, y = 0;
  if (k < 0.6) { k = 0.6; x = Math.max(0, TOPO.rootX - r.width / k / 2); }
  else { x = (TOPO.size[0] - r.width / k) / 2; y = (TOPO.size[1] - r.height / k) / 2; }
  TOPO.vb = { x, y, w: r.width / k, h: r.height / k };
  applyVb();
}
function zoomAt(cx, cy, f) {
  const r = svgEl().getBoundingClientRect(), v = TOPO.vb;
  if (!v || !r.width) return;
  const k = clamp(r.width / v.w * f, 0.15, 4), w = r.width / k, h = r.height / k;
  const px = v.x + (cx - r.left) / r.width * v.w, py = v.y + (cy - r.top) / r.height * v.h;
  TOPO.vb = { x: px - (cx - r.left) / r.width * w, y: py - (cy - r.top) / r.height * h, w, h };
  applyVb();
}

function renderTopo(sws) {
  const L = layout(sws);
  TOPO.flows = L.flows;
  if (L.markup !== TOPO.markup) {
    $("topo-g").innerHTML = L.markup;
    TOPO.markup = L.markup;
    TOPO.size = [L.W, L.H];
    TOPO.rootX = L.rootX;
  }
  if (!TOPO.vb) fit();
  updateFlows();
  const n = devices().length;
  $("topo-note").textContent = tp(sws.length, "{n} switch", "{n} switches") + " · " + tp(links(sws).length, "{n} link", "{n} links") + " · " + tp(n, "{n} device", "{n} devices");
}

(() => {
  const el = svgEl(), ptrs = new Map();
  let moved = 0;
  el.addEventListener("pointerdown", e => { ptrs.set(e.pointerId, [e.clientX, e.clientY]); moved = 0; });
  el.addEventListener("pointermove", e => {
    const p = ptrs.get(e.pointerId);
    if (!p || !TOPO.vb) return;
    const r = el.getBoundingClientRect();
    if (ptrs.size === 1) {
      const dx = e.clientX - p[0], dy = e.clientY - p[1];
      moved += Math.abs(dx) + Math.abs(dy);
      if (moved > 6) {
        if (!el.hasPointerCapture(e.pointerId)) el.setPointerCapture(e.pointerId); // only now: a plain tap stays a click
        TOPO.vb.x -= dx * TOPO.vb.w / r.width;
        TOPO.vb.y -= dy * TOPO.vb.h / r.height;
        applyVb();
      }
    } else if (ptrs.size === 2) { // pinch: zoom on the middle of the two fingers
      const [a, b] = [...ptrs.values()], o = a === p ? b : a;
      const before = Math.hypot(p[0] - o[0], p[1] - o[1]), after = Math.hypot(e.clientX - o[0], e.clientY - o[1]);
      moved += 100;
      if (before > 0) zoomAt((e.clientX + o[0]) / 2, (e.clientY + o[1]) / 2, after / before);
    }
    ptrs.set(e.pointerId, [e.clientX, e.clientY]);
  });
  const up = e => ptrs.delete(e.pointerId);
  el.addEventListener("pointerup", up);
  el.addEventListener("pointercancel", up);
  // a drag is not a click on what was under the finger
  el.addEventListener("click", e => { if (moved > 6) { e.stopPropagation(); e.preventDefault(); } }, true);
  el.addEventListener("wheel", e => { e.preventDefault(); zoomAt(e.clientX, e.clientY, Math.pow(1.0018, -e.deltaY)); }, { passive: false });
  el.addEventListener("keydown", e => {
    if (e.key === "Enter" || e.key === " ") { const a = e.target.closest("a"); if (a) { e.preventDefault(); a.dispatchEvent(new MouseEvent("click", { bubbles: true })); } }
  });
  window.addEventListener("resize", () => {
    const v = TOPO.vb, r = el.getBoundingClientRect();
    if (!v || !r.width) return;
    const k = Math.min(r.width / v.w, 4);
    TOPO.vb = { x: v.x, y: v.y, w: r.width / k, h: r.height / k };
    applyVb();
  });
  document.addEventListener("click", e => {
    const b = e.target.closest("[data-act]");
    if (!b) return;
    const r = el.getBoundingClientRect(), mid = [r.left + r.width / 2, r.top + r.height / 2];
    switch (b.dataset.act) {
      case "topo-toggle": {
        const key = b.dataset.key;
        TOPO.open[key] = b.getAttribute("aria-expanded") !== "true";
        return render(true);
      }
      case "topo-in": return zoomAt(...mid, 1.25);
      case "topo-out": return zoomAt(...mid, 0.8);
      case "topo-fit": return fit();
      case "kind-edit": return kindForm(b.dataset.mac);
    }
  });
})();
