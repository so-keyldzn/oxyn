<!-- oxyn-translation source="docs/adr/0051-automatic-updates-from-github-releases.md" sha256="801552aea14e" -->

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
([I-08](../../CLAUDE.md#i-08)), sans fonctionnalités par défaut : il n'apporte
aucune pile TLS à lui et utilise le `reqwest` que le workspace construit déjà,
sur `rustls` avec le fournisseur `aws-lc-rs` des drivers.
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

La page de release s'ouvre dans le navigateur du système par
`tauri-plugin-opener`, lui aussi utilisé depuis Rust seulement, sans
capability : son URL est un préfixe fixe suivi d'une version que Rust a
validée comme semver. Le plugin est pris plutôt que de lancer `open` ou
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

L'endpoint est une constante d'un seul module (`updates/channel.rs`) ; il
n'est ni dans `tauri.conf.json` ni dans une préférence, et la webview ne peut
pas en fournir un. Seul le canal **stable** existe ; c'est l'isolement qui
permettra d'ajouter un canal bêta sans toucher au reste.

`tauri.conf.json` porte la clé publique minisign et
`requireSignedVersion: true`. La clé privée n'existe que dans les secrets de
livraison et dans deux sauvegardes hors ligne
([RELEASE](../RELEASE.md#clé-de-signature-des-mises-à-jour),
[I-03](../../CLAUDE.md#i-03)).

### 3. Le manifeste est écrit en dernier, par le workflow de livraison

Tauri signe les artefacts de mise à jour au build (`createUpdaterArtifacts`,
activé seulement par `make desktop PROFIL=release MISE_A_JOUR=1`, pour qu'un
build de release local n'ait pas besoin de la clé privée). L'archive macOS est
construite par le bundler à partir du `.app` **après** sa signature, sa
notarisation et son agrafage ; `script/apple-release verify` contrôle le
`.app` qu'elle contient.

Un dernier job, `manifeste`, démarre une fois les deux jobs de paquets au
vert, sans autre secret que `GH_TOKEN` : il lit les fichiers `.sig` du
brouillon, exige exactement une archive macOS et une AppImage, écrit
`latest.json` et le dépose. C'est le seul job qui dépose `latest.json`. Comme
`/releases/latest/download` ignore les brouillons, rien n'est proposé à
personne avant que le mainteneur publie.

### 4. Quand elle vérifie, télécharge et installe

* **Vérifie** 60 s après le lancement, puis toutes les 24 h, sur
  `tauri::async_runtime` ([I-05](../../CLAUDE.md#i-05)). L'échéance se calcule
  à l'horloge murale, relue toutes les heures — un sommeil de 24 h se
  figerait pendant la veille de la machine.
* **Télécharge** en arrière-plan quand les mises à jour automatiques sont
  actives. Les octets restent **en mémoire** ; la signature est vérifiée par
  le plugin avant que rien ne soit gardé. Après un plantage ou une fermeture
  sans installation, le téléchargement recommence simplement.
* **Installe à la fermeture**, jamais en forçant une relance. Une action
  « Restart now » est proposée ; elle lance l'arrêt ordonné, puis installe,
  puis appelle `request_restart()` — jamais `restart()`. La fermeture est
  inscrite avant l'installation, si bien que la relance n'affiche pas d'écran
  de récupération. `cancel_exit` efface l'intention de relance, sinon un ⌘Q
  ultérieur relancerait.
* **Le Quit du Dock et la fermeture de session** (`RunEvent::Exit`)
  installent aussi, après le départ des fenêtres, si bien que le thread
  principal ne fige rien de visible.
* **Là où Oxyn ne peut pas écrire** (le `.app` dans un dossier qui
  n'appartient pas à l'utilisateur), rien n'est installé à la fermeture :
  l'invite administrateur n'apparaît qu'après un clic sur « Restart now ».
* **Une opération à la fois.** Une vérification pendant `checking`,
  `downloading` ou `ready` ne fait rien ; `ready` dure jusqu'à la relance.
* **Échecs.** Un échec réseau en arrière-plan est silencieux et retenté à
  l'échéance suivante ; un échec de signature ou d'installation est toujours
  affiché.
* **Les notes de version** sont plafonnées à 4 Kio, coupées sur une frontière
  de caractère ([I-09](../../CLAUDE.md#i-09)), et rendues en texte brut.

### 5. Une préférence de l'application, pas du workspace

`app_config_dir()/updates.json`, `{"format":1,"automatic":true}`, lisible
sans Oxyn ([I-11](../../CLAUDE.md#i-11)). Un champ inconnu est ignoré ; un
fichier corrompu vaut la valeur par défaut.
`OXYN_UPDATES=off` dans l'environnement verrouille les mises à jour, pour un
poste administré ou hors ligne.

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
  laisser les utilisateurs sur une version vulnérable. Accepté : l'alternative
  est une seconde chaîne de signature pour le manifeste, et l'utilisateur voit
  la version qui tourne dans les Réglages.
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
