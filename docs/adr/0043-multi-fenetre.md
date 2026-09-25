# ADR-0043 — Plusieurs fenêtres dans un seul processus, chacune propriétaire de ses consoles et de ses sessions

**Statut :** proposé · **Date :** 2026-09-25

**Précise :** [ADR-0015](0015-consoles-independantes.md), sur le point
suivant : une session appartient à une fenêtre, et deux fenêtres ouvertes sur
la même connexion ne partagent aucune session, y compris celle du catalogue et
de l'aperçu.

**Précise :** [ADR-0013](0013-preferences-workspace.md), sur le point suivant :
`object_location` devient un état de fenêtre, et les réglages de disposition
ne s'appliquent aux autres fenêtres qu'à leur prochaine ouverture.

**Précise :** [ADR-0021](0021-marqueur-d-arret.md), sur trois points :
l'arrêt propre attend le vidage des brouillons de **toutes** les fenêtres ;
après une fermeture ordinaire, les copies de travail ouvertes ne restent plus
seulement « accessibles par la bibliothèque » : elles reviennent, hors ligne,
dans leur fenêtre — l'écran de reprise, lui, reste réservé à l'arrêt anormal ;
et une transaction ouverte retient l'arrêt ordonné le temps d'une décision
(« Transaction ouverte à la sortie »).

**Précise :** [ADR-0038](0038-un-plantage-s-annonce-une-fois.md), sur le point
suivant : `⌘Q` déclenche toujours l'arrêt ordonné, mais une étape le précède,
qui peut être annulée par l'utilisateur quand une transaction est ouverte. Le
Quit d'Oxyn, son raccourci et l'arrêt ordonné lui-même ne changent pas.

**Précise :** [ADR-0039](0039-etat-de-transaction-d-une-session.md) (proposé),
qui laissait hors de sa portée les fermetures hors console et n'ajoutait aucun
`Commit` ni `Rollback` : la sortie et la fermeture d'une fenêtre les proposent,
sur l'état qu'il fait constater. Cette partie de la décision suppose ADR-0039
accepté.

**Précise :** [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md), sur
un point : une sortie qu'il inscrit annule les transactions ouvertes, et le
journal le dit.

**Précise :** [ADR-0029](0029-interface-tauri-shadcn.md), sur le point suivant :
la capability de la webview couvre les fenêtres par un motif de libellé, à
permissions inchangées.

## Contexte

L'utilisateur a placé le multi-fenêtre dans le périmètre de la V1 le
2026-09-25. Le besoin concret est de travailler sur deux écrans à la fois :
une connexion `staging` à gauche, `production` à droite, ou deux consoles de la
même base côte à côte.

**Le code suppose partout une seule webview.** Relevé le 2026-09-25 :

| Où | Ce qui suppose une fenêtre unique | Ce qu'une deuxième fenêtre provoquerait |
|---|---|---|
| `crates/oxyn-desktop/tauri.conf.json` | une fenêtre `main`, 1280 × 820, minimum 960 × 600, `titleBarStyle: Overlay`, `hiddenTitle` | — |
| `capabilities/main.json` | `"windows": ["main"]` | une fenêtre d'un autre libellé n'a **aucune** permission : ni déplacement par la barre, ni sélecteur de fichier |
| `commands.rs`, `subscribe_events` ; `commands/metadata.rs`, `subscribe_refresh_signals` | un compteur de génération **statique au processus** (`commands/subscriptions.rs`) : tout nouvel abonnement termine le précédent, pour qu'un rechargement ne laisse pas de tâche orpheline | la deuxième fenêtre qui s'abonne **coupe en silence** le flux d'exécution et les signaux de catalogue de la première |
| `backend/recovery.rs`, `LocalWork::shutdown` | un seul `Option<Channel<ShutdownSignal>>` | la demande de vidage des brouillons n'atteint que la dernière fenêtre abonnée ; les autres perdent leurs 250 dernières millisecondes de frappe ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)) |
| `commands/recovery.rs`, `on_run_event` | tout `CloseRequested` est retenu et lance l'arrêt ordonné, puis `app.exit(0)` ; le Quit d'Oxyn (`oxyn-quit`, [ADR-0038](0038-un-plantage-s-annonce-une-fois.md)) et un `ExitRequested` non attendu y mènent aussi ; `RunEvent::Exit` inscrit la fermeture d'une sortie que macOS ne laisse pas retenir ([ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) | fermer **n'importe quelle** fenêtre quitte l'application |
| `backend/settings/location.rs`, `write_object_location` | un seul `object_location` dans les préférences du workspace, portant sa connexion ; `workspace-screen.tsx` le rouvre comme un emplacement quand la fenêtre ouvre cette connexion | deux fenêtres qui naviguent s'écrasent l'onglet d'objet sauvegardé ; les deux rouvrent le même au lancement suivant |
| `backend/confirm.rs`, `NativeDialog` et `Confirmations` ([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)) | le dialogue critique n'a pas de fenêtre parente, et un seul est ouvert pour tout le processus | — (correct à plusieurs fenêtres, à condition de dire à laquelle il répond : voir « Fermeture ») |
| `backend.rs`, `disconnect` | `Command::Disconnect { connection }` ferme toutes les sessions de la connexion, et `release_agents(connection)` tous ses agents | une fenêtre qui change de connexion ferme les sessions et arrête les agents d'une autre fenêtre ouverte sur la même base |
| `backend/ai/conversation.rs`, `ai_open_thread` | une conversation vivante reçoit le `Channel` de qui l'ouvre, et remplace le précédent (`threads.rs`, `ThreadState::channel`) | ouvrir la même conversation dans une deuxième fenêtre lui vole son flux |
| `apps/desktop/src/features/session.ts` | `open`, `restored`, `recoveryOffered` : un seul état de session par webview ; la clé de rechargement en `sessionStorage` | chaque fenêtre croit être la seule à avoir proposé la reprise |
| `features/consoles/draft-registry.ts` | `flushAllDrafts` vide les brouillons **de sa** webview | — (correct par fenêtre, à condition que chaque fenêtre soit appelée) |

Aucune de ces suppositions n'échoue bruyamment. Une deuxième fenêtre ouverte
sans décision produirait une première fenêtre muette, puis une fermeture qui
quitte tout.

**Ce que Tauri permet, vérifié dans les sources** des versions de `Cargo.lock`
— `tauri` 2.11.5, `tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4 —, dépaquetées
dans le registre local, le 2026-09-25 ([I-12](../../CLAUDE.md#i-12)) :

- le champ `windows` d'une capability accepte un motif glob, `admin-*` par
  exemple (`tauri-utils`, `src/acl/capability.rs`, doc de `Capability::windows`) ;
- une commande peut recevoir la `Webview` ou la `WebviewWindow` qui l'invoque :
  l'identité vient de l'environnement d'exécution, pas du contenu envoyé par le
  JavaScript (`tauri`, implémentations de `CommandArg` dans
  `src/webview/mod.rs` et `src/webview/webview_window.rs`) ;
- un `Channel` livre à la webview qui l'a créé, et à elle seule
  (`src/ipc/channel.rs`, `from_callback_fn(webview, …)`) ;
- l'écouteur d'événements de menu est global : il est appelé « for any menu
  event, whether it is coming from this window, another window or from the tray
  icon menu » (`src/webview/webview_window.rs`, doc de `on_menu_event`) ;
- créer une fenêtre « deadlocks when used in a synchronous command or event
  handlers » sous Windows (même fichier, doc de `WebviewWindowBuilder::new`) ;
- quand la dernière fenêtre est détruite, l'environnement émet
  `RunEvent::ExitRequested { code: None }`, que l'application peut empêcher
  (`tauri-runtime-wry`, `src/lib.rs`, traitement de `TaoWindowEvent::Destroyed`) ;
  `RunEvent::Reopen` n'existe que sous macOS (`tauri`, `src/app.rs`) ;
- une fenêtre déclarée avec `"create": false` sert de gabarit à
  `WebviewWindowBuilder::from_config`, sous un autre libellé (`tauri-utils`,
  `src/config.rs`, champ `WindowConfig::create`).

Ces faits sont reportés dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#menus-fenêtres-et-ddl-destructeur--vérification-du-2026-09-25).

Deux décisions écrites en parallèle bornent celle-ci : ADR-0041 (registre
d'actions ; barre de menus native en Rust sous macOS, barre web dans chaque
fenêtre sous Windows et Linux) et ADR-0042 (revue destructive sur place, dans la
vue qui l'a demandée). Quatre autres, du même jour, fixent ce qu'elle ne change
pas : le dialogue natif des décisions critiques
([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)), les
chemins d'arrêt ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md),
[ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) et l'état de
transaction d'une console
([ADR-0039](0039-etat-de-transaction-d-une-session.md), proposé).

## Décision

### Un processus, plusieurs fenêtres, une identité par fenêtre

- Toutes les fenêtres vivent dans le processus `oxyn-desktop`, sur le même
  `Backend`, le même `Executor` et le même runtime Tokio.
- Une fenêtre a une identité persistante, `WindowKey` (un UUID), déclarée dans
  `crates/oxyn-desktop/src/backend/windows.rs`. Son libellé Tauri en dérive :
  `workspace-<uuid sans tirets>`. **Le front ne choisit jamais un libellé** :
  aucune commande IPC n'en reçoit un.
- La fenêtre `main` de `tauri.conf.json` devient un gabarit, libellé `workspace`
  et `"create": false`. Toutes les fenêtres, la première comprise, sont
  construites par `WebviewWindowBuilder::from_config` sous leur libellé
  `workspace-…` : dimensions, minimum, `titleBarStyle` et `hiddenTitle` restent
  écrits à un seul endroit.
- **Au plus 16 fenêtres.** C'est une borne de garde contre un geste répété ou
  une webview compromise, pas une mesure : le coût mémoire d'une webview
  supplémentaire n'est pas mesuré. La 17ᵉ demande est refusée avec un message
  qui le dit.
- `Backend` tient un registre, `WindowRegistry`, seule source de vérité sur ce
  qu'une fenêtre possède. Une commande IPC qui vise une console, une session,
  une commande en cours, un résultat ou une conversation reçoit la `Webview`
  appelante ; `commands/` en tire la `WindowKey`, et `Backend` refuse ce que
  cette fenêtre ne possède pas (« This console belongs to another window »).
  Les commandes de niveau workspace — drivers, connexions enregistrées,
  préférences, bibliothèque, historique, réglages IA — n'ont pas de
  propriétaire.

### Ce qu'une fenêtre possède, et ce qui est partagé

**Une console appartient à une seule fenêtre à la fois.** De même une
conversation de l'assistant, l'agent externe qui la sert, une revue
destructive en cours (ADR-0042), une confirmation en attente, un export engagé.

**Une connexion ouverte dans deux fenêtres, c'est deux workspaces de connexion
indépendants.** Chacun établit ses propres sessions par le bus : la session
réservée au catalogue et à l'aperçu, puis une par console
([ADR-0015](0015-consoles-independantes.md)). Aucune session ne se partage
entre fenêtres, pour la raison qui a fait écarter le partage entre consoles :
une validation, une annulation de transaction ou un changement de contexte de
session ([ADR-0019](0019-contexte-de-session.md)) décidé dans une fenêtre ne
peut pas se produire dans l'autre.

Sont partagés, parce qu'ils appartiennent au workspace et non à une vue : la
configuration enregistrée de la connexion, **son marquage d'environnement et
son niveau de confidentialité** ([I-02](../../CLAUDE.md#i-02),
[I-04](../../CLAUDE.md#i-04) — le niveau reste attaché à la connexion, jamais à
la fenêtre), le cache de catalogue, les préférences, la bibliothèque,
l'historique, les résultats retenus
([ADR-0017](0017-retention-resultats.md)), les déclarations de fournisseurs et
d'agents.

Conséquences sur les chemins existants :

- **Une fenêtre n'émet plus `Command::Disconnect`.** Elle ferme ses propres
  sessions par `Command::CloseSession` et arrête ses propres conversations.
  `Backend` émet `Disconnect` lui-même quand la dernière fenêtre qui tenait la
  connexion la relâche : il voit toutes les fenêtres, une fenêtre n'en voit
  qu'une. `release_agents` devient par fenêtre.
- **Supprimer une connexion enregistrée** que tient une autre fenêtre est
  refusé : « This connection is open in another window. Close it there first. »
- **Modifier une connexion** — marquage, niveau, lecture seule — vaut aussitôt
  pour toutes les fenêtres : le `PolicyGate` lit déjà la configuration
  enregistrée par connexion. Chaque fenêtre qui la tient reçoit
  `WindowSignal::ConnectionChanged` et relit ce qu'elle affiche ; un repère de
  niveau périmé dans une fenêtre serait un repère faux.
- **Ouvrir ce qu'une autre fenêtre possède** — un document depuis la
  bibliothèque, une conversation depuis la liste — ne le duplique pas et ne le
  vole pas : la fenêtre propriétaire passe au premier plan, sur cet onglet.
  `ai_open_thread` ne remplace plus le `Channel` d'une conversation qu'une autre
  fenêtre possède.

### Déplacer une console

La V1 offre un geste de déplacement : **`Open in new window`**, sur un onglet.
`New window` ouvre une fenêtre vide, sur l'écran de choix de connexion ;
aucune connexion n'est ouverte d'office. Les raccourcis de ces actions sont
tenus par le registre d'ADR-0041.

Déplacer une console **transfère sa session**, sans la fermer ni la rouvrir :

1. la fenêtre source vide le brouillon de la console (autosauvegarde,
   [ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)) ;
2. `Backend` crée la fenêtre, inscrit la console, sa session et son résultat au
   nom de la cible, puis prépare un `ConsoleHandoff` : identifiant de document,
   connexion, session, référence de résultat, valeurs liées ;
3. la cible adopte le `ConsoleHandoff`, se déclare lectrice du résultat, **puis**
   la source relâche sa vue : entre les deux, le résultat a toujours un lecteur,
   et la rétention d'[ADR-0017](0017-retention-resultats.md) ne l'évince pas ;
4. la cible établit sa propre session de catalogue. La console est utilisable
   tout de suite : sa session l'a suivie.

Rien n'est réexécuté : la grille relit les pages du même `ResultBuffer`
([I-06](../../CLAUDE.md#i-06)).

Le déplacement est **refusé** — l'entrée est grisée, avec sa raison — tant que
la console a une exécution en cours, une confirmation en attente, une revue
destructive ouverte (ADR-0042) ou un export engagé. L'issue d'une commande
revient comme réponse de l'`invoke` de la webview qui l'a émise : déplacée en
vol, cette issue arriverait dans une fenêtre qui ne tient plus la console.

**Les valeurs liées suivent la console, en mémoire seulement.** Le
`ConsoleHandoff` ne dérive pas `Debug`, n'est jamais écrit, et se consomme une
fois ; si la cible se ferme avant de l'adopter, il est abandonné avec elles
([I-03](../../CLAUDE.md#i-03)). **L'historique d'annulation de l'éditeur ne
suit pas** : il vit dans l'instance CodeMirror de la source.

Un onglet d'objet déplacé rouvre le même emplacement dans la cible, qui le
charge comme le ferait une sélection : c'est une lecture par le bus, pas un
transfert.

**Hors V1** : arracher un onglet en le glissant hors de sa fenêtre, et
déplacer une console vers une fenêtre **existante**. Le second geste réutilise
le même `ConsoleHandoff` et ne change aucun format ; il demande seulement une
interface de choix de la cible, que l'implémentation actuelle rend délicate —
une fenêtre n'y tient qu'une connexion à la fois (`session.ts`, `open`).

### Les actions visent la fenêtre qui a le focus

- `Backend` suit le focus par `WindowEvent::Focused` : `WindowRegistry::focused`
  est la dernière fenêtre à l'avoir reçu, et redevient vide quand elle se ferme.
- **Sous macOS**, la barre de menus est globale au processus (ADR-0041). Une
  action de portée fenêtre est envoyée à la fenêtre qui a le focus **au moment
  où l'événement de menu est reçu**, par son propre canal
  (`WindowSignal::Action`), jamais aux autres. Les actions de portée
  application — `New window`, `Settings…`, `Quit Oxyn` — n'ont pas de cible.
  Sans fenêtre au focus, les actions de portée fenêtre sont grisées.
- **L'état actif ou grisé suit le focus.** Chaque webview déclare l'état de ses
  actions à `Backend` quand il change (`report_action_state`, identifiants du
  registre d'ADR-0041 ; un identifiant inconnu est refusé). `Backend` garde
  l'état de chaque fenêtre et n'applique au menu natif que celui de la fenêtre
  au focus, à chaque changement de focus et à chaque déclaration de celle-ci.
- **La fenêtre qui reçoit une action la revérifie** avant de l'exécuter, et
  ignore celle qui n'est plus disponible. L'état du menu peut avoir une
  déclaration de retard sur un changement de focus ; il ne fait jamais foi.
- **Sous Windows et Linux**, la barre vit dans chaque webview : l'action part
  de la fenêtre qui la déclenche, et aucun routage n'est nécessaire.

### IPC : rien n'est diffusé

- **Un abonnement par fenêtre et par flux.** `subscribe_events`,
  `subscribe_refresh_signals`, `subscribe_shutdown` et le nouveau
  `subscribe_window` sont indexés par `WindowKey`. La logique de
  `commands/subscriptions.rs` est conservée, mais son compteur de génération
  devient **par fenêtre** : le rechargement d'une fenêtre termine son ancien
  abonnement, et celui-là seul.
- **Un événement d'exécution va à la fenêtre qui possède la commande.** Le
  registre inscrit le propriétaire d'un `CommandId` **avant** le dispatch, pour
  qu'aucun événement ne le précède ; la tâche d'abonnement d'une fenêtre ne
  transmet que les événements de ses commandes. Le filtre est en Rust : une
  fenêtre ne reçoit jamais le flux d'une autre pour le trier elle-même. Une
  commande d'agent a pour propriétaire la fenêtre de sa conversation.
- **Un signal de catalogue va aux fenêtres qui tiennent la connexion**, pas
  aux autres. Un changement de préférences ou de connexion enregistrée va à
  chaque fenêtre, sur son propre canal, sous forme d'avis
  (`WindowSignal::PreferencesChanged { revision }`,
  `WindowSignal::ConnectionChanged`) ; la fenêtre relit par le bus.
- **Les six méthodes d'émission de `tauri::Emitter`** — `emit`, `emit_to`,
  `emit_filter` et leurs variantes `emit_str*`, relevées dans `src/lib.rs` de
  `tauri` 2.11.5 le 2026-09-25 — **sont interdites** dans `clippy.toml` : elles
  sont le chemin de la diffusion, et le
  front n'a de toute façon pas la permission d'écouter un événement
  (`capabilities/main.json` n'accorde rien de `core:event`).
- Tout nouveau message — réponse d'`invoke` ou message de `Channel` :
  `WindowSignal`, `ConsoleHandoff` vu du front, état des fenêtres — a son
  schéma dans `apps/desktop/src/lib/ipc/` et est validé à l'entrée, par `call`
  ou par la garde de `Channel` de `client.ts`
  ([ADR-0031](0031-validation-des-reponses-ipc.md)). Dans l'autre sens,
  `commands/` parse ce que déclare une webview avant de le confier à `Backend`.

### Capabilities : un motif, pas une permission

Le fichier `capabilities/main.json` garde **exactement** ses trois permissions —
`core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize`,
`dialog:allow-open` — et son champ `windows` devient `["workspace-*"]`. Aucune
permission de création de fenêtre ou de webview n'est accordée à la webview :
**une fenêtre ne s'ouvre que par une commande d'Oxyn**, exécutée en Rust, bornée
à 16, et relue comme tout changement de surface IPC
([SECURITY](../SECURITY.md#surface-dentrée), surface 5). Une XSS dans une
fenêtre peut donc au pire ouvrir des fenêtres jusqu'à la borne ; elle ne gagne
aucune API de la webview.

### Persistance : deux tables du fichier de workspace

Une migration SQLite du store — la 18 si aucune autre ne la précède : la 17,
d'[ADR-0038](0038-un-plantage-s-annonce-une-fois.md), est la dernière le
2026-09-25 — ajoute deux tables, lisibles avec n'importe quel client
SQLite et sans Oxyn ([I-11](../../CLAUDE.md#i-11)) — des colonnes plutôt qu'un
JSON, parce que la contrainte d'appartenance doit être tenue par le fichier
lui-même :

```sql
CREATE TABLE workspace_windows (
    id               TEXT PRIMARY KEY NOT NULL,  -- WindowKey
    workspace_id     TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    app_session_id   TEXT NOT NULL,              -- dernier lancement qui l'a écrite
    ordinal          INTEGER NOT NULL,           -- ordre de restauration
    x                REAL,                       -- pixels logiques ; NULL = placement du système
    y                REAL,
    width            REAL NOT NULL,
    height           REAL NOT NULL,
    maximized        INTEGER NOT NULL,
    object_location  TEXT,                       -- même forme que dans les préférences
    active_document  TEXT REFERENCES documents(id) ON DELETE SET NULL,
    revision         INTEGER NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;

CREATE TABLE workspace_window_consoles (
    window_id    TEXT NOT NULL REFERENCES workspace_windows(id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    PRIMARY KEY (window_id, document_id),
    UNIQUE (document_id)
) STRICT;
```

`UNIQUE (document_id)` est l'invariant « une console, une fenêtre » écrit dans
le fichier : une écriture qui le violerait échoue au lieu de produire deux
fenêtres rivales sur le même document.

- **Les écritures passent par le bus**, comme les préférences
  ([ADR-0013](0013-preferences-workspace.md)) : `Command::WriteWindowLayout`,
  réservée à `Actor::Human` par la politique par défaut, comptée par
  `is_local_write` pour que l'arrêt l'attende, exécutée sur le pool bloquant
  ([ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)).
  Un agent n'ouvre, ne ferme ni ne dispose aucune fenêtre.
- **Quand on écrit.** L'appartenance — ouverture, fermeture ou déplacement
  d'une console, onglet actif — s'écrit aussitôt. La géométrie s'écrit une
  seconde après le dernier `Moved` ou `Resized`, et à la fermeture : une rafale
  de redimensionnement ne produit qu'une écriture.
- **Deux instances sur le même fichier** ([ADR-0021](0021-marqueur-d-arret.md)
  le prévoit) : un lancement n'adopte que les lignes dont `app_session_id`
  désigne une session terminée — fermée ou abandonnée au sens d'ADR-0021 — et
  les réécrit à son nom. Les fenêtres d'une instance vivante ne sont pas
  reprises par l'autre.
- **Le fichier est une entrée hostile**
  ([SECURITY](../SECURITY.md#surface-dentrée), surface 3). À la lecture :
  au plus 16 fenêtres et 256 consoles par fenêtre, le surplus ignoré et
  journalisé, ses documents laissés dans la bibliothèque ; une largeur ou une
  hauteur sous le minimum du gabarit est ramenée au minimum ; une fenêtre dont
  le rectangle ne recoupe aucun écran disponible est replacée par le système à
  la taille du gabarit ; un `object_location` illisible vaut absent.
- `object_location` quitte les préférences pour la fenêtre. Aujourd'hui, il est
  un champ des préférences qui porte sa connexion, écrit par
  `write_object_location` et rouvert comme un emplacement, sans lecture,
  quand la fenêtre ouvre cette connexion. Ce comportement ne change pas : il
  devient celui de chaque fenêtre, sur sa propre ligne. Au premier lancement
  après la migration, la première fenêtre reprend la valeur des préférences ;
  ensuite, le champ des préférences n'est plus écrit, et reste lisible par une
  version antérieure, conformément à la règle d'ADR-0013 sur les champs.
- `--temporary-workspace` tient ces tables dans son store en mémoire, comme le
  reste : rien n'est écrit.

### Restauration et reprise, par fenêtre

- **Après une fermeture ordinaire**, chaque fenêtre revient à sa place et à sa
  taille. L'écran de reprise ne s'affiche pas : il reste réservé à l'arrêt
  anormal ([ADR-0021](0021-marqueur-d-arret.md)).
- **Ses consoles reviennent avec elle, hors ligne** — arbitré par l'utilisateur
  le 2026-09-25, pour `⌘Q`, `File ▸ Exit` et la fermeture de la dernière
  fenêtre ; de même après le Quit du Dock, dont ADR-0040 inscrit la
  fermeture. Texte, nom et autosauvegarde, sans reconnexion ni exécution, par le
  mécanisme qui rouvre déjà ainsi les éditeurs choisis sur l'écran de reprise
  (`features/session.ts`, `restored` ; `components/oxyn/offline-console-bar.tsx`)
  et selon les règles de la
  [restauration sélective](../UX-SPEC.md#restauration-sélective-au-démarrage) :
  le raccordement à une connexion reste un geste séparé. Aucune sélection
  n'est demandée : c'est ce qui distingue une fermeture ordinaire d'un arrêt
  anormal. Jusqu'ici, ces copies ne se retrouvaient que dans la bibliothèque
  (ADR-0021) ; elles y restent aussi.
- **Après un arrêt anormal**, les fenêtres reviennent à leur place et chacune
  propose la sélection de reprise **de ses propres consoles** et de son onglet
  d'objet, comme l'écran de reprise le fait aujourd'hui pour la fenêtre
  unique. Une console non retenue quitte la fenêtre et reste intacte dans la
  bibliothèque. L'avertissement d'écriture interrompue
  ([I-13](../../CLAUDE.md#i-13)) porte sur l'historique, pas sur une fenêtre :
  il s'affiche une fois, dans la première fenêtre restaurée. Le constat
  « proposé une fois par lancement » (`recoveryOffered`) et le refus de
  l'onglet d'objet (`objectPlaceDeclined`) passent de `session.ts` au
  registre, par fenêtre.
- Une copie de travail ouverte qu'aucune fenêtre ne réclame — écrite par une
  version antérieure, ou détachée par une fermeture non confirmée (ci-dessous)
  — revient dans la première fenêtre.
- **L'autosauvegarde reste par console** ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)) ;
  ce qui s'ajoute par fenêtre est la ligne de disposition, écrite comme
  ci-dessus.

### Fermeture

**Fermer une fenêtre qui n'est pas la dernière ferme ses consoles.**
`CloseRequested` est retenu ; la fenêtre reçoit `WindowSignal::CloseRequested`
et applique les règles de fermeture d'une console
([ADR-0015](0015-consoles-independantes.md)) à toutes les siennes, dans **un
seul dialogue** qui nomme la fenêtre, liste les consoles portant du SQL non
sauvegardé ou une opération, et place le focus sur `Cancel`. Une console dont
une transaction est ouverte y est résolue comme à la sortie (« Transaction
ouverte à la sortie », ci-dessous), pour les seules sessions de cette fenêtre.
Confirmée, la fermeture vide les
brouillons de cette fenêtre, ferme ses documents, ses sessions et ses
conversations, retire sa ligne de disposition, puis détruit la fenêtre. Une
opération en cours y est annulée comme par `⌘W` ; une écriture dont l'issue
devient inconnue reste signalée, jamais rejouée ([I-13](../../CLAUDE.md#i-13)).

**Le dialogue critique reste celui du processus, pas d'une fenêtre.** Le
dialogue natif d'[ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)
garde son absence de fenêtre parente : l'analyse de son § 3 — le bouton que
répond Entrée, sous `CFUserNotificationDisplayAlert` — vaut pour ce cas. Avec
une fenêtre parente, `rfd` passe à un autre mécanisme, un `NSAlert` en feuille
(`rfd` 0.16.0, `src/backend/macos/message_dialog.rs`, `show` et
`show_async`, lu le 2026-09-25), dont le comportement face à cette analyse
n'est pas vérifié. Ce qui identifie la décision est ce que le backend
écrit : la connexion, son environnement, son adresse et l'instruction — pas la
fenêtre, qui n'a pas de nom. Un seul dialogue critique est ouvert pour tout le
processus ; une décision critique venue d'une autre fenêtre pendant ce temps
est refusée sans être consommée, comme le prévoit son § 3.

**Une approbation en attente dans une fenêtre qu'on ferme est rejetée** à la
confirmation de la fermeture, comme une confirmation d'onglet fermé. Le plugin
ne sait pas fermer un dialogue natif : s'il est resté à l'écran, sa réponse
n'approuve plus rien, puisque `confirm_held` ne confirme que la décision qu'il
a montrée et qu'elle n'est plus en attente.

Une webview qui ne répond pas dans le délai de vidage de `backend/recovery.rs`
(`FLUSH_GRACE`, 2 s) n'empêche pas la fermeture ; ses consoles ne sont alors
**pas** fermées : leurs documents restent ouverts, détachés de toute fenêtre,
et le lancement suivant les traite comme une copie de travail qu'aucune
fenêtre ne réclame (« Restauration et reprise, par fenêtre »). C'est la
lecture prudente d'une fermeture que personne n'a confirmée, la même
qu'ADR-0021.

« La dernière » se décide dans `WindowRegistry`, sous son verrou : une fenêtre
dont la fermeture est confirmée ne compte plus, une fenêtre dont le dialogue
est ouvert compte encore.

**Fermer la dernière fenêtre quitte l'application, sur toutes les plateformes,
macOS compris** — tranché par l'utilisateur le 2026-09-25, pour les raisons
ci-dessous. C'est l'arrêt ordonné déjà en place : vidage des brouillons,
attente des écritures locales soumises
([ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)),
inscription de la fermeture ([ADR-0021](0021-marqueur-d-arret.md)), annulation
des sessions. Il ne ferme pas les consoles : leurs documents restent ouverts
dans le store, et reviennent dans leur fenêtre au lancement suivant.

Les chemins de sortie :

| Sortie | Chemin | À plusieurs fenêtres |
|---|---|---|
| Fermeture de la dernière fenêtre | `CloseRequested`, résolution des transactions, arrêt ordonné | inchangé |
| `⌘Q`, `Quit Oxyn` (macOS) | élément `oxyn-quit` ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md)), résolution des transactions, arrêt ordonné | quel que soit le nombre de fenêtres ; aucune n'est fermée une à une |
| `File ▸ Exit` (Windows, Linux) | commande `request_exit` (ADR-0041), même chemin que `⌘Q` | idem |
| `ExitRequested` non attendu | retenu, même chemin que `⌘Q` | inchangé |
| Quit du Dock, fermeture de session macOS | `RunEvent::Exit`, fermeture inscrite après les écritures locales, brouillons non vidés ([ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) ; transactions annulées, et journalisées | aucune fenêtre n'est interrogée ; chacune peut perdre ses 250 dernières millisecondes de frappe |

Les quatre premières passent par une seule fonction Rust, celle qu'appelle
aujourd'hui `on_run_event` (`request_exit`, `commands/recovery.rs`) ; aucune
ne la contourne.

Dans l'arrêt ordonné, le vidage est demandé à **toutes** les fenêtres en même
temps, et `FLUSH_GRACE` borne l'ensemble, pas chaque fenêtre ; la fermeture
n'est inscrite qu'une fois toutes les webviews ayant répondu, ou le délai
écoulé. Un dialogue natif ouvert au moment de l'arrêt vaut refus
(ADR-0037 § 3). `RunEvent::Reopen` n'est pas traité : il n'existe pas d'état
sans fenêtre.

Sous Windows et Linux, `File ▸ Exit` quitte toutes les fenêtres d'un geste,
comme `⌘Q` : aucune n'est fermée une à une, et toutes rendent leurs consoles
au lancement suivant.

### Transaction ouverte à la sortie

Arbitré par l'utilisateur le 2026-09-25 : **une transaction ouverte retient la
sortie, et l'utilisateur choisit `Commit`, `Rollback` ou `Cancel`.** Tout ce
qui suit suppose [ADR-0039](0039-etat-de-transaction-d-une-session.md)
accepté : sans état constaté, il n'y a rien à lister, et la sortie annule
comme aujourd'hui.

**Quand.** Avant l'arrêt ordonné, et avant `begin_shutdown` : tant qu'une
transaction n'est pas résolue, rien n'est vidé, rien n'est inscrit, et
`Cancel` rend l'application intacte. Pour la fermeture d'une fenêtre qui n'est
pas la dernière, la même étape porte sur ses seules sessions.

**Ce qui est listé.** Le backend liste chaque session de console qui déclare
`TRANSACTIONS`, avec le **dernier état que l'exécuteur a constaté** pour elle :
celui que `Outcome::Connected` porte à l'ouverture de la console, puis celui de
chaque `Event::TransactionState` publié à la fin d'une exécution
([ADR-0039](0039-etat-de-transaction-d-une-session.md) § 3 et § 4). Une
session dont une instruction de console est en cours — envoyée par
`run_console` ou approuvée par `decide` — compte comme `Unknown` jusqu'à sa
réponse, quel que soit l'événement qui arrive entre-temps ; des événements manqués par un abonné en
retard rendent `Unknown` toutes les sessions. Avant de lire, le backend
applique les événements déjà publiés, en attendant au plus `FLUSH_GRACE`
(2 s) ; au-delà, toutes comptent comme `Unknown`. Une session `Open`, ou
`Unknown`, est listée. Aucune : l'arrêt ordonné commence aussitôt, comme
aujourd'hui. Le front ne fournit pas la liste : il pourrait l'avoir périmée,
et une XSS pourrait la vider.

*Précision de mise en œuvre, 2026-09-25 :* le texte proposé faisait lire
l'état par `Session::transaction_state` depuis le pont. C'était un appel au
driver hors du bus, celui qu'ADR-0039 § 4 a déjà retiré au nom
d'[I-01](../../CLAUDE.md#i-01) : la sortie lit l'état que l'exécuteur a
constaté et publié, sans rien demander à la session
(`crates/oxyn-desktop/src/backend/exit.rs`). Le signal porte la session et non
le document, que le backend ne connaît pas : la fenêtre nomme la console
d'après ses onglets, et le journal d'une sortie sans accusé nomme la connexion
et la session.

**Où le dialogue vit : dans la webview.** Le backend envoie à chaque fenêtre
concernée, par son canal d'arrêt (`subscribe_shutdown`, par fenêtre), le
signal `ShutdownSignal::ResolveTransactions`, portant les consoles à résoudre
— document, connexion, environnement, état. La fenêtre passe au premier plan
et ouvre un `alert-dialog` qui nomme, pour chaque console, la console, la
connexion et son environnement, et l'état (`Transaction open`, ou
`Transaction state unknown`), avec trois actions : `Commit`, `Rollback`,
`Cancel`, le focus sur `Cancel`, Entrée seule ne validant rien. `Commit` et
`Rollback` valent pour toute la liste de cette fenêtre ; une décision
console par console reste possible avant, depuis chaque console, en tapant
`COMMIT` ou `ROLLBACK`.

Pourquoi pas le dialogue natif d'ADR-0037 : `Commit` n'y relève d'aucune
famille. Il n'écrit rien que le gate n'ait déjà laissé passer —
`oxyn-query` classe `COMMIT` et `ROLLBACK` en lecture
(`crates/oxyn-query/src/classify.rs`, contrôle de transaction), et chaque
écriture de la transaction a été retenue et, sur `production`, confirmée dans
le dialogue natif au moment de son exécution. Un script qui voudrait valider
une transaction n'a pas besoin de ce dialogue : il tape `COMMIT` dans la
console par `run_console`, ce qu'il peut déjà. Le dialogue de sortie n'ouvre
donc aucune capacité nouvelle, et reste là où l'on lit et choisit.

**Ce que font les boutons.** `Commit` et `Rollback` émettent, pour chaque
session listée, un `Command::Execute` portant `COMMIT` ou `ROLLBACK`, sous
`Actor::Human`, par le chemin de `run_console` — le même que si l'utilisateur
l'avait tapé : gate, `audit_journal`, `query_history`
([I-01](../../CLAUDE.md#i-01)). Aucune variante de `Command` n'est ajoutée, et
les méthodes `Session::commit` et `rollback` restent sans appelant. La fenêtre
attend l'`Event::TransactionState` de chaque session (ADR-0039 § 3) :

* toutes `Idle` — la fenêtre rappelle `request_exit`, qui relit l'état ;
  l'arrêt ordonné commence quand plus aucune fenêtre n'a de transaction ;
* une erreur — le message du serveur s'affiche, la sortie ne continue pas, et
  le dialogue reste ouvert sur les sessions non résolues. Un `COMMIT` dont
  l'issue est ambiguë n'est **jamais** rejoué ([I-13](../../CLAUDE.md#i-13)) :
  c'est l'état constaté ensuite qui dit s'il a pris ;

  *Précision de mise en œuvre, 2026-09-25 :* les instructions partent une à
  une et la première erreur arrête la série. Un `COMMIT` qui a échoué, ou dont
  la réponse n'est pas arrivée, n'est plus proposé pour cette session : seuls
  `Rollback` et `Cancel` restent, et `COMMIT` tapé dans la console reste
  possible après `Cancel`. `run_console` ne rend la main qu'après la
  publication de l'état : la fenêtre rappelle `request_exit` dès que chaque
  instruction a répondu, et c'est la relecture du backend qui tranche ;
* `Cancel`, dans n'importe quelle fenêtre — la fenêtre appelle `cancel_exit`,
  et la sortie est abandonnée pour toutes : les autres fenêtres referment leur
  dialogue (`ShutdownSignal::ExitCancelled`), et rien n'a été vidé ni inscrit.

**Une webview muette ne retient pas la sortie.** Chaque fenêtre accuse
réception du signal par `shutdown_acknowledged`, sous `FLUSH_GRACE` (2 s). Faute d'accusé — webview figée, rechargée, sans
abonnement —, la sortie continue pour cette fenêtre comme aujourd'hui : ses
transactions sont annulées par la fermeture des sessions, et le journal le dit
(`warn`, connexion et console nommées, jamais le SQL). Une fois l'accusé reçu,
l'attente n'a pas de borne : c'est l'utilisateur qui décide, comme devant le
dialogue de fermeture d'une console.

**Deux commandes IPC de plus, sans argument** : `shutdown_acknowledged` et
`cancel_exit`, sur le modèle de `shutdown_flushed`. Elles ne portent aucune
donnée et n'agissent que sur une sortie déjà demandée ; hors de ce cas, elles
ne font rien. Ce qu'une XSS en tire : retenir ou annuler une sortie que
l'utilisateur a demandée — une gêne, pas une donnée ni une écriture
([SECURITY](../SECURITY.md#surface-dentrée), point 5).

**Le Quit du Dock et la fermeture de session macOS** ne peuvent pas attendre
(ADR-0040) : les transactions ouvertes y sont annulées par la fin du
processus. `close_on_forced_exit` ajoute une ligne de journal `warn` par
session listée à cet instant — lue sans attendre ce qui est en cours, donc
possiblement `Unknown` —, et rien d'autre ne change sur ce chemin.

> **Pourquoi on s'écarte de la convention macOS.** Elle garde l'application
> vivante quand sa dernière fenêtre se ferme, et en rouvre une au clic sur
> l'icône du Dock (`RunEvent::Reopen`). La V1 ne la suit pas :
>
> - l'arrêt ordonné est déjà le chemin de la fermeture de la dernière fenêtre,
>   et celui de `⌘Q` depuis ADR-0038. Un état sans fenêtre demanderait un
>   second régime : une fermeture de la dernière fenêtre qui n'arrête rien,
>   puis un arrêt sans webview à vider ;
> - un état « zéro fenêtre » obligerait à décider ce qu'il garde : des sessions
>   serveur ouvertes sans aucune vue pour les montrer, ou leur fermeture — donc
>   une fermeture de la dernière fenêtre qui ferme ses consoles, et un
>   lancement suivant qui ne les rend plus. Le Quit du Dock, seule sortie
>   restante de cet état avec `⌘Q`, n'annule en outre aucune instruction en
>   cours (ADR-0040) ;
> - rien, en V1, ne travaille sans fenêtre : pas d'export en arrière-plan, pas
>   de surveillance, pas d'agent autonome.
>
> Le prix : un utilisateur de macOS qui ferme la fenêtre en attendant que
> l'application reste dans le Dock la voit quitter, et relance. Ce prix est
> borné par la restauration : il retrouve ses fenêtres à leur place, avec
> leurs consoles, hors ligne.
>
> **Reconsidérer si** une activité doit survivre à la fermeture des fenêtres
> (export long, [rafraîchissement automatique](0022-rafraichissement-automatique.md)
> programmé, agent qui travaille seul), si le démarrage à froid de l'interface
> Tauri, mesuré, dépasse le budget de 1 s de
> [PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction), ou si l'utilisateur
> relève l'écart comme un défaut.

### Threads

Rien, à l'ouverture d'une fenêtre, ne bloque le thread principal
([I-05](../../CLAUDE.md#i-05)) :

- `open_window` et le déplacement sont des commandes **`async`** : jamais
  synchrones, à cause du blocage documenté sous Windows et parce qu'une commande
  synchrone tourne sur le thread principal
  ([ARCHITECTURE](../ARCHITECTURE.md#le-modèle-de-threads)) ;
- l'écriture de la disposition passe par le bus et le pool bloquant ; la
  construction de la fenêtre n'attend ni cette écriture ni la session de
  catalogue, qui s'ouvre après, annulable ;
- le traitement de `Moved`, `Resized`, `Focused` et `CloseRequested` dans
  `on_run_event` ne fait que mettre à jour le registre en mémoire et lancer une
  tâche ; il n'écrit rien et n'attend aucun verrou tenu par une tâche ;
- la disposition est lue par `Backend::open`, **avant** la boucle
  d'événements — l'exception déjà admise pour l'assemblage du backend, bornée à
  16 fenêtres et 256 consoles par fenêtre. Au démarrage, la fenêtre qui avait
  le focus est construite la première ; les autres suivent, chacune dans sa
  tâche, sans retarder la première.

### Budgets

Aucun budget nouveau n'est inventé ; ceux de
[PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction) s'appliquent. `New window`
et `Open in new window` montrent un retour sous **100 ms** — le cadre de la
fenêtre — et une fenêtre utilisable sous **1 s**, le budget de l'ouverture à
froid. Au démarrage, ce budget de 1 s vaut pour la première fenêtre, pas pour
l'ensemble. Aucune de ces valeurs n'est mesurée sur la webview Tauri à ce jour.

### Précisions de mise en œuvre, 2026-09-25 (lot 7, première partie)

Le lot 7 est livré en deux pull requests : la première rend les fenêtres
vivantes — registre, propriété, abonnements, menu, fermeture et sortie —,
la seconde apporte la disposition persistée, la restauration par fenêtre et
`Open in new window`. Ce que la première a précisé, en écrivant le code :

- **L'assistant appartient à une fenêtre par connexion, pas par
  conversation.** Le code tient l'état de l'assistant par connexion
  (`backend/ai` : conversations, agent en attente, échantillons demandés).
  Le rendre propre à chaque fenêtre aurait refait ce module entier. Le
  registre tient donc `assistants : ConnectionId → WindowKey`. La première
  fenêtre qui s'en sert le prend ; une autre fenêtre ouverte sur la même
  connexion reçoit « The assistant of this connection is open in another
  window ». La fenêtre propriétaire ne passe au premier plan que sur
  l'ouverture d'une conversation, pas sur chaque refus : un script qui
  bouclerait sur un refus volerait sinon le clavier d'une autre fenêtre. Une
  fenêtre ne prend l'assistant que d'une connexion qu'elle tient, comme elle
  n'ouvre de console que sur une connexion qu'elle tient. La règle reste
  plus stricte que le texte : aucune conversation n'est partagée. Elle sera
  à reconsidérer si deux assistants sur la même base, dans deux fenêtres,
  sont demandés.
- **`report_action_state` est `set_menu_state`**, la commande qui existait,
  désormais par fenêtre. Elle refusait déjà un identifiant inconnu. Le focus
  reste à la dernière fenêtre qui l'a reçu quand l'application passe derrière
  une autre, et ne se vide qu'à la fermeture de cette fenêtre. Sans fenêtre au
  focus, la barre ne garde actives que `New window`, `Settings…` et
  `Quit Oxyn`. Une action d'application, sans cible, va alors à la première
  fenêtre.
- **La fermeture d'une fenêtre non dernière se fait en deux temps**, dans
  cet ordre. D'abord, ses transactions. Le dialogue d'`ExitTransactionsDialog`
  est repris et porte `scope: window` : `Commit` et `Rollback` y rappellent
  `close_window`, et non `request_exit`. Ensuite, s'il reste des consoles qui
  perdraient du travail, **un seul** dialogue (`CloseWindowDialog`) les liste.
  Il nomme la fenêtre par ses connexions, puisqu'elle n'a pas de nom, et
  n'offre que `Cancel`, qui a le focus, et `Discard and close window`. Une
  console à garder se sauvegarde avant, depuis la console. Sans console à
  risque, la fenêtre se ferme sans dialogue, comme une console sans travail.
  `shutdown_acknowledged` et `cancel_exit` servent aux deux étapes.
  S'ajoutent `subscribe_window`, `close_window` et `confirm_window_close`,
  sans argument, et `open_window`.
- **Les événements d'une commande d'agent ne vont à aucune fenêtre** : aucune
  webview ne les lisait, l'assistant suit son agent par le `Channel` de sa
  conversation. Une décision ou un résultat qu'aucune fenêtre n'a reçu —
  ceux d'un agent, que l'assistant affiche — revient à la première fenêtre qui
  s'en sert. Leurs identifiants ne se devinent pas (UUID v7).
- **Un document appartient à la fenêtre dont la console l'écrit**, dès sa
  première sauvegarde, et seules les consoles sauvegardent. Une console
  d'une autre fenêtre ne peut plus l'écrire, le fermer ni le supprimer. Il
  reste à faire passer la fenêtre propriétaire au premier plan quand on
  rouvre ce document depuis la bibliothèque d'une autre fenêtre : c'est la
  seconde partie, avec le déplacement de console.
- **Seule la fenêtre construite au lancement propose l'écran de reprise**
  (`recovery_status`). Une fenêtre ouverte ensuite n'a pas vu l'arrêt qui l'a
  précédée, et deux écrans de reprise offriraient deux fois les mêmes copies.
- **`PreferencesChanged` relit tout, sauf la disposition** : thème, densité et
  format des cellules suivent aussitôt. La barre latérale et l'inspecteur de
  la fenêtre restent tels quels jusqu'à sa prochaine ouverture (§ ADR-0013,
  ci-dessus).
- **Une webview qui n'accuse pas réception de sa fermeture** voit ses
  sessions fermées, donc ses transactions annulées, avec une ligne `warn` par
  transaction. Ses documents restent ouverts.

### Précisions de mise en œuvre, 2026-09-26 (lot 7, disposition et restauration)

- **La migration 18 est celle écrite plus haut**, avec un index
  `workspace_windows_order (workspace_id, ordinal)`. `oxyn-store/src/windows.rs`
  l'écrit (`save`, `remove`) et la relit (`adopt`). `WindowLayout` et
  `WindowLayoutChange` vivent dans `oxyn-core`, sans type de Tauri
  ([I-08](../../CLAUDE.md#i-08)). La ligne nomme le lancement qui l'écrit :
  l'exécuteur reçoit l'`AppSessionId` du lancement (`with_app_session`) et le
  passe au store. `WindowLayout::validate` refuse, avant le fichier, une
  taille ou une position qui n'est pas un nombre fini, plus de 256 consoles,
  une console en double ou une console au premier plan qui n'en est pas une.
- **Une console jamais enregistrée n'est pas écrite** : sa ligne de
  `documents` n'existe pas encore, et la clé étrangère la refuserait. Elle
  rejoint la disposition à sa première sauvegarde. Une console qu'une autre
  ligne liste la quitte (`ON CONFLICT (document_id)`) ; c'est le registre qui
  empêche une fenêtre de prendre la console d'une autre, en écartant de son
  rapport tout document qu'une autre fenêtre écrit.
- **La lecture adopte en une transaction**, bornée à 64 lignes lues et à 257
  consoles par fenêtre. Chaque colonne se lit sans échouer : un fichier
  retouché peut avoir perdu `STRICT`, et une valeur mal typée ne coûte que sa
  ligne ou sa colonne. Une ligne dont l'identifiant ne
  se relit pas, et celles au-delà de 16, sont retirées du fichier ; les
  consoles au-delà de 256 aussi, journalisées, leurs documents laissés dans la
  bibliothèque. Une console dont le document est fermé ou supprimé ne revient
  pas. Une taille qui n'est pas un nombre fini vaut `0`, ramenée au minimum du
  gabarit par le desktop ; une position hors bornes vaut absente.
- **Le rectangle est en pixels logiques** : position extérieure et taille
  intérieure, divisées par l'échelle de la fenêtre. Il est confronté à la
  zone utile de chaque écran branché, chacune à sa propre échelle. Une
  fenêtre réduite dans le Dock garde le rectangle qu'elle avait. Il s'écrit à
  la construction de la fenêtre, une seconde après le dernier `Moved` ou
  `Resized`, et avant l'arrêt ordonné, qui attend cette écriture comme les
  autres écritures locales. Le Quit du Dock ne la fait pas : le dernier
  rectangle stabilisé fait foi.
- **L'appartenance vient de la webview** : `report_window_consoles` porte les
  documents des consoles de tous les workspaces de la fenêtre, affichés ou
  retenus, dans l'ordre des onglets, et celui du premier plan. Le front
  n'envoie qu'une liste changée, une fois par tour de rendu ; Rust n'écrit
  pas une liste inchangée. Rust écarte ce qu'une autre fenêtre écrit ou ce
  que sa ligne liste : les consoles d'une fenêtre restaurée lui appartiennent
  dès le lancement, avant toute sauvegarde, et les copies orphelines à la
  première fenêtre dès qu'elle les reçoit.
- **`restored_consoles`** rend les copies qu'une fenêtre rouvre : les siennes,
  dans l'ordre des onglets, puis — pour la première fenêtre du lancement, une
  fois — les copies ouvertes qu'aucune fenêtre ne réclame, lues dans la
  bibliothèque par pages de 200, seize au plus ; au-delà, elles restent dans
  la bibliothèque. Après une fermeture ordinaire, `routes/index.tsx` les
  confie à `restoreWorkingCopies`, sans question. Après un arrêt anormal,
  l'écran de reprise de chaque fenêtre ne propose qu'elles, et celles qui ne
  sont pas choisies quittent aussitôt la ligne de la fenêtre. Un rechargement
  de la webview les rend aussi : ce sont les consoles qu'elle tenait.
- **`recoveryOffered` et `objectPlaceDeclined` restent dans `session.ts`** :
  ce magasin appartient à la webview, donc déjà à la fenêtre. Les déplacer au
  registre n'aurait rien ajouté. Toutes les fenêtres construites au lancement
  voient l'arrêt anormal ; seule la première dit l'écriture interrompue.
- **`read_object_location` et `write_object_location` visent la fenêtre
  appelante.** La première fenêtre construite quand le fichier n'en tient
  aucune reprend la valeur des préférences ; le champ des préférences n'est
  plus écrit.
- **La ligne est retirée** quand une fenêtre non dernière se ferme, que ce
  soit par confirmation ou parce que sa webview ne répond pas. La sortie ne
  retire rien : c'est ce qu'elle doit rendre.
- **La politique refuse `WriteWindowLayout` à un agent**, dans `policy.rs`,
  à côté du refus des préférences. Le test est côté desktop
  (`backend/windows/layout/tests.rs`).

### Précisions de mise en œuvre, 2026-09-26 (lot 7, `Open in new window`)

- **Le déplacement est préparé avant la fenêtre.** `open_in_new_window`
  réserve la cible, lui transfère la console et ouvre sa session de
  catalogue, puis seulement construit la fenêtre : sa webview trouve le
  `ConsoleHandoff` dès qu'elle le demande (`take_console_handoff`, une fois).
  Une fenêtre qui ne se construit pas rend la console à la source, session et
  transaction comprises.
- **Session et document changent de fenêtre sous un seul verrou**
  (`WindowRegistry::hand_over`) : aucune commande ne voit la console à
  personne, ni aux deux. La source garde la connexion et son catalogue.
- **Le résultat** est compté pour la cible avant que la source ne démonte sa
  console, qui relâche alors sa propre vue. La cible relit les colonnes ; un
  résultat expiré entre-temps n'est pas montré, et sa vue est rendue.
- **Refus.** Le menu grise l'entrée, avec sa raison, pendant une exécution,
  une confirmation, un export ou une sauvegarde de la console, et sur une
  console hors ligne. Le backend refuse de son côté une console dont une
  instruction tourne encore, d'après l'état de transaction qu'il suit, et
  tout ce qu'une autre fenêtre possède. Une revue destructive ouverte tient
  son dialogue au premier plan : le menu d'onglet n'est pas atteignable.
- **La cible ne restaure rien d'autre** : ni écran de reprise, ni copies. Sa
  première console est la console déplacée, avec la mention « Moved from
  another window. Nothing was executed. »

## Conséquences

* **+** Deux connexions, ou deux consoles, se regardent côte à côte, sur deux
  écrans.
* **+** Aucune session n'est partagée entre fenêtres : ce que garantit
  ADR-0015 entre consoles vaut entre fenêtres, sans règle nouvelle.
* **+** La propriété est tenue à trois endroits qui se recoupent : le
  registre refuse une commande hors de sa fenêtre, le filtre Rust ne livre que
  ses événements, et le fichier refuse une console dans deux fenêtres.
* **+** La surface de la webview ne s'élargit pas : même trois permissions, et
  la création de fenêtre reste en Rust, bornée.
* **+** Fermer puis relancer rend les fenêtres à leur place, avec leurs
  consoles hors ligne ; c'est ce qui rend supportable une fermeture qui quitte.
* **+** Quitter ne jette plus en silence une transaction ouverte, sauf par le
  Dock, que macOS ne laisse pas retenir — et le journal le dit alors.
* **+** Aucun chemin d'arrêt nouveau : ceux d'ADR-0021, 0038 et 0040 restent
  les seuls, avec un vidage de brouillons de plus par fenêtre.
* **−** Presque toute la surface IPC change de signature : chaque commande qui
  vise une console, une session, une commande, un résultat ou une conversation
  reçoit la `Webview` appelante et subit un contrôle de propriété. C'est le gros
  du travail, et un oubli se voit mal : une commande sans contrôle marche,
  depuis n'importe quelle fenêtre.
* **−** Chaque fenêtre ouverte sur une connexion y consomme au moins **deux**
  sessions serveur — catalogue et première console. Sur un serveur proche de sa
  limite de connexions, la deuxième fenêtre peut échouer à se connecter là où
  la première réussissait.
* **−** Le coût mémoire d'une webview supplémentaire n'est pas mesuré ; la
  borne de 16 est une garde, pas une capacité démontrée.
* **−** Déplacer une console perd l'historique d'annulation de son éditeur.
* **−** La barre latérale et l'inspecteur restent des préférences communes :
  replier la barre dans une fenêtre ne replie pas les autres tout de suite, mais
  les suivantes s'ouvriront repliées. C'est la condition de reconsidération
  d'ADR-0013 qui commence à se réaliser.
* **−** Fermer une fenêtre ferme ses consoles, fermer la dernière les garde :
  une asymétrie à apprendre, la même que celle des navigateurs.
* **−** Sous macOS, Oxyn s'écarte de la convention de la plateforme.
* **−** `⌘Q` peut ne plus quitter : `Cancel` devant une transaction ouverte
  laisse l'application ouverte. Et un `Commit` en échec retient la sortie
  jusqu'à ce que l'utilisateur choisisse `Rollback` ou `Cancel`.
* **−** L'étape de transaction ajoute jusqu'à 2 s à la sortie quand une
  session tarde à rendre son état, et autant quand une webview n'accuse pas
  réception.
* **−** Sur PostgreSQL, rien n'est listé tant que le driver ne déclare pas
  `TRANSACTIONS` : l'étape ne sert aujourd'hui qu'à SQLite.
* **−** Un dialogue critique ouvert pour une fenêtre bloque ceux des autres
  jusqu'à sa fermeture — refus sans consommation, à redemander.
* **−** Une migration de plus, donc un format de plus à porter, et deux tables
  à valider comme toute entrée hostile.

**Coût de sortie :** revenir à une fenêtre unique demande de retirer le
registre (`backend/windows.rs`), les gestes et les abonnements par fenêtre, et
de rétablir le libellé fixe dans la capability. Le paramètre `Webview` ajouté
aux commandes peut rester, inerte. Ce qui ne se défait pas : la migration —
une version sans multi-fenêtre ignorerait les deux tables, sans les supprimer.
Le coût est borné par ce que le registre concentre : aucune crate du cœur ne
sait qu'il existe des fenêtres ([I-08](../../CLAUDE.md#i-08)) ; seules
`Command::WriteWindowLayout` et le type de disposition entrent dans `oxyn-core`
et `oxyn-store`, sans type Tauri.

**Reconsidérer si** l'usage réel dépasse 16 fenêtres ; si une fenêtre
supplémentaire au repos, mesurée, coûte plus de 150 Mio de mémoire résidente
(seuil de produit, pas une mesure) ; si le double coût en sessions serveur est
relevé comme un défaut, auquel cas un partage **explicite et visible** de la
session de catalogue serait à décider, jamais un partage silencieux ; ou si le
geste d'arrachage par glisser est demandé — il supposerait de vérifier ce que
les webviews des trois plateformes rapportent d'un glisser qui sort de la
fenêtre.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Fenêtre unique, onglets seulement | Ne répond pas au besoin posé par l'utilisateur le 2026-09-25 : deux connexions ou deux consoles visibles en même temps, sur deux écrans. Des onglets en partage d'écran dans une fenêtre ne s'étendent pas à un second moniteur |
| Un processus par fenêtre | Chaque processus aurait son backend : budgets de résultats ([ADR-0017](0017-retention-resultats.md)) et cache de catalogue (1 024 scopes, 50 000 objets) multipliés par le nombre de fenêtres ; aucune console déplaçable, une session ne franchissant pas un processus ; conflits de révision des préférences entre instances, déjà relevés en conséquence négative d'ADR-0013 ; un marqueur d'arrêt par processus ; une icône par processus sous macOS |
| Fenêtres secondaires en lecture seule | Deux sortes de fenêtres, donc deux versions de chaque geste. La lecture seule appartient à la connexion (`ConnectionConfig::read_only`) et se décide au `PolicyGate`, pas à une vue : une fenêtre « en lecture » qui propose `Explain` exécute `EXPLAIN ANALYZE`, donc la requête ([I-07](../../CLAUDE.md#i-07)) |
| Partager les sessions d'une connexion entre fenêtres | Rejeté entre consoles par [ADR-0015](0015-consoles-independantes.md) pour la même raison : une transaction ou un contexte de session décidés dans une vue changent l'autre. Partager la seule session de catalogue lierait en plus la durée de vie de deux fenêtres |
| Diffuser les événements à toutes les webviews (`emit`) et filtrer dans le front | Chaque fenêtre sérialiserait et recevrait le flux de toutes les autres ; le filtre, reproduit dans chaque webview, serait le seul rempart d'une propriété que le backend connaît déjà ; et `listen` exigerait une permission `core:event` que la capability n'accorde pas |
| Garder `main` pour la première fenêtre et un motif pour les suivantes | Deux régimes de nommage pour une seule sorte de fenêtre ; la première cesserait d'être restaurable comme les autres |
| Laisser le front créer ses fenêtres par l'API JavaScript de Tauri | Exige d'accorder à la webview une permission de création de fenêtre ou de webview : une XSS gagnerait une API, sans borne |
| Un greffon tiers de mémorisation des fenêtres | Il tiendrait sa propre persistance hors du fichier de workspace — une seconde source d'état, non versionnée avec le workspace, que `--temporary-workspace` ne contiendrait pas. *Comportement exact à vérifier dans ses sources avant de rouvrir la question* |
| Déplacer une console en la fermant puis en la rouvrant dans la cible | Ferme sa session : transaction et contexte de session perdus, résultat à réexécuter — exactement ce qu'[I-06](../../CLAUDE.md#i-06) et [I-13](../../CLAUDE.md#i-13) interdisent de faire en silence |
