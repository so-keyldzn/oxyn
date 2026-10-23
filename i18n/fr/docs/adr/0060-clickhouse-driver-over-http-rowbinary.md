<!-- oxyn-translation source="docs/adr/0060-clickhouse-driver-over-http-rowbinary.md" sha256="e285e87d95ae" -->

> Traduction française de [docs/adr/0060-clickhouse-driver-over-http-rowbinary.md](../../../../docs/adr/0060-clickhouse-driver-over-http-rowbinary.md). **La version anglaise fait foi.**

# ADR-0060 — Le pilote ClickHouse parle l'interface HTTP, lit lui-même `RowBinaryWithNamesAndTypes` et ne garde aucune session serveur

**Statut :** proposé · **Date :** 2026-10-23

## Contexte

ClickHouse est un serveur analytique orienté colonnes. C'est un protocole à
part entière, pas un produit qui parlerait un protocole qu'Oxyn implémente déjà
([ADR-0003](0003-driver-capabilities.md)) : `SqlDialect::ClickHouse` existe dans
`oxyn-core` depuis la phase 1, mais aucun pilote ne le sert. Ce qui coûte cher à
défaire une fois le pilote écrit, c'est le protocole de transport, la
bibliothèque cliente, la façon dont les valeurs deviennent de l'Arrow, et ce
qu'est une « session ». Vérifié le 2026-10-23
([RESEARCH-NOTES](../RESEARCH-NOTES.md#protocole-clickhouse-et-bibliothèques-clientes--vérifié-le-2026-10-23)) :

* **Deux protocoles.** Une interface HTTP (8123, 8443 sous TLS) et un protocole
  TCP natif (9000, 9440). Les deux portent du SQL ; le HTTP est celui que toute
  offre gérée expose et que tout proxy comprend.
* **Lignes de serveur.** Docker Hub publie 26.9.25.15 (la plus récente),
  26.3.59.10 et 25.8.33.6 — les lignes `.3` et `.8` étant les lignes longues,
  toujours corrigées. Le pilote est testé sur 26.3.
* **`clickhouse` 0.15.3** (la crate officielle, HTTP) décode les lignes dans
  des types Rust connus à la compilation, par `serde` ; elle n'a ni ligne non
  typée, ni `Dynamic`, ni flux d'octets brut d'un format choisi. Oxyn exécute
  du SQL arbitraire dont le schéma n'est connu qu'à l'arrivée de la réponse.
* **`clickhouse-arrow` 0.2.1** (protocole natif vers Arrow) dépend d'`arrow ^57`
  alors que le workspace est en 59, et de `webpki-roots` plutôt que du
  vérificateur de la plateforme. **`klickhouse` 0.16.0** (natif) décode dans
  son propre modèle de valeurs. **`clickhouse-rs` 0.1.21** a été publiée pour
  la dernière fois en 2023-09.
* **`FORMAT ArrowStream`** a l'air du chemin le plus court, et ne l'est pas : en
  sortie, ClickHouse écrit `DateTime` en `UINT32` (son fuseau perdu), `Date32`
  en `UINT16`, `Int128`/`UInt128`/`Int256`/`UInt256` et `IPv6` en
  `FIXED_SIZE_BINARY`, `Enum` en son entier (le libellé perdu), et `JSON`,
  `Dynamic` ou un état d'agrégat en binaire opaque par défaut
  (`output_format_arrow_unsupported_types = binary`). Chacun demanderait une
  réécriture par type après décodage, à partir d'un nom de type qu'Arrow ne
  porte pas.
* **Une exception levée après le premier octet voyage dans le corps.** Une fois
  des lignes parties, le statut est déjà 200 ; ClickHouse 26.3 ajoute alors
  `\r\n__exception__\r\n<tag>\r\n<message>…` au corps, `<tag>` étant le texte
  aléatoire de l'en-tête `X-ClickHouse-Exception-Tag` (observé). Un lecteur
  Arrow IPC, comme tout décodeur, le lit comme des données à moins que quelque
  chose ne le découpe d'abord.
* **HTTP est sans état** tant qu'aucun `session_id` n'est envoyé ; `SET`, `USE`
  et les tables temporaires vivent dans cette session. Le serveur refuse deux
  instructions dans une requête (`Multi-statements are not allowed`, observé).
* **Pas de transaction** dans le cas général. Un `INSERT … SELECT` écrit bloc
  par bloc et garde les blocs d'avant un échec ; une mutation
  (`ALTER TABLE … UPDATE/DELETE`) est mise en file et continue après le retour
  de l'instruction qui l'a créée, et après `KILL QUERY`.
* **La lecture seule est celle du serveur** : `readonly = 1` refuse écritures,
  DDL, fonctions de table qui sortent (`url()`), et tout réglage que
  l'instruction porte elle-même — un `SELECT … SETTINGS max_threads = 2`
  compris (observé, code 164). `readonly = 2` laisse passer les réglages et
  `url()`. Aucun des deux ne peut être abaissé par une requête.
* **`KILL QUERY WHERE query_id = …`** arrête une instruction depuis une autre
  requête ; le compte a besoin du droit même pour sa propre instruction.
  `KILL … TEST` vérifie le droit sans rien arrêter, et répond 200 avec un corps
  vide et sans format de résultat quand rien ne correspond (observé).

## Décision

1. **L'interface HTTP, par le `reqwest` du workspace** (rustls sur `aws-lc-rs`,
   vérificateur de la plateforme), sans proxy : un proxy système porterait le
   mot de passe et les données par un hôte que personne n'a configuré. Le TLS
   suit l'[ADR-0052](0052-verified-tls-outside-local.md) : `verify-full` est le
   seul mode hors d'une connexion marquée Local. Le mot de passe voyage dans
   l'en-tête `X-ClickHouse-Key`, marqué sensible, jamais dans l'URL.
2. **Une instruction, une requête, aucun `session_id`.** Chaque requête porte ce
   qui décide de son sens : la base (celle du sélecteur de contexte, par le
   paramètre `database` — [ADR-0019](0019-contexte-de-session.md)), le format de
   sortie, la borne de lecture seule, un `query_id` choisi par le pilote. Un
   `USE`, un `SET` ou un `CREATE TEMPORARY TABLE` tapés sont refusés avant
   l'envoi, avec un texte fixe qui nomme l'issue : le sélecteur de contexte, la
   clause `SETTINGS` de l'instruction, une table `Memory`. `SESSION_CONTEXT` est
   déclaré ; `TRANSACTIONS` et `MULTIPLE_STATEMENTS` ne le sont pas.
3. **Les résultats arrivent en `RowBinaryWithNamesAndTypes`, décodés par le
   pilote.** L'en-tête donne le type ClickHouse exact de chaque colonne sous
   forme de texte ; le pilote l'analyse en `ChType` (imbrication bornée, aucune
   récursion sur une profondeur hostile), mesure chaque ligne entière avant de
   la décoder, et l'ajoute à des constructeurs Arrow. La table de
   correspondance et ses pertes vivent dans le module `schema` de la crate et
   sont testées contre le `toString` du serveur lui-même pour les mêmes
   valeurs. Aucun entier ne devient un flottant : `UInt64` reste `UInt64`,
   `Int128` devient `Decimal256(39, 0)`, les entiers de 256 bits leurs chiffres
   exacts, `Decimal(P, S)` `Decimal128`/`Decimal256`. Un instant porte le
   fuseau que son type déclare, sinon le fuseau de la requête que la réponse
   nomme dans `X-ClickHouse-Timezone` — celui du serveur, jamais celui du poste.
4. **Un type dont le pilote ne connaît pas la disposition sur le fil est refusé
   en nommant sa colonne**, avant la première ligne : `RowBinary` ne donne
   aucune longueur pour le sauter, il ne peut donc pas être rendu en octets
   bruts. `Variant`, `Dynamic`, `AggregateFunction` et `Object` sont de tels
   types aujourd'hui. L'aperçu de table les lit par `toString(…)` (un état
   d'agrégat par `hex(toString(…))`), si bien qu'un clic sur une table n'échoue
   jamais à cause d'une colonne. Une `String` qui n'est pas de l'UTF-8 fait
   échouer le résultat en nommant la colonne et en indiquant `hex()`.
5. **Les lots se ferment à 1 Mio d'octets décodés** (ou 8 192 lignes, pour que
   les lignes étroites atteignent la grille tôt), une ligne de plus de 64 Mio
   est refusée, et la tâche qui lit le corps passe les lots au curseur par un
   canal à une place : le réseau n'est lu qu'au rythme où l'appelant vide
   ([I-06](../../../../CLAUDE.md#i-06)). `ExecLimits::max_rows` est au pilote de
   tenir : une lecture qui le dépasse est arrêtée sur le serveur et marquée
   tronquée ; toute autre instruction est lue jusqu'au bout, ses lignes
   écartées, pour que ce qu'elle a écrit soit connu.
6. **Le marqueur d'exception est découpé du corps** grâce au tag de la réponse,
   en retenant la longueur du marqueur à la fin de chaque morceau jusqu'à
   l'arrivée de la suite, pour qu'un marqueur coupé entre deux morceaux ne soit
   jamais décodé comme une valeur ; le message est lu jusqu'à sa ligne de
   clôture. Un serveur dont le 200 ne porte pas de tag — vérifié à la
   connexion, pas déduit d'une version — reçoit `wait_end_of_query = 1` sur
   chaque requête, pour qu'une erreur garde son propre statut ; un compte qui
   ne peut porter aucun réglage est alors refusé plutôt que de voir une erreur
   affichée comme des lignes.
7. **Un arrêt est un `KILL QUERY … ASYNC` sur le `query_id` du pilote**, depuis
   une requête séparée, l'identifiant lié comme paramètre de requête. L'arrêt
   est rapporté d'après l'intention de l'instruction et la fin que le serveur
   lui donne : une lecture est `Cancelled` ; tout le reste est
   `OutcomeUnknown`, puisque rien ne peut être annulé
   ([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)).
   `SERVER_SIDE_CANCEL` n'est déclaré que si `KILL … TEST` réussit pour le
   compte à la connexion. Chaque requête porte aussi
   `cancel_http_readonly_queries_on_client_close = 1`, qui n'agit que sur une
   requête à `readonly` supérieur à 0 (observé sur 26.3) : une lecture non
   bornée en lecture seule porte donc `readonly = 2`, qui lui laisse sa
   propre clause `SETTINGS` et ses fonctions de table, si bien qu'une lecture
   dont le curseur est abandonné s'arrête sur le serveur même pour un compte
   qui ne peut pas tuer, ou quand un kill est arrivé avant que le serveur
   n'enregistre l'instruction — un kill qui la trouve encore en cours est
   renvoyé.
8. **Les erreurs sont classées d'après le code du serveur, l'intention de
   l'instruction et le résumé `written_rows` de la réponse.** Les conditions
   passagères d'une lecture (mémoire, délai, charge, réseau entre serveurs)
   sont transitoires ; une écriture n'est permanente que si le serveur l'a
   refusée avant de rien exécuter (syntaxe, objet inconnu, privilège) **et** a
   dit n'avoir rien écrit ; tout autre échec d'une écriture est ambigu et
   jamais rejoué ([I-13](../../../../CLAUDE.md#i-13)). Le pilote ne retente rien.
9. **Une exécution en lecture seule porte `readonly = 1`**, la borne la plus
   forte que le serveur fasse respecter. Le prix est accepté et dit : une
   lecture bornée en lecture seule — toute lecture d'une connexion de
   production — ne peut pas porter sa propre clause `SETTINGS` ; le serveur la
   refuse avec le code 164. Un compte que le serveur tient déjà en lecture
   seule n'est pas borné davantage, et ne déclare ni `DML` ni `DDL`.
10. **Aucune page après la première dans un aperçu.** Une clé primaire
    ClickHouse est le préfixe de la clé de tri et n'est pas unique, si bien
    qu'aucun ordre n'est total et qu'un `OFFSET` répéterait ou sauterait des
    lignes ([ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)).
    Aucune colonne n'est marquée clé primaire dans le catalogue ; la clé
    primaire est listée comme un index non unique, et l'instruction `CREATE` de
    la table est sa définition.
11. **Les valeurs liées partent au serveur comme paramètres de requête.** La
    n-ième valeur du panneau de paramètres de l'éditeur lie le n-ième marqueur
    `{name:Type}` distinct de l'instruction, dans l'ordre de première
    apparition — chaînes, commentaires et heredocs ignorés ; un nombre
    différent est refusé avant la requête. Le texte atteint le serveur
    inchangé, les valeurs comme `param_<name>` avec leurs échappements
    tabulés, `\N` pour un `NULL` : le serveur les type, le pilote ne les écrit
    jamais dans le texte ([I-10](../../CLAUDE.md#i-10)). Un booléen se lie en
    `1`/`0`, un instant en secondes depuis l'epoch avec ses nanosecondes ; les
    octets, les intervalles, les tableaux et les instants antérieurs à 1970
    sont refusés en nommant leur rang.

## Conséquences

* **+** Chaque type ClickHouse que le pilote connaît atteint Arrow exactement,
  son type serveur dans les métadonnées du champ (`oxyn:clickhouse_type`), sans
  autre dépendance que le client HTTP que le workspace livre déjà.
* **+** Rien de ce que fait une instruction ne survit à sa requête : une
  reconnexion, une nouvelle tentative de l'appelant ou une seconde console ne
  peuvent tomber sur un réglage oublié par l'utilisateur.
* **+** L'annulation est prouvée dans `system.processes`, pour une lecture, un
  curseur abandonné, une session fermée et un `INSERT` arrêté.
* **−** Le pilote possède un décodeur `RowBinary` : environ 1 500 lignes qui
  doivent suivre chaque nouveau type ClickHouse. Un type ajouté par le serveur
  est refusé jusqu'à ce que le décodeur l'apprenne.
* **−** `Variant`, `Dynamic`, `AggregateFunction` et `Object` ne peuvent pas
  être lus par une requête de console sans conversion.
* **−** Les lectures de production ne peuvent pas utiliser de clause
  `SETTINGS` ; un professionnel qui règle `max_threads` en production doit le
  faire sur une connexion non bornée, ou attendre une option `readonly = 2` par
  connexion.
* **−** Les tables temporaires et `SET` ne sont pas disponibles en console.
* **−** Un compte sans le droit `KILL QUERY` ne peut pas arrêter une écriture
  sur le serveur : le pilote le déclare, et l'écriture continue.

**Coût de sortie :** le décodeur (`types`, `measure`, `decode`, `body`,
`schema`) peut être remplacé par un autre format de transport sans toucher la
session, le catalogue ni le desktop, qui ne voient que des `RecordBatch`. Passer
à un client du protocole natif réécrirait `http` et `stream`, environ un tiers
de la crate. Une session serveur (`session_id`) serait additive : une option
déclarée, les instructions refusées aujourd'hui alors acceptées.

**À reconsidérer si** la sortie Arrow de ClickHouse porte le fuseau d'un
`DateTime`, le libellé d'un `Enum` et les entiers de 128 et 256 bits comme
nombres — le décodeur se réduirait alors au découpage de l'exception ; si une
crate maintenue du protocole natif décode des résultats non typés sur l'Arrow du
workspace ; ou si les utilisateurs ont besoin de tables temporaires ou de `SET`
en console, ce qui appellerait un `session_id` déclaré.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| La crate `clickhouse` | les lignes doivent être des types Rust connus à la compilation ; aucun résultat non typé, pas de `Dynamic`, pas de flux brut d'un format choisi |
| `clickhouse-arrow` (protocole natif) | un second Arrow (^57) dans le graphe à côté du 59 du workspace, `webpki-roots` au lieu du vérificateur de la plateforme, et une crate 0.2 sur la plus large frontière externe |
| `klickhouse` (protocole natif) | son propre modèle de valeurs à reconvertir, et le port natif, que les offres gérées et les proxys exposent moins que le HTTP |
| `FORMAT ArrowStream` | un `DateTime` perd son fuseau, un `Enum` son libellé, les entiers de 128/256 bits et `IPv6` deviennent des octets ; les corrections par type demanderaient les noms de types ClickHouse qu'Arrow ne porte pas, et le marqueur d'exception devrait quand même être découpé d'abord |
| Un `session_id` par console | un état que l'utilisateur ne voit pas, perdu à toute reconnexion, et une session que le serveur expire selon son propre délai : l'instruction suivante s'exécuterait sous d'autres réglages sans un mot |
| `readonly = 2` pour les exécutions en lecture seule | laisse `url()` et les autres fonctions de table sortir depuis une lecture de production |
| `wait_end_of_query = 1` (le serveur met tout le résultat en tampon, si bien qu'une exception arrive comme un statut) | la première ligne attend la dernière, et le serveur garde tout le résultat en mémoire ou sur disque |
