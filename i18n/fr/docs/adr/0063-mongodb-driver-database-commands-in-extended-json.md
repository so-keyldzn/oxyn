<!-- oxyn-translation source="docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md" sha256="3f2fe7b7fa08" -->

> Traduction française de [docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md](../../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md). **La version anglaise fait foi.**

# ADR-0063 — Driver MongoDB : des commandes de base en Extended JSON, des documents vers un schéma Arrow déclaré et sans perte

**Statut :** accepté · **Date :** 2026-10-23

**Précise :** [ADR-0003](0003-driver-capabilities.md), sur le point suivant :
une base documentaire reçoit sa propre crate (OP_MSG et BSON ne partagent rien
avec les drivers déjà écrits), et son langage de requête est
`QueryLanguage::MongoQuery`, défini ci-dessous comme des commandes de base, pas
comme un shell JavaScript.

## Contexte

MongoDB est la première base documentaire à laquelle Oxyn parle. Quatre
propriétés du produit rendent fausse chaque habitude SQL des drivers existants
([RESEARCH-NOTES](../RESEARCH-NOTES.md#bibliothèque-cliente-et-serveur-mongodb--vérifié-le-2026-10-23),
vérifié le 2026-10-23) :

* **Il n'y a pas de langage de requête textuel sur le fil.** Le serveur reçoit
  des documents de commande BSON (`{find: "users", filter: {…}}`) dans des
  messages OP_MSG. Ce que les utilisateurs appellent « le langage de requête de
  MongoDB » est l'API JavaScript de `mongosh`, un programme Node.js qui
  construit ces commandes. L'exécuter, c'est embarquer un moteur JavaScript et
  la bibliothèque du shell — un second environnement d'exécution dont chaque
  assistant peut écrire.
* **Les documents ne déclarent aucune colonne.** Deux documents d'une même
  collection peuvent ne partager aucun champ, ou porter un `Int32` dans l'un et
  un sous-document dans l'autre sous la même clé. Un `RecordBatch` a un seul
  schéma ([ADR-0002](0002-arrow-result-model.md)).
* **Les documents sont gros et imbriqués.** Un document peut peser 16 Mio ; un
  lot `getMore` n'a pas de limite par défaut hormis les 16 Mio d'un message. Un
  lot compté en documents n'est pas borné en octets ([I-06](../../../../CLAUDE.md#i-06)).
* **Le client officiel retente.** `mongodb` (le driver Rust, Apache-2.0)
  retente par défaut une lecture et une écriture une fois ; une écriture
  retentée sans le numéro de transaction que porte une écriture réessayable
  peut s'appliquer deux fois ([I-13](../../../../CLAUDE.md#i-13)).

La chaîne de connexion `mongodb://user:password@host/` porte le mot de passe,
et un nom `mongodb+srv://` demande des résolutions DNS avant d'atteindre le
moindre serveur.

## Décision

### Crate et client

`drivers/oxyn-driver-mongodb`, sur `mongodb` `=3.9.1` et `bson` `3.1.0`, avec
`default-features = false` et exactement `compat-3-3-0`, `bson-3`,
`rustls-tls-aws-lc` (rustls sur aws-lc-rs, le fournisseur du workspace) et
`dns-resolver` (listes de graines SRV). Ni compression, ni authentification
AWS, Kerberos ou OIDC, ni chiffrement côté client : rien de cela n'est testé.
Le client accepte les serveurs de version de protocole 9 à 29, MongoDB 4.4 et
suivants ; les tests et la CI tournent sur `mongo:8.0.34`. DocumentDB, Cosmos
DB pour MongoDB et FerretDB ne sont **pas revendiqués** : ils seraient des
capacités de cette crate une fois testés, jamais d'autres crates.

### Connexion : des champs, jamais une chaîne de connexion

Le formulaire prend les parties — hôtes (une liste de graines), port, `srv`,
base, utilisateur, mot de passe, `authSource`, `authMechanism` (`default`,
`SCRAM-SHA-256`, `SCRAM-SHA-1`), `replicaSet`, `directConnection`, `tls`,
`tlsCAFile`, `tlsCertificateKeyFile` — et le mot de passe va au trousseau
comme pour tout driver ([I-03](../../../../CLAUDE.md#i-03)). Les options du client
sont construites **par programme** : aucune chaîne contenant le mot de passe
n'est jamais composée. La seule chaîne composée est `mongodb+srv://<nom>/`, à
partir d'un nom vérifié comme nom DNS, pour que rien en lui ne puisse ouvrir
une partie utilisateur, chemin ou option. Un paramètre inconnu est refusé
plutôt qu'ignoré. Le TLS suit l'[ADR-0052](0052-verified-tls-outside-local.md) :
`verify-full` partout sauf sur une connexion marquée Local. `ConnectSpec` a un
`Debug` écrit à la main.

`retry_reads` et `retry_writes` sont **coupés** : la politique de nouvelle
tentative appartient à l'appelant ([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md)).
La connexion exécute `buildInfo` (version), un `killSessions` qui ne nomme
personne (cette session peut-elle arrêter ses commandes ?) et
`connectionStatus` avec les privilèges (ce compte peut-il écrire ?), chacun en
course avec le jeton d'annulation.

### La console : une commande de base, en Extended JSON

Une requête `MongoQuery` est **un document JSON, la commande que le serveur
exécute**, en Extended JSON v2 de MongoDB, canonique ou relâché :
`{"find": "users", "filter": {"age": {"$gt": 30}}, "limit": 100}`,
`{"aggregate": "orders", "pipeline": […], "cursor": {}}`. Une console en
contient plusieurs, l'une après l'autre ; Exécuter prend l'objet de premier
niveau sous le curseur (`oxyn_query::current_request`). Elle part **telle
qu'écrite** — c'est la fonctionnalité — avec trois règles :

* le nom est la **première clé**, si bien que le texte est lu en arbre
  ordonné ; une clé répétée dans un objet est refusée, car deux lecteurs
  gardant des copies différentes, c'est ainsi qu'une lecture classée devient
  une écriture envoyée ;
* un enrobage de type est un objet dont les clés sont **exactement** celles de
  l'enrobage (`{"$oid": …}`) ; tout autre objet est un document, clés en `$`
  comprises. Un littéral entier au-delà de 64 bits est refusé, jamais arrondi
  en double. DBPointer, déprécié, n'est pas accepté en retour ;
* `$db` est lu comme la base cible ; les champs d'enveloppe que le driver
  possède — `lsid`, `txnNumber`, `autocommit`, `startTransaction`,
  `$clusterTime`, `$readPreference` — sont refusés, parce qu'ils sortiraient
  la commande de la session que l'annulation tue. Les paramètres liés sont
  refusés : une commande porte ses valeurs dans son propre document.

Les noms de bases et de collections sont des **valeurs** chaîne BSON, jamais
du texte qu'Oxyn colle dans une commande ([I-10](../../../../CLAUDE.md#i-10)) : un
aperçu composé est construit en BSON puis sérialisé.

### Classification : une table de noms de commandes, lue par la session

`oxyn_query::classify_mongo` lit le même arbre contre des tables construites à
partir de la référence des commandes : lectures (`find`, `aggregate`, `count`,
`distinct`, `listCollections`, `listIndexes`, `dbStats`, `collStats`,
`usersInfo`, `currentOp`…), écritures (`insert`, `update`, `delete`,
`findAndModify`, `bulkWrite`), DDL (`create`, `drop`, `createIndexes`,
`collMod`, `renameCollection`…), droits (commandes d'utilisateurs et de
rôles). Un `aggregate` ou un `explain` qui contient `$out` ou `$merge` à
n'importe quelle profondeur écrit. `$where`, `$function` et `$accumulator`
exécutent du JavaScript sur le serveur : `Unknown`. Un nom que les tables ne
contiennent pas est `Unknown`, donc mutant.

L'exécuteur y accède par `Session::classify_intent`
([ADR-0064](0064-a-session-classifies-non-sql-requests.md)) : le
`classify_language` d'`oxyn-query` garde `Unknown` pour MongoDB, et la session
répond avec la lecture que son `execute` applique — `None` pour `Unknown` et
pour les verbes de transaction qu'elle refuse. Un seul lecteur décide de la
production.

### Résultats : un échantillon déclaré, une colonne sans perte

Les premiers documents d'un résultat — au plus 1 000 et environ 8 Mio — sont
lus avant le retour d'`execute` ; le schéma en est inféré. Chaque colonne de
champ porte `oxyn:inferred` et son type BSON (`oxyn:bson_type`), le schéma la
taille de l'échantillon. Un champ que l'échantillon a vu avec deux types ou
plus est une colonne texte **mixte** en Extended JSON relâché, jamais
convertie. La **dernière colonne, `$document`, porte chaque document entier en
Extended JSON canonique** : un champ hors échantillon, au-delà du plafond de
1 024 colonnes, ou répété dans un document n'est jamais perdu. Une valeur
rencontrée après l'échantillon avec un type que sa colonne ne porte pas termine
le flux par une erreur qui nomme le champ et les deux types, après les lignes
qui la précèdent.

| BSON | Arrow | Perte |
|---|---|---|
| Double | `Float64` | aucune ; NaN et ±∞ gardés |
| String | `Utf8` | aucune ; un UTF-8 invalide termine le flux |
| Int32 / Int64 | `Int32` / `Int64` | aucune ; exact au-delà de 2^53 |
| Boolean | `Boolean` | aucune |
| Date | `Timestamp(ms, "UTC")` | aucune : des millisecondes UTC par définition |
| ObjectId | `Utf8`, 24 chiffres hexadécimaux | aucune |
| Decimal128 | `Utf8`, le texte du décimal | aucune ; jamais un double |
| Binary | `Binary`, sous-type dans `oxyn:bson_subtype` | aucune ; un UUID est le sous-type 4 |
| Timestamp, Regex, JavaScript, Symbol, DBPointer, MinKey, MaxKey, Undefined, document, tableau | `Utf8`, Extended JSON relâché | le type est dans l'enrobage et dans la colonne canonique ; un Timestamp n'est jamais une date |
| type d'élément inconnu | — | le flux se termine en nommant l'octet de type : BSON ne lui donne aucune longueur |

### Flux et bornes

Le client tient un lot serveur (au plus 16 Mio) et un lot Arrow d'au plus
1 Mio ou 8 192 lignes ; rien ne collecte un curseur. `maxTimeMS` et
`batchSize` sont à l'utilisateur ; la limite de 16 Mio par document et la
limite mémoire de 100 Mo par étape du serveur reviennent comme leurs erreurs
(`BSONObjectTooLarge`, `QueryExceededMemoryLimitNoDiskUseAllowed`), remède
ajouté.

### Annulation : une session logique par exécution, `killSessions`

Chaque exécution démarre sa propre session logique, cohérence causale coupée.
`Session::cancel` et le jeton d'annulation envoient `killSessions` avec son
identifiant : le serveur interrompt ses opérations et ferme ses curseurs. Le
verdict attend la réponse du serveur à la commande arrêtée : une lecture est
`Cancelled` ; tout le reste est `OutcomeUnknown`, puisqu'une écriture
multi-documents garde ce qu'elle a modifié. Une session dont le serveur refuse
`killSessions` (Atlas M0, Flex) ne déclare pas `SERVER_SIDE_CANCEL`. Prouvé
dans `$currentOp` par les tests d'intégration.

### Erreurs

Le `code`, le `codeName` et les étiquettes du serveur sont la donnée. Les codes
réseau, les rétrogradations de primaire, `WriteConflict`, `LockTimeout` et les
étiquettes `TransientTransactionError` et `RetryableWriteError` sont
transitoires pour une lecture et **ambigus** pour tout le reste ;
`UnknownTransactionCommitResult` est toujours ambigu ; une coupure de transport
pendant une écriture est `OutcomeUnknown`. Les messages propres à la
bibliothèque, qui citent des hôtes, sont remplacés par ceux du driver.

### État de session, lecture seule, transactions

La base cible est le contexte de session (`set_context`), côté client
seulement ; préférence de lecture `primary`, préoccupations de lecture et
d'écriture du serveur, pas de cohérence causale — fixées et déclarées.
`READ_ONLY_SESSION`, sans `DML`, `DDL` ni `GRANT_REVOKE`, n'est déclaré que si
`connectionStatus` montre chaque action de privilège parmi celles des rôles
intégrés `read` : alors le serveur l'impose. Sinon, une exécution bornée en
lecture seule est refusée sauf si la commande se classe en lecture : une garde
côté client, à laquelle le JavaScript côté serveur échapperait — et qui est
donc classé `Unknown`. `readPreference` n'est pas une garde d'écriture et
n'est pas utilisée comme telle.

`TRANSACTIONS` n'est pas déclaré : une transaction multi-documents demande un
replica set et une session logique gardée d'une exécution à l'autre, ce que la
console ne tient pas ; `commitTransaction` et `abortTransaction` sont refusés.

### Catalogue et aperçu

Les bases sont des espaces de noms ; collections, vues et collections de
séries temporelles sont des relations. Le nombre de lignes est `count` sans
filtre — des métadonnées, jamais un parcours — sur les collections ordinaires
seulement. Les champs sont inférés d'un échantillon de 1 000 documents et
marqués inférés ; `_id` est la clé primaire d'une collection ordinaire. Un
aperçu est une commande `find`, écrite en BSON, sans ordre imposé
([ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)) ; un
tri demandé est complété par `_id`, et une page au-delà de la première est
refusée sur une vue ou une collection de séries temporelles, qui n'ont pas de
clé unique. Un champ dont le nom contient un point ou commence par `$` ne peut
être ni trié ni projeté par `find` : refusé, jamais approché. `PREVIEW_FILTER`
n'est pas déclaré : le filtre d'aperçu est du SQL.

## Conséquences

* **+** Ce qui s'exécute est ce que l'utilisateur a écrit, au sens près ;
  aucun moteur JavaScript n'est livré, et aucun assistant du shell n'écrit
  derrière une lecture.
* **+** Aucun champ n'est jamais perdu ni converti : la grille montre les
  colonnes de l'échantillon, la colonne `$document` porte tout, une surprise
  termine le flux en se nommant.
* **+** La garde de la production a un seul lecteur d'une commande MongoDB,
  testé contre la référence des commandes ; un arrêt est prouvé dans le
  `$currentOp` du serveur.
* **−** Les utilisateurs de `mongosh` doivent écrire des commandes, pas
  `db.users.find(…)` ; le confort des assistants du shell (`ObjectId(…)`,
  `ISODate(…)`) disparaît. L'agent écrit les commandes pour eux.
* **−** Les colonnes dépendent des premiers documents : un champ qui n'apparaît
  qu'après l'échantillon n'est visible que dans `$document`, et un changement
  de type termine le flux.
* **−** Pas de transaction en console, pas de valeur liée, et la garde de
  lecture seule d'un compte en lecture-écriture est côté client.
* **−** Une nouvelle commande du serveur est `Unknown` jusqu'à ce que la table
  l'apprenne : une confirmation sur la production pour une lecture que la
  table ne connaît pas encore.

**Coût de sortie :** moyen. La syntaxe de la console est visible de
l'utilisateur : les requêtes enregistrées et l'historique gardent des commandes
sous cette forme. Une syntaxe de shell ajoutée plus tard serait un second
langage à côté de celui-ci, pas un remplacement. Le client est confiné à cette
crate.

**À reconsidérer si** les utilisateurs ont besoin de transactions en console
(une session gardée d'une exécution à l'autre, `TRANSACTIONS` déclaré par
replica set), si un produit compatible MongoDB est testé et demande d'autres
capacités, ou si le client Rust cesse d'être maintenu.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Une console JavaScript compatible `mongosh` | Embarque un moteur JavaScript et la bibliothèque du shell ; ses assistants écrivent, et un texte qui n'appelle que `find` ne se distingue pas de celui qui appelle `deleteMany` sans l'exécuter — la classification dont dépend la production serait une supposition |
| Une petite syntaxe `db.coll.find({…})` au-dessus des commandes | Une seconde grammaire qui ressemble à du JavaScript et n'en accepte qu'une partie : les utilisateurs taperaient le reste et rencontreraient des erreurs que le shell ne donne jamais ; la forme commande est exacte et documentée par MongoDB lui-même |
| Un champ chaîne de connexion, mot de passe compris | La chaîne atteindrait le fichier de workspace et toute erreur qui la cite (I-03) ; analyser une URI collée pour en extraire le mot de passe est une suite du formulaire, pas une raison de la stocker |
| Les lectures et écritures réessayables de la bibliothèque | Une commande retentée est une seconde exécution que l'appelant n'a pas décidée ; le contrat des drivers l'interdit, et une commande de console n'a pas de numéro de transaction qui la rende idempotente |
| Un schéma tiré des seuls validateurs `$jsonSchema` | La plupart des collections n'en ont pas, et un validateur décrit ce qui peut être écrit, pas ce qui est stocké |
| Une colonne par champ, valeurs converties vers un type commun | Un `Int32` dans un document et un sous-document dans un autre perdraient l'un des deux ; la conversion est la perte silencieuse que DRIVER-CONTRACT §3 interdit |
| `killOp` avec un `comment` ajouté à la commande de l'utilisateur | Envoie une autre commande que celle tapée, et écrase un `comment` écrit par l'utilisateur ; l'identifiant de session logique fait partie de l'enveloppe que le driver possède |
