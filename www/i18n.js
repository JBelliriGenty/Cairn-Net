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
    // topology (www/topo.js)
    "Topology": "Topologie", "Fit": "Ajuster", "Zoom in": "Zoom avant", "Zoom out": "Zoom arrière", "Network topology": "Topologie du réseau",
    "Line thickness and motion follow the traffic; dashed: link down or held by STP. A switch opens its Infrastructure page, a link or a port its Ports view, a device the Devices list; a group of devices folds and unfolds. Drag to move, wheel or pinch to zoom.":
      "Épaisseur et mouvement des lignes suivent le trafic ; pointillés : lien coupé ou tenu en réserve par STP. Un switch ouvre sa page Infrastructure, un lien ou un port sa vue Ports, un appareil la liste Appareils ; un groupe d'appareils se plie et se déplie. Glisser pour déplacer, molette ou pincement pour zoomer.",
    "{n} link": "{n} liaison", "{n} links": "{n} liaisons", "down": "coupé", "blocked by STP": "en réserve (STP)",
    "{n} device on this port": "{n} appareil sur ce port", "{n} devices on this port": "{n} appareils sur ce port",
    "Kind": "Type", "Router": "Routeur", "Wi-Fi access point": "Borne Wi-Fi", "Server or hypervisor": "Serveur ou hyperviseur",
    "NAS": "NAS", "Computer": "Ordinateur", "Phone": "Téléphone", "Printer": "Imprimante", "Camera": "Caméra", "Connected device": "Objet connecté", "Unknown": "Inconnu",
    "set by hand": "choisi à la main", "announced as {v}": "s'annonce comme {v}", "from its name ({v})": "d'après son nom ({v})", "vendor: {v}": "constructeur : {v}",
    "hosts {n} virtual machine": "héberge {n} machine virtuelle", "hosts {n} virtual machines": "héberge {n} machines virtuelles",
    "random (private) address": "adresse aléatoire (privée)", "nothing to go by": "aucun indice",
    "Device {name}": "Appareil {name}", "Kind · guess: {kind}, {why}": "Type · deviné : {kind}, {why}", "Automatic": "Automatique", "Name it": "Nommer",
    "none found (DHCP, DNS, LLDP)": "aucun trouvé (DHCP, DNS, LLDP)",
    "Kept by Cairn Net for this MAC address; nothing is written to the switches. An empty name goes back to the one found.":
      "Gardé par Cairn Net pour cette adresse MAC ; rien n'est écrit sur les switchs. Un nom vide revient à celui trouvé.",
    "invalid name (63 characters at most)": "nom invalide (63 caractères au plus)",
    "invalid MAC address": "adresse MAC invalide", "unknown kind": "type inconnu",
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
    "Names from the leases": "Noms tirés des baux",
    "Static DNS names": "Noms DNS statiques", "Add a name": "Ajouter un nom", "New DNS name": "Nouveau nom DNS",
    "{n} name in {file}.": "{n} nom dans {file}.", "{n} names in {file}.": "{n} noms dans {file}.", "{file} unreadable: {err}": "{file} illisible : {err}",
    "Compared with what {servers} answer.": "Comparés à ce que répondent {servers}.",
    "Set a DNS hosts integration to compare with what the DNS servers answer.": "Réglez une publication DNS « hosts » pour comparer avec ce que répondent les serveurs DNS.",
    "served": "servi", "no answer": "pas de réponse", "answers {ip}": "répond {ip}", "unknown": "inconnu",
    "No name in the file yet.": "Aucun nom dans le fichier pour l'instant.", "Static DNS names file in it": "Fichier des noms DNS statiques",
    "Committing writes the file in the repository under your name; your own tool (Ansible, CI) then publishes the names.": "Le commit écrit le fichier dans le dépôt à votre nom ; votre outil (Ansible, CI) publie ensuite les noms.",
    "Committed ({c}). Your own tool (Ansible, CI) publishes the names.": "Commité ({c}). Votre outil (Ansible, CI) publie les noms.",
    "Nothing to change: the file already says this.": "Rien à changer : le fichier dit déjà cela.",
    "{who}: invalid name (letters, digits, dashes, dots)": "{who} : nom invalide (lettres, chiffres, tirets, points)",
    "{who}: listed twice with {v}": "{who} : en double avec {v}",
    "{who}: already the name of the DHCP reservation of {v}": "{who} : déjà le nom de la réservation DHCP de {v}",
    "{who}: already the name of a lease ({v})": "{who} : déjà le nom d'un bail ({v})",
    "{who} is already in the file": "{who} est déjà dans le fichier", "{who} is not in the file": "{who} n'est pas dans le fichier", "No switch yet: add one in Settings.": "Aucun switch pour l'instant : ajoutez-en un dans Réglages.", "the file": "le fichier",
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
    "{who}: the file paths must be relative to the repository (e.g. network/dhcp.yml, network/firewall.json, network/dns.yml)": "{who} : les chemins des fichiers doivent être relatifs au dépôt (ex. network/dhcp.yml, network/firewall.json, network/dns.yml)",
    "{who}: the firewall is read-only (direct mode, or git mode with a firewall file, writes it)": "{who} : pare-feu en lecture seule (le mode direct, ou le mode Git avec un fichier de pare-feu, l'écrit)",
    "{who}: not in git mode with a firewall file": "{who} : pas en mode Git avec un fichier de pare-feu",
    "a change is still waiting for its confirmation: import once it is kept or rolled back": "une modification attend encore sa confirmation : importez une fois qu'elle est conservée ou annulée",
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
    "Other DHCP options, one per line: code value": "Autres options DHCP, une par ligne : code valeur",
    "e.g. 66 TFTP server, 67 boot file, 150 TFTP address, 252 proxy (WPAD) URL. A plain value is sent as text, an address as 4 bytes; 0x… is raw hex.":
      "Par ex. 66 serveur TFTP, 67 fichier de démarrage, 150 adresse TFTP, 252 URL du proxy (WPAD). Une valeur simple part en texte, une adresse en 4 octets ; 0x… est de l'hexadécimal brut.",
    "watches for other DHCP servers": "surveille les autres serveurs DHCP", "expected: {m}": "attendus : {m}",
    "other DHCP server": "autre serveur DHCP",
    "Another DHCP server answers on {iface}: {macs}. Two servers on one network hand out conflicting addresses.":
      "Un autre serveur DHCP répond sur {iface} : {macs}. Deux serveurs sur un même réseau distribuent des adresses qui se contredisent.",
    "{n} other DHCP servers answer on {iface}: {macs}. Two servers on one network hand out conflicting addresses.":
      "{n} autres serveurs DHCP répondent sur {iface} : {macs}. Deux serveurs sur un même réseau distribuent des adresses qui se contredisent.",
    "Another DHCP server answers on {iface}: {macs}.": "Un autre serveur DHCP répond sur {iface} : {macs}.",
    "Watch for other DHCP servers on this interface": "Surveiller les autres serveurs DHCP sur cette interface",
    "Other DHCP servers that are expected (MAC addresses, comma separated)": "Autres serveurs DHCP attendus (adresses MAC, séparées par des virgules)",
    "The switch listens for DHCP answers and sends a discovery once a minute; any other server shows on this card.":
      "Le switch écoute les réponses DHCP et envoie une découverte par minute ; tout autre serveur apparaît sur cette carte.",
    "{who}: invalid MAC address of an expected DHCP server": "{who} : adresse MAC invalide pour un serveur DHCP attendu",
    "Publish the names of the leases (DNS)": "Publier les noms des baux (DNS)", "No": "Non",
    "Hosts file over SSH (Pi-hole, dnsmasq)": "Fichier hosts par SSH (Pi-hole, dnsmasq)",
    "Static DNS of this switch (write account)": "DNS statique de ce switch (compte d'écriture)",
    "Hosts targets (user@host[:port], comma separated)": "Cibles hosts (compte@hôte[:port], séparées par des virgules)",
    "Of these DHCP servers only (comma separated, empty = all)": "De ces serveurs DHCP seulement (séparés par des virgules, vide = tous)",
    "DNS hosts targets": "Cibles DNS (fichier hosts)",
    "To publish the names of the leases in a hosts file (Pi-hole, dnsmasq): copy examples/cairn-net-hosts to the DNS server (/usr/local/bin, mode 755), then add this line to the authorized_keys of the target account. The key can then only write that file. Adjust the two paths: the file Cairn Net writes, then the names it must not publish again.":
      "Pour publier les noms des baux dans un fichier hosts (Pi-hole, dnsmasq) : copiez examples/cairn-net-hosts sur le serveur DNS (/usr/local/bin, mode 755), puis ajoutez cette ligne aux authorized_keys du compte cible. La clé ne peut alors qu'écrire ce fichier. Adaptez les deux chemins : le fichier qu'écrit Cairn Net, puis les noms qu'il ne doit pas republier.",
    "Published to {where} every 2 minutes.": "Publiés vers {where} toutes les 2 minutes.", "the static DNS of {name}": "le DNS statique de {name}",
    "Not sent yet.": "Pas encore envoyés.", "failed ({err})": "échec ({err})", "last sent {d} ago": "dernier envoi il y a {d}",
    "switch unreachable at the last pass: the previous names stay": "switch injoignable au dernier passage : les noms précédents restent",
    "unknown DNS integration": "intégration DNS inconnue",
    "DNS hosts targets: one or more user@host[:port]": "Cibles DNS hosts : une ou plusieurs, compte@hôte[:port]",
    "RouterOS static DNS needs the write account": "le DNS statique de RouterOS demande le compte d'écriture",
    "invalid DHCP server name": "nom de serveur DHCP invalide",
    "Delete server {name}?": "Supprimer le serveur {name} ?",
    "Its network, pool and options go too, unless another server uses them.": "Son réseau, sa plage et ses options partent aussi, sauf si un autre serveur s'en sert.",
    "Its {n} reservation is deleted too.": "Sa réservation est supprimée aussi.", "Its {n} reservations are deleted too.": "Ses {n} réservations sont supprimées aussi.",
    "{who}: invalid option code (1-254)": "{who} : code d'option invalide (1-254)",
    "{who}: option set by its own field or by RouterOS (1, 3, 6, 15, 42, 51, 53, 54, 121)": "{who} : option réglée par son propre champ ou par RouterOS (1, 3, 6, 15, 42, 51, 53, 54, 121)",
    "{who}: same option code twice": "{who} : même code d'option deux fois",
    "{who}: invalid option value (0x…, a number, 'text' or s'text')": "{who} : valeur d'option invalide (0x…, un nombre, 'texte' ou s'texte')",
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
    // infrastructure
    "Infrastructure": "Infrastructure", "System": "Système", "Identity": "Identité", "Architecture": "Architecture", "Update": "Mise à jour",
    "{v} available": "{v} disponible", "up to date": "à jour", "not checked": "non vérifiée", "Routing and DNS": "Routage et DNS",
    "inactive": "inactive", "none": "aucune", "Gateway (DHCP client)": "Passerelle (client DHCP)", "DNS servers": "Serveurs DNS",
    "DNS from DHCP": "DNS reçus par DHCP", "Answers DNS for the network": "Répond au DNS du réseau", "yes": "oui", "no": "non",
    "NAT": "NAT", "{n} rule": "{n} règle", "{n} rules": "{n} règles", "Addresses": "Adresses", "Bridges and ports": "Ponts et ports",
    "Origin": "Origine", "active": "active", "invalid": "invalide", "VLAN {id} on {iface}": "VLAN {id} sur {iface}",
    "Cairn Net's path": "chemin de Cairn Net", "DHCP client": "client DHCP", "static": "statique", "No IP address.": "Aucune adresse IP.",
    "Bridge": "Pont", "VLAN filtering": "Filtrage VLAN", "Protocol": "Protocole", "Comment": "Commentaire", "no port": "aucun port",
    "outside any bridge": "hors de tout pont", "Chain": "Chaîne", "Action": "Action", "Match": "Critères", "To": "Vers",
    "from {v}": "de {v}", "to {v}": "vers {v}", "in {v}": "entrée {v}", "out {v}": "sortie {v}", "everything": "tout", "No NAT rule.": "Aucune règle NAT.",
    "Keep these settings on {name}?": "Garder ces réglages sur {name} ?", "rollback in {n} s": "retour arrière dans {n} s",
    "rollback at {time}": "retour arrière à {time}", "Keep": "Garder",
    "Cairn Net read the switch back as asked. Without your confirmation, the switch puts the old settings back by itself.":
      "Cairn Net a relu le switch comme demandé. Sans votre confirmation, le switch remet de lui-même les anciens réglages.",
    "DNS servers, one per line": "Serveurs DNS, un par ligne", "Address to change": "Adresse à modifier", "New address": "Nouvelle adresse",
    "Netmask": "Masque", "Gateway (default route)": "Passerelle (route par défaut)", "Preview": "Aperçu",
    "{iface} is how Cairn Net reaches the switch: it is not offered here.": "{iface} est le chemin de Cairn Net vers le switch : elle n'est pas proposée ici.",
    "Nothing is written yet: the next step shows the commands.": "Rien n'est encore écrit : l'étape suivante montre les commandes.",
    "Nothing to change.": "Rien à modifier.", "Commands": "Commandes",
    "Rollback, run by the switch after {n} s unless you keep the change": "Retour arrière, lancé par le switch au bout de {n} s si vous ne gardez pas la modification",
    "the switch puts the old settings back within {n} s": "le switch remet les anciens réglages d'ici {n} s", "Kept on {name}.": "Gardé sur {name}.",
    "{who}: Cairn Net reaches the switch through this interface, it is not edited from here": "{who} : Cairn Net joint le switch par cette interface, elle ne se modifie pas d'ici",
    "{who}: invalid gateway": "{who} : passerelle invalide", "{who}: the gateway must be in one of the switch's networks": "{who} : la passerelle doit être dans un des réseaux du switch",
    "{who}: invalid address (a.b.c.d/len)": "{who} : adresse invalide (a.b.c.d/longueur)",
    "invalid identity (1 to 64 characters)": "identité invalide (1 à 64 caractères)", "DNS servers: up to 4 IPv4 addresses": "Serveurs DNS : 4 adresses IPv4 au plus",
    "several default routes: change them on the switch": "plusieurs routes par défaut : modifiez-les sur le switch",
    "this address is gone from the switch: reload": "cette adresse n'existe plus sur le switch : rechargez",
    "a change is still waiting for its confirmation: confirm it or wait for the rollback": "une modification attend encore sa confirmation : confirmez-la ou attendez le retour arrière",
    "no change waiting: the switch already put the old settings back": "aucune modification en attente : le switch a déjà remis les anciens réglages",
    "{v} would take the network of the management path ({net})": "{v} prendrait le réseau du chemin de gestion ({net})",
    "{v} overlaps {net}, already on the switch": "{v} chevauche {net}, déjà sur le switch",
    "{v} is dynamic (DHCP client): it cannot be edited": "{v} est dynamique (client DHCP) : non modifiable",
    "{who} is not in direct mode: nothing is written to it": "{who} n'est pas en mode direct : rien n'y est écrit",
    // firewall
    "Firewall": "Pare-feu", "Filter": "Filtre", "Mangle": "Mangle", "Raw": "Raw", "Rules": "Règles", "{n} disabled": "{n} désactivée(s)",
    "Connection tracking": "Suivi de connexion", "running": "actif", "not running": "inactif", "Setting": "Réglage", "Connections": "Connexions",
    "Off while the firewall is empty: the first rule turns it on, and connections already open (SSH, WinBox) may be cut once.":
      "Inactif tant que le pare-feu est vide : la première règle l'active, et les connexions déjà ouvertes (SSH, WinBox) peuvent être coupées une fois.",
    "Packets": "Paquets", "Bytes": "Octets", "No firewall rule on this switch.": "Aucune règle de pare-feu sur ce switch.",
    "Address lists": "Listes d'adresses", "Interface lists": "Listes d'interfaces", "List": "Liste", "Timeout": "Expiration",
    "dynamic": "dynamique", "No address list.": "Aucune liste d'adresses.", "Interfaces": "Interfaces", "no interface": "aucune interface",
    "No interface list.": "Aucune liste d'interfaces.",
    "Add a rule": "Ajouter une règle", "Add an address": "Ajouter une adresse", "No rule.": "Aucune règle.", "new": "nouvelle",
    "Enable": "Activer", "Disable": "Désactiver", "Disabled": "Désactivée",
    "Other properties, one key=value per line": "Autres propriétés, une clé=valeur par ligne",
    "Matchers and action parameters, one key=value per line (RouterOS names)": "Critères et paramètres de l'action, une clé=valeur par ligne (noms RouterOS)",
    "{who}: invalid property name": "{who} : nom de propriété invalide",
    "A change that could cut Cairn Net's path ({app} → {iface}, API port) is refused.": "Toute modification qui pourrait couper le chemin de Cairn Net ({app} → {iface}, port de l'API) est refusée.",
    "any interface": "n'importe quelle interface",
    "Firewall file in it": "Fichier du pare-feu dans le dépôt",
    "Repository": "Dépôt",
    "file unreadable": "fichier illisible",
    "differs from the file": "écart avec le fichier",
    "in line with the file": "conforme au fichier",
    "The file holds no table yet: import the switch's rules to start from what it runs.": "Le fichier ne contient encore aucune table : importez les règles du switch pour partir de ce qu'il applique.",
    "The switch does not run these tables as the file says: {list}.": "Le switch n'applique pas ces tables comme le dit le fichier : {list}.",
    "Apply the file: {table}": "Appliquer le fichier : {table}",
    "Import the switch's rules into the file": "Importer les règles du switch dans le fichier",
    "Once kept, the table is committed to {path} under your name.": "Une fois conservée, la table est commitée dans {path} à votre nom.",
    "The switch already runs this table as edited: only the file is committed.": "Le switch applique déjà cette table telle quelle : seul le fichier est commité.",
    "Committed: {sha}": "Commit : {sha}",
    "Kept on {name}, but the commit failed: {err}. Import the switch's rules to bring the file in line.": "Conservé sur {name}, mais le commit a échoué : {err}. Importez les règles du switch pour remettre le fichier en accord.",
    "Every table of {name} (filter, NAT, mangle, raw, address lists) replaces the one in {path}, committed under your name. Nothing is written to the switch.": "Chaque table de {name} (filtre, NAT, mangle, raw, listes d'adresses) remplace celle de {path}, commitée à votre nom. Rien n'est écrit sur le switch.",
    "Import": "Importer",
    "The file already holds these rules.": "Le fichier contient déjà ces règles.",
    "applying needs the write account (Settings)": "l'application demande le compte d'écriture (Réglages)",
    "This change starts connection tracking: connections already open to the switch (SSH, WinBox) may be cut once.":
      "Cette modification démarre le suivi de connexion : les connexions déjà ouvertes vers le switch (SSH, WinBox) peuvent être coupées une fois.",
    "this change starts connection tracking: confirm it in the preview": "cette modification démarre le suivi de connexion : confirmez-la dans l'aperçu",
    "{who}: this row is gone from the switch: reload": "{who} : cette ligne n'existe plus sur le switch : rechargez",
    "{who}: line breaks and control characters are not allowed": "{who} : retours à la ligne et caractères de contrôle interdits",
    "every row needs {who}": "chaque ligne doit avoir {who}",
    // firewall templates
    "From a template": "À partir d'un modèle", "Template": "Modèle", "Next": "Suivant", "Add the rules": "Ajouter les règles",
    "The rules are added to the table being edited: nothing is written before the preview.": "Les règles sont ajoutées à la table en cours de modification : rien n'est écrit avant l'aperçu.",
    "Isolated network, Internet only": "Réseau isolé, Internet seulement",
    "The network reaches the Internet, never the private networks (LAN, other VLANs, management) nor the switch itself, except DHCP, ping and, if ticked, the switch's DNS. The DNS servers listed stay reachable.":
      "Le réseau atteint Internet, jamais les réseaux privés (LAN, autres VLAN, gestion) ni le switch lui-même, sauf le DHCP, le ping et, si coché, le DNS du switch. Les serveurs DNS indiqués restent joignables.",
    "Block traffic between two networks": "Bloquer le trafic entre deux réseaux", "Allow a service from one network to another": "Autoriser un service d'un réseau vers un autre",
    "Placed before the drop rules of the chain, after the rule that accepts established connections.": "Placée avant les règles de rejet de la chaîne, après celle qui accepte les connexions établies.",
    "Reach the switch itself from a network": "Joindre le switch lui-même depuis un réseau", "Share the uplink (masquerade)": "Partager la sortie (masquerade)",
    "Hosts of the network go out with the switch's address on the uplink: for a network the upstream router has no route back to.":
      "Les machines du réseau sortent avec l'adresse du switch sur la sortie : pour un réseau vers lequel le routeur amont n'a pas de route retour.",
    "Port forward to an inner host": "Redirection de port vers une machine interne",
    "Interface of the network": "Interface du réseau", "DNS servers it may reach (optional, comma separated)": "Serveurs DNS joignables (facultatif, séparés par des virgules)",
    "The switch answers DNS for this network": "Le switch répond au DNS de ce réseau", "From (address, network or range)": "De (adresse, réseau ou plage)",
    "To (address, network or range)": "Vers (adresse, réseau ou plage)", "Both ways": "Dans les deux sens",
    "Port(s), e.g. 443 or 8000-8100 (optional for icmp)": "Port(s), par ex. 443 ou 8000-8100 (sans objet pour icmp)",
    "Uplink interface": "Interface de sortie", "Incoming interface": "Interface d'entrée", "Inner host": "Machine interne", "Port on the inner host (optional)": "Port sur la machine interne (facultatif)",
    "Connections already open": "Connexions déjà ouvertes",
    "{iface}: DHCP": "{iface} : DHCP", "{iface}: DNS of the switch": "{iface} : DNS du switch", "{iface}: ping of the gateway": "{iface} : ping de la passerelle",
    "{iface}: nothing else to the switch": "{iface} : rien d'autre vers le switch", "{iface}: invalid packets": "{iface} : paquets invalides", "{iface}: DNS": "{iface} : DNS",
    "{iface}: no private network": "{iface} : aucun réseau privé", "{iface}: Internet": "{iface} : Internet",
    "{a} -> {b}: blocked": "{a} -> {b} : bloqué", "{a} -> {b}: {p}": "{a} -> {b} : {p}", "{a} -> switch: {p}": "{a} -> switch : {p}",
    "{a}: masquerade on {b}": "{a} : masquerade sur {b}", "{p} on {a} -> {b}": "{p} sur {a} -> {b}",
    "{who} could cut Cairn Net off the switch ({v}, {net}): refused": "{who} pourrait couper Cairn Net du switch ({v}, chaîne {net}) : refusé",
    "a new connection of Cairn Net": "une nouvelle connexion de Cairn Net", "the open connection of Cairn Net": "la connexion ouverte de Cairn Net",
    "replies of the switch": "les réponses du switch",
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
