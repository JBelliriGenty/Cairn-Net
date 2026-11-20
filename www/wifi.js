// cairn-net: Wi-Fi page (src/wifi.rs), loaded after panel.js (uses its helpers):
// every access point as one network: radios, SSIDs merged by name with the
// differences between access points, the channel map, the clients of all
// access points with their name and IP, connections and roaming read from
// the access points' logs (lot Wi-Fi 1). The Wi-Fi state (lot Wi-Fi 3), taken
// once from an access point: SSIDs common to all (name, VLAN, security,
// password, isolation, hidden, bands), radios of each (channel, width,
// power, set from the access point's card); each access point shows what
// applying it would change, applied one at a time in direct mode, previewed,
// read back, undone by the access point unless kept (lot Wi-Fi 2). A new
// access point gets a first-setup script, then is adopted.
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
  "Set": "Régler", "automatic": "automatique", "up to {n} MHz": "jusqu'à {n} MHz",
  "Transmit power (dBm)": "Puissance d'émission (dBm)", "automatic, max {n}": "automatique, max {n}", "radar": "radar",
  "Clients isolated from each other": "Clients isolés entre eux",
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
  // lot Wi-Fi 3: the Wi-Fi state, new access points
  "No Wi-Fi state yet: Cairn Net takes it from an access point (SSIDs, passwords in direct mode, radios), then keeps every access point in line with it.":
    "Pas encore d'état voulu : Cairn Net le reprend d'une borne (SSID, mots de passe en mode direct, radios), puis tient toutes les bornes en accord avec lui.",
  "Take it from {name}": "Le reprendre de {name}", "Edit the SSIDs": "Modifier les SSID",
  "Take the Wi-Fi state from {name}?": "Reprendre l'état voulu de {name} ?",
  "Its SSIDs replace those of the state (with their passwords in direct mode), and its radios' settings are kept for it. Nothing is written to the access points.":
    "Ses SSID remplacent ceux de l'état voulu (avec leurs mots de passe en mode direct), et les réglages de ses radios sont gardés pour elle. Rien n'est écrit sur les bornes.",
  "Wi-Fi state taken from {name}.": "État voulu repris de {name}.",
  "{n} SSID in the state, the same on every access point. Edit it, then apply it on each access point from its card.":
    "{n} SSID dans l'état voulu, les mêmes sur toutes les bornes. Modifiez-le, puis appliquez-le sur chaque borne depuis sa carte.",
  "{n} SSIDs in the state, the same on every access point. Edit it, then apply it on each access point from its card.":
    "{n} SSID dans l'état voulu, les mêmes sur toutes les bornes. Modifiez-le, puis appliquez-le sur chaque borne depuis sa carte.",
  "in line with the Wi-Fi state": "conforme à l'état voulu", "{n} change to apply": "{n} changement à appliquer", "{n} changes to apply": "{n} changements à appliquer",
  "{name}: {n} change to apply (Wi-Fi state)": "{name} : {n} changement à appliquer (état voulu)", "{name}: {n} changes to apply (Wi-Fi state)": "{name} : {n} changements à appliquer (état voulu)",
  "{name}: the Wi-Fi state cannot be applied": "{name} : l'état voulu ne peut pas être appliqué",
  "Apply the state": "Appliquer l'état", "Apply the Wi-Fi state on {name}?": "Appliquer l'état voulu sur {name} ?", "Adopt": "Adopter",
  "Passwords are compared once applying (the read account cannot see them).": "Les mots de passe sont comparés à l'application (le compte de lecture ne les voit pas).",
  "Radios of {name}": "Radios de {name}",
  "Saved in the Wi-Fi state, then previewed on this access point: nothing is written before you apply.":
    "Enregistré dans l'état voulu, puis aperçu sur cette borne : rien n'est écrit avant que vous appliquiez.",
  "SSIDs of the Wi-Fi state": "SSID de l'état voulu", "New SSID": "Nouveau SSID", "Name": "Nom", "none (untagged)": "aucun (non étiqueté)",
  "Security": "Sécurité", "Password": "Mot de passe", "required": "requis", "Hidden SSID": "SSID masqué",
  "Remove this SSID from every access point": "Retirer ce SSID de toutes les bornes",
  "Saved in the Wi-Fi state only. Each access point shows what applying it changes; a removed SSID disappears from them, a new VLAN must already pass through the switch port of each access point.":
    "Enregistré dans l'état voulu seulement. Chaque borne montre ce que son application change ; un SSID retiré disparaît des bornes, un nouveau VLAN doit déjà passer par le port du switch de chaque borne.",
  "Wi-Fi state saved: apply it on each access point.": "État voulu enregistré : appliquez-le sur chaque borne.",
  "Add an access point": "Ajouter une borne", "First setup of a new access point": "Première configuration d'une nouvelle borne",
  "Cairn Net writes a first-setup script without any secret: bridge and uplink, management address, radios off, accounts for Cairn Net (the write one with its SSH key). Then it adopts the access point and applies the Wi-Fi state there.":
    "Cairn Net écrit un script de première configuration sans aucun secret : pont et lien montant, adresse de gestion, radios coupées, comptes pour Cairn Net (celui d'écriture avec sa clé SSH). Puis il adopte la borne et y applique l'état voulu.",
  "Address (management)": "Adresse (gestion)", "Gateway": "Passerelle", "Management VLAN": "VLAN de gestion", "Uplink port": "Port montant",
  "Untagged VLAN of the uplink": "VLAN non étiqueté du lien montant", "DNS servers": "Serveurs DNS", "Country (as RouterOS spells it)": "Pays (comme l'écrit RouterOS)",
  "2.4 GHz channel": "Canal 2,4 GHz", "5 GHz channel": "Canal 5 GHz", "Read account like": "Compte de lecture comme", "a new one": "un nouveau",
  "Script": "Script", "Script for {name}": "Script de {name}", "Download {file}": "Télécharger {file}", "Adopt now": "Adopter maintenant",
  "On the blank access point (reset without defaults), upload this file, check it with /import file-name={file} verbose=yes dry-run, then reset again with run-after-reset={file}. Once it answers at {ip}, adopt it.":
    "Sur la borne vierge (remise à zéro sans configuration), déposez ce fichier, vérifiez-le par /import file-name={file} verbose=yes dry-run, puis refaites une remise à zéro avec run-after-reset={file}. Dès qu'elle répond en {ip}, adoptez-la.",
  "Adopt {name}?": "Adopter {name} ?",
  "First, once on {name}: these two RouterOS lines create cairn-write with Cairn Net's key. Without them, adopting is refused.":
    "D'abord, une fois sur {name} : ces deux lignes RouterOS créent cairn-write avec la clé de Cairn Net. Sans elles, l'adoption est refusée.",
  "Or in one command from your computer, as": "Ou en une commande depuis votre ordinateur, en tant que",
  "Then Cairn Net logs in as cairn-write with its SSH key, gives that account a password only Cairn Net knows, checks the API certificate and puts {name} in direct mode.":
    "Ensuite Cairn Net se connecte en cairn-write avec sa clé SSH, donne à ce compte un mot de passe que lui seul connaît, vérifie le certificat de l'API et passe {name} en mode direct.",
  "cairn-write does not accept Cairn Net's key: run the two RouterOS lines first": "cairn-write n'accepte pas la clé de Cairn Net : passez d'abord les deux lignes RouterOS",
  "cairn-write does not accept Cairn Net's key: the first-setup script did not run there":
    "cairn-write n'accepte pas la clé de Cairn Net : le script de première configuration n'y a pas tourné",
  "{name} adopted: direct mode.": "{name} adoptée : mode direct.",
  "{who}: no password for it in the Wi-Fi state: set one": "{who} : pas de mot de passe pour lui dans l'état voulu : donnez-en un",
  "{who}: its clients reconnect in another VLAN (new address)": "{who} : ses clients se reconnectent dans un autre VLAN (nouvelle adresse)",
  "{who}: some devices must join it again": "{who} : certains appareils doivent le rejoindre de nouveau",
  "VLAN {v} does not pass through {net}: add it there first": "le VLAN {v} ne passe pas par {net} : ajoutez-le d'abord là",
  "{v}: no uplink found to carry VLAN {net}": "{v} : aucun lien montant pour porter le VLAN {net}",
  "{v}: the switch across its uplink is unknown, VLAN {net} not checked there": "{v} : switch en face de son lien montant inconnu, VLAN {net} non vérifié de ce côté",
  "{v}: removed from {net}, its clients are cut": "{v} : retiré de {net}, ses clients sont déconnectés",
  "no Wi-Fi state yet: take it from an access point first": "pas encore d'état voulu : reprenez-le d'abord d'une borne",
  "the Wi-Fi state has no SSID": "l'état voulu n'a aucun SSID",
  "{who}: twice in the list": "{who} : deux fois dans la liste", "{who}: VLAN 1 to 4094, or none": "{who} : VLAN de 1 à 4094, ou aucun",
  "{who}: a Wi-Fi password is 8 to 63 printable ASCII characters": "{who} : un mot de passe Wi-Fi fait 8 à 63 caractères ASCII imprimables",
  "{who}: letters, digits and dashes": "{who} : lettres, chiffres et tirets", "{who}: a.b.c.d/prefix": "{who} : a.b.c.d/préfixe", "{who}: an IPv4 address": "{who} : une adresse IPv4",
  "{who}: IPv4 addresses": "{who} : des adresses IPv4", "{who}: its English name, as RouterOS spells it": "{who} : son nom anglais, comme l'écrit RouterOS",
  "the API certificate is not the one Settings trust: not adopted": "le certificat de l'API n'est pas celui que les Réglages approuvent : pas adoptée",
  "{who}: the management VLAN, 1 to 4094": "{who} : le VLAN de gestion, de 1 à 4094", "{who}: an interface name": "{who} : un nom d'interface",
  "{who}: the management VLAN is tagged, the untagged VLAN is another one": "{who} : le VLAN de gestion est étiqueté, le VLAN non étiqueté en est un autre",
  // lot Wi-Fi 4a: roaming, blocked devices, guests
  "Fast roaming (802.11r)": "Itinérance rapide (802.11r)", "Roaming help (802.11k/v)": "Aide à l'itinérance (802.11k/v)",
  "Steer dual-band devices to 5 GHz": "Orienter les appareils bi-bande vers le 5 GHz", "5 GHz first": "5 GHz d'abord",
  "Band steering": "Orientation de bande",
  "{who}: some older devices cannot join with 802.11r: check them after applying": "{who} : certains appareils anciens ne se connectent pas avec le 802.11r : vérifiez-les après l'application",
  "{who}: 802.11r needs a password (WPA2 or WPA3)": "{who} : le 802.11r demande un mot de passe (WPA2 ou WPA3)",
  "QR code": "QR code", "Join {ssid}": "Rejoindre {ssid}", "Open network: no password": "Réseau ouvert : pas de mot de passe",
  "Point a phone camera at the code to join.": "Visez le code avec l'appareil photo d'un téléphone pour se connecter.",
  "New password": "Nouveau mot de passe", "Close": "Fermer", "Give {ssid} a new password?": "Donner un nouveau mot de passe à {ssid} ?",
  "Cairn Net draws a new random password, saves it in the Wi-Fi state, then applies the state on each access point. Every device on {ssid} needs the new password: the old one stops working once applied.":
    "Cairn Net tire un nouveau mot de passe au hasard, l'enregistre dans l'état voulu, puis applique l'état sur chaque borne. Chaque appareil de {ssid} doit recevoir le nouveau mot de passe : l'ancien ne marche plus une fois appliqué.",
  "New password for {ssid} saved: keep it on each access point, then show the QR code.": "Nouveau mot de passe de {ssid} enregistré : gardez-le sur chaque borne, puis montrez le QR code.",
  "{who}: not in the Wi-Fi state": "{who} : absent de l'état voulu",
  "{v} is open (no security): no password to change": "{v} est ouvert (sans sécurité) : pas de mot de passe à changer",
  "Actions": "Actions", "Disconnect": "Déconnecter", "Block": "Bloquer", "Unblock": "Débloquer",
  "Disconnect {name}?": "Déconnecter {name} ?", "It leaves {ap} now and usually joins again at once: block it to keep it out.":
    "Il quitte {ap} tout de suite et revient en général aussitôt : bloquez-le pour le tenir dehors.",
  "{name} disconnected from {ap}.": "{name} déconnecté de {ap}.", "{v} is not connected to {net}": "{v} n'est pas connecté à {net}",
  "Restart radio": "Relancer", "Restart {iface} on {ap}?": "Relancer {iface} sur {ap} ?",
  "The radio stops three seconds, then starts again with its SSIDs: their clients join again by themselves. Useful when devices can no longer join while those connected stay.":
    "La radio s'arrête trois secondes puis repart avec ses SSID : leurs clients se reconnectent d'eux-mêmes. Utile quand des appareils n'arrivent plus à se connecter alors que ceux déjà connectés restent.",
  "{iface} of {ap} restarted: its clients join again within a few seconds.": "{iface} de {ap} relancée : ses clients reviennent dans quelques secondes.",
  "{who}: no such Wi-Fi interface": "{who} : pas d'interface Wi-Fi de ce nom",
  "{who}: an SSID, not a radio: restart its radio": "{who} : un SSID, pas une radio : relancez sa radio",
  "{who}: disabled, nothing to restart": "{who} : désactivée, rien à relancer",
  "Block {name}?": "Bloquer {name} ?",
  "Saved in the Wi-Fi state, then applied on each access point: every one refuses {mac} (access list) and disconnects it. Blocking the device you are using cuts you off: then do not keep the change, it undoes itself.":
    "Enregistré dans l'état voulu, puis appliqué sur chaque borne : toutes refusent {mac} (liste d'accès) et le déconnectent. Bloquer l'appareil que vous utilisez vous coupe : ne gardez pas le changement, il s'annule seul.",
  "Unblock {name}?": "Débloquer {name} ?", "Saved in the Wi-Fi state, then applied on each access point: {mac} may join again.":
    "Enregistré dans l'état voulu, puis appliqué sur chaque borne : {mac} peut de nouveau se connecter.",
  "Blocked on every access point:": "Bloqués sur toutes les bornes :",
  "{v}: blocked, disconnected from {net}": "{v} : bloqué, déconnecté de {net}",
  "{who}: not a MAC address (AA:BB:CC:DD:EE:FF)": "{who} : pas une adresse MAC (AA:BB:CC:DD:EE:FF)", "{who}: blocked twice": "{who} : bloqué deux fois",
  // lot Wi-Fi 4b: SSID schedules
  "On air from": "En service de", "until": "à", "all day": "toute la journée",
  "Mon": "lun.", "Tue": "mar.", "Wed": "mer.", "Thu": "jeu.", "Fri": "ven.", "Sat": "sam.", "Sun": "dim.",
  "Hours left empty: always on air. Otherwise Cairn Net turns the SSID on and off on those days, by each access point's clock (from 22:00 until 07:00 runs past midnight). Best for an SSID of its own (guests): the radios' main SSID cannot go off without those it carries.":
    "Heures vides : toujours en service. Sinon Cairn Net allume et coupe le SSID ces jours-là, à l'heure de chaque borne (de 22:00 à 07:00 passe minuit). Pour un SSID à part (invités) : le SSID principal des radios ne peut pas être coupé sans ceux qu'il porte.",
  "{who}: a schedule is days 1 (Monday) to 7, from and to as HH:MM": "{who} : un créneau, ce sont des jours de 1 (lundi) à 7 et des heures HH:MM",
});
PROBLEMS.push(
  [/^([0-9A-F:]{17}): blocked, disconnected from (\S+)$/, "{v}: blocked, disconnected from {net}", 1],
  [/^(\S+) is not connected to (\S+)$/, "{v} is not connected to {net}", 1],
  [/^(.+) is open \(no security\): no password to change$/, "{v} is open (no security): no password to change", 1],
  [/^([0-9A-Fa-f:]*): not a MAC address \(AA:BB:CC:DD:EE:FF\)$/, "{who}: not a MAC address (AA:BB:CC:DD:EE:FF)"],
  [/^([0-9A-F:]{17}): blocked twice$/, "{who}: blocked twice"],
  [/^(\S*): no such Wi-Fi interface$/, "{who}: no such Wi-Fi interface"],
  [/^(\S+): an SSID, not a radio: restart its radio$/, "{who}: an SSID, not a radio: restart its radio"],
  [/^(\S+): disabled, nothing to restart$/, "{who}: disabled, nothing to restart"],
);
PROBLEMS.push(
  [/^VLAN (\d+) does not pass through (.+): add it there first$/, "VLAN {v} does not pass through {net}: add it there first", 1],
  [/^(\S+): no uplink found to carry VLAN (\d+)$/, "{v}: no uplink found to carry VLAN {net}", 1],
  [/^(\S+): the switch across its uplink is unknown, VLAN (\d+) not checked there$/, "{v}: the switch across its uplink is unknown, VLAN {net} not checked there", 1],
  [/^(.+): removed from (\S+), its clients are cut$/, "{v}: removed from {net}, its clients are cut", 1],
);

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
  for (const a of v.state ? v.aps : []) {
    if (a.plan_error) out.push(["bad", t("{name}: the Wi-Fi state cannot be applied", { name: a.name })]);
    else if ((a.plan || []).length) out.push(["warn", tp(a.plan.length, "{name}: {n} change to apply (Wi-Fi state)", "{name}: {n} changes to apply (Wi-Fi state)", { name: a.name })]);
  }
  return out;
}

function renderWifi() {
  const v = WF, vd = $("wf-verdict");
  wfPending(v);
  if (!v || v.error || !v.aps.length) {
    vd.className = "verdict" + (v && v.error ? " bad" : v ? " warn" : "");
    vd.innerHTML = v ? '<span class="dot ' + (v.error ? "bad" : "warn") + '"></span>' + esc(v.error ? t("Read failed: {err}", { err: v.error }) :
      t("No access point: the switches with Wi-Fi radios (RouterOS wifi package) show up here.")) : esc(t("Loading…"));
    for (const id of ["wf-aps", "wf-map", "wf-state", "wf-ssids", "wf-clients", "wf-events"]) wfSet(id, "");
    return;
  }
  const iss = wfIssues(v), lvl = worst(iss.map(([l]) => l));
  vd.className = "verdict " + lvl;
  vd.innerHTML = '<span class="dot ' + lvl + '"></span>' + esc(iss.length ? iss.map(([, m]) => m).join(" · ") :
    t("All good: {aps}, {clients}, {ssids}", { aps: tp(v.aps.length, "{n} access point", "{n} access points"),
      clients: tp(v.clients.length, "{n} Wi-Fi client", "{n} Wi-Fi clients"), ssids: tp(v.ssids.length, "{n} SSID", "{n} SSIDs") }));
  wfSet("wf-aps", wfAps(v));
  wfSet("wf-map", wfMap(v));
  wfSet("wf-state", wfStateBlock(v));
  wfSet("wf-ssids", wfSsids(v));
  wfSet("wf-clients", wfClients(v));
  wfSet("wf-events", wfEvents(v));
  $("view-title").textContent = t(VIEWS.wifi) + " · " + tp(v.clients.length, "{n} client", "{n} clients");
}

/* access points and their radios */

// control channel, and the channels a wider block covers: "36 (36–40)"
const wfChanOf = f => (f === 2484 ? 14 : f < 3000 ? (f - 2407) / 5 : f < 5925 ? (f - 5000) / 5 : (f - 5950) / 5);
const wfChanCell = r => (r.channel == null ? "—" : r.width > 20 ? r.channel + " (" + wfChanOf(r.lo + 10) + "–" + wfChanOf(r.hi - 10) + ")" : String(r.channel));

const WF_OPEN = new Set(); // access points whose list of changes is open, kept across redraws
document.addEventListener("toggle", e => {
  const p = e.target.dataset && e.target.dataset.plan;
  if (p) e.target.open ? WF_OPEN.add(p) : WF_OPEN.delete(p);
}, true);

// how far an access point is from the Wi-Fi state (read account: passwords aside)
function wfPlan(v, a) {
  if (!v.state) return "";
  if (a.plan_error) return '<p class="banner bad-note">' + esc(problemText(a.plan_error)) + "</p>";
  const plan = a.plan || [];
  if (!plan.length) return '<div class="toolbar"><span class="pill ok">' + esc(t("in line with the Wi-Fi state")) + "</span></div>";
  return '<details data-plan="' + esc(a.name) + '"' + (WF_OPEN.has(a.name) ? " open" : "") + '><summary><span class="pill warn">' +
    esc(tp(plan.length, "{n} change to apply", "{n} changes to apply")) + '</span></summary><pre class="diff">' + esc(plan.join("\n")) + "</pre>" +
    (a.plan_warn || []).map(w => '<p class="sub">' + esc(problemText(w)) + "</p>").join("") +
    '<p class="sub">' + esc(t("Passwords are compared once applying (the read account cannot see them).")) + "</p></details>";
}

const wfBtn = (act, ap, label, why, cls) => '<button type="button"' + (cls ? ' class="' + cls + '"' : "") + ' data-act="' + act + '" data-ap="' + esc(ap) + '"' +
  (why ? ' disabled title="' + esc(t(why)) + '"' : "") + ">" + esc(t(label)) + "</button>";

function wfAps(v) {
  return '<div class="grid wf-aps">' + v.aps.map(a => {
    if (a.error) return '<article class="card srv bad"><div class="card-head"><h3>' + esc(a.name) + '</h3><span class="pill bad">' + esc(t("unreachable")) +
      '</span></div><p class="sub">' + esc(a.error) + "</p></article>";
    const lv = a.radios.some(r => r.state !== "running" && r.state !== "disabled") ? "warn" : "ok";
    const direct = a.mode === "direct" && a.has_write;
    const why = !direct ? "Direct mode only" : !v.state ? "no Wi-Fi state yet: take it from an access point first" : "";
    return '<article class="card srv ' + lv + '"><div class="card-head"><h3>' + esc(a.name) + '</h3><span class="pill">' + esc(tp(a.clients, "{n} client", "{n} clients")) +
      '</span></div><p class="sub">' + esc(a.model) + '</p><div class="table-wrap"><table>' + wfTh([t("Band"), t("Channel"), t("Width"), t("Power"), t("State"), t("Clients")]) + "<tbody>" +
      a.radios.map(r => '<tr class="' + (r.state === "running" ? "" : "dim") + '"><td><b>' + esc(wfBand(r.band)) + '</b><br><small class="sub">' + esc([r.iface, ...r.ssids].join(" · ")) + "</small></td>" +
        "<td>" + esc(wfChanCell(r)) + (r.auto ? ' <span class="pill">' + esc(t("auto")) + "</span>" : "") + "</td>" +
        '<td title="' + esc(r.width_cfg) + '">' + esc(r.width ? r.width + " MHz" : "—") + "</td>" +
        "<td>" + esc(r.tx_power != null ? r.tx_power + " dBm" : "—") + (r.max_power != null ? '<br><small class="sub">' + esc(t("max {n} dBm", { n: r.max_power })) + "</small>" : "") + "</td>" +
        '<td><span class="pill ' + (r.state === "running" ? "ok" : r.state === "disabled" ? "" : "warn") + '">' + esc(wfState(r.state)) + "</span>" +
        (direct && r.state !== "disabled" ? '<br><button type="button" data-act="wf-restart" data-ap="' + esc(a.name) + '" data-iface="' + esc(r.iface) + '">' + esc(t("Restart radio")) + "</button>" : "") +
        '</td><td class="num">' + esc(r.clients) + "</td></tr>").join("") +
      "</tbody></table></div>" + wfPlan(v, a) + '<div class="toolbar">' + wfBtn("wf-edit", a.name, "Set", why) +
      wfBtn("wf-apply", a.name, "Apply the state", why, (a.plan || []).length ? "primary" : "") +
      (direct ? "" : wfBtn("wf-adopt", a.name, "Adopt")) + "</div></article>";
  }).join("") + '</div><div class="toolbar">' + wfBtn("wf-add", "", "Add an access point") + "</div>";
}

/* one access point (direct mode): its radios in the state, then the state previewed, applied, kept */

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

// a POST whose refusal goes to the banner; fields: server field names -> labels of the form
async function wfPost(path, body, fields) {
  let r;
  try { r = await post(path, body); } catch (e) { r = { ok: false, error: e.message }; }
  if (r.ok) return r;
  const e = String(r.error || ""), m = e.match(/^(\w+): (.*)$/);
  banner(t("Refused: {err}", { err: m && fields && fields[m[1]] ? t("{who}: " + m[2], { who: t(fields[m[1]]) }) : problemText(e) }));
  return null;
}

const WF_WIDTH = { 20: "20mhz", 40: "20/40mhz", 80: "20/40/80mhz", 160: "20/40/80/160mhz", 320: "20/40/80/160/320mhz" };
const wfOpt = (val, label, cur) => '<option value="' + esc(val) + '"' + (val === cur ? " selected" : "") + ">" + esc(label) + "</option>";

// c: the radio as the state has it ({frequency, width, tx_power}, "" = automatic)
function wfRadioForm(r, c) {
  const n = esc(r.iface), ws = [...new Set(r.channels.flatMap(x => x.w.map(([w]) => w)))].sort((x, y) => x - y);
  const radar = x => (x.w.find(([w]) => w === 20) || [0, false])[1];
  const fo = [wfOpt("", t("automatic"), c.frequency), ...r.channels.map(x => wfOpt(String(x.f), t("ch {c}", { c: x.ch }) + " · " + x.f + " MHz" + (radar(x) ? " · " + t("radar") : ""), c.frequency))];
  if (c.frequency && !r.channels.some(x => String(x.f) === c.frequency)) fo.push(wfOpt(c.frequency, c.frequency, c.frequency)); // a list or range set by hand
  const wo = [wfOpt("", t("automatic"), c.width), ...ws.map(w => wfOpt(WF_WIDTH[w], t("up to {n} MHz", { n: w }), c.width))];
  if (c.width && !ws.some(w => WF_WIDTH[w] === c.width)) wo.push(wfOpt(c.width, c.width, c.width));
  return '<fieldset class="rows"><legend>' + esc(r.iface + " · " + wfBand(r.band)) + '</legend><div class="cols2">' +
    "<label>" + esc(t("Channel")) + '<select name="f-' + n + '">' + fo.join("") + "</select></label>" +
    "<label>" + esc(t("Width")) + '<select name="w-' + n + '">' + wo.join("") + "</select></label></div>" +
    "<label>" + esc(t("Transmit power (dBm)")) + '<input type="number" name="p-' + n + '" min="1" max="' + esc(r.max_power || 30) + '" step="1" value="' + esc(c.tx_power) +
    '" placeholder="' + esc(t("automatic, max {n}", { n: r.max_power || "?" })) + '"></label></fieldset>';
}

async function wfEdit(name) {
  const a = (WF.aps || []).find(x => x.name === name);
  if (!a || a.error || !WF.state) return;
  const was = (WF.state.radios || {})[name] || {};
  const cur = r => was[r.iface] || { frequency: r.freq_cfg, width: r.width_cfg, tx_power: r.tx_cfg };
  const fd = await ask(t("Radios of {name}", { name }), a.radios.map(r => wfRadioForm(r, cur(r))).join("") +
    '<p class="sub">' + esc(t("Saved in the Wi-Fi state, then previewed on this access point: nothing is written before you apply.")) + "</p>", t("Preview"));
  if (!fd) return;
  const field = k => String(fd.get(k) ?? "").trim();
  const radios = Object.fromEntries(a.radios.map(r => [r.iface, { frequency: field("f-" + r.iface), width: field("w-" + r.iface), tx_power: field("p-" + r.iface) }]));
  const changed = a.radios.some(r => ["frequency", "width", "tx_power"].some(k => radios[r.iface][k] !== (cur(r)[k] || "")));
  if (changed && !(await wfPost("/api/wifi/state", { op: "radios", switch: name, radios }))) return;
  if (changed) await loadWifi();
  return wfSend(name);
}

// the Wi-Fi state on one access point: preview, then guarded apply
async function wfSend(name) {
  const p = await wfPost("/api/wifi/preview", { switch: name });
  if (!p) return;
  if (!p.commands.length) return banner(t("Nothing to change."), "ok");
  banner("");
  if (!(await ask(t("Apply the Wi-Fi state on {name}?", { name }),
    p.warnings.map(x => '<p class="banner">' + esc(problemText(x)) + "</p>").join("") +
    "<h3>" + esc(t("Commands")) + '</h3><pre class="diff">' + esc(p.commands.join("\n")) + "</pre>" +
    "<h3>" + esc(t("Rollback, run by the switch after {n} s unless you keep the change", { n: p.rollback_s })) + '</h3><pre class="diff">' + esc(p.rollback.join("\n")) + "</pre>",
    t("Apply")))) return;
  let r;
  try { r = await post("/api/wifi/apply", { switch: name }); } catch (e) { r = { ok: false, error: e.message }; }
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

/* adoption: Cairn Net logs in with its SSH key, sets passwords only it knows, direct mode */

async function wfAdopt(name) {
  let st = {}, who = "admin";
  try { st = await get("/api/settings"); } catch (e) { /* the lines keep placeholders */ }
  try { who = localStorage.getItem("cairn-ap-admin") || who; } catch { /* default account */ }
  const app = WF.app || "<Cairn Net>", host = ((st.switches || []).find(x => x.name === name) || {}).host || "<" + name + ">";
  const lines = ["/user add name=cairn-write group=full address=" + app + '/32 password=[:rndstr length=32] comment="Cairn Net: writes"',
    '/user ssh-keys add user=cairn-write key="' + (st.git_key || "ssh-ed25519 …") + '"'];
  const cmd = u => "ssh " + u + "@" + host + " '" + lines.join("; ") + "'";
  const body = '<p class="banner warn-note">' + esc(t("First, once on {name}: these two RouterOS lines create cairn-write with Cairn Net's key. Without them, adopting is refused.", { name })) + "</p>" +
    '<pre class="diff">' + esc(lines.join("\n")) + "</pre><label>" + esc(t("Or in one command from your computer, as")) +
    ' <input name="wf-who" value="' + esc(who) + '" autocomplete="off" spellcheck="false"></label><pre class="diff" id="wf-cmd">' + esc(cmd(who)) + "</pre>" +
    '<div class="toolbar"><button type="button" id="wf-cmd-copy">' + esc(t("Copy")) + "</button></div>" +
    '<p class="sub">' + esc(t("Then Cairn Net logs in as cairn-write with its SSH key, gives that account a password only Cairn Net knows, checks the API certificate and puts {name} in direct mode.", { name })) + "</p>";
  if (!(await ask(t("Adopt {name}?", { name }), body, t("Adopt"), d => {
    const i = d.querySelector("[name=wf-who]"), pre = d.querySelector("#wf-cmd");
    i.addEventListener("keydown", e => { if (e.key === "Enter") e.preventDefault(); }); // Enter would adopt
    i.addEventListener("input", () => {
      const u = i.value.trim() || "admin";
      pre.textContent = cmd(u);
      try { localStorage.setItem("cairn-ap-admin", u); } catch { /* not remembered */ }
    });
    // clipboard refused (plain http, browser policy): the command is selected for Ctrl+C instead
    d.querySelector("#wf-cmd-copy").addEventListener("click", e => navigator.clipboard.writeText(pre.textContent)
      .then(() => { e.target.textContent = t("Copied"); }, () => getSelection().selectAllChildren(pre)));
  }))) return;
  return wfAdoptPost({ name });
}

async function wfAdoptPost(body) {
  const r = await wfPost("/api/wifi/adopt", body);
  if (r) banner(t("{name} adopted: direct mode.", { name: r.name }), "ok");
  await loadWifi();
}

/* a new access point: first-setup script (no secret), then adoption */

let WF_BOOT = null; // the last form, filled in again
const WF_BOOT_FIELDS = { name: "Name", address: "Address (management)", gateway: "Gateway", vlan: "Management VLAN", uplink: "Uplink port",
  pvid: "Untagged VLAN of the uplink", dns: "DNS servers", country: "Country (as RouterOS spells it)", ch24: "2.4 GHz channel", ch5: "5 GHz channel" };

async function wfAdd() {
  const ok = WF.aps.filter(a => !a.error);
  const b = WF_BOOT || { uplink: "ether1", like: ((ok.find(a => a.mode === "direct") || ok[0]) || {}).name || "" };
  const inp = (k, attrs) => "<label>" + esc(t(WF_BOOT_FIELDS[k])) + '<input name="' + k + '" value="' + esc(b[k] || "") + '" autocomplete="off" spellcheck="false" ' + attrs + "></label>";
  const fd = await ask(t("First setup of a new access point"), '<p class="sub">' + esc(t("Cairn Net writes a first-setup script without any secret: bridge and uplink, management address, radios off, accounts for Cairn Net (the write one with its SSH key). Then it adopts the access point and applies the Wi-Fi state there.")) + "</p>" +
    '<div class="cols2">' + inp("name", 'required maxlength="40" pattern="[A-Za-z0-9-]+" placeholder="ap-hall"') +
    inp("address", 'required pattern="\\d+\\.\\d+\\.\\d+\\.\\d+/\\d+" placeholder="192.168.88.2/24"') + "</div>" +
    '<div class="cols2">' + inp("gateway", 'required pattern="\\d+\\.\\d+\\.\\d+\\.\\d+" placeholder="192.168.88.1"') + inp("vlan", 'type="number" required min="1" max="4094"') + "</div>" +
    '<div class="cols2">' + inp("uplink", 'required pattern="[A-Za-z0-9-]+"') + inp("pvid", 'type="number" min="1" max="4094" placeholder="' + esc(t("none (untagged)")) + '"') + "</div>" +
    '<div class="cols2">' + inp("dns", 'required placeholder="192.168.88.1"') + inp("country", 'required placeholder="France"') + "</div>" +
    '<div class="cols2">' + inp("ch24", 'type="number" min="1" max="13" placeholder="' + esc(t("automatic")) + '"') + inp("ch5", 'type="number" min="32" max="177" placeholder="' + esc(t("automatic")) + '"') + "</div>" +
    "<label>" + esc(t("Read account like")) + '<select name="like">' + [...ok.map(a => wfOpt(a.name, a.name, b.like)), wfOpt("", t("a new one"), b.like)].join("") + "</select></label>",
    t("Script"));
  if (!fd) return;
  WF_BOOT = Object.fromEntries([...fd.entries()].map(([k, v]) => [k, String(v).trim()]));
  const r = await wfPost("/api/wifi/bootstrap", WF_BOOT, WF_BOOT_FIELDS);
  if (!r) return;
  banner("");
  const ip = WF_BOOT.address.split("/")[0], url = URL.createObjectURL(new Blob([r.script], { type: "text/plain" }));
  const go = await ask(t("Script for {name}", { name: WF_BOOT.name }), '<p class="sub">' + esc(t("On the blank access point (reset without defaults), upload this file, check it with /import file-name={file} verbose=yes dry-run, then reset again with run-after-reset={file}. Once it answers at {ip}, adopt it.", { file: r.file, ip })) +
    '</p><div class="toolbar"><a class="button" download="' + esc(r.file) + '" href="' + url + '">' + esc(t("Download {file}", { file: r.file })) + '</a></div><pre class="diff">' + esc(r.script) + "</pre>", t("Adopt now"));
  URL.revokeObjectURL(url);
  if (go) return wfAdoptPost({ name: WF_BOOT.name, host: ip, like: WF_BOOT.like });
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

const WF_WHAT = { vlan: "VLAN", security: "Security", isolation: "Isolation", hidden: "Hidden", enabled: "Enabled", bands: "Bands",
  ft: "802.11r", kv: "802.11k/v", band_steering: "Band steering" };
// roaming settings of an SSID, as pills
const wfRoam = x => [...(x.ft ? ["802.11r"] : []), ...(x.kv ? ["802.11k/v"] : []), ...(x.band_steering ? [t("5 GHz first")] : [])];
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
      wfRoam(s).map(r => ' <span class="pill">' + esc(r) + "</span>").join("") +
      "</td><td>" + esc(s.vlan || t("untagged")) + "</td><td>" + esc(wfSec(s.security)) + "</td><td>" + (s.isolation ? '<span class="pill">' + esc(t("isolated")) + "</span>" : "—") +
      '</td><td class="wrap">' + where.map(esc).join("<br>") + '</td><td class="num">' + esc(s.clients) + '</td><td class="wrap">' +
      (s.drift.length ? s.drift.map(d => '<span class="pill warn">' + esc(wfDrift(d)) + "</span>").join("<br>") : many ? '<span class="pill ok">' + esc(t("same everywhere")) + "</span>" : "—") + "</td></tr>";
  }).join("") + "</tbody></table>";
}

/* the Wi-Fi state: taken from an access point, SSIDs edited here, applied from each card */

function wfStateBlock(v) {
  const from = v.aps.filter(a => !a.error).map(a => wfBtn("wf-import", a.name, t("Take it from {name}", { name: a.name }))).join("");
  if (!v.state) return '<p class="sub">' + esc(t("No Wi-Fi state yet: Cairn Net takes it from an access point (SSIDs, passwords in direct mode, radios), then keeps every access point in line with it.")) +
    '</p><div class="toolbar">' + from + "</div>";
  const ss = v.state.ssids || [], bl = v.state.blocked || [];
  return '<p class="sub">' + esc(tp(ss.length, "{n} SSID in the state, the same on every access point. Edit it, then apply it on each access point from its card.",
    "{n} SSIDs in the state, the same on every access point. Edit it, then apply it on each access point from its card.")) + "</p>" +
    // each SSID with its QR code button (data-ap carries the SSID)
    '<div class="toolbar">' + ss.map(x => '<span class="wf-pair"><span class="pill">' + esc([x.ssid, x.vlan ? "VLAN " + x.vlan : t("untagged"), wfSec(x.security),
      (x.bands || []).length ? x.bands.map(wfBand).join(" + ") : t("SSID off"), ...(x.schedule ? [wfSched(x.schedule)] : []),
      ...(x.isolation ? [t("isolated")] : []), ...(x.hidden ? [t("hidden")] : []), ...wfRoam(x)].join(" · ")) +
      "</span>" + wfBtn("wf-share", x.ssid, "QR code") + "</span>").join("") + "</div>" +
    (bl.length ? '<p class="sub">' + esc(t("Blocked on every access point:")) + '</p><div class="toolbar">' + bl.map(b => '<span class="wf-pair"><span class="pill bad">' +
      esc(b.name ? b.name + " · " + b.mac : b.mac) + '</span><button type="button" data-act="wf-unblock" data-mac="' + esc(b.mac) + '" data-name="' + esc(b.name || "") + '">' +
      esc(t("Unblock")) + "</button></span>").join("") + "</div>" : "") +
    '<div class="toolbar">' + wfBtn("wf-ssid-edit", "", "Edit the SSIDs", "", "primary") + from + "</div>";
}

async function wfImport(name) {
  if (!(await ask(t("Take the Wi-Fi state from {name}?", { name }), '<p class="sub">' + esc(t("Its SSIDs replace those of the state (with their passwords in direct mode), and its radios' settings are kept for it. Nothing is written to the access points.")) + "</p>",
    t("Take it from {name}", { name })))) return;
  if (await wfPost("/api/wifi/state", { op: "import", switch: name })) banner(t("Wi-Fi state taken from {name}.", { name }), "ok");
  await loadWifi();
}

const WF_SEC = [["wpa2-psk,wpa3-psk", "WPA2 / WPA3"], ["wpa3-psk", "WPA3"], ["wpa2-psk", "WPA2"], ["", "open"]];
// the bands of the radios, and those the state already names
const wfBands = v => ["2.4", "5", "6"].filter(b => v.aps.some(a => (a.radios || []).some(r => r.band === b)) || (v.state.ssids || []).some(x => (x.bands || []).includes(b)));
const wfBandKey = (b, i) => "band-" + b.replace(".", "_") + "-" + i;
// a schedule {days: [1..7], from, to} as "07:00–22:00 sat. sun." (every day: no day named)
const WF_DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const wfSched = w => (w.from === w.to ? t("all day") : w.from + "–" + w.to) +
  ((w.days || []).length < 7 ? " " + (w.days || []).map(d => t(WF_DAYS[d - 1])).join(" ") : "");

// one SSID of the state in the form (x = {} for a new one)
function wfSsidSet(x, i, bands) {
  const fresh = !x.ssid, sec = fresh ? WF_SEC[0][0] : x.security || "", on = fresh ? bands : x.bands || [], w = x.schedule || {};
  const so = WF_SEC.map(([k, l]) => wfOpt(k, t(l), sec));
  if (!WF_SEC.some(([k]) => k === sec)) so.push(wfOpt(sec, wfSec(sec), sec));
  const box = (k, label, c) => '<label class="check"><input type="checkbox" name="' + k + '"' + (c ? " checked" : "") + "> " + esc(label) + "</label>";
  // a password is required for a new SSID, or one that leaves "open", unless the state has one
  const need = !x.has_passphrase && (fresh || !x.security);
  return '<fieldset class="rows" data-i="' + i + '" data-need="' + (need ? 1 : 0) + '"><legend>' + esc(fresh ? t("New SSID") : x.ssid) + "</legend>" +
    '<div class="cols2"><label>SSID<input name="ssid-' + i + '" maxlength="32" value="' + esc(x.ssid || "") + '"' + (fresh ? "" : " required") + ' autocomplete="off" spellcheck="false"></label>' +
    '<label>VLAN<input type="number" name="vlan-' + i + '" min="1" max="4094" value="' + esc(x.vlan || "") + '" placeholder="' + esc(t("none (untagged)")) + '"></label></div>' +
    '<div class="cols2"><label>' + esc(t("Security")) + '<select name="sec-' + i + '">' + so.join("") + "</select></label>" +
    "<label>" + esc(t("Password")) + '<input type="text" class="mono" name="pw-' + i + '" minlength="8" maxlength="63" autocomplete="off" spellcheck="false"></label></div>' +
    '<div class="toolbar">' + bands.map(b => box(wfBandKey(b, i), wfBand(b), on.includes(b))).join("") + "</div>" +
    '<div class="cols2"><label>' + esc(t("On air from")) + '<input type="time" name="from-' + i + '" value="' + esc(w.from || "") + '"></label>' +
    "<label>" + esc(t("until")) + '<input type="time" name="to-' + i + '" value="' + esc(w.to || "") + '"></label></div>' +
    '<div class="toolbar">' + WF_DAYS.map((d, k) => box("day" + (k + 1) + "-" + i, t(d), !w.days || w.days.includes(k + 1))).join("") + "</div>" +
    box("iso-" + i, t("Clients isolated from each other"), x.isolation) + box("hid-" + i, t("Hidden SSID"), x.hidden) +
    box("ft-" + i, t("Fast roaming (802.11r)"), x.ft) + box("kv-" + i, t("Roaming help (802.11k/v)"), x.kv) +
    box("bs-" + i, t("Steer dual-band devices to 5 GHz"), x.band_steering) +
    (fresh ? "" : box("del-" + i, t("Remove this SSID from every access point"), false)) + "</fieldset>";
}

// no password field for an open SSID; required where wfSsidSet says so
function wfPwRule(d) {
  d.querySelectorAll("fieldset[data-i]").forEach(f => {
    const i = f.dataset.i, el = k => f.querySelector('[name="' + k + "-" + i + '"]'), need = f.dataset.need === "1";
    const sync = () => {
      const sec = el("sec").value, pw = el("pw"), ft = el("ft");
      pw.disabled = ft.disabled = !sec; // 802.11r needs a password
      if (!sec) ft.checked = false;
      pw.required = !!sec && need && !!el("ssid").value.trim();
      pw.placeholder = sec ? t(pw.required ? "required" : "unchanged") : "";
    };
    el("sec").addEventListener("change", sync);
    el("ssid").addEventListener("input", sync);
    sync();
  });
}

async function wfSsidEdit() {
  if (!WF.state) return;
  const all = [...(WF.state.ssids || []), {}], bands = wfBands(WF);
  const fd = await ask(t("SSIDs of the Wi-Fi state"), all.map((x, i) => wfSsidSet(x, i, bands)).join("") +
    '<p class="sub">' + esc(t("Hours left empty: always on air. Otherwise Cairn Net turns the SSID on and off on those days, by each access point's clock (from 22:00 until 07:00 runs past midnight). Best for an SSID of its own (guests): the radios' main SSID cannot go off without those it carries.")) + "</p>" +
    '<p class="sub">' + esc(t("Saved in the Wi-Fi state only. Each access point shows what applying it changes; a removed SSID disappears from them, a new VLAN must already pass through the switch port of each access point.")) + "</p>",
    t("Save"), wfPwRule);
  $("dlg-body").innerHTML = ""; // typed passwords leave the page
  if (!fd) return;
  const g = (k, i) => String(fd.get(k + "-" + i) ?? "");
  const ssids = all.flatMap((x, i) => fd.get("del-" + i) === "on" || !g("ssid", i).trim() ? [] : [{
    ssid: g("ssid", i).trim(), orig: x.ssid || "", vlan: g("vlan", i).trim(), security: g("sec", i), passphrase: g("pw", i),
    isolation: fd.get("iso-" + i) === "on", hidden: fd.get("hid-" + i) === "on", bands: bands.filter(b => fd.get(wfBandKey(b, i)) === "on"),
    ft: fd.get("ft-" + i) === "on", kv: fd.get("kv-" + i) === "on", band_steering: fd.get("bs-" + i) === "on",
    schedule: g("from", i) || g("to", i) ? { days: [1, 2, 3, 4, 5, 6, 7].filter(d => fd.get("day" + d + "-" + i) === "on"), from: g("from", i), to: g("to", i) } : null }]);
  if (await wfPost("/api/wifi/state", { op: "ssids", ssids })) banner(t("Wi-Fi state saved: apply it on each access point."), "ok");
  await loadWifi();
}

/* clients of all access points */

function wfClients(v) {
  const q = $("wf-q").value.trim().toLowerCase();
  const rows = v.clients.filter(c => !q || q.split(/\s+/).every(w => [c.name, c.ip, c.mac, c.ssid, c.vlan, c.ap, c.iface, wfPhy(c.band)].join(" ").toLowerCase().includes(w)));
  // disconnect: where Cairn Net writes; block: in the state, applied everywhere
  const writes = new Set(v.aps.filter(a => !a.error && a.mode === "direct" && a.has_write).map(a => a.name));
  const acts = c => {
    const d = ' data-mac="' + esc(c.mac) + '" data-name="' + esc(c.name || "") + '"';
    return (writes.has(c.ap) ? '<button type="button" data-act="wf-kick" data-ap="' + esc(c.ap) + '"' + d + ">" + esc(t("Disconnect")) + "</button> " : "") +
      (v.state ? '<button type="button" data-act="wf-block"' + d + ">" + esc(t("Block")) + "</button>" : "");
  };
  return "<table>" + wfTh([t("Device"), t("IP address"), "SSID", t("Access point"), t("Signal"), t("Rate ↓ / ↑"), t("Connected for"), t("Roams"), t("Actions")]) + "<tbody>" +
    (rows.map(c => "<tr><td>" + '<button class="chip" data-goto="clients" data-q="' + esc(c.mac) + '">' + kindSvg(c.kind || "unknown") + esc(c.name || c.mac) + "</button>" +
      (c.name ? '<br><small class="sub mono">' + esc(c.mac) + "</small>" : "") + (c.authorized ? "" : ' <span class="pill warn">' + esc(t("not authorized")) + "</span>") +
      "</td><td>" + esc(c.ip) + "</td><td>" + esc(c.ssid) + (c.vlan ? '<br><small class="sub">VLAN ' + esc(c.vlan) + "</small>" : "") +
      "</td><td>" + esc(c.ap + " · " + c.iface) + '<br><small class="sub">' + esc(wfPhy(c.band)) + "</small>" +
      '</td><td><span class="pill ' + wfSignal(c.signal) + '">' + esc(c.signal != null ? c.signal + " dBm" : "—") + "</span></td>" +
      '<td class="num">' + esc(bits(num(c.tx_rate)) + " / " + bits(num(c.rx_rate))) + '<br><small class="sub" title="' + esc(t("Now ↓ / ↑")) + '">' +
      esc(bits(num(c.tx_bps)) + " / " + bits(num(c.rx_bps))) + "</small></td><td>" + esc(duration(rosSeconds(c.uptime))) + '</td><td class="num">' +
      (c.roams ? '<button class="chip" data-act="wf-hist" data-mac="' + esc(c.mac) + '" title="' + esc(c.last_roam) + '">' + esc(c.roams) + "</button>" : "0") + "</td>" +
      '<td class="wf-acts">' + acts(c) + "</td></tr>").join("") ||
      '<tr><td colspan="9" class="sub">' + esc(t(v.clients.length ? "No client matches." : "No client.")) + "</td></tr>") + "</tbody></table>";
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

/* lot Wi-Fi 4a: guests (QR code, new password), clients disconnected or blocked */

// the Wi-Fi state on every access point Cairn Net writes, one after the other (each previewed, then kept or undone)
async function wfApplyAll() {
  for (const a of WF.aps.filter(a => !a.error && a.mode === "direct" && a.has_write)) await wfSend(a.name);
}

async function wfShare(ssid) {
  const r = await wfPost("/api/wifi/share", { ssid });
  if (!r) return;
  let rotate = false;
  await ask(t("Join {ssid}", { ssid }), '<div class="wf-qr">' + r.svg + '</div><p class="wf-qrpw">' +
    (r.open ? esc(t("Open network: no password")) : esc(t("Password")) + ' <b class="mono">' + esc(r.password) + "</b>") + "</p>" +
    '<p class="sub">' + esc(t("Point a phone camera at the code to join.")) + "</p>" +
    (r.open ? "" : '<div class="toolbar"><button type="button" id="wf-rotate">' + esc(t("New password")) + "</button></div>"),
    t("Close"), d => {
      $("dlg-cancel").hidden = true; // "Close" says it all
      const b = d.querySelector("#wf-rotate");
      if (b) b.addEventListener("click", () => { rotate = true; closeDialog("cancel"); });
    });
  $("dlg-cancel").hidden = false;
  $("dlg-body").innerHTML = ""; // the password leaves the page
  if (!rotate) return;
  if (!(await ask(t("Give {ssid} a new password?", { ssid }), '<p class="banner">' + esc(t("Cairn Net draws a new random password, saves it in the Wi-Fi state, then applies the state on each access point. Every device on {ssid} needs the new password: the old one stops working once applied.", { ssid })) + "</p>",
    t("New password")))) return;
  if (!(await wfPost("/api/wifi/state", { op: "rotate", ssid }))) return;
  await loadWifi();
  await wfApplyAll(); // the read account cannot see passwords: the cards would not show this change
  if (!$("banner").textContent) banner(t("New password for {ssid} saved: keep it on each access point, then show the QR code.", { ssid }), "ok");
}

async function wfKick(ap, d) {
  const name = d.name || d.mac;
  if (!(await ask(t("Disconnect {name}?", { name }), '<p class="sub">' + esc(t("It leaves {ap} now and usually joins again at once: block it to keep it out.", { ap })) + "</p>", t("Disconnect")))) return;
  if (await wfPost("/api/wifi/kick", { switch: ap, mac: d.mac })) banner(t("{name} disconnected from {ap}.", { name, ap }), "ok");
  await loadWifi();
}

async function wfRestart(ap, d) {
  const iface = d.iface;
  if (!(await ask(t("Restart {iface} on {ap}?", { iface, ap }), '<p class="sub">' + esc(t("The radio stops three seconds, then starts again with its SSIDs: their clients join again by themselves. Useful when devices can no longer join while those connected stay.")) + "</p>", t("Restart radio")))) return;
  if (await wfPost("/api/wifi/restart", { switch: ap, iface })) banner(t("{iface} of {ap} restarted: its clients join again within a few seconds.", { iface, ap }), "ok");
  setTimeout(loadWifi, 8000); // the radio is back after ~5 s
}

async function wfBlock(_, d, op = "block") {
  const name = d.name || d.mac;
  const what = op === "block" ? "Saved in the Wi-Fi state, then applied on each access point: every one refuses {mac} (access list) and disconnects it. Blocking the device you are using cuts you off: then do not keep the change, it undoes itself."
    : "Saved in the Wi-Fi state, then applied on each access point: {mac} may join again.";
  if (!(await ask(t(op === "block" ? "Block {name}?" : "Unblock {name}?", { name }), '<p class="sub">' + esc(t(what, { mac: d.mac })) + "</p>", t(op === "block" ? "Block" : "Unblock")))) return;
  if (!(await wfPost("/api/wifi/state", { op, mac: d.mac, name: d.name || "" }))) return;
  await loadWifi();
  return wfApplyAll();
}

const WF_ACTS = { "wf-edit": wfEdit, "wf-apply": wfSend, "wf-keep": wfKeep, "wf-adopt": wfAdopt, "wf-add": wfAdd, "wf-import": wfImport, "wf-ssid-edit": wfSsidEdit,
  "wf-share": wfShare, "wf-kick": wfKick, "wf-restart": wfRestart, "wf-block": wfBlock, "wf-unblock": (_, d) => wfBlock(_, d, "unblock") };
document.addEventListener("click", e => {
  const b = e.target.closest("button[data-act^='wf-']"), f = b && WF_ACTS[b.dataset.act];
  if (f && WF && !WF.error) f(b.dataset.ap, b.dataset);
});
document.addEventListener("click", e => {
  const b = e.target.closest("button[data-act='wf-hist']");
  if (!b || !WF || !WF.events) return;
  const c = WF.clients.find(x => x.mac === b.dataset.mac) || { mac: b.dataset.mac };
  ask(t("Roaming of {name}", { name: c.name || c.mac }), '<div class="table-wrap">' + wfEvTable(WF.events.filter(x => x.mac === c.mac)) + "</div>", t("OK"));
});
$("wf-q").addEventListener("input", () => render(true));
