<!-- oxyn-translation source="docs/adr/0051-automatic-updates-from-github-releases.md" sha256="8d3649369455" -->

> Traduction française de [docs/adr/0051-automatic-updates-from-github-releases.md](../../../../docs/adr/0051-automatic-updates-from-github-releases.md). **La version anglaise fait foi.**

# ADR-0051 — Oxyn se met à jour depuis les GitHub Releases, en Rust seulement, et installe à la fermeture

**Statut :** proposé · **Date :** 2026-10-02

## Contexte

Un utilisateur qui a téléchargé Oxyn doit revenir chercher chaque nouvelle
version à la main. Rien n'existe pour les mises à jour au 2026-10-02 : ni
plugin, ni clé de signature, ni manifeste. Ce que le dépôt a déjà, et qui borne
le choix :

* **La chaîne de livraison** est `.github/workflows/livraison.yml` et
  `script/livraison`, sans `tauri-action` : un tag `v*` poussé construit un
  **brouillon** de release GitHub, que le mainteneur publie à la main
  ([RELEASE](../RELEASE.md)). Le dépôt est public.
* **Les paquets.** macOS produit un DMG Apple Silicon, signé et notarisé
  (#117) ; Linux produit deb, rpm et AppImage. macOS Intel et Windows ne sont
  pas construits.
* **La sortie.** Oxyn a déjà un arrêt ordonné — transactions, brouillons,
  dispositions, puis la fermeture inscrite pour que le lancement suivant ne
  propose pas de reprise ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md),
  [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md),
  [ADR-0043](0043-multi-fenetre.md)). Une relance qui le contourne perd du
  travail ou affiche un faux écran de plantage.
* **La webview est une surface d'entrée** : une XSS atteint les commandes
  Tauri ([SECURITY](../SECURITY.md#surface-dentrée), point 5). Une mise à jour
  est du code qui s'exécutera avec les droits de l'utilisateur.

Vérifié le 2026-10-02 dans les sources des versions concernées
([RESEARCH-NOTES](../RESEARCH-NOTES.md#contrats-de-mise-à-jour-tauri--vérifiés-le-2026-10-02)) :

* `tauri-plugin-updater` **2.13.1** (crates.io, 2026-09-29 ; une ligne alpha
  3.0.0 existe, non prise) vérifie un endpoint, télécharge l'archive en
  mémoire, contrôle sa signature **minisign** contre une clé publique compilée
  dans l'application, et l'installe. Il refuse un endpoint non `https` dans un
  build de release. Sa comparaison par défaut ne propose qu'une version
  **strictement supérieure** à celle qui tourne. Il exige `tauri` 2.12, comme
  `tauri-plugin-opener` 2.7.0 : le workspace passe avec eux de `tauri` 2.11.5
  à 2.12.1.
* La signature couvre les octets de l'archive et — depuis la CLI Tauri
  2.12.0 — un commentaire de confiance portant la version pour laquelle
  l'archive a été signée. Avec `requireSignedVersion: true`, le plugin rejette
  une archive dont la version signée diffère de celle qu'annonce le manifeste.
  Le manifeste lui-même n'est pas signé.
* Sur macOS, le plugin remplace le `.app` par un renommage, et demande un mot
  de passe administrateur par AppleScript quand le renommage est refusé ; sous
  Linux, il remplace l'AppImage désignée par `$APPIMAGE`, et lancerait
  `dpkg`/`rpm` avec élévation pour un deb ou un rpm.
* `https://github.com/<owner>/<repo>/releases/latest/download/<asset>` sert le
  fichier de la dernière release **publiée et non préliminaire** : un
  brouillon y est invisible.
* Tauri 2.12.1 offre `restart()`, qui ne rend jamais la main et, depuis un
  autre thread, dort indéfiniment une fois la sortie demandée, et
  `request_restart()`, qui passe par `RunEvent::ExitRequested` et
  `RunEvent::Exit` comme une sortie ordinaire, puis relance.

## Décision

### 1. La mise à jour vit en Rust, dans `oxyn-desktop`, et la webview ne l'appelle jamais

`tauri-plugin-updater` est une dépendance du seul `oxyn-desktop`
([I-08](../../CLAUDE.md#i-08)), avec `default-features = false` et **aucune**
fonctionnalité TLS : son `rustls-tls` tirerait `ring` et l'installerait comme
`CryptoProvider` par défaut du processus, sous les drivers. Il réutilise le
`reqwest` qu'`oxyn-llm` construit déjà sur `rustls` avec `aws-lc-rs` ;
`cargo tree` ne montre ni `ring` ni nouveau doublon.
Son paquet JavaScript `@tauri-apps/plugin-updater` n'est pas installé, et
`capabilities/main.json` n'accorde **aucune** permission `updater:*` — un test
d'`oxyn-desktop` en refuse une. La webview n'atteint la mise à jour que par
des commandes Oxyn étroites (`get_update_state`, `subscribe_updates`,
`check_for_updates`, `cancel_update`, `download_update`,
`set_automatic_updates`, `restart_to_update`, `open_release_page`,
`take_update_notice`) qui ne prennent jamais d'URL, de chemin ni de version.

Une mise à jour est **de la plomberie d'interface, pas une `Command`** : elle
ne touche aucune connexion de base de données, et aucun agent ne peut la
déclencher. Le précédent est la barre de menus et l'arrêt ordonné
([ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md),
[ADR-0038](0038-un-plantage-s-annonce-une-fois.md)), qui ne passent pas non
plus par le bus ([I-01](../../CLAUDE.md#i-01) porte sur les chemins
d'exécution vers un driver).

La page de release s'ouvre dans le navigateur du système par la fonction
libre `open_url` de `tauri-plugin-opener`, appelée depuis Rust ; le plugin
opener n'est **pas enregistré** — aucun gestionnaire JavaScript, aucune
commande, aucune capability. L'URL est un préfixe fixe suivi d'une version que
Rust a validée comme semver. Le plugin est pris plutôt que de lancer `open` ou
`xdg-open` à la main : sa crate `open` parcourt déjà dans l'ordre les lanceurs
Linux (`xdg-open`, `gio`, `gnome-open`, `kde-open`, WSL), empêche un argument
commençant par `-` d'être lu comme une option, et détache le processus
enfant — une seconde implémentation serait un second endroit à auditer.

### 2. Un endpoint, compilé : les GitHub Releases du dépôt

```
https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json
```

`latest.json` suit le format de manifeste **statique** du plugin : `version`,
`notes`, `pub_date`, et une entrée par clé de plateforme avec `url` et
`signature`. Ses clés sont `darwin-aarch64` et `linux-x86_64-appimage` —
jamais un `linux-x86_64` seul, que le plugin proposerait aussi à une
installation deb ou rpm.

**Amendement (2026-10-04).** Les paquets Linux sont aussi construits pour
ARM64, nativement sur le runner `ubuntu-24.04-arm`. Le manifeste gagne la clé
`linux-aarch64-appimage` — jamais un `linux-aarch64` seul, pour la même
raison — et `manifeste` exige exactement une `*_amd64.AppImage` et une
`*_aarch64.AppImage`, chacune avec son `.sig`, et aucune autre AppImage, là où
cet ADR dit « une AppImage ». Rien d'autre ne change dans la décision
([RELEASE](../RELEASE.md)).

L'endpoint est une constante d'un seul module (`updates/channel.rs`) ; il
n'est ni dans `tauri.conf.json` ni dans une préférence, et la webview ne peut
pas en fournir un. Seul le canal **stable** existe ; c'est l'isolement qui
permettra d'ajouter un canal bêta sans toucher au reste.

`tauri.conf.json` porte la clé publique minisign et
`requireSignedVersion: true`. La clé privée n'existe que dans les secrets de
l'environnement GitHub `release`, vers lequel seuls les tags `v*` peuvent
déployer, et dans deux sauvegardes hors ligne
([RELEASE](../RELEASE.md#clé-de-signature-des-mises-à-jour),
[I-03](../../CLAUDE.md#i-03)). La clé en service, d'identifiant minisign
`F7DFD985E0B86DFC`, a été générée par le mainteneur le 2026-10-02. Si
`tauri.conf.json` portait de nouveau la valeur de substitution
`REPLACE_WITH_UPDATER_PUBLIC_KEY` — un fork, une clé en cours de
remplacement —, le workflow de livraison la refuse avant de construire
(`script/livraison cle`) et avant d'écrire un manifeste, et un build qui la
porte refuse toute mise à jour comme un échec de signature.

### 3. Les artefacts sont signés après le build, et vérifiés avant le manifeste

**Aucun build ne signe.** `createUpdaterArtifacts` reste désactivé : avec
lui, `tauri build` signe à la fin du bundling, et la clé doit être dans
l'environnement de tout le build — chaque `build.rs` et chaque proc-macro de
`Cargo.lock`, Vite et ses plugins, dont n'importe lequel pourrait devenir,
par une version compromise d'une dépendance, un lecteur de cet
environnement. Les jobs de paquets construisent sans la clé ; l'archive
macOS est écrite par `script/apple-release build` à partir du `.app`
**après** sa signature, sa notarisation et son agrafage, comme
`tauri-bundler` 2.10.1 l'écrit (le `.app` à la racine d'un `.tar.gz`, liens
conservés), et `script/apple-release verify` contrôle le `.app` qu'elle
contient. L'AppImage est celle que le bundler a écrite : le type de bundle
qu'il inscrit dans le binaire ne dépend pas de `createUpdaterArtifacts`. Une
étape ultérieure, `script/livraison signer`, détient seule la clé : elle
lance le `tauri signer sign --app-version <version>` épinglé, qui écrit le
même commentaire de confiance qu'un `tauri build` qui signe.

**Le manifeste est écrit en dernier.** Un dernier job, `manifeste`, démarre
une fois les deux jobs de paquets au vert, sans autre secret que
`GH_TOKEN` : il exige exactement une archive macOS et une AppImage dans le
brouillon, les télécharge avec leur `.sig`, vérifie chaque paire avec
`minisign -V` 0.12 (épinglé par empreinte) contre la clé publique de
`tauri.conf.json`, puis contrôle la version signée, écrit `latest.json` et le
dépose. La CLI ne fait qu'avertir quand la clé privée ne correspond pas à
cette clé publique, et une relance peut apparier l'archive d'une exécution à
la signature d'une autre : sans contrôle, l'un comme l'autre livrerait une
release que toute copie installée refuse comme un échec de signature.
`manifeste` est le seul job qui dépose `latest.json`. Comme
`/releases/latest/download` ignore les brouillons, rien n'est proposé à
personne avant que le mainteneur publie.

### 4. Quand elle vérifie, télécharge et installe

* **Vérifie** 60 s après le lancement, puis toutes les 24 h, sur
  `tauri::async_runtime` ([I-05](../../CLAUDE.md#i-05)). L'échéance se calcule
  à l'horloge murale, relue toutes les heures — un sommeil de 24 h se
  figerait pendant la veille de la machine. Une vérification est bornée à
  **30 s** ; cette borne n'atteint pas le téléchargement, que le plugin lance
  sans délai maximal global (`Update.timeout` vaut `None`) : une liaison
  lente ne doit pas le perdre. Chaque requête du client de mise à jour est en
  HTTPS seulement, redirections comprises, et échoue après **60 s** sans
  recevoir un octet — par lecture, si bien qu'un flux bloqué finit en
  `error{offline, retryable: true}` au lieu de garder `downloading` pour la
  session. `cancel_update` interrompt la tâche, et la requête HTTP avec elle.
* **Télécharge** en arrière-plan quand les mises à jour automatiques sont
  actives. Les octets restent **en mémoire** ; la signature est vérifiée par
  le plugin avant que rien ne soit gardé. Après un plantage ou une fermeture
  sans installation, le téléchargement recommence simplement. Une archive dont
  l'URL n'est pas un asset des releases du projet
  (`https://github.com/so-keyldzn/oxyn/releases/download/…`) est refusée avant
  qu'un octet soit demandé, en `error{server, retryable: false}` : le plugin
  n'impose HTTPS qu'à l'endpoint. Une archive de plus de **256 Mio**, annoncée
  ou reçue, est abandonnée en `error{server, retryable: true}` : le plugin met
  tout le corps en mémoire avant de vérifier sa signature, si bien qu'un asset
  non vérifié ne doit pas choisir cette taille — la plus grosse publiée,
  l'AppImage de v0.0.1, pèse 89 Mo.
* **Installe à la fermeture**, jamais en forçant une relance. Une action
  « Restart now » est proposée ; elle lance l'arrêt ordonné, puis installe,
  puis appelle `request_restart()` — jamais `restart()`. La fermeture est
  inscrite avant l'installation, si bien que la relance n'affiche pas d'écran
  de récupération. `cancel_exit` efface l'intention de relance, sinon un ⌘Q
  ultérieur relancerait.
* **« Restart now » nomme le travail qu'il arrêterait.** `restart_to_update`
  répond `busy{running, exports}` tant que des instructions tournent ou que
  des exports s'écrivent dans une fenêtre ; les exports sont comptés par une
  garde autour de l'export. Une fois confirmée, la sortie n'attend pas un
  export : sa destination garde son contenu précédent, et un
  `.oxyn-export-*.part` égaré peut rester à côté.
* **Le Quit du Dock et la fermeture de session** (`RunEvent::Exit`)
  installent aussi, sous macOS seulement, après le départ des fenêtres, sur un
  thread auxiliaire que le thread principal attend au plus **10 s** — mesuré
  le 2026-10-02, l'extraction de l'archive de 12 Mo d'un bundle de 25 Mo
  prend 0,6 s. Passé ce délai, le processus se termine sans attendre
  davantage ; 10 s est une borne pour un disque lent, pas une durée attendue.
  Risque accepté : le plugin extrait toute l'archive avant de toucher au
  `.app` installé, puis l'échange par deux renommages à quelques
  microsecondes d'écart ; coupé entre les deux, `/Applications` n'a plus
  Oxyn.app tant que la sauvegarde laissée dans `$TMPDIR` n'est pas remise en
  place. Sous Linux, le plugin écrit la nouvelle AppImage en plusieurs
  secondes, si bien que la couper laisserait une image tronquée : une telle
  sortie n'installe rien, et le lancement suivant télécharge de nouveau la
  mise à jour.
* **Là où Oxyn ne peut pas écrire** (le `.app` dans un dossier qui
  n'appartient pas à l'utilisateur, sur une image montée sous `/Volumes`, ou
  déplacé par la translocation de Gatekeeper), rien n'est installé à la
  fermeture : l'invite administrateur n'apparaît qu'après un clic sur
  « Restart now ». Le dossier est sondé sur le pool bloquant une fois un
  téléchargement vérifié, jamais au lancement : le lancement ne lit que
  `updates.json` et `update-notice.json`, chacun borné à 64 Kio.
* **Une opération à la fois.** Une vérification pendant `checking`,
  `downloading` ou `ready` ne fait rien ; `ready` dure jusqu'à la relance, ou
  jusqu'à ce que les mises à jour automatiques soient coupées.
* **Échecs.** Un échec réseau ou serveur en arrière-plan est gardé en
  `error{offline|server, retryable: true}`, affiché dans les Réglages et nulle
  part ailleurs, et retenté à l'échéance suivante ; un échec de signature ou
  d'installation est toujours affiché.
* **Le mode automatique coupé** repose dans `disabled{user}` ; une
  vérification manuelle part encore de là. Le couper, comme l'annonce
  l'interrupteur (UX-SPEC), interrompt un téléchargement en cours et
  abandonne une mise à jour `ready` avec ses octets, sous un nouveau ticket
  pour que l'achèvement tardif soit refusé : quitter n'installe alors rien. Un
  téléchargement lancé à la main alors qu'il était déjà coupé reste celui de
  l'utilisateur. Les enregistrements concurrents de la préférence sont
  sérialisés de l'écriture à la publication, chacun par un fichier temporaire
  qui lui est propre.
* **Les notes de version** sont plafonnées à 4 Kio, coupées sur une frontière
  de caractère ([I-09](../../CLAUDE.md#i-09)), et rendues en texte brut.

### 5. Une préférence de l'application, pas du workspace

`app_config_dir()/updates.json`, `{"format":1,"automatic":true}`, lisible
sans Oxyn ([I-11](../../CLAUDE.md#i-11)). Un champ inconnu est ignoré ; un
fichier corrompu vaut la valeur par défaut.
`OXYN_UPDATES=off` dans l'environnement verrouille les mises à jour, pour un
poste administré ou hors ligne.

Ce que la sortie dit au lancement suivant traverse la relance dans un second
fichier ouvert à côté, `app_config_dir()/update-notice.json`, sous l'une de
deux formes :

```json
{"format":1,"type":"installed","from":"0.0.2","to":"0.0.3"}
{"format":1,"type":"installFailed","version":"0.0.3","message":"…"}
```

`message` est la phrase montrée à l'utilisateur, suivie de l'erreur du
plugin ; il ne porte aucun secret. Le fichier est écrit de façon atomique par
l'installation, lu et **supprimé** au lancement suivant, avant que l'avis ne
soit affiché — un avis se dit une fois. `installed` n'est affiché que si `to`
est la version qui se lance (une copie plus ancienne démarrée d'ailleurs ne
dit rien) ; un fichier qui ne se lit pas est ignoré. Les deux fichiers sont
bornés à 64 Kio à la lecture. Un `format` futur est une nouvelle valeur de ce
champ, jamais un changement silencieux des formes ci-dessus.

### 6. Où elle s'applique

| Installation | Comportement |
|---|---|
| `.app` macOS (DMG) | se met à jour |
| AppImage Linux (`$APPIMAGE` défini) | se met à jour |
| deb, rpm Linux | désactivée : « managed by your package manager » — le plugin lancerait sinon `dpkg`/`rpm` avec élévation dans le dos du gestionnaire de paquets |
| Build de développement | désactivée |
| Windows, macOS Intel | pas construits aujourd'hui ; aucune clé de manifeste |

## Conséquences

* **+** Un Oxyn installé reste à jour sans que l'utilisateur revienne, et sans
  jamais perdre de travail à cause d'une relance.
* **+** La webview ne gagne aucun pouvoir : aucune permission de plugin,
  aucune URL, aucun chemin. Une XSS peut au plus demander une vérification ou
  une relance que l'utilisateur se verrait proposer de toute façon.
* **+** Un compte GitHub ou un CDN compromis ne peut pas livrer de code :
  l'archive doit porter une signature minisign d'une clé que GitHub ne détient
  jamais.
* **+** Le rejeu par mensonge de version est fermé : `requireSignedVersion`
  lie chaque archive à la version pour laquelle elle a été signée, si bien
  qu'un `latest.json` forgé ne peut pas proposer une vieille archive
  authentiquement signée sous un numéro de version plus haut.
* **−** **`latest.json` n'est pas signé.** Qui contrôle l'endpoint peut encore
  **retenir** les mises à jour — servir un vieux manifeste, ou aucun — et
  laisser les utilisateurs sur une version vulnérable — et choisir les notes
  de version que les Réglages affichent à côté d'une archive authentique.
  Accepté : l'alternative est une seconde chaîne de signature pour le
  manifeste, et l'utilisateur voit la version qui tourne dans les Réglages ;
  les notes sont plafonnées à 4 Kio et rendues en texte brut, si bien qu'une
  note forgée relève de l'ingénierie sociale, pas du code.
* **−** **L'étape de signature fait encore confiance à la CLI Tauri.** La clé
  atteint un seul processus, le binaire `@tauri-apps/cli` épinglé dans le
  lockfile ; les centaines de crates et de paquets npm du build ne la voient
  jamais.
* **−** **La disponibilité de GitHub devient celle d'Oxyn.** Si GitHub est en
  panne, les vérifications échouent en silence jusqu'à son retour.
* **−** **Les utilisateurs de v0.0.1 mettent à jour à la main une fois.** Elle
  est livrée sans mise à jour automatique ; seules les versions construites
  après cet ADR se mettent à jour seules.
* **−** **Perdre la clé privée met fin aux mises à jour.** Les utilisateurs
  doivent alors télécharger à la main un build signé avec une nouvelle clé ;
  le renouvellement n'est possible que tant que l'ancienne clé existe
  ([RELEASE](../RELEASE.md#clé-de-signature-des-mises-à-jour)).
* **−** Les utilisateurs deb et rpm ne reçoivent pas de mises à jour d'Oxyn ;
  ils attendent leur gestionnaire de paquets, qui n'a aujourd'hui aucun dépôt
  à lire.

**Coût de sortie :** faible côté code — la mise à jour est une arborescence de
modules dans `oxyn-desktop` (`updates/`) plus ses commandes, et rien hors de
la crate desktop ne sait qu'elle existe. Élevé côté confiance : la clé
publique est compilée dans chaque copie installée. Changer l'endpoint ou la
clé exige une release, signée avec l'ancienne clé, qui porte les nouveaux.

**À reconsidérer si** un build Windows ou macOS Intel est ajouté (nouvelles
clés de manifeste, l'installeur Windows quitte l'application lui-même) ; si
les GitHub Releases limitent ou changent `/releases/latest/download` ; si un
canal bêta devient nécessaire ; si la ligne 3.x de `tauri-plugin-updater`
devient stable, ou si la 2.x cesse de recevoir des correctifs.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| L'API JavaScript du plugin (`@tauri-apps/plugin-updater`) appelée depuis la webview | Exige des permissions `updater:*` : une XSS pourrait alors déclencher vérifications, téléchargements et installations, et les versions antérieures à 2.12.0 laissaient même la webview assouplir la comparaison de versions. Rust garde la décision là où la webview ne l'atteint pas |
| `tauri-action` pour construire et publier la release | La chaîne de livraison existe déjà sans lui, avec son brouillon et ses règles de refus ([RELEASE](../RELEASE.md)) ; un second publieur entrerait en concurrence avec le premier. Le manifeste tient en quelques lignes de `script/livraison`, testées comme le reste |
| Garder l'archive téléchargée sur disque pour l'installer à un lancement ultérieur | Un fichier sur disque est un fichier qu'un tiers peut remplacer entre la vérification et l'installation ; il faudrait revérifier la signature, et un plantage en pleine écriture laisse une demi-archive. Retélécharger coûte un transfert |
| Une préférence par workspace | Une mise à jour remplace l'application, pas un workspace : un workspace qui dit « non » n'arrêterait pas un autre qui dit « oui » sur la même machine |
| Un service de mise à jour dédié (CrabNebula Cloud, un CDN à nous) | Un second fournisseur à qui se fier et à payer, pour ce que les GitHub Releases d'un dépôt public servent déjà ; à rouvrir avec un build Windows ou un canal bêta si GitHub devient la limite |
| `restart()` pour « Restart now » | Il ne rend jamais la main et, depuis un autre thread que le principal, dort indéfiniment : l'arrêt ordonné ne finirait jamais ses étapes |
| Laisser `requireSignedVersion` désactivé | Rien avec quoi rester compatible — aucune release ne porte encore la mise à jour — et, désactivé, le manifeste non signé peut rejouer une vieille archive signée sous une version plus haute |
