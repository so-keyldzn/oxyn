<!-- oxyn-translation source="docs/adr/0061-redis-driver-speaks-resp3-itself.md" sha256="cc09ef0dd3cb" -->

> Traduction française de [docs/adr/0061-redis-driver-speaks-resp3-itself.md](../../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md). **La version anglaise fait foi.**

# ADR-0061 — Le driver Redis parle RESP3 lui-même, prouve une lecture par sa propre liste et la table du serveur, et n'arrête que ce que Redis sait arrêter

**Statut :** proposé · **Date :** 2026-10-22

## Contexte

`oxyn-driver-redis` est le premier driver dont le serveur n'a ni schéma, ni
tables, ni SQL. L'[ADR-0003](0003-driver-capabilities.md) établit que Redis et
Valkey sont un seul driver — tous deux parlent RESP —, et que la session déclare
ce qu'elle sait faire. Reste ouvert, et coûteux à défaire : le client, la forme
d'une réponse en Arrow, la façon de connaître l'intention d'une commande, et ce
que fait un arrêt. Vérifié le 2026-10-22
([RESEARCH-NOTES](../RESEARCH-NOTES.md#protocole-redis-et-client--vérifié-le-2026-10-22)) :

* **Serveurs.** Redis 8.10.2 (`redis:8.10.2`, Docker Hub, 2026-10-22) et Valkey
  9.1.2 (`valkey/valkey:9.1.2`, 2026-09-21). Tous deux répondent à `HELLO 3`
  par une map portant `server`, `version`, `proto`, `id`, `mode`, `role` et
  `modules`. Redis 8.10.2 connaît 447 commandes (`COMMAND COUNT`), modules
  compris.
* **Le protocole.** RESP3 ajoute à RESP2 les nuls, booléens, doubles (`inf`,
  `-inf`, `nan`), grands nombres, chaînes verbatim, maps, ensembles, attributs
  et push. Redis n'émet ni chaînes ni agrégats en flux. Une chaîne bulk fait au
  plus 512 Mo par défaut (`proto-max-bulk-len`). Une réponse d'erreur commence
  par un préfixe en majuscules ; la documentation le dit convention.
* **`redis` (redis-rs) 1.7.1** (BSD-3-Clause, MSRV 1.88, 2026-09-25) :
  `MultiplexedConnection::send_packed_command` rend un `RedisResult<Value>`
  entier — la réponse est analysée en arbre avant que l'appelant n'en voie
  rien ; son `Value::Array` est un `Vec`. L'ouverture enchaîne
  `CLIENT SETINFO LIB-NAME redis-rs` et `LIB-VER` sauf si on l'en dispense ;
  `ConnectionManager` se reconnecte de lui-même.
* **Métadonnées des commandes.** `COMMAND` rend, par commande, ses drapeaux
  (`readonly`, `write`, `blocking`, `admin`, `pubsub`, `noscript`…), ses
  catégories ACL (`@read`, `@write`, `@dangerous`, `@scripting`,
  `@blocking`…) et ses sous-commandes (`config|set`). Observé sur Redis
  8.10.2 : `KEYS` est `readonly` et `@dangerous` ; `FLUSHALL` est `write` et
  `@dangerous` ; `EVAL` n'a ni `readonly` ni `write` ; `EVAL_RO` est
  `readonly` ; `PING` n'a que `fast` ; `INFO` est `@dangerous` ; `BLPOP` est
  `write` et `blocking` ; `XREAD` est `readonly` et `blocking` ; aucune
  sous-commande d'`ACL` n'a `readonly`. Le drapeau est la parole du serveur,
  pas une preuve : sur Redis 8.10.2, le module de recherche marque `readonly`
  `FT.CURSOR DEL` et les sous-commandes de `_FT.DEBUG` qui forcent un
  ramasse-miettes ou changent une limite (2026-10-24), et un fork, un proxy
  ou un point d'accès hostile répond la table qu'il veut.
* **Arrêter.** `CLIENT UNBLOCK <id> ERROR` (`@admin`, `@dangerous`) débloque un
  client bloqué par une commande bloquante ; il lit alors `-UNBLOCKED`.
  `SCRIPT KILL` et `FUNCTION KILL` arrêtent le script en cours — quel qu'il
  soit — sauf s'il a déjà écrit, auquel cas ils répondent `-UNKILLABLE`
  (`src/script.c`, `scriptKill`, Redis 8.10.2) ; seul `SHUTDOWN NOSAVE` arrête
  un tel script. Toute autre commande va à son terme : Redis est mono-fil et
  n'a pas de point d'interruption.
* **Erreurs.** Les préfixes qu'écrit Redis 8.10.2 (`src/*.c`) : `ERR`,
  `WRONGTYPE`, `NOAUTH`, `WRONGPASS`, `NOPERM`, `NOPROTO`, `NOSCRIPT`,
  `NOGROUP`, `BUSYGROUP`, `BUSYKEY`, `EXECABORT`, `LOADING`, `BUSY`, `OOM`,
  `MISCONF`, `MASTERDOWN`, `NOREPLICAS`, `READONLY`, `TRYAGAIN`,
  `CLUSTERDOWN`, `CROSSSLOT`, `MOVED`, `ASK`, `UNBLOCKED`, `UNKILLABLE`,
  `NOTBUSY`, `DENIED`, `IOERR`. Les transitoires sont refusés dans
  `processCommand` avant que la commande ne tourne.
* **Classement par session.** `oxyn-exec` reclasse chaque requête d'après son
  texte avec `oxyn-query`, qui répond `Unknown` pour tout langage qui n'est pas
  SQL : sans source de vérité pour Redis, `GET` demanderait la confirmation de
  production et serait refusé à un agent. Un crochet `Session::classify_intent`,
  auquel le driver répond sans I/O, est décidé à part dans l'ADR-0064, pour
  tous les drivers non SQL.

## Décision

1. **Le driver lit et écrit RESP3 lui-même**, sur `tokio` (TCP ou socket Unix)
   et `tokio-rustls` 0.26 sur `aws-lc-rs`, avec `webpki-roots` pour les racines
   publiques. Aucune bibliothèque cliente Redis n'est une dépendance. Le lecteur
   livre une *tête* à la fois, si bien qu'un agrégat devient des lignes élément
   par élément ; aucune longueur venue du serveur ne dimensionne une allocation
   au-delà de 1 024 éléments ; une ligne est bornée à 1 Mio ; une imbrication
   de plus de 32 niveaux est sautée itérativement et marquée ; les types RESP3
   en flux sont refusés comme malformés. Une connexion compte les réponses
   qu'elle a demandées et pas lues jusqu'au bout ; celle qui n'est plus en
   phase — son lecteur abandonné par un délai ou un onglet fermé — est fermée
   et on n'y écrit plus jamais.
2. **Poignée de main et état déclaré :** `HELLO 3 [AUTH user password] SETNAME
   oxyn`, puis `SELECT n` quand le formulaire nomme une base autre que 0. Un
   serveur sans `HELLO` (Redis avant 6) est refusé. Un serveur dont le `HELLO`
   dit `cluster` ou `sentinel` est refusé avec un message qui nomme le mode : un
   cluster répondrait `MOVED`, que le driver ne suit jamais en silence.
3. **Une lecture est prouvée par la liste du driver et la table `COMMAND` du
   serveur ensemble ; la table ne fait jamais que durcir.** Le driver compile
   les commandes prouvées en lecture seule (`reads.rs`, 157, datées dans
   RESEARCH-NOTES) : celles que Redis 8.10.2 et Valkey 9.1.2 marquent toutes
   deux `readonly` hors `@dangerous`, et les commandes de lecture des modules
   que Redis charge. Dans l'ordre : toute commande `ACL` est `Grant` ; une
   écriture dont le driver connaît la portée (point 15) est un `Write`, quoi
   que la table dise ou omette ; `EVAL`, `EVALSHA` et `FCALL` valent
   `Unknown` même quand la table les marque `write`, le corps d'un script
   n'étant pas lu ; une commande `@dangerous` selon la table vaut `Unknown` ;
   une que la table marque `write` est `Write` ; une qui est sur la liste
   **et** que la table marque `readonly` est `Read` ; tout le reste — aucun
   drapeau, `readonly` mais hors liste, un nom que le serveur ne connaît pas,
   un conteneur nommé sans sous-commande connue — ne prouve rien et vaut
   `Unknown`. Une table qui marque `SET`, `CONFIG SET` ou `FLUSHALL`
   `readonly` ne prouve donc aucune lecture, et une qui omet `FLUSHALL` ne
   l'abaisse pas. Le driver répond par `Session::classify_intent`
   (ADR-0064) ; la table est lue une fois par session.
4. **Ce qui change la connexion est refusé en console**, par la liste du driver
   lui-même — aucun drapeau ne le dit : `UNSUBSCRIBE` et sa famille, `SYNC`, `PSYNC`, `REPLCONF`, `HELLO`, `AUTH`, `SELECT` (le contexte de session
   choisit la base), `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`, `RESET`,
   `QUIT`, `READONLY`/`READWRITE`/`ASKING`, et `CLIENT TRACKING`, `CACHING`,
   `REPLY`, `SETNAME`, `SETINFO`, `NO-EVICT`, `NO-TOUCH`. Les commandes
   bloquantes sont envoyées ; le délai de l'exécution et l'arrêt les bornent.
5. **Une exécution est une commande**, lue comme `sdssplitargs` de
   `redis-cli` la lit ; une seconde ligne hors guillemets et un NUL tapé sont
   refusés. Une console porte une commande par ligne, et Run sans sélection
   envoie la ligne sous le curseur, un saut de ligne entre guillemets gardé
   dans son argument (`oxyn_query::redis::current_command`) ; un guillemet
   resté ouvert rend la limite inconnue, et une sélection est demandée. Les clés sont affichées, prévisualisées et composées par le rendu
   inverse, qui se relit vers les mêmes octets exactement. La grammaire vit
   dans `oxyn_query::redis::argv` : le driver construit avec elle l'argv
   qu'il envoie, et le caviardage du point 12 lit un texte de console avec la
   même fonction de lecture d'argument, si bien que ce qui est gardé et ce
   qui s'exécute ne peuvent pas se lire de deux façons.
6. **Une exécution en lecture seule n'envoie qu'une lecture prouvée**
   (point 3), et revérifie la liste du driver à partir de l'argv seul, en
   plus de l'intention : une table qui ment ne peut pas l'ouvrir à elle
   seule. Redis n'a pas de session en lecture seule, donc la session ne
   déclare pas `READ_ONLY_SESSION` ; `EVAL_RO`, `EVALSHA_RO` et `FCALL_RO`
   qualifient, et le serveur refuse une écriture en leur sein.
7. **Arrêter :** une commande `blocking` est arrêtée par `CLIENT UNBLOCK <id>
   ERROR` depuis une seconde connexion, prouvé par le `-UNBLOCKED` de la
   commande arrêtée elle-même (une commande bloquante de module répond à son
   déblocage par une erreur de son choix, qui ne prouve rien) ; un script de la famille `EVAL` ou
   `FCALL` par `SCRIPT KILL` ou `FUNCTION KILL`, qui arrêtent le script en
   cours, quel qu'il soit. Le driver resserre : `FUNCTION STATS` doit montrer
   la ligne de commande de cette session pour une fonction, et répond `BUSY`
   pendant un `EVAL` ; puis la connexion arrêtée a 250 ms pour montrer une
   réponse à elle — la commande de cette session refusée par `BUSY` derrière
   le script d'un autre client —, et une réponse en attente veut dire que rien
   n'est tué. Reste une course d'un aller-retour, où le script d'un autre
   client qui n'a pas écrit est tué ; cette décision l'accepte plutôt que de
   laisser un script emballé de cette session tenir le serveur. Seule la
   réponse de la commande arrêtée prouve l'arrêt : `Script killed by user`,
   ou `BUSY`. Toute autre commande, et un script qui a
   déjà écrit, ne peuvent être arrêtés : le driver ferme la connexion, ce qui
   libère la réponse que le serveur écrit, et dit que la commande continue.
   `SERVER_SIDE_CANCEL` est déclaré par session quand le compte peut envoyer
   `CLIENT UNBLOCK`.
8. **Verdicts.** Un arrêt finit en `Cancelled` (ou `Timeout`) seulement quand la
   commande n'a jamais été envoyée, quand le serveur a répondu à la commande
   arrêtée par sa propre erreur, ou quand le serveur marque la commande
   `readonly`. Sinon il finit en `OutcomeUnknown`, qui nomme ce qui a pu se
   passer.
9. **Classes d'erreur d'après le préfixe :** `LOADING`, `BUSY`, `OOM`,
   `MISCONF`, `NOREPLICAS`, `MASTERDOWN`, `TRYAGAIN`, `CLUSTERDOWN` sont
   transitoires ; tout autre préfixe est permanent. Toute erreur d'un script
   (`@scripting`, ou un nom `EVAL`/`FCALL` quand la table est illisible) sauf
   `NOSCRIPT` est ambiguë, quel que soit son préfixe : Redis garde ce qu'un
   script en échec a écrit, et le script choisit le préfixe — un `TRYAGAIN`
   qu'il rend après un `DECRBY` n'est pas le refus du serveur. `IOERR` est
   ambigu : le transfert de `MIGRATE` a échoué et la clé est peut-être sur les
   deux instances. Une connexion perdue après l'envoi d'une commande qui peut
   écrire vaut `OutcomeUnknown`, et un échec **pendant** son envoi aussi :
   `write_all` ne dit pas combien d'octets sont partis avant l'échec, et un
   flux TLS accepte le texte clair avant que l'écriture sur la socket
   n'échoue, si bien qu'aucun résultat d'I/O ne prouve qu'aucun octet n'a
   atteint le serveur. Seule une lecture reste transitoire.
10. **Les réponses en Arrow.** Le premier octet de la réponse décide des
    colonnes : un scalaire est une ligne typée (`Int64`, `Float64`, `Boolean` ;
    un grand nombre est `Utf8` marqué `oxyn:redis_type`) ; un tableau, un
    ensemble ou un push donne une ligne par élément (`index`, `type`, `value`,
    `value_bytes`, `truncated`) ; une map une ligne par entrée. Une chaîne va
    dans `value` (`Utf8`, à l'identique) quand ses octets sont de l'UTF-8 et
    dans `value_bytes` (`Binary`) sinon — jamais échappée, jamais devinée. Un
    agrégat imbriqué est une cellule, dans un texte compact proche du JSON. Un
    lot se ferme à 1 Mio, une valeur est coupée à 16 Mio et sa ligne le dit. La
    commande choisit une forme plus riche pour la famille `SCAN`,
    `ZRANGE … WITHSCORES`, `XRANGE`/`XREVRANGE`, `TS.RANGE`, `INFO`,
    `SLOWLOG GET` (une ligne par entrée, sa durée en microsecondes) et
    `CLIENT LIST`/`CLIENT INFO` (une ligne par client : ses champs usuels en
    colonnes, et sa ligne entière, pour qu'un champ ajouté par un serveur plus
    récent ne se perde pas). Un tableau de maps — `XINFO GROUPS`,
    `XINFO CONSUMERS`, `ACL LOG`, `MODULE LIST`, `FUNCTION LIST` — donne une
    ligne par map, ses colonnes étant les clés de la première, lue avant le
    premier lot ; une clé que la première n'avait pas va dans `other_fields`.
    Une réponse
    abandonnée avant sa fin coûte la connexion ; la session la rouvre avec le
    même état déclaré.
11. **Le catalogue :** les bases sont des catalogues nommés `db0`, `db1`… (d'après
    `CONFIG GET databases`, ou `INFO keyspace` quand le compte ne peut pas lire
    la configuration), au plus 256 en plus des non vides et de celle du
    formulaire ; une base liste au plus 1 000 clés et 1 Mio de noms de clés,
    trouvées par au plus 50 appels `SCAN … COUNT 1000` d'au plus 4 Mio
    chacun, chaque clé avec son type, comme relations `KeyPattern` ; `KEYS`
    n'est jamais envoyé. Un nom de clé est écrit sous la forme entre
    guillemets que la console relit vers les mêmes octets, caractères de
    format invisibles échappés. Le catalogue et chaque aperçu ont leurs propres
    connexions, si bien que parcourir l'arbre ne déplace jamais la base de la
    console.
12. **Les identifiants tapés dans une console sont caviardés** avant que
    l'historique, le journal ou une conversation ne garde le texte, comme
    l'ADR-0053 le fait pour le SQL : le mot de passe d'`AUTH` et de
    `HELLO … AUTH`, les règles `>`/`<`/`#`/`!` d'`ACL SETUSER`, la valeur de
    `CONFIG SET` pour `requirepass`, `masterauth`, `masteruser`,
    `tls-key-file-pass` et `tls-client-key-file-pass`, et les mots de passe
    `AUTH`/`AUTH2` de `MIGRATE` (`oxyn_query::redis::redact_secrets`). La
    décision se prend sur l'**argv décodé**, lu par la grammaire même de
    l'exécution (point 5), et le remplacement couvre l'argument tel qu'il est
    écrit : `"A\x55TH" hunter2` est `AUTH`, `"\x3ehunter2"` une règle `>`. Ce
    que le driver refuse — un guillemet ouvert ou collé, un NUL — est tout de
    même lu, en gardant le plus d'octets dans un seul argument. Un
    réceptacle qui connaît le langage caviarde selon lui — l'historique,
    l'entrée de journal construite à partir d'un `Execute`. Celui qui ne le
    connaît pas — un enregistrement d'appel d'outil et son réencodage, une
    entrée d'audit réécrite, une requête sauvegardée mentionnée à un agent —
    applique toutes les grammaires (`oxyn_query::redact_any_language`) :
    imposer le SQL y gardait `CONFIG SET requirepass` entier, dans le
    workspace et dans l'invite suivante. Le prix est qu'une ligne SQL
    commençant par un mot-clé d'identifiant Redis perd un argument dans la
    copie gardée, jamais dans ce qui s'exécute.
13. **Pub/Sub et `MONITOR` sont des captures bornées.** `SUBSCRIBE`,
    `PSUBSCRIBE`, `SSUBSCRIBE` et `MONITOR` tournent sur une connexion à eux,
    jamais celle de la console ; ce qui arrive devient des lignes jusqu'à la
    limite de lignes, une fenêtre qui se ferme une seconde avant le délai de
    l'exécution, ou l'arrêt. Fermer cette connexion met fin à l'abonnement ou
    au monitor sur le serveur, et un arrêt est `Cancelled` : une capture
    n'applique rien. Leur intention reste celle du serveur — aucune des deux
    commandes n'est marquée `readonly`, et `MONITOR` est `@dangerous` —, si
    bien qu'une capture est confirmée en production et refusée à un agent.
    Un message est daté par l'horloge d'Oxyn à son arrivée, une ligne de
    `MONITOR` par celle du serveur.
14. **L'assistant a un agent à lui sur Redis**,
    `crates/oxyn-ai/agents/redis.md` (`languages: [redis]`, ADR-0065) :
    jamais `KEYS`, une clé mesurée avant d'être lue en entier, la portée
    d'une écriture montrée avant l'écriture, aucune transaction sur laquelle
    s'appuyer. Ce qu'il envoie est classé par la session comme le dit le
    point 3, comme toute requête de console.
15. **Une écriture qui atteint tout ce que contient sa cible porte un
    risque** (`Session::classify_risk`, ADR-0066), d'après la liste du
    driver — aucun drapeau ne le dit : `FLUSHALL` et `FLUSHDB` sont
    `Truncate`, `SWAPDB` `UnboundedUpdate`, et `FUNCTION FLUSH`,
    `FUNCTION DELETE`, `FUNCTION RESTORE … FLUSH`, `XGROUP DESTROY`,
    `ACL DELUSER`, `FT.DROPINDEX` et `FT.DROP` sont `DropObject`. Un risque
    n'est répondu que pour une requête prouvée mutante, jamais pour une
    lecture. `FLUSHALL`, `FLUSHDB`, `SWAPDB` et `FT.DROPINDEX` sont `write` et
    `@dangerous` : laissé `Unknown`, un `FLUSHALL` d'un humain sur staging
    partirait en un clic ; une commande de cette liste est donc un `Write`
    avant que la table ne soit lue — une table qui la marque autrement ou
    l'omet n'y change rien —, et son risque demande une approbation sur tous
    les environnements. Une clé nommée explicitement est bornée, si grande
    soit-elle : `DEL`, `UNLINK` et `MIGRATE` ne portent aucun risque. Un
    script n'en porte pas non plus — `EVAL` ne prouve aucune intention, reste
    `Unknown` et son risque n'est jamais demandé, si bien qu'un
    `redis.call('flushall')` à l'intérieur est confirmé comme une simple
    écriture en production et part sur staging ; cette décision l'accepte, le
    corps d'un script étant du code que le driver ne lit pas.

## Conséquences

* **+** Un `HGETALL`, `LRANGE 0 -1` ou `SMEMBERS` de toute taille atteint la
  grille en lots bornés ; une valeur de 512 Mo coûte 16 Mo.
* **+** Redis 7, Redis 8 et Valkey, avec ou sans modules, marquent leurs
  propres écritures ; un serveur dont la table ment ne peut pas faire passer
  une écriture pour une lecture, ni au portail ni dans une exécution en
  lecture seule.
* **+** Rien n'est envoyé que l'utilisateur n'ait tapé ou que le formulaire n'ait
  déclaré : pas de `CLIENT SETINFO`, pas de reconnexion automatique au milieu
  d'une commande, pas de nouvel essai.
* **−** Un décodeur de protocole à maintenir : environ 500 lignes, et les cas
  limites de RESP3 qu'une bibliothèque aurait rencontrés d'abord.
* **−** `PING`, `INFO`, `KEYS`, `CONFIG GET` et tout script valent `Unknown` :
  confirmés en production, refusés là à un agent. Les drapeaux du serveur ne
  disent rien de plus sûr, et deviner serait la faute qu'interdit I-02.
* **−** Une lecture que la liste ne contient pas — une commande de la
  prochaine version, d'un module qu'elle ne nomme pas — vaut `Unknown` tant
  que la liste n'est pas mise à jour : confirmée en production, refusée là à
  un agent, refusée dans une exécution en lecture seule. La liste est datée,
  et réobservée à chaque version de serveur.
* **−** Pas de session en lecture seule : une exécution en lecture seule repose
  sur le refus, par le driver, de ce qu'il ne peut pas prouver être une
  lecture.
* **−** Une commande lente non bloquante ne peut pas être arrêtée : Stop ferme
  la connexion, le serveur finit la commande. Un `KEYS *` de production sur
  cinquante millions de clés bloque le serveur quoi que fasse Oxyn — c'est
  pourquoi il y demande une confirmation.
* **−** Cluster, Sentinel et `MULTI/EXEC` sont refusés à cette étape.
* **−** Une capture est une fenêtre, pas un abonnement qui dure : un message
  publié après sa fermeture n'est pas vu. `MONITOR` ralentit un serveur chargé
  tant que la fenêtre dure.
* **−** **Limite connue, décision ouverte.** Le catalogue de la décision 11
  liste des clés *exactes*, et le contexte de l'IA écrit chaque relation qu'il
  retient par son nom, quel que soit son genre ([AI-PROVIDERS](../AI-PROVIDERS.md)) :
  sous un niveau qui porte les noms, les noms exacts des clés retenues pour une
  question atteignent l'invite, et un nom de clé peut embarquer lui-même une
  donnée. Cet ADR ne décide pas s'il faut continuer de les envoyer ou envoyer
  des motifs de clés à la place : ce choix est celui du mainteneur, et l'issue
  [#339](https://github.com/so-keyldzn/oxyn/issues/339) (ouverte le 2026-10-25)
  le pose. Tant qu'il n'est pas pris, AI-PROVIDERS dit ce qui part.

**Coût de sortie :** modéré. Remplacer la couche RESP écrite à la main par une
bibliothèque réécrit `resp.rs`, `connection.rs` et la boucle de lecture du
curseur, mais pas le classement, les formes ni les tests, qui parlent valeurs
RESP et Arrow. Les formes (décision 10) sont la partie coûteuse à changer :
elles atteignent les exports et les résultats sauvegardés
([I-11](../../../../CLAUDE.md#i-11)).

**À reconsidérer si** un client Rust Redis maintenu expose une réponse élément
par élément avec une mise en mémoire bornée ; si Redis gagne une session en
lecture seule ou un point d'interruption pour les commandes en cours ; ou quand
le support de Cluster est décidé, puisque le routage par slots changerait le
modèle de connexion.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| `redis` 1.7.1 | analyse une réponse entière en arbre `Value` avant de la rendre : une grande collection est matérialisée en entier (I-06). Il envoie aussi `CLIENT SETINFO` et se reconnecte de lui-même sauf configuration contraire |
| Classer les écritures d'après une liste écrite dans le driver | faux pour toute commande de module et pour la prochaine version de Redis ou Valkey ; le serveur décrit ses propres écritures, et une écriture qu'il marque est une écriture |
| Se fier au seul drapeau `readonly` du serveur | le drapeau est la parole du serveur : une table hostile ou étrange fait de `SET` ou `CONFIG SET` une lecture au portail et pour le garde de lecture seule, et le module de recherche de Redis 8.10.2 lui-même marque `FT.CURSOR DEL` `readonly` |
| Prouver « rien n'est parti » à partir d'une écriture en échec | `write_all` cache le compte, et TLS accepte le texte clair avant que la socket n'échoue : une écriture rejouée pourrait s'appliquer deux fois (I-13) |
| Un second lecteur, plus léger, pour le caviardage | il divergeait de l'exécution sur les échappements : `"\x3ehunter2"` s'exécutait comme règle de mot de passe et était gardé tel quel |
| Traiter les lectures `@dangerous` (`KEYS`) comme des lectures | une lecture que le serveur lui-même range parmi les dangereuses bloque une instance de production ; une confirmation est le minimum |
| Suivre automatiquement les curseurs de `SCAN` dans la console | envoie des commandes que l'utilisateur n'a pas tapées ; le curseur de la page est montré sur chaque ligne à la place |
| `SCRIPT KILL` à chaque arrêt d'un script | il arrête le script qui tourne, quel qu'il soit, peut-être celui d'une autre application |
| Échapper les octets non UTF-8 dans la colonne texte | une valeur copiée depuis la grille dans un `SET` écrirait d'autres octets |
| Une crate par produit (Redis, Valkey, KeyDB) | contredit l'ADR-0003 : un protocole |
