// cairn-net: Terminal page (src/term.rs), loaded after panel.js (uses its helpers).
// One RouterOS command line at a time through the API; output kept per switch
// for the page's life, history (secrets already hidden by the server) per browser.
"use strict";

VIEWS.term = "Terminal";
const TE_OUT = {}; // switch -> [{cmd, out, ok, err, account}]
const TE_HIST_KEY = "cairn-net-term-history";
let TE_HIST = [], TE_POS = 0, TE_BUSY = false, TE_DRAWN = "";
try { TE_HIST = JSON.parse(localStorage.getItem(TE_HIST_KEY) || "[]"); } catch (e) { TE_HIST = []; }
TE_POS = TE_HIST.length;

const teWrite = sw => { const s = (CFG.switches || []).find(x => x.name === sw); return s && s.mode === "direct"; };

function renderTerm() {
  $("te-tabs").innerHTML = CFG.switches.map(s => '<button class="chip' + (s.name === SW ? " on" : "") + '" data-goto="term/' + esc(s.name) + '">' + esc(s.name) + "</button>").join("");
  const w = teWrite(SW);
  $("te-mode").className = "pill " + (w ? "warn" : "ok");
  $("te-mode").textContent = t(w ? "Read and write (write account)" : "Read only: print, export, monitor");
  $("te-prompt").textContent = "[" + (SW || "") + "] >";
  // a line typed for one switch never runs on another
  if (renderTerm.sw !== SW) { renderTerm.sw = SW; $("te-in").value = ""; TE_POS = TE_HIST.length; }
  const lines = TE_OUT[SW] || [];
  const key = SW + "|" + lines.length + "|" + LANG;
  if (key === TE_DRAWN) return; // untouched: keeps the scroll and a selection
  TE_DRAWN = key;
  const out = $("te-out");
  out.innerHTML = lines.length ? lines.map(l => '<span class="te-cmd">[' + esc(SW) + "] &gt; " + esc(l.cmd) + "</span>\n" +
    (l.out ? '<span class="' + (l.ok ? "" : "te-err") + '">' + esc(l.out.replace(/\s+$/, "")) + "</span>\n" : "") +
    (l.err ? '<span class="te-err">' + esc(problemText(l.err)) + "</span>\n" : "")).join("") :
    '<span class="sub">' + esc(t("Type a RouterOS command, e.g. /interface print")) + "</span>";
  out.scrollTop = out.scrollHeight;
}

async function teRun(cmd, confirm) {
  const sw = SW;
  TE_BUSY = true; $("te-in").disabled = true;
  try {
    const r = await post("/api/term/run", { switch: sw, command: cmd, confirm });
    if (r.confirm) {
      const f = await ask(t("Confirm on {name}", { name: sw }), "<p>" + esc(t("This command touches:")) + "</p><ul>" +
        r.confirm.map(x => "<li>" + esc(problemText(x)) + "</li>").join("") + '</ul><pre class="tl-cmd">' + esc(r.command) + "</pre><p>" +
        esc(t("Type {name} to run it.", { name: sw })) + '</p><label><input name="confirm" required autocomplete="off" autocapitalize="off"></label>', t("Run"));
      if (!f) return;
      return await teRun(cmd, f.get("confirm"));
    }
    if (r.command) teRemember(r.command);
    (TE_OUT[sw] = TE_OUT[sw] || []).push({ cmd: r.command || cmd, out: r.output || "", ok: r.ok, err: r.error || "", account: r.account });
    if (r.ok || r.output !== undefined) $("te-in").value = "";
  } catch (e) {
    (TE_OUT[sw] = TE_OUT[sw] || []).push({ cmd, out: "", ok: false, err: e.message });
  } finally {
    TE_BUSY = false; $("te-in").disabled = false; $("te-in").focus();
    if (VIEW === "term") renderTerm();
  }
}

function teRemember(cmd) {
  if (TE_HIST[TE_HIST.length - 1] !== cmd) TE_HIST.push(cmd);
  TE_HIST = TE_HIST.slice(-200);
  TE_POS = TE_HIST.length;
  try { localStorage.setItem(TE_HIST_KEY, JSON.stringify(TE_HIST)); } catch (e) { /* private window */ }
}

$("te-form").addEventListener("submit", e => {
  e.preventDefault();
  const cmd = $("te-in").value.trim();
  if (cmd && !TE_BUSY) teRun(cmd);
});

$("te-in").addEventListener("keydown", e => {
  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
  e.preventDefault();
  TE_POS = Math.max(0, Math.min(TE_HIST.length, TE_POS + (e.key === "ArrowUp" ? -1 : 1)));
  e.target.value = TE_HIST[TE_POS] || "";
});

document.addEventListener("click", e => {
  const b = e.target.closest("button");
  if (!b || VIEW !== "term") return;
  if (b.dataset.act === "te-copy") {
    const said = ok => { b.textContent = t(ok ? "Copied" : "Copy failed"); setTimeout(() => { b.textContent = t("Copy"); }, 1500); };
    // clipboard API refused (permissions, embedded browser): the text selected and copied the old way
    const old = () => { getSelection().selectAllChildren($("te-out")); let ok = false; try { ok = document.execCommand("copy"); } catch (e) { /* stays selected */ } said(ok); };
    (navigator.clipboard ? navigator.clipboard.writeText($("te-out").textContent) : Promise.reject()).then(() => said(true), old);
  }
  if (b.dataset.act === "te-clear") { TE_OUT[SW] = []; renderTerm(); $("te-in").focus(); }
});

PROBLEMS.push(
  [/^management path \((.*)\)$/, "management path ({v})", 1],
  [/^(\S+) is read-only here \(mode (\S+)\): print, export and monitor only$/, "{v} is read-only here (mode {net}): print, export and monitor only", 1],
);

for (const [k, v] of Object.entries({
  "Terminal": "Terminal", "Read and write (write account)": "Lecture et écriture (compte d'écriture)", "Read only: print, export, monitor": "Lecture seule : print, export, monitor",
  "Type a RouterOS command, e.g. /interface print": "Tapez une commande RouterOS, par exemple /interface print", "Copy": "Copier", "Clear": "Effacer", "Run": "Lancer",
  "Copied": "Copié", "Copy failed": "Échec de la copie", "Confirm on {name}": "Confirmer sur {name}", "This command touches:": "Cette commande touche :",
  "Type {name} to run it.": "Tapez {name} pour la lancer.", "accounts (/user, /password)": "les comptes (/user, /password)", "services (/ip service)": "les services (/ip service)",
  "restart or shutdown": "le redémarrage ou l'arrêt", "management path ({v})": "le chemin de gestion ({v})",
  "{v} is read-only here (mode {net}): print, export and monitor only": "{v} est en lecture seule ici (mode {net}) : print, export et monitor seulement",
  "/system reset-configuration is refused: it would drop the app's accounts and addresses": "/system reset-configuration est refusé : il effacerait les comptes et adresses de l'application",
  "no end after 15 s, stopped: add once (monitor) or count= (ping)": "pas de fin au bout de 15 s, arrêté : ajoutez once (monitor) ou count= (ping)",
  "one line of at most 4096 characters": "une ligne de 4096 caractères au plus",
  "Commands go through the RouterOS API one at a time (no interactive prompts, no Tab completion). Logged with your name; values of password=, secret= and key= are hidden.":
    "Les commandes passent une à une par l'API RouterOS (pas d'invite interactive, pas de complétion Tab). Journalisées à votre nom ; les valeurs de password=, secret= et key= sont masquées.",
})) if (!(k in DICT.fr)) DICT.fr[k] = v;
