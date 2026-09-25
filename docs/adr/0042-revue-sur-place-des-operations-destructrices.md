# ADR-0042 — Drop, Truncate et Rename s'exécutent depuis une revue sur place, comme du SQL utilisateur ordinaire

**Statut :** proposé · **Date :** 2026-09-25

**Précise :** [ADR-0025](0025-proposition-de-changement-de-schema.md), sur le
point suivant : un geste composé par Oxyn n'exécutait jamais rien. Pour les trois
entrées `Drop…`, `Truncate…` et `Rename…` du menu contextuel du catalogue, le
texte composé **peut** être exécuté, depuis une boîte de revue et sans passer
par une console. Le reste d'ADR-0025 reste en vigueur : `Propose change…`
compose toujours un modèle entièrement commenté, ouvert dans une console, et sa
portée exclut toujours `DROP TABLE`.

**Complète :** [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md),
sur un chemin d'approbation qu'il ne pouvait pas nommer : `run_object_operation`
approuve une commande retenue sans passer par `decide`. Quand l'approbation est
critique au sens de son § 1, famille 1 — une écriture ou un DDL sur une
connexion `production` —, elle passe par le même contrôle que `decide`, donc
par le dialogue natif, et la boîte de la webview n'en accorde aucune.

## Contexte

Le menu contextuel du catalogue (ADR-0041) porte `Rename…`, `Truncate…` et
`Drop…`. L'utilisateur a arrêté qu'ils font partie de la V1, et qu'ils
s'exécutent depuis une boîte de revue — le composant shadcn `alert-dialog` —
plutôt qu'en ouvrant une console. Par le trajet d'ADR-0025, un `DROP TABLE`
coûterait quatre gestes : ouvrir la console, décommenter, `Run`, approuver. Il
laisserait aussi derrière lui un document autosauvegardé pour une instruction
jouée une fois.

Ce que le code impose, relevé le 2026-09-25 :

* **La politique réclame déjà une approbation.** `DefaultPolicy::authorize`
  (`oxyn-core/src/policy.rs`) rend `RequireApproval` pour tout `MutationRisk`
  non nul, quel que soit l'acteur ou l'environnement. `oxyn-query` classe
  `DROP …` en `Ddl` + `MutationRisk::DropObject`, `TRUNCATE` en `Ddl` +
  `MutationRisk::Truncate`, et `ALTER TABLE … RENAME` en `Ddl` sans risque. Ce
  dernier est donc `Allow` pour un humain hors production, et soumis à approbation
  en production.
* **L'approbation a déjà son écran, et sa garantie est ailleurs.**
  `ApprovalDialog` (`apps/desktop/src/components/oxyn/approval-dialog.tsx`) est
  un `alert-dialog` : `Cancel` y a le focus initial et Entrée seule n'approuve
  pas. C'est l'écran d'une commande retenue dans la console et sur l'écran de
  connexion ; son accord appelle `decide`. Depuis
  [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md),
  `Backend::decide` appelle d'abord `confirm_held`
  (`crates/oxyn-desktop/src/backend/confirm.rs`) : pour une commande mutante
  d'un `Actor::Human` que le gate retient comme `production` au moment de
  l'approbation, rien ne s'exécute sans le dialogue natif, composé en Rust ;
  sinon `confirm_held` rend `NotCritical` et l'accord de la webview suffit.
  Ce qu'une XSS peut faire d'un bouton de la webview, elle peut le faire d'un
  appel direct à la commande Tauri : sur `production`, seule la confirmation
  dessinée par l'hôte fait garantie.
* **L'invalidation est déjà là.** Après un `Execute` réussi d'intention `Ddl`,
  l'exécuteur appelle `invalidate_catalog` puis publie `Event::CatalogUpdated`
  (`oxyn-exec/src/executor.rs`), et les abonnés
  d'[ADR-0022](0022-rafraichissement-automatique.md) relisent l'arbre. Rien de
  tout cela n'a lieu sur le chemin d'erreur.
* **Trois sessions possibles, aucune convenable telle quelle.** La session
  initiale est réservée au catalogue et à l'aperçu
  ([ADR-0015](0015-consoles-independantes.md)). Ses lectures d'introspection
  tiennent le verrou de session jusqu'à leur fin (`oxyn-exec/src/sessions.rs`) :
  un `DROP` qui y attendrait un verrou de table figerait tout l'arbre de la
  connexion jusqu'au délai de 30 s. La session d'une console porte la
  transaction éventuellement ouverte par l'utilisateur. Un `DROP` qui s'y
  joindrait serait défait par son `ROLLBACK`, ou tiendrait son verrou jusqu'à
  son `COMMIT`.
* **La citation existe, et un de ses styles ne cite pas.**
  `oxyn_catalog::quote_identifier` avec `QuoteStyle::Bare` rend le nom tel quel.
  `CatalogPath::qualify_sql` et `QuoteStyle::for_dialect` ne choisissent jamais
  ce style.
* **Les capacités ne disent pas ce que la boîte doit dire.** Aucun drapeau de
  `Capabilities` n'indique si le DDL est transactionnel, si le moteur refuse de
  supprimer un objet dont d'autres dépendent, ni si `TRUNCATE` existe.

Ce que les moteurs font, et la source de chaque fait :

| Fait | Source, date |
|---|---|
| PostgreSQL : `DROP TABLE` vaut `RESTRICT` par défaut, qui « refuse to drop the table if any objects depend on it » | [sql-droptable](https://www.postgresql.org/docs/current/sql-droptable.html), version 18, lu le 2026-09-25 |
| PostgreSQL : `TRUNCATE` refuse par défaut une table référencée par une clé étrangère, et « is transaction-safe » | [sql-truncate](https://www.postgresql.org/docs/current/sql-truncate.html), version 18, lu le 2026-09-25 |
| PostgreSQL : le DDL est transactionnel, sauf la création et la suppression de base ou de tablespace | [wiki PostgreSQL, Transactional DDL](https://wiki.postgresql.org/wiki/Transactional_DDL_in_PostgreSQL:_A_Competitive_Analysis), lu le 2026-09-25 |
| Redshift : `TRUNCATE` « commits the transaction in which it is run » ; `DROP TABLE` vaut `RESTRICT` par défaut | [r_TRUNCATE](https://docs.aws.amazon.com/redshift/latest/dg/r_TRUNCATE.html), [r_DROP_TABLE](https://docs.aws.amazon.com/redshift/latest/dg/r_DROP_TABLE.html), lus le 2026-09-25 |
| MySQL 8.4 : `DROP TABLE`, `TRUNCATE TABLE`, `RENAME TABLE` et `ALTER TABLE` provoquent une validation implicite | [implicit-commit](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html), lu le 2026-09-25 |
| SQLite : pas d'instruction `TRUNCATE` (`near "TRUNCATE": syntax error`) ; `DROP TABLE` réussit alors qu'une vue référence la table ; avec `foreign_keys` à `0`, il réussit aussi alors que des lignes filles la référencent ; `BEGIN; DROP TABLE t; ROLLBACK;` rend la table | constaté le 2026-09-25 avec le client `sqlite3` 3.51.0 ; le driver embarque SQLite 3.50.2 ([RESEARCH-NOTES](../RESEARCH-NOTES.md)) |
| Le driver SQLite ne pose jamais `PRAGMA foreign_keys` : les clés étrangères n'y sont pas vérifiées par défaut | `drivers/oxyn-driver-sqlite/src/session.rs`, en-tête du module |

## Décision

**La boîte compose, montre, puis soumet ce texte par le chemin de tout SQL
utilisateur : `Command::Execute`, sous `Actor::Human`, sur une session ouverte
pour elle. Aucune variante de `Command` n'est créée, et aucun chemin
d'exécution non plus.**

### Ce qui est composé, et où

`crates/oxyn-desktop/src/backend/object_operations.rs` porte
`Backend::review_object_operation(connection, address, operation)`. C'est une
lecture du cache de catalogue, sans I/O réseau ni `Command`. Elle rend un
`ObjectOperationReview` : le SQL, le nom et l'environnement de la connexion lus
dans sa `ConnectionConfig`, les capacités qui gouvernent le texte de la boîte,
et les dépendances connues. `operation` vaut `Drop`, `Truncate` ou
`Rename { new_name }`. Le front ne compose aucun SQL : chaque frappe dans le
champ du nouveau nom redemande la composition au backend.

* Le nom de l'objet est qualifié comme pour l'aperçu : schéma et table dans la
  base de la session pour PostgreSQL, convention du catalogue pour les bases
  attachées SQLite. Chaque segment passe par `quote_identifier` avec
  `QuoteStyle::for_dialect(oxyn_query::dialect_for(&config.driver))`, jamais
  `Bare` ([I-10](../../CLAUDE.md#i-10)). Le nouveau nom d'un `Rename` est cité
  de la même manière. La boîte montre le résultat cité, donc `Orders` s'affiche
  `"Orders"`.
* Le texte est **une seule instruction, sans commentaire**. Ce qui s'affiche est
  ce qui part, et ce qui s'inscrit dans `query_history`. L'en-tête commenté
  d'ADR-0025 servait un texte destiné à circuler vers un ticket. Ici, la trace
  est l'historique et le journal. Un commentaire partirait en outre vers le
  serveur et ses journaux, avec le nom local de la connexion.
* Les formes composées sont : `DROP TABLE|VIEW|MATERIALIZED VIEW <objet>` selon
  le `RelationKind`, `TRUNCATE TABLE <table>`,
  `ALTER TABLE <table> RENAME TO <nom>` et
  `ALTER TABLE <table> RENAME COLUMN <colonne> TO <nom>`. Aucun `IF EXISTS` :
  un objet déjà disparu doit échouer. Sinon, un cache périmé passerait pour un
  succès.
* **`CASCADE` n'est jamais dans le texte par défaut.** La boîte offre une case
  `CASCADE`, décochée à chaque ouverture et jamais mémorisée, seulement si la
  session déclare `RESTRICT_DEPENDENTS` (voir plus bas). La cocher fait
  recomposer le texte par le backend. Elle rend aussi la saisie du nom de
  l'objet obligatoire **dans tous les environnements**, car la portée réelle de
  la suppression n'est plus connue d'avance.
* Un nom portant un caractère de contrôle (catégorie Unicode `Cc`), U+2028,
  U+2029, ou un contrôle de direction (U+202A–U+202E, U+2066–U+2069) n'est pas
  composé. Échapper ces caractères, comme le fait `visible` dans
  `backend/proposal.rs`, désignerait un autre objet que celui qu'on exécute.
  L'entrée est grisée avec la raison
  `This name holds control characters: write the statement in a console.`

### Disponibilité : par capacité, jamais par nom de produit

Trois drapeaux s'ajoutent à `Capabilities` (`oxyn-core/src/capabilities.rs`),
dans la plage libre qui suit `PREVIEW_FILTER` (bits 44 à 46) :

| Drapeau | Ce qu'il affirme | Déclaré aujourd'hui par |
|---|---|---|
| `TRUNCATE` | la session accepte l'instruction `TRUNCATE` | PostgreSQL, Redshift |
| `TRANSACTIONAL_DDL` | toute instruction DDL acceptée, `TRUNCATE` compris, s'applique entière ou pas du tout, et obéit à la transaction qui l'entoure | PostgreSQL, SQLite ; **pas** Redshift, dont `TRUNCATE` valide la transaction |
| `RESTRICT_DEPENDENTS` | sans `CASCADE`, le moteur refuse `DROP` et `TRUNCATE` tant qu'un autre objet en dépend | PostgreSQL ; **pas** Redshift, dont `TRUNCATE` ignore les clés étrangères ; **pas** SQLite |

Un driver ne déclare aucun de ces drapeaux sans le test d'intégration qui le
prouve contre le moteur qu'il embarque ou qu'il atteint. Le tableau des sources
ci-dessus justifie la déclaration, mais ne remplace pas ce test.

| Entrée | Offerte si la session du catalogue déclare | Sinon, grisée avec |
|---|---|---|
| `Drop…` | `DDL`, sur une table, une vue ou une vue matérialisée | `This connection does not accept schema changes.` |
| `Truncate…` | `DDL` et `TRUNCATE`, sur une table | `This database has no TRUNCATE statement.` |
| `Rename…` | `DDL`, sur une table ou une colonne | `This connection does not accept schema changes.` |

Sur SQLite, cela donne `Drop…` et `Rename…` offerts, et `Truncate…` grisé. Une
connexion en lecture seule perd `DDL` et grise les trois entrées ; le
`PolicyGate` la refuserait de toute façon.

### Ce que la boîte montre

La boîte est un `alert-dialog`. Elle applique
[« Les opérations destructrices »](../UX-SPEC.md#les-opérations-destructrices) :
`Cancel` a le focus initial, Entrée seule ne valide rien, et le bouton par
défaut n'est jamais l'action. Elle montre :

1. **le SQL entier**, dans un bloc en lecture seule, sélectionnable ;
2. **la connexion et son environnement**, par `EnvironmentBadge`. Un
   environnement non renseigné s'affiche et se traite comme `production`
   ([I-02](../../CLAUDE.md#i-02),
   [SECURITY](../SECURITY.md#marquage-des-connexions)) ;
3. **en `production`**, un champ où taper le nom de l'objet, sans correction ni
   majuscule automatique. Le bouton d'action ne s'active que sur une égalité
   exacte, casse comprise, avec le nom non qualifié. C'est le nom actuel pour un
   `Rename`, et le nom de la colonne pour un renommage de colonne. **Cette
   saisie est une aide contre la méprise, pas une garantie** : elle oblige à
   lire le nom de l'objet visé par un clic dans l'arbre, rien de plus. Le
   backend ne la reçoit pas et ne la vérifie pas — un script qui appelle
   `run_object_operation` connaît le nom et le fournirait. Sur `production`,
   ce qui fait garantie est le dialogue natif qui suit la décision du gate
   (« Ce qui est exécuté ») ;
4. **les dépendances connues**, pour une table : les clés étrangères entrantes,
   lues par `refresh_relation_facet` avec `RelationFacet::IncomingKeys`, donc
   `RefreshCatalogScope::IncomingForeignKeys`, à l'ouverture de la boîte si le
   cache ne les a pas. Le bouton attend la fin de cette lecture. Une lecture en
   échec se dit, et ne se lit jamais comme « aucune ». Sans
   `INCOMING_FOREIGN_KEYS`, la boîte écrit
   `Dependents are not reported by this connection.` Dans tous les cas, elle
   écrit `Views, routines and triggers that use this object are not listed.` :
   aucune capacité ne les expose aujourd'hui ([ADR-0003](0003-driver-capabilities.md)) ;
5. **le filet du moteur**. Avec `RESTRICT_DEPENDENTS` :
   `Without CASCADE, the server refuses if other objects depend on it.` Sans :
   `This database drops the object even if other objects still use it.` ;
6. **le caractère transactionnel**. Avec `TRANSACTIONAL_DDL` :
   `Applied whole or not at all. Once it succeeds it is committed: there is no undo.`
   Sans : `This database does not run DDL in a transaction: it cannot be rolled back, and a failure may leave part of it applied.`

### Ce qui est exécuté, et sur quelle session

Le bouton d'action appelle une commande Tauri,
`run_object_operation(command_id, connection, operation, sql)`, qui délègue à
`Backend::run_object_operation`. Celle-ci fait, dans l'ordre :

1. elle ouvre une session **propre à la revue** par `Command::Connect`, la paire
   que `open_console` emploie déjà. Cette session est en autocommit, n'hérite
   d'aucun contexte de session ([ADR-0019](0019-contexte-de-session.md)) et
   d'aucune transaction ouverte. Un verrou attendu ne fige ni le catalogue ni une
   console ;
2. elle vérifie sur **cette** session les capacités requises (`DDL`, et
   `TRUNCATE` pour `Truncate`), parce que les capacités s'évaluent par session.
   Elle vérifie aussi que `oxyn_query::split` ne trouve qu'une instruction, et
   que sa classification correspond à l'opération annoncée : `DropObject` pour
   `Drop`, `Truncate` pour `Truncate`, `Ddl` sans risque pour `Rename`. Ce n'est
   pas une autorisation, qui reste au `PolicyGate`. Ce contrôle garantit que le
   libellé du bouton dit ce qui part ;
3. elle dispatche `Command::Execute { connection, session, request }` par
   `Backend::run`, donc `Executor::dispatch_as(id, Actor::Human, …)`. La requête
   est construite comme dans `Backend::execute` : `ExecRequest::new`, avec
   `limits.read_only = config.read_only`. À partir d'ici, rien ne diffère d'un
   `Run` de console. `oxyn-query` reclassifie le texte, le `PolicyGate` décide,
   `audit_journal` et `query_history` inscrivent la ligne, et le délai par
   défaut annule côté serveur ;
4. sur `NeedsApproval`, elle appelle `confirm_held`, la fonction même que
   `Backend::decide` appelle avant toute approbation
   ([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md), § 1
   et § 4). C'est elle, et non la boîte, qui dit si la décision est critique :
   * **`Confirmed`** — la connexion est `production` au sens du gate, et
     l'utilisateur a confirmé dans le dialogue natif, que le backend compose
     avec l'intention, le motif du gate et le texte de l'instruction. La
     commande est approuvée comme par `decide`, et rien ne repasse par la
     webview ;
   * **`Refused`** — Annuler, fermeture, confirmation dans la première
     seconde, échéance : la commande est rejetée et la décision consommée,
     comme par `decide` ;
   * **`NotCritical`** — hors `production`, la commande reste en attente et
     `run_object_operation` rend `NeedsApproval` à la boîte ;
   * **un autre dialogue critique est ouvert** — refus sans consommation
     (ADR-0037 § 3) : la commande reste en attente, `run_object_operation`
     rend `NeedsApproval` avec ce motif, et l'accord d'`ApprovalDialog`, qui
     passe par `decide`, redemandera le dialogue natif ;
5. elle ferme la session par `Command::CloseSession` dès l'issue terminale.
   Sur `NeedsApproval` rendu à la boîte, elle garde la session, indexée par le
   `CommandId`, et la ferme au retour de `decide`, que la commande soit
   approuvée, rejetée ou périmée.

**La boîte n'accorde aucune approbation.** Sur `NeedsApproval` rendu par le
backend — `Drop` et `Truncate` hors `production`, ou un dialogue critique déjà
ouvert —, elle cède la place à `ApprovalDialog` dans la même surface, sans
empiler une seconde modale. Elle y affiche la raison et la prévisualisation que
le gate a rendues sur le texte reclassifié. Une approbation donnée avant que le
gate ait parlé porterait sur une décision qui n'existe pas encore. Sur
`production`, c'est le backend qui enchaîne sur le dialogue natif, après le
gate : la webview n'a aucun appel qui approuve une commande critique.

Le trajet, par environnement :

| Opération | Hors `production` | `production` |
|---|---|---|
| `Rename` | revue, puis exécution : le gate rend `Allow` | revue et nom tapé, puis dialogue natif |
| `Drop`, `Truncate` | revue, puis `ApprovalDialog` | revue et nom tapé, puis dialogue natif |

Deux décisions dans tous les cas où le gate retient la commande, dont la
seconde, en `production`, est la seule qu'une XSS ne peut pas prendre à la
place de l'utilisateur.

Pendant la soumission, `Stop` atteint l'ouverture de session comme
l'instruction : le `CancelToken` du dispatch est celui de `Backend::track`.

### Après l'exécution

* **Succès.** L'invalidation et `CatalogUpdated` viennent de l'exécuteur, comme
  pour tout DDL ; la boîte n'ajoute aucun rafraîchissement. Elle se ferme, et
  après un `Drop` la sélection du catalogue passe au parent. Rien n'est
  optimiste ([UX-SPEC](../UX-SPEC.md#ce-qui-nest-jamais-optimiste)) : la boîte
  attend l'`Outcome` avant de rien retirer de l'arbre.
* **Erreur permanente.** Le message du serveur s'affiche tel quel. Pour un
  `DROP` refusé à cause de dépendances, la boîte propose
  `Open in console`, qui emprunte le trajet d'ADR-0025 avec le texte non
  commenté. C'est le seul endroit où l'utilisateur peut écrire `CASCADE`
  lui-même sans la case.
* **Erreur ambiguë ou annulation après envoi.** `OxynError::Timeout` et
  `OutcomeUnknown` relèvent d'`ErrorClass::Ambiguous`. La boîte écrit
  `The server may have applied this. Nothing will be retried. Refresh the catalog to see the current state.`
  Elle offre `Refresh catalog`, qui émet `RefreshCatalogScope` sur l'espace de
  noms parent, et aucun bouton de relance
  ([I-13](../../CLAUDE.md#i-13),
  [DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)).
  L'annulation obtenue par `Stop` pendant l'instruction reçoit le même texte.
  Un DDL annulé peut avoir été validé avant que l'annulation n'arrive.
* **Échec d'ouverture de la session.** Aucune instruction n'est partie. La boîte
  le dit et permet de soumettre à nouveau.

### Trace et provenance

La trace est celle de tout SQL exécuté : une ligne `audit_journal` avec
`Actor::Human` et la décision du gate, et une ligne `query_history` avec le
texte exact. **Aucune provenance** : aucun document n'est créé. Le texte, composé
par Oxyn, n'est écrit ni par l'utilisateur ni par un agent, et c'est la règle
qu'ADR-0025 applique à `Propose change…` et au modèle de requête liée de
`metadata.rs` ([ADR-0023](0023-fournisseurs-declares-et-provenance.md)).
`query_history` ne distingue pas une ligne venue de la boîte d'une ligne venue
d'une console. C'est assumé : l'utilisateur a relu et lancé ce texte.

### Jamais pour un agent

Les trois entrées sont des actions humaines du registre d'ADR-0041. Elles
n'apparaissent dans aucun menu ni aucune liste construite pour un
`Actor::Agent`. Aucun outil d'`oxyn-ai` n'atteint `review_object_operation` ni
`run_object_operation`. Un test `object_operations_are_not_tools`, voisin de
`le_changement_de_schema_n_est_pas_un_outil` dans `oxyn-ai/src/tools.rs`,
échoue le jour où un tel outil apparaît. Un agent qui veut supprimer une table
écrit du SQL comme n'importe qui. En production, le gate le **refuse**
([I-02](../../CLAUDE.md#i-02), [I-07](../../CLAUDE.md#i-07)). Ailleurs, il
exige une approbation. Cette décision ne change rien à cette politique.

### Plusieurs fenêtres

La boîte, sa session de revue et l'approbation qu'elle attend appartiennent à la
fenêtre dont le menu l'a ouverte (ADR-0043). Fermer cette fenêtre rejette
l'approbation en attente et ferme la session. Un dialogue natif resté à l'écran
— le plugin ne sait pas le fermer (ADR-0037 § 3) — n'approuve alors plus rien :
`confirm_held` ne confirme que la décision qu'il a montrée, et elle n'est plus
en attente. L'invalidation porte sur la connexion : toutes les fenêtres qui
l'affichent la reçoivent par `CatalogUpdated`.

## Conséquences

* **+** I-01 tient par construction. Le texte emprunte `Command::Execute`, et
  tout ce qui protège le SQL de l'utilisateur le protège : reclassification, gate,
  journal, historique, annulation, invalidation.
* **+** Un `DROP` coûte deux décisions, au lieu de quatre gestes et d'un
  document résiduel.
* **+** Sur `production`, l'approbation d'un `DROP`, d'un `TRUNCATE` ou d'un
  renommage n'a pas de chemin propre : c'est celui de toute écriture, par
  `confirm_held` et le dialogue natif d'ADR-0037. Ce qu'une XSS peut faire de
  la boîte se borne à ouvrir ce dialogue.
* **+** Ce que la boîte affirme vient de capacités déclarées et testées par
  driver, pas du nom du produit. Redshift, qui parle le protocole PostgreSQL,
  reçoit donc le texte qui lui correspond.
* **−** **Deux paliers** pour `Drop` et `Truncate`, même en local : la revue,
  puis l'approbation du gate. En production, il faut en plus taper le nom
  avant le dialogue natif. La seconde étape paraîtra redondante, et c'est elle
  que l'habitude finira par cliquer — en production, c'est pourtant la seule
  qui fasse garantie.
* **−** Deux écrans d'approbation différents selon l'environnement :
  `ApprovalDialog` hors `production`, le dialogue natif en `production`. Le
  second est pauvre — ni coloration, ni liste de dépendances — : ce que la
  revue montre ne s'y relit pas.
* **−** `run_object_operation` attend le dialogue natif, jusqu'à son échéance
  de cinq minutes, et garde sa session de revue ouverte pendant ce temps.
* **−** Chaque soumission ouvre une session, donc une poignée de main avec le
  serveur, TLS compris. Une session reste ouverte tant qu'une approbation attend,
  jusqu'au délai de 5 min du registre d'approbations.
* **−** La liste des dépendances est partielle : les clés étrangères entrantes,
  pas les vues ni les routines. Sur SQLite, dont le moteur ne refuse rien, cette
  liste partielle est le seul filet, et la boîte doit le dire.
* **−** Trois bits de `Capabilities` sont consommés. Comme tout drapeau, ils
  sont sérialisés et ne se renumérotent plus.
* **−** `query_history` ne sait pas qu'une ligne vient de la boîte. Retrouver
  « les suppressions faites depuis le menu » demande de lire le texte.

**Coût de sortie :** faible, et borné par l'absence de variante de commande. Le
composeur est une fonction pure testable sans base ni interface. La commande
Tauri se retire, la boîte aussi, et les trois entrées reviennent au trajet
d'ADR-0025. Seuls les trois drapeaux survivent à un retour arrière, parce qu'ils
sont sérialisés. Ils restent vrais et réutilisables.

**Reconsidérer si** un `Drop…` ou un `Truncate…` lancé depuis cette boîte est
rapporté comme ayant visé le mauvais objet ou la mauvaise connexion : la revue
n'aurait alors pas rempli son office. Reconsidérer aussi si une capacité
d'introspection des dépendances (vues, routines) apparaît dans un driver : la
case `CASCADE` pourrait alors annoncer ce qu'elle supprime, au lieu de dire
qu'elle ne le sait pas.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Ouvrir une console, comme ADR-0025 | Quatre gestes pour un `DROP`, un modèle à décommenter pour une instruction évidente, et un document autosauvegardé qui reste dans la bibliothèque. C'est la lourdeur qu'ADR-0025 pose comme condition de reconsidération |
| Exécuter directement au clic, sans revue | Hors production, `Rename` est `Allow` : un clic manqué renommerait sans rien montrer. Pour `Drop`, `ApprovalDialog` seul ne montre ni les dépendances, ni le caractère transactionnel, ni la saisie du nom |
| Une commande dédiée, `Command::DropObject { path }` hors SQL | Un second chemin d'exécution ([I-01](../../CLAUDE.md#i-01)) : le gate aurait besoin d'une règle à part, `oxyn-query` ne classerait rien, l'historique n'aurait pas de SQL, et le driver composerait du DDL, ce qu'ADR-0025 a déjà écarté. Une telle commande se mappe aussi en un outil d'agent en une ligne |
| Exécuter sur la session du catalogue | Réservée au catalogue et à l'aperçu ([ADR-0015](0015-consoles-independantes.md)). Un `DROP` qui attend un verrou y fige l'arbre de la connexion pendant 30 s, puis finit en erreur ambiguë |
| Exécuter sur la session d'une console | Elle porte la transaction de l'utilisateur. Le `DROP` serait défait par son `ROLLBACK`, ou tiendrait son verrou jusqu'à son `COMMIT` |
| Composer le SQL dans le front | Une seconde implémentation de la citation, en TypeScript, hors des tests d'`oxyn-catalog` ([I-10](../../CLAUDE.md#i-10)) |
| La boîte appelle elle-même `decide(true)` après sa propre confirmation | Elle approuverait avant que le gate ait rendu sa décision et sa prévisualisation. Et un appel à `decide` hors `ApprovalDialog` crée le précédent qu'un flux automatisé réemprunterait |
| En `production`, céder aussi à `ApprovalDialog` avant le dialogue natif | Trois confirmations pour un `DROP` : la revue, un écran qui en répète le contenu, puis le dialogue. L'écran du milieu n'ajoute aucune garantie — la webview ne décide rien sur `production` — et nourrit la lassitude qu'[ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md) nomme comme son premier coût |
| Retirer la saisie du nom en `production` | Le dialogue natif nomme la connexion et cite l'instruction, mais se ferme d'un clic ; la saisie oblige à lire le nom de l'objet avant, là où la méprise se commet — un clic sur la mauvaise ligne de l'arbre. Elle est gardée pour cela, et seulement pour cela |
| Faire vérifier par le backend le nom tapé | Un script qui appelle `run_object_operation` connaît le nom et le fournirait : la vérification aurait l'air d'une garantie sans en être une |
| Cocher `CASCADE` d'office quand des dépendances sont connues | Les dépendances connues ne sont qu'une partie de ce que `CASCADE` supprime. Le pré-cocher, c'est décider à la place de l'utilisateur d'une portée que personne n'a vue |
| Ajouter `IF EXISTS` | Un objet déjà supprimé ailleurs passerait pour un succès, et masquerait que le catalogue affiché était périmé |
| Griser les entrées par nom de produit, par exemple « SQLite : renommage seul » | Contraire à [ADR-0003](0003-driver-capabilities.md). SQLite a `DROP TABLE`, et Redshift diffère de PostgreSQL sur `TRUNCATE` malgré le même protocole |
