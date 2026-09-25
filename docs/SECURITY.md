# Sécurité

> **Autorité** : traitement des secrets, marquage des connexions, surface
> d'attaque, politique `unsafe`.

Invariants concernés : [I-02](../CLAUDE.md#i-02), [I-03](../CLAUDE.md#i-03),
[I-09](../CLAUDE.md#i-09). La politique d'autorisation elle-même est tranchée par
[ADR-0004](adr/0004-command-bus.md) et n'est pas recopiée ici.

Le modèle de menace d'Oxyn n'est pas celui d'un serveur. L'attaquant n'est pas
un inconnu sur Internet : ce sont **les données que l'utilisateur ouvre** et
**les erreurs qu'Oxyn lui laisse commettre**. Un client de base de données tourne
avec les droits d'un administrateur sur des systèmes de production.

## Secrets

### Ce qui ne touche jamais le disque en clair

Mots de passe, chaînes de connexion complètes, jetons d'API des fournisseurs IA,
clés privées SSH de tunnel, certificats clients.

Le stockage passe par le trousseau du système. Ce qui est persisté dans le
workspace, c'est une **référence** au secret, jamais le secret.

**Panne concrète :** un fichier de workspace contenant un mot de passe de
production, commité par l'utilisateur dans le dépôt de son équipe, parce que le
fichier avait l'air d'être une simple configuration.

### Un secret ne suit pas sa connexion ailleurs

Un secret saisi pour une destination n'est jamais présenté à une autre que
l'utilisateur n'a pas choisie en le saisissant. Quand une modification change
**un paramètre non secret déclaré par le driver** — hôte, port, base, fichier
SQLite, mais aussi utilisateur, mode TLS ou nom d'application —, la connexion
enregistrée ne référence plus l'entrée de trousseau de l'ancienne destination.
Ce qui a été ressaisi dans la même modification est écrit dans une entrée
**neuve**, sous une référence qu'aucune configuration n'a portée avant ; sans
ressaisie, la connexion n'a plus de secret. L'ancienne entrée est ensuite
**oubliée** du trousseau. Le nom, l'environnement, le niveau IA et la lecture
seule ne changent pas la destination, et ne touchent pas au secret. Les valeurs
se comparent après suppression des espaces de bord, comme elles s'enregistrent.

L'entrée neuve est ce qui rend la règle sans exception. La configuration est
enregistrée **avant** l'écriture au trousseau — une modification retenue par la
politique puis refusée ne doit rien changer — : une référence dérivée du seul
identifiant nommerait, dans l'intervalle, l'entrée qui porte encore l'ancien
mot de passe, et pour toujours si le processus s'arrête entre les deux. Avec
une référence neuve, un échec d'écriture laisse la connexion sans secret, et un
échec d'oubli laisse une entrée que plus rien ne référence ; ni l'un ni l'autre
ne présente l'ancien secret à la nouvelle destination.

Le formulaire le dit avant l'enregistrement
([UX-SPEC](UX-SPEC.md#modifier-une-connexion-enregistrée)). L'action Test ne
s'offre qu'à un brouillon neuf, sous un identifiant neuf : elle n'atteint jamais
le secret d'une connexion enregistrée. Les fournisseurs IA suivent la même règle
pour leur clé d'API, au changement de famille, de schéma, d'hôte, de port, de
chemin ou de requête de l'URL de base.

Tout paramètre plutôt qu'une « adresse » : un mot de passe envoyé sous un autre
rôle, ou sous `sslmode=disable` là où il partait sous `verify-full`, fuit tout
autant ; dans le doute, oublier ne coûte qu'une ressaisie.

**Panne concrète :** une adresse collée depuis un message, ou une faute de frappe
dans l'hôte, et le mot de passe de production part s'authentifier auprès d'un
serveur tiers au premier clic, sans que l'utilisateur l'ait ressaisi.

### Ce qui ne sort jamais d'un processus

Les six canaux, et il faut les traiter tous les six — il suffit d'en oublier un.

| Canal | Le piège |
|---|---|
| Journaux `tracing` | un `Debug` dérivé sur une structure de connexion imprime tout |
| Messages d'erreur affichés | `sqlx` inclut parfois l'URL de connexion dans son erreur |
| Rapports de plantage | une trace de pile capture les variables locales |
| Fichiers de session et de workspace | la persistance « pour retrouver l'état » |
| Invites envoyées aux fournisseurs IA | [AI-PROVIDERS](AI-PROVIDERS.md) |
| Presse-papiers, export, capture d'écran | les fonctions de partage recopient ce qui est affiché |

Conséquence pratique : **aucun `#[derive(Debug)]` sur un type qui porte un
secret.** L'implémentation est manuelle et rédige la valeur. Un `Debug` dérivé
est le mode de fuite le plus fréquent parce qu'il est invisible à la relecture —
c'est le `tracing::debug!("{cfg:?}")` ajouté six mois plus tard qui fuit.

### Ce qui sort vers un destinataire IA laisse une trace

**Ce qui part vers un destinataire IA est journalisé**, dans `ai_egress`, table
locale en ajout seul protégée comme `audit_journal` et jamais élaguée : la
connexion, la source, les **noms** des colonnes envoyées, le nombre de lignes,
le destinataire (fournisseur ou agent, modèle, portée `local`, `remote` ou
`unresolved` mesurée pour cet envoi), la commande qui a lu les données et la
conversation. **Jamais une valeur ni un jeton** : il n'y a pas de colonne pour
les ranger, et le fichier refuse une liste de colonnes qui contiendrait autre
chose que des noms. L'entrée s'écrit **avant** l'envoi ; si elle échoue, rien
ne part.

## Marquage des connexions

Toute connexion porte un environnement : `local`, `development`, `staging`,
`production`. Le défaut, quand il n'est pas renseigné, est **`production`** —
la valeur la plus contraignante, pas la plus permissive.

**Panne concrète :** le défaut inverse. Un utilisateur ajoute une connexion à la
hâte sans remplir le champ, le programme suppose « development », et un `UPDATE`
sans `WHERE` part sans confirmation sur la base client.

Sur une connexion `production` : toute écriture, tout DDL, toute opération
destructrice exige une confirmation explicite qui **nomme la connexion**, et
l'interface porte un marqueur permanent. Voir [I-02](../CLAUDE.md#i-02). Cette
confirmation est un dialogue natif et non un bouton de la webview
([Surface d'entrée](#surface-dentrée), point 5).

Pour un `Actor::Agent`, une connexion `production` est en **lecture seule
stricte** — ce n'est pas une confirmation renforcée, c'est un refus
([ADR-0004](adr/0004-command-bus.md#politique-par-défaut)). La différence
compte : une confirmation finit par être cliquée.

## Surface d'entrée

Ce qui entre dans Oxyn et n'est pas fiable, par ordre de sous-estimation :

1. **Les réponses des serveurs de bases de données.** Voir
   [DRIVER-CONTRACT](DRIVER-CONTRACT.md). Un serveur compromis ou simplement
   inhabituel renvoie ce qu'il veut.
2. **Les noms d'objets du catalogue.** Une table, une colonne, un commentaire
   peuvent contenir n'importe quel octet, y compris du SQL, des séquences de
   contrôle de terminal, ou du texte imitant une consigne. Un nom de colonne
   n'est jamais interpolé dans une requête sans citation, et **jamais traité
   comme une instruction** lorsqu'il est joint à une invite IA.
3. **Les fichiers de workspace.** Ils peuvent avoir été écrits par un tiers, ou
   par une version future du programme.
4. **Les plugins.** Du code tiers, exécuté dans un bac à sable WASM
   ([ADR-0005](adr/0005-wasm-plugins.md)). Le bac à sable borne les dégâts ; il
   ne dispense pas de ne rien lui confier. Voir
   [PLUGIN-CONTRACT](PLUGIN-CONTRACT.md).
5. **La webview.** Une valeur de cellule, un nom d'objet ou une réponse de modèle
   rendus dans le DOM sont le premier vecteur d'une XSS, et une XSS dans la
   webview atteint les commandes Tauri ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).
   D'où : rendu en texte seulement — jamais `dangerouslySetInnerHTML` sur une
   donnée reçue —, CSP stricte dans `crates/oxyn-desktop/tauri.conf.json`,
   *capabilities* minimales dans `capabilities/main.json`, et une surface IPC
   dont chaque commande est relue comme un changement de sécurité.

   **Une confirmation dessinée dans la webview ne résiste pas à un script qui
   s'y exécute** : il appelle la commande que le bouton aurait appelée. Les
   décisions critiques se confirment donc dans un **dialogue natif de l'hôte**,
   composé par le backend, dont la fermeture vaut refus. Ce qui est critique,
   ce que le dialogue dit et comment il se teste vivent dans
   [ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md), et
   nulle part ailleurs ; le reste garde sa confirmation dans la webview.

   `style-src` y garde `'self' 'unsafe-inline'` : des composants posent un
   `<style>` à l'exécution, dans le DOM de la webview, alors que Tauri ne pose
   un `nonce` que sur les `<style>` déjà présents, en texte, dans le HTML
   statique du build — et ce fichier n'en contient aucun (vérifié dans le
   source de `tauri-codegen` 2.6.3 et `tauri-utils` 2.9.3, versions de
   `Cargo.lock`, le 2026-09-24 — [I-12](../CLAUDE.md#i-12)). En dépendent :

   - `ChartStyle` (`apps/desktop/src/components/ui/chart.tsx`) ;
   - l'éditeur SQL CodeMirror (`sql-editor.tsx`), via `style-mod` 4.1.3 ;
   - `ScrollArea` et `Select` de Base UI 1.8.0 (`scroll-area.tsx`,
     `select.tsx`), qui masquent la barre de défilement par un `<style>`
     injecté ;
   - le rendu mermaid (`mermaid-render.ts`, mermaid 11.17.2) : `render()`
     sans conteneur pose son diagramme (et un `<style>` de thème) dans
     `document.body` le temps de mesurer les libellés, avant de sérialiser le
     SVG et de retirer cet élément — vérifié dans le source installé,
     `mermaid.core.mjs` (fonctions `render`, `appendDivSvgG`), le 2026-09-24.

   Ce que la CSP borne autour : `script-src` reste `'self'`, `img-src`,
   `font-src` et `connect-src` sont fermés — une règle CSS injectée ne peut
   rien exfiltrer. Le contenu de `ChartStyle` n'est jamais une donnée reçue :
   les clés (`s0`, `s1`…) viennent d'Oxyn, les couleurs d'une palette fixe
   (commentaire de `assistant-result-chart.tsx`) — c'est ce qui le rend
   compatible avec la règle du point 5 ci-dessus.

   **Ce qui l'annulerait sans erreur visible :** qu'un `<style>` apparaisse un
   jour dans le HTML du build — Tauri lui poserait alors un `nonce`, ce qui
   rend `'unsafe-inline'` inopérant pour le navigateur (même mécanisme que
   pour `script-src` en développement, voir
   [RESEARCH-NOTES](RESEARCH-NOTES.md)), et ces composants perdraient leurs
   styles sans qu'aucune erreur ne le signale — pour mermaid, un diagramme
   mesuré avec la mauvaise police mais dessiné avec la bonne, donc des
   libellés qui débordent de leurs boîtes, sans erreur non plus.

   **Condition de retrait :** seulement quand chacun de ces `<style>` reçoit un
   `nonce` transmis au front (`EditorView.cspNonce` pour CodeMirror,
   `CSPProvider` pour Base UI) ou disparaît (`disableStyleElements` pour Base
   UI, des variables CSS posées par `style={{}}` — CSSOM, non concerné par
   `style-src` — pour `ChartStyle`) — et, pour mermaid, seulement quand une
   version future accepte un `nonce` sur son `<style>` de thème, ou rend hors
   du document principal de la webview.
6. **Les réponses des fournisseurs IA.** Ce sont des propositions, pas des
   ordres : elles passent par le `PolicyGate` comme n'importe quelle commande
   ([ADR-0004](adr/0004-command-bus.md)). Voir [I-07](../CLAUDE.md#i-07).

## Politique `unsafe`

**`unsafe` est refusé à la compilation.** `[workspace.lints.rust]` porte
`unsafe_code = "deny"`, et aucune des quatorze crates — douze sous `crates/`,
deux drivers sous `drivers/` — n'en contient ni ne le réautorise. C'est le manifeste qui fait foi ici, parce que c'est lui qui est
exécuté : ce document décrivait auparavant une politique d'encadrement que la
compilation n'accorde pas, et [ADR-0021](adr/0021-marqueur-d-arret.md) a fondé
une décision d'architecture — ne pas vérifier un pid — sur le refus, pas sur
l'encadrement.

Lever ce refus est une décision d'architecture, pas un `#[allow]` local :

- elle passe par un **ADR** qui dit ce que `unsafe` achète et ce qu'il coûte ;
- le `#[allow(unsafe_code)]` qui en découle est porté au plus près, jamais au
  workspace ;
- chaque bloc porte un `// SAFETY:` qui énonce l'invariant qui le rend correct,
  et **qui** le garantit ;
- il passe par la relecture de l'agent `relecteur-securite` ;
- la crate qui l'expose derrière une API sûre documente les conditions de cette
  sûreté.

Les dépendances externes, elles, en contiennent — Tauri, ses
webviews et les FFI de pilotes en imposent. Le refus porte sur **ce que ce dépôt écrit**.

Un `// SAFETY:` qui paraphrase le code (« on déréférence un pointeur valide »)
ne vaut rien : il doit dire **pourquoi** le pointeur est valide à cet endroit et
ce qui le maintiendra valide.

## Dépendances

- toute nouvelle dépendance directe est justifiée en revue : ce qu'elle apporte,
  et le coût de s'en passer ;
- `cargo deny` sur les licences et les avis de sécurité fait partie de la porte
  de qualité, par la cible `make deny` que `make qualite` appelle. Elle **avertit
  sans bloquer** quand `cargo-deny` n'est pas installé : une porte qui échoue sur
  un outil absent finit par être contournée, et c'est alors tout le contrôle qui
  disparaît. La configuration vit dans `deny.toml`. Toute licence qu'elle
  accepte est compatible avec la GPLv3, la licence de l'application
  ([ADR-0044](adr/0044-licence-gpl-et-contrat-apache.md)) ;
- une dépendance qui n'est utilisée qu'à un seul endroit pour une seule fonction
  est un candidat à la réécriture, pas une évidence ;
- une crate non maintenue sur une frontière externe est un risque à documenter,
  pas à ignorer ;
- un avis de sécurité écarté l'est **dans `deny.toml`, avec sa raison écrite**.
  Le seul aujourd'hui est RUSTSEC-2024-0429 — une *unsoundness* de
  `glib::VariantStrIter`, atteinte par `tauri` → `gtk 0.18` → `atk` → `glib 0.18`.
  La correction est dans `glib 0.20`, que `gtk 0.18` refuse : rien ne se monte
  ici tant que Tauri n'a pas changé de GTK. Ce code est celui de Linux ; il
  n'est pas compilé sur macOS, la seule cible livrée aujourd'hui, et Oxyn
  n'appelle pas cet itérateur. À rouvrir à la prochaine montée de Tauri.

Voir aussi [`/securite`](../.claude/commands/securite.md) et
[la liste de contrôle](../.claude/checklists/revue-securite.md).
