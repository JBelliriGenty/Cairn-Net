// cairn-net translations (same engine as cairn-nas). The code is written in
// English; each language maps an English source string to its translation
// (missing = English shown). Placeholders: t("{n} ports", { n: 3 }).
// Plurals: tp(n, "{n} port", "{n} ports").
"use strict";

const LANGS = { en: { name: "English", locale: "en-GB" }, fr: { name: "Français", locale: "fr-FR" } };

const DICT = {
  fr: {
    // units
    "B": "o", "KB": "Ko", "MB": "Mo", "GB": "Go", "TB": "To", "PB": "Po",
    "b/s": "b/s", "Kb/s": "Kb/s", "Mb/s": "Mb/s", "Gb/s": "Gb/s", "{d} d": "{d} j",
    // frame
    "Language: {name}": "Langue : {name}",
    "Theme: {name}": "Thème : {name}", "auto": "auto", "light": "clair", "dark": "sombre",
    "Overview": "Vue d'ensemble", "Ports": "Ports", "Devices": "Appareils", "VLANs": "VLAN",
    "Loading…": "Chargement…", "refreshed {time}": "actualisé à {time}",
    "{n} switch": "{n} switch", "{n} switches": "{n} switchs",
    "Read failed: {err}": "Lecture impossible : {err}", "Unreadable configuration: {err}": "Configuration illisible : {err}",
    // overview
    "All good: {sw}, {ports}, {links}, {devs}": "Tout va bien : {sw}, {ports}, {links}, {devs}",
    "{n} switch reachable": "{n} switch joignable", "{n} switches reachable": "{n} switchs joignables",
    "{n} active port": "{n} port actif", "{n} active ports": "{n} ports actifs",
    "{n} link between switches up": "{n} liaison entre switchs complète", "{n} links between switches up": "{n} liaisons entre switchs complètes",
    "{n} device": "{n} appareil", "{n} devices": "{n} appareils",
    "{n} device seen": "{n} appareil vu", "{n} devices seen": "{n} appareils vus",
    "{n} VLAN": "{n} VLAN", "{n} VLANs": "{n} VLAN",
    "unreachable": "injoignable", "{name} unreachable: {err}": "{name} injoignable : {err}",
    "{name}: {what} {value}": "{name} : {what} {value}", "{name}: {what} {value} °C": "{name} : {what} {value} °C",
    "{name}: CPU {pct} %": "{name} : CPU {pct} %", "{name}: {bond} down": "{name} : {bond} hors service",
    "{name}: {bond} {up} / {total} links": "{name} : {bond} {up} / {total} liens",
    "{name}: firmware {v} available": "{name} : firmware {v} disponible",
    "{up} / {total} ports up": "{up} / {total} ports actifs", "CPU {pct} %": "CPU {pct} %", "up for {d}": "allumé depuis {d}",
    "Links between switches": "Liaisons entre switchs", "Link": "Liaison", "Via": "Par", "Speed": "Débit", "State": "État",
    "Traffic": "Trafic", "{bond} (LACP)": "{bond} (LACP)", "{up} / {total} up": "{up} / {total} actifs",
    "No link between switches seen (LLDP / MNDP).": "Aucune liaison entre switchs vue (LLDP / MNDP).",
    // ports
    "Model": "Modèle", "RouterOS": "RouterOS", "Firmware": "Firmware", "Serial number": "Numéro de série", "Up for": "Allumé depuis",
    "CPU": "CPU", "Memory": "Mémoire", "healthy": "en bonne santé", "{n} problem": "{n} problème", "{n} problems": "{n} problèmes",
    "Port": "Port", "Name": "Nom", "VLAN": "VLAN", "Connected to": "Branché à", "Link drops": "Coupures",
    "up": "actif",
    "Temperature ({what})": "Température ({what})", "board": "carte", "cpu": "processeur", "switch": "puce de commutation",
    "phy": "PHY", "sfp": "SFP", "Fan {n}": "Ventilateur {n}", "Power supply {n}": "Alimentation {n}", "Fans": "Ventilation", "no link": "sans lien", "disabled": "désactivé",
    // devices, VLANs
    "Search: name, IP, MAC, port…": "Rechercher : nom, IP, MAC, port…", "Device": "Appareil", "IP address": "Adresse IP",
    "MAC address": "Adresse MAC",
    // DHCP
    "DHCP": "DHCP", "Reservations": "Réservations", "Add a reservation": "Ajouter une réservation", "Dynamic leases": "Baux dynamiques",
    "Names from the leases": "Noms tirés des baux", "No switch yet: add one in Settings.": "Aucun switch pour l'instant : ajoutez-en un dans Réglages.", "the file": "le fichier",
    "Cancel": "Annuler", "OK": "Valider", "Edit": "Modifier", "Add": "Ajouter", "Delete": "Supprimer", "Apply": "Appliquer", "Discard": "Abandonner",
    "Edit {name}": "Modifier {name}", "New reservation": "Nouvelle réservation", "Comment (optional, one line in the file)": "Commentaire (facultatif, une ligne dans le fichier)",
    "Refused: {err}": "Refusé : {err}", "Lease": "Bail", "Note": "Note", "Host name": "Nom de l'appareil", "Reserve": "Réserver",
    "No DHCP server “{srv}” on {name}.": "Pas de serveur DHCP « {srv} » sur {name}.",
    "DHCP server “{srv}” of {name} is on · pool {pool} · leases {time}": "Serveur DHCP « {srv} » de {name} actif · plage {pool} · baux de {time}",
    "DHCP server “{srv}” of {name} is off. Its reservations are already on the switch, ready when it is turned on.":
      "Serveur DHCP « {srv} » de {name} éteint. Ses réservations sont déjà sur le switch, prêtes pour sa mise en service.",
    "{n} pending change": "{n} modification en attente", "{n} pending changes": "{n} modifications en attente", "not applied": "non appliqué",
    "Read-only copy of the file: set a repository (git mode) in Settings to change it from here.":
      "Copie du fichier en lecture seule : réglez un dépôt (mode Git) dans Réglages pour le modifier d'ici.",
    "bound, expires in {d}": "actif, expire dans {d}", "never seen": "jamais vu", "seen {d} ago": "vu il y a {d}",
    "missing": "absente", "has {ip}": "a {ip}", "up to date": "à jour",
    "not in the file: will be removed": "absente du fichier : sera retirée", "added by hand on the switch": "ajoutée à la main sur le switch",
    "No dynamic lease.": "Aucun bail dynamique.", "None while the server is off.": "Aucun tant que le serveur est éteint.",
    "No bound lease with a usable name yet.": "Pas encore de bail actif avec un nom utilisable.",
    "{who}: invalid MAC address {v}": "{who} : adresse MAC invalide {v}", "{who}: invalid IP address {v}": "{who} : adresse IP invalide {v}",
    "{who}: {v} is outside {net}": "{who} : {v} est hors de {net}", "{who}: invalid name (letters, digits, dashes)": "{who} : nom invalide (lettres, chiffres, tirets)",
    "{who}: same {v} as {net}": "{who} : même {v} que {net}", "MAC": "MAC", "IP": "IP", "name": "nom",
    "DHCP servers": "Serveurs DHCP",
    "Sign-in": "Connexion", "Username": "Identifiant", "Password": "Mot de passe", "Sign in": "Se connecter", "Sign out": "Se déconnecter",
    "wrong user name or password": "Identifiant ou mot de passe incorrect",
    "Too many attempts: try again in {n} s": "Trop d'essais : réessayez dans {n} s",
    "Commit to the repository?": "Commiter dans le dépôt ?", "One commit under your name with these changes.": "Un commit à votre nom avec ces modifications.",
    "Commit": "Commiter", "Committed ({c}). Your own tool (Ansible, CI) applies it to {name}.": "Commité ({c}). Votre outil (Ansible, CI) l'applique sur {name}.",
    "Commit refused: {err}": "Commit refusé : {err}",
    "Committing writes the file in the repository under your name; your own tool (Ansible, CI) then applies it to {name}.":
      "Commiter écrit le fichier dans le dépôt à votre nom ; votre outil (Ansible, CI) l'applique ensuite sur {name}.",
    "Git mode without a repository: set it in Settings.": "Mode Git sans dépôt : à régler dans Réglages.",
    "Repository (git mode)": "Dépôt (mode Git)", "DHCP file in it": "Fichier DHCP dans le dépôt", "Branch": "Branche",
    "Repository refused: {err}. Is the deploy key added with write access?": "Dépôt refusé : {err}. La clé de déploiement est-elle ajoutée, en écriture ?",
    "No key: ssh-keygen is missing in the image.": "Pas de clé : ssh-keygen manque dans l'image.",
    "Git deploy key": "Clé de déploiement Git",
    "Git mode: add this public key to each repository as a deploy key with write access. Cairn Net clones the repository with it and pushes one commit per change, under the name of whoever applies.":
      "Mode Git : ajoutez cette clé publique à chaque dépôt comme clé de déploiement en écriture. Cairn Net clone le dépôt avec elle et pousse un commit par modification, au nom de qui applique.",
    "{who}: invalid repository, branch or path": "{who} : dépôt, branche ou chemin invalide",
    "{who}: the file path must be relative to the repository (e.g. network/dhcp.yml)": "{who} : le chemin du fichier doit être relatif au dépôt (ex. network/dhcp.yml)",
    "Discard the pending changes?": "Abandonner les modifications en attente ?", "Apply on {name}?": "Appliquer sur {name} ?",
    "{done} of {n} commands run on {name}": "{done} commandes sur {n} passées sur {name}",
    "the switch does not read back as expected": "le switch ne se relit pas comme prévu",
    "Applied on {name}: {n} commands, read back as expected.": "Appliqué sur {name} : {n} commandes, relu comme prévu.",
    "{name} is managed directly: changes are written to the switch through its API. {on} of {n} DHCP servers on.":
      "{name} est géré en direct : les modifications sont écrites sur le switch par son API. {on} serveurs DHCP allumés sur {n}.",
    "Commands for {name}": "Commandes pour {name}", "none: the switch already runs this": "aucune : le switch fait déjà tourner cela",
    "Fix the problems above first.": "Corrigez d'abord les problèmes ci-dessus.",
    "Settings": "Réglages", "Switches": "Switchs", "Add a switch": "Ajouter un switch", "Prepare a switch": "Préparer un switch",
    "Before adding a switch: paste this script in its terminal. It creates the accounts, the certificate and the encrypted API, reachable only from Cairn Net.":
      "Avant d'ajouter un switch : collez ce script dans son terminal. Il crée les comptes, le certificat et l'API chiffrée, joignable seulement depuis Cairn Net.",
    "Address of Cairn Net as the switch sees it": "Adresse de Cairn Net vue par le switch", "Copy": "Copier", "Copied": "Copié",
    "Read only": "Lecture seule", "Direct": "Direct", "Git": "Git",
    "Cairn Net only reads this switch.": "Cairn Net lit ce switch, sans jamais y écrire.",
    "Cairn Net writes to the switch through its API, after showing the difference.": "Cairn Net écrit sur le switch par son API, après avoir montré la différence.",
    "Cairn Net edits a file in a repository; your own tool (Ansible, CI) applies it.": "Cairn Net modifie un fichier dans un dépôt ; votre outil (Ansible, CI) l'applique.",
    "connected": "connecté", "no password": "sans mot de passe", "Read account": "Compte de lecture", "Write account": "Compte d'écriture",
    "Certificate fingerprint (sha256)": "Empreinte du certificat (sha256)", "No switch yet: add the first one.": "Aucun switch : ajoutez le premier.",
    "unchanged": "inchangé", "New switch": "Nouveau switch", "Address": "Adresse", "API port (api-ssl)": "Port de l'API (api-ssl)",
    "Its web interface (optional)": "Son interface web (facultatif)", "How this switch is managed": "Comment ce switch est géré",
    "Its password": "Son mot de passe", "Write account (direct mode)": "Compte d'écriture (mode direct)",
    "Health entries to ignore (comma separated, e.g. psu2-state)": "Mesures de santé à ignorer (séparées par des virgules, ex. psu2-state)",
    "Saving first reads the switch's certificate: you compare its fingerprint, then the accounts are tested.":
      "L'enregistrement lit d'abord le certificat du switch : vous comparez son empreinte, puis les comptes sont testés.",
    "Save": "Enregistrer", "Cannot reach {host}: {err}": "{host} injoignable : {err}", "Trust this certificate?": "Faire confiance à ce certificat ?",
    "The certificate of {host} CHANGED. New fingerprint (sha256):": "Le certificat de {host} a CHANGÉ. Nouvelle empreinte (sha256) :",
    "Fingerprint (sha256) of the certificate of {host}:": "Empreinte (sha256) du certificat de {host} :",
    "Compare it with the switch's own: /certificate print detail (fingerprint), or the last line of the preparation script.":
      "Comparez-la à celle du switch : /certificate print detail (fingerprint), ou la dernière ligne du script de préparation.",
    "Trust": "Faire confiance", "Read account refused: {err}": "Compte de lecture refusé : {err}", "Write account refused: {err}": "Compte d'écriture refusé : {err}",
    "Delete {name}?": "Supprimer {name} ?", "Cairn Net forgets this switch and its accounts. Nothing changes on the switch.": "Cairn Net oublie ce switch et ses comptes. Rien ne change sur le switch.",
    "{who}: invalid switch name (letters, digits, dashes)": "{who} : nom de switch invalide (lettres, chiffres, tirets)", "{who}: invalid address": "{who} : adresse invalide",
    "{who}: invalid port": "{who} : port invalide", "{who}: the certificate fingerprint is missing: test the connection first": "{who} : empreinte du certificat absente : testez d'abord la connexion",
    "{who}: unknown mode": "{who} : mode inconnu", "{who}: the read account is missing": "{who} : compte de lecture absent",
    "{who}: direct mode needs the write account": "{who} : le mode direct demande le compte d'écriture", "{who}: invalid account": "{who} : compte invalide",
    "{who}: same name as another switch": "{who} : même nom qu'un autre switch", "{who}: unknown switch": "{who} : switch inconnu",
    "Add a server": "Ajouter un serveur", "not on the switch": "pas encore sur le switch", "differs from the file": "≠ fichier",
    "The file asks for another value ({what}): your tool sets it back at its next run.": "Le fichier demande une autre valeur ({what}) : votre outil la remet à son prochain passage.",
    "Turn off": "Éteindre", "Turn on": "Allumer", "Turn on {name}?": "Allumer {name} ?", "Edit server {name}": "Modifier le serveur {name}",
    "New DHCP server": "Nouveau serveur DHCP", "Server on": "Serveur allumé", "Interface": "Interface", "Choose…": "Choisir…",
    "Network": "Réseau", "Gateway": "Passerelle", "Range": "Plage", "Lease time": "Durée du bail",
    "DNS servers (comma separated)": "Serveurs DNS (séparés par des virgules)", "NTP servers (comma separated)": "Serveurs NTP (séparés par des virgules)",
    "Domain": "Domaine", "Routes (option 121), one per line: network gateway": "Routes (option 121), une par ligne : réseau passerelle",
    "A client that gets routes ignores the gateway: put the default route (0.0.0.0/0) in them.": "Un client qui reçoit des routes ignore la passerelle : mettez-y la route par défaut (0.0.0.0/0).",
    "Another DHCP server may already serve this network: turn it off first, never two DHCP servers on one network.": "Un autre serveur DHCP sert peut-être déjà ce réseau : coupez-le d'abord, jamais deux serveurs DHCP sur un même réseau.",
    "{who}: invalid server name (letters, digits, dashes)": "{who} : nom de serveur invalide (lettres, chiffres, tirets)",
    "{who}: same name as another server": "{who} : même nom qu'un autre serveur", "{who}: invalid interface": "{who} : interface invalide",
    "{who}: same interface as another server": "{who} : même interface qu'un autre serveur",
    "{who}: invalid network (address/length)": "{who} : réseau invalide (adresse/longueur)", "{who}: same network as another server": "{who} : même réseau qu'un autre serveur",
    "{who}: invalid range (first-last, inside the network)": "{who} : plage invalide (première-dernière, dans le réseau)",
    "{who}: invalid gateway (an address of the network)": "{who} : passerelle invalide (une adresse du réseau)",
    "{who}: invalid DNS server address": "{who} : adresse de serveur DNS invalide", "{who}: invalid NTP server address": "{who} : adresse de serveur NTP invalide",
    "{who}: invalid domain": "{who} : domaine invalide", "{who}: invalid lease time (e.g. 1d, 12h, 30m)": "{who} : durée de bail invalide (ex. 1d, 12h, 30m)",
    "{who}: invalid route (network/length,gateway)": "{who} : route invalide (réseau/longueur,passerelle)",
    "{who}: routes without a default route (0.0.0.0/0): clients would lose Internet": "{who} : routes sans route par défaut (0.0.0.0/0) : les clients perdraient Internet",
    "{who} is already a server": "{who} est déjà un serveur", "{who} is not a server": "{who} n'est pas un serveur",
    "only letters, digits and . : / - _ , are allowed": "seuls lettres, chiffres et . : / - _ , sont permis",
    "on": "actif", "off": "éteint", "invalid": "invalide", "VLAN {id}": "VLAN {id}", "gateway {gw}": "passerelle {gw}",
    "route {r}": "route {r}", "domain {d}": "domaine {d}", "lease {d}": "bail {d}", "leased": "en bail", "reserved": "réservées", "free": "libres", "of {n} in the range": "sur {n} dans la plage",
    "{a} leased, {b} reserved, {c} free of {n}": "{a} en bail, {b} réservées, {c} libres sur {n}",
    "+ {n} reservation outside the range": "+ {n} réservation hors de la plage", "+ {n} reservations outside the range": "+ {n} réservations hors de la plage",
    "Managed by {who}": "Géré par {who}", "hand (RouterOS)": "la main (RouterOS)", "No DHCP server on the switches.": "Aucun serveur DHCP sur les switchs.",
    "{who} is already reserved": "{who} est déjà réservée", "{who} is not reserved": "{who} n'est pas réservée",
    "quotes, backslashes and line breaks are not allowed": "guillemets, barres obliques inverses et retours à la ligne interdits",
 "Switch": "Switch", "No device matches.": "Aucun appareil ne correspond.",
    "U = untagged (port VLAN), T = tagged. The bridge itself (the switch's own CPU) is left out.":
      "U = non étiqueté (VLAN du port), T = étiqueté. Le bridge lui-même (le processeur du switch) n'est pas listé.",
  },
};

let LANG = "en";

function t(s, vars) {
  let r = (DICT[LANG] && DICT[LANG][s]) || s;
  if (vars) r = r.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? vars[k] : m));
  return r;
}
const tp = (n, one, many, vars) => t(n > 1 || n === 0 && LANG === "en" ? many : one, Object.assign({ n }, vars));
const LOCALE = () => LANGS[LANG].locale;

// Order: the viewer's choice, then the server default (config "lang"), then the browser.
function pickLang(serverDefault) {
  let saved = null;
  try { saved = localStorage.getItem("cairn-lang"); } catch (e) { /* storage blocked */ }
  const browser = (navigator.languages || [navigator.language]).map(l => String(l).slice(0, 2)).find(l => LANGS[l]);
  return [saved, serverDefault, browser, "en"].find(l => l && LANGS[l]);
}

// Static page text: <b data-i18n>Overview</b>, plus data-i18n-title / -placeholder / -aria-label.
function applyStatic(root = document) {
  root.querySelectorAll("[data-i18n]").forEach(e => {
    if (!e.dataset.i18n) e.dataset.i18n = e.textContent.trim();
    e.textContent = t(e.dataset.i18n);
  });
  for (const attr of ["title", "placeholder", "aria-label"]) {
    const k = "data-i18n-" + attr;
    root.querySelectorAll("[" + k + "]").forEach(e => {
      if (!e.getAttribute(k)) e.setAttribute(k, e.getAttribute(attr) || "");
      e.setAttribute(attr, t(e.getAttribute(k)));
    });
  }
  document.documentElement.lang = LANG;
}

LANG = pickLang();
