<!-- oxyn-translation source="docs/adr/0069-turso-driver-hrana-over-http.md" sha256="410e8fda9ffd" -->

> Traduction française de [docs/adr/0069-turso-driver-hrana-over-http.md](../../../../docs/adr/0069-turso-driver-hrana-over-http.md). **La version anglaise fait foi.**

# ADR-0069 — Le driver Turso Cloud parle lui-même Hrana sur HTTP, ne garde aucun flux entre deux instructions, et lit une réponse entière sous un budget

**Statut :** proposé · **Date :** 2026-11-04

## Contexte

Turso Cloud héberge des bases libSQL, un fork de SQLite, derrière une API HTTP.
Les décisions du mainteneur sur la proposition (issue #350) sont : on
l'interroge en HTTP (`/v2/pipeline`) plutôt qu'en l'ouvrant comme un fichier,
donc il lui faut un driver à lui ; le dialecte SQL reste celui de SQLite ; et
une connexion est l'URL de la base plus un jeton **de base** (`turso db tokens
create <db>`), pas le jeton de l'API Platform. Ce qui est coûteux à défaire,
c'est le protocole sur le fil, le fait de lier ou non une bibliothèque, ce
qu'est une « session » sur un protocole sans connexion, et ce qu'on fait d'un
résultat que le serveur envoie en entier. Vérifié le 2026-11-04
([RESEARCH-NOTES](../RESEARCH-NOTES.md#turso-cloud-et-hrana-sur-http--vérifié-le-2026-11-04)) :

* **Hrana sur HTTP.** `POST <URL de la base, en https>/v2/pipeline`, en-tête
  `Authorization: Bearer <jeton>`, un corps JSON `{"requests": [...]}` dont
  chaque requête est `execute` (une instruction et des `args` typés) ou
  `close`, et un `baton` qui rattache des requêtes à un *flux*. Les valeurs sont
  typées : `null`, `integer` (une **chaîne**, pour la précision sur 64 bits),
  `float`, `text`, `blob` (base64). Un `execute` prend **une** instruction.
  Une erreur d'instruction est une entrée `error` de `results`, avec un
  `message` et un `code` que la spécification dit « pas encore stabilisés » ;
  le corps d'une erreur HTTP 4xx/5xx peut être n'importe quoi.
* **Un flux est une connexion.** Il porte l'état de transaction. Turso en
  ferme un après 10 secondes sans requête, et laisse 5 secondes à une
  transaction pour se terminer. La spécification n'a aucun appel qui
  interrompt une instruction en cours.
* **La réponse est un seul document.** `execute` renvoie `cols`, **toutes** les
  `rows`, `affected_row_count`, `query_duration_ms`. La spécification définit
  aussi un point d'accès `cursor` en flux (`/v3/cursor`) ; la référence de
  Turso ne documente que `/v2/pipeline`.
* **Les bibliothèques.** `libsql` lie un moteur C, `libsqlite3-sys`, dont le
  graphe tient une seule version ([ADR-0010](0010-contraintes-natives-sqlite.md)) ;
  le client HTTP serait le `reqwest` du workspace.
* **Les jetons.** Un jeton de base peut être créé `--read-only` et avec une
  expiration (`turso db tokens create`).

## Décision

1. **Une crate à part, `oxyn-driver-turso`, id `turso`, sur le dialecte de
   SQLite.** Le protocole est celui d'un serveur, pas d'un fichier
   ([ADR-0003](0003-driver-capabilities.md) : un driver par protocole), donc ce
   ne peut pas être une capacité du driver SQLite. Il parle Hrana lui-même et ne
   dépend **ni de `libsql` ni de `libsqlite3-sys`** ; `dialect_for` associe
   `turso` à `SqlDialect::Sqlite`, si bien que la classification, le découpage,
   la citation et l'invite de dialecte de l'IA sont ceux de SQLite. Rien n'est
   partagé avec le code du driver SQLite : la table des types logiques et la
   composition de l'aperçu sont copiées, avec un commentaire qui le dit, un
   driver ne pouvant pas dépendre d'un autre.
2. **Le `reqwest` du workspace, du JSON, aucune dépendance nouvelle.** Pas de
   proxy, pas de redirection (l'en-tête `Authorization` la suivrait), `https`
   seulement, TLS toujours vérifié ([ADR-0052](0052-verified-tls-outside-local.md)) :
   il n'y a pas de `sslmode` ni de moyen de désactiver la vérification. Le
   base64 tient en quarante lignes dans la crate. Le point d'accès est
   `/v2/pipeline`, celui que Turso documente.
3. **Deux champs de formulaire.** `url`, conservée avec la connexion, accepte
   exactement `https://<hôte>[:port]` et `libsql://<hôte>[:port]` (lue comme
   `https://`) ; un utilisateur, un mot de passe, un chemin, une requête, un
   fragment, tout autre schéma (`http://`, `ws://`, `file:`) est refusé avec
   **un texte fixe qui ne répète jamais l'entrée**, puisqu'un `?authToken=…`
   collé est un secret. `token` est un `FieldKind::Password` : il vit dans
   l'emplacement « jeton » du trousseau et jamais dans le fichier de workspace
   ([I-03](../../CLAUDE.md#i-03)) ; le driver le lit dans `Credentials::token`,
   l'envoie dans un en-tête `Authorization` sensible, le retire de chaque
   message du serveur, et n'a aucun `Debug` qui le montre. L'aide du formulaire
   dit que c'est le jeton de la base et non celui de l'API Platform.
4. **Une instruction, un flux, fermé dans la même requête.** Chaque instruction
   est un pipeline `execute` puis `close`, sans `baton`. La session ne garde
   rien entre deux instructions : rien de ce qu'une instruction fait à une
   connexion ne lui survit, et les instructions qui le feraient sont **refusées
   avant l'envoi, avec un texte fixe qui nomme la sortie** : `BEGIN`, `COMMIT`,
   `END`, `ROLLBACK`, `SAVEPOINT`, `RELEASE` ; `PRAGMA nom = valeur` ;
   `ATTACH`, `DETACH` ; et ce que l'exécuteur signale comme un changement de
   contexte. Un `PRAGMA` qui lit (`PRAGMA table_info(t)`) passe.
   `TRANSACTIONS`, `SAVEPOINTS`, `MULTIPLE_STATEMENTS` et `SESSION_CONTEXT` ne
   sont pas déclarées. Garder un flux ouvert n'est pas une option que le
   protocole offre à une console : 10 secondes d'inactivité le fermeraient sous
   les pieds de l'utilisateur.
5. **Une réponse entière est lue sous un budget de 32 Mio** de JSON
   (`MAX_RESPONSE_BYTES`) : une longueur annoncée au-delà est refusée avant le
   premier octet lu, le corps est lu morceau par morceau, et la requête est
   abandonnée dès que le budget est franchi ([I-06](../../CLAUDE.md#i-06)). Le
   refus est une erreur `Query` permanente qui renvoie à `LIMIT`. `max_rows` est
   tenu par le driver après réception (le serveur envoie quand même toutes les
   lignes) et marque le résultat tronqué. Les lots Arrow sont construits à partir
   des lignes lues à mesure que le curseur est vidé, fermés à 1 Mio de valeurs
   ou 8 192 lignes.
6. **Les types de colonnes viennent de toutes les valeurs du résultat.** La
   réponse étant entière, le type d'une colonne est décidé avant le premier lot
   à partir de toutes ses valeurs, pas du premier lot : entiers → `Int64`,
   flottants → `Float64`, entiers mêlés à des flottants → `Utf8` (aucun `f64` ne
   tient tous les `i64`), texte → `Utf8`, un blob avec n'importe quoi →
   `Binary` ; une colonne sans valeur suit son affinité déclarée. Le type
   déclaré, l'inférence et les classes trouvées sont gardés dans les métadonnées
   du champ. Un `DATETIME` n'est ni converti ni muni d'un fuseau.
7. **Un arrêt abandonne la requête et promet un verdict, pas une
   interruption.** Hrana sur HTTP ne peut pas interrompre une instruction : donc
   `SERVER_SIDE_CANCEL` **n'est pas déclarée** et l'exécuteur n'appelle pas
   `Session::cancel` sur ce driver ; le jeton de l'exécution abandonne la
   requête. Une lecture arrêtée est `Cancelled` ; une instruction qui peut avoir
   écrit — une intention inconnue compte — est `OutcomeUnknown`, jamais
   `Cancelled` ([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)).
   Une requête jamais envoyée est `Cancelled` dans les deux cas. L'échéance
   d'une instruction qui expire est `Timeout`, qui est ambiguë.
8. **Les erreurs sont classées d'après l'endroit où elles surviennent,
   l'intention de l'instruction et les mots du serveur.** Le tableau est dans le
   module `error` et testé. En bref : une connexion jamais établie est
   transitoire pour toute instruction ; un échange rompu, un HTTP 5xx/408/429 ou
   une réponse qui n'est pas du Hrana sont transitoires ou permanents pour une
   lecture et `OutcomeUnknown` pour tout ce qui peut écrire ; 401 et 403 sont
   `Authentication` en nommant le jeton de base ; 404 est `Config` ; une erreur
   d'instruction est permanente (SQLite a défait l'instruction en entier), sauf
   un verrou, transitoire pour une lecture seulement (derrière un primaire, rien
   ne prouve qu'une écriture verrouillée a été défaite : elle est ambiguë). Un
   corps qui se rompt en vol est un échange raté, pas une réponse étrangère. Les codes sont ceux
   du serveur et « pas encore stabilisés » : un code ne décide donc que là où il
   existe, et un message seulement par phrases entières (une colonne nommée
   `locked` n'est pas un verrou). Le driver ne retente rien
   ([I-13](../../CLAUDE.md#i-13)).
9. **Le mode lecture seule est la classification seule.** Le serveur n'est pas
   prié de l'imposer : `READ_ONLY_SESSION` n'est donc pas déclarée, et le driver
   refuse une requête modifiante qui porte une borne en lecture seule. Pour que
   Turso l'impose, il faut un jeton créé avec `--read-only` ; le formulaire ne le
   détecte pas.
10. **Le catalogue lit `sqlite_master` et les fonctions-tables `pragma_*`.**
    Chaque instruction est une constante ; le nom d'une relation est un argument
    typé (`pragma_table_info(?1)`), jamais dans le texte
    ([I-10](../../CLAUDE.md#i-10)). Un seul espace de noms, `main` ; tables et
    vues ; colonnes avec leur rang de clé ; index (une jointure, pas d'aller-retour
    par index) ; clés étrangères dont la cible implicite est résolue vers la clé
    primaire. Un aperçu arrive dans l'ordre de la clé pour une table rowid
    ([ADR-0068](0068-a-preview-comes-in-key-order.md)) et pas pour une table
    virtuelle. Pas encore lus, donc pas déclarés : contraintes, clés étrangères
    entrantes, définitions d'objets, prédicat d'un index partiel, `TRUNCATE`.
11. **Les valeurs liées sont des `args` typés.** La n-ième valeur se lie au
    n-ième paramètre positionnel ; les conversions sont celles de SQLite (un
    décimal reste du texte, un instant est en RFC 3339 UTC, un intervalle ou un
    tableau est refusé en nommant son rang, jamais sa valeur).
    `PREPARED_STATEMENTS` est déclarée pour cette promesse.

## Conséquences

* **+** Pas de second moteur SQLite ni de dépendance : le graphe reste ce qu'il
  était, plus une crate qui utilise le `reqwest`, l'`arrow` et le `serde_json`
  du workspace.
* **+** Le jeton n'a qu'un chemin : trousseau, en-tête, nulle part ailleurs, et
  les tests le vérifient face à un serveur qui renvoie l'en-tête en écho.
* **+** Rien de ce qu'une instruction fait ne survit à sa requête : une
  reconnexion, une nouvelle tentative de l'appelant ou une seconde console ne
  peut pas rencontrer un réglage ou une transaction que l'utilisateur a oublié.
* **+** La suite tourne face à un serveur local qui parle Hrana : pas de compte,
  pas de secret, aucune attente sur l'horloge réelle.
* **−** **Pas de transaction, pas de réglage `PRAGMA`, pas d'`ATTACH`** dans une
  console. Un utilisateur qui a besoin que plusieurs instructions soient
  atomiques ne peut pas l'obtenir d'Oxyn aujourd'hui.
* **−** **Un arrêt n'arrête pas l'instruction.** La requête est abandonnée ;
  Turso peut la terminer. Le verdict est honnête, l'effet n'est pas une
  annulation, et une écriture arrêtée doit être vérifiée par l'utilisateur.
* **−** **Un gros résultat est refusé, pas tronqué.** Un `SELECT * FROM events`
  sans `LIMIT` sur une grande table échoue en renvoyant vers `LIMIT` là où le
  driver SQLite afficherait 10 000 lignes, parce que la réponse est un seul
  document. Sous le budget, la mémoire est de 100 à 300 Mio à la limite (les
  lignes lues pèsent 3 à 9 fois le JSON), hors de la comptabilité du
  `ResultBuffer`.
* **−** Le driver ne connaît pas les vrais codes d'erreur du serveur : ils sont
  instables et n'ont pu être vérifiés sans compte. Un code non reconnu est une
  erreur `Query` permanente, pour une écriture aussi, ce qui repose sur
  l'hypothèse qu'une instruction qui a échoué en autocommit n'a rien appliqué.
  Cette hypothèse n'est pas vérifiée, notamment derrière le renvoi des
  écritures vers un primaire ; là où elle est fausse, l'utilisateur lit
  « refusé » pour une instruction qui a pris effet.
* **−** Le catalogue suppose que les fonctions `pragma_*` sont appelables sur
  Turso Cloud, ce qui n'a **pas été vérifié** (pas de compte). Sinon chaque
  introspection échoue visiblement et l'arbre reste vide.
* **−** Deux copies de la table des types logiques et de la composition de
  l'aperçu (ici et dans le driver SQLite).

**Coût de sortie :** la crate est autonome : `wire`, `http` et `convert` sont le
protocole et les types, `catalog` et `preview` le SQL. Passer au point d'accès
`cursor` en flux réécrit `http` et la tête de `convert`, pas la session, le
catalogue ni le desktop, qui ne voient que des `RecordBatch`. Garder un flux
(transactions) serait additif : une capacité déclarée, les instructions refusées
aujourd'hui alors acceptées.

**À reconsidérer si** Turso documente un moyen d'interrompre une instruction
(alors déclarer `SERVER_SIDE_CANCEL`), documente `/v3/cursor` comme pris en
charge sur Cloud (alors lire en flux, supprimer le refus à 32 Mio et faire de
`max_rows` une lecture arrêtée tôt), laisse un flux vivre assez longtemps pour
une transaction interactive, ou si une exécution vérifiée sur une vraie base
montre que l'introspection ou les codes d'erreur se comportent autrement que
supposé ici.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| La crate `libsql` (mode distant) | lie un SQLite en C et son `libsqlite3-sys`, dont le graphe tient une seule version ([ADR-0010](0010-contraintes-natives-sqlite.md)) ; un client de plus avec sa propre pile TLS sur la frontière la plus large |
| Une capacité du driver SQLite | le protocole est celui d'un serveur : pas de fichier, pas de `sqlite3_interrupt`, pas de `rusqlite` ; le thread de travail du driver SQLite n'a rien à offrir à un protocole requête/réponse |
| Garder un `baton` par console | Turso ferme un flux après 10 secondes d'inactivité et une transaction après 5 ; l'instruction suivante tournerait sur un autre flux avec une transaction disparue en silence — la défaillance que le contrat des drivers appelle un `SET` invisible |
| Accepter `http://` pour une base locale | le jeton circulerait en clair ; `sqld` sur un portable est un autre produit que Turso Cloud, et pourra revenir comme option explicite |
| Envelopper le `SELECT` de l'utilisateur dans un `LIMIT` pour borner la réponse | « envoyé tel quel, ou refusé — jamais réécrit » ([DRIVER-CONTRACT §6](../DRIVER-CONTRACT.md#6-il-échappe-tout-identifiant-quil-compose)) |
| `PRAGMA query_only = 1` avant chaque lecture, pour faire imposer la lecture seule par le serveur | ne marche que si Turso autorise le pragma, ce qui n'a pu être vérifié ; une session en lecture seule qui fait échouer chaque lecture de production est pire que la classification seule |
| Le jeton de l'API Platform | il administre l'organisation et n'ouvre aucune base ; décision du mainteneur |
| Déclarer `STREAMING` | la réponse est un seul document lu en entier ; le drapeau dirait le contraire du budget |
