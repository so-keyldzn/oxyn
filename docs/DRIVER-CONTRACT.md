# Contrat de driver

> **Autorité** : ce que tout driver de base de données doit garantir, et ce
> qu'il lui est interdit de faire. C'est la frontière externe la plus large
> d'Oxyn — un SGBD est un système hostile par défaut : il peut être lent,
> mentir sur ses types, fermer une connexion au milieu d'une réponse.

Dérive de [ADR-0002](adr/0002-arrow-result-model.md) (Arrow),
[ADR-0003](adr/0003-driver-capabilities.md) (capacités) et
[ADR-0007](adr/0007-driver-sidecar.md) (sidecar). Quand ce document paraît
contredire un ADR, c'est ce document qui est faux.

Invariants concernés : [I-02](../CLAUDE.md#i-02), [I-06](../CLAUDE.md#i-06),
[I-09](../CLAUDE.md#i-09), [I-10](../CLAUDE.md#i-10).

## Ce qu'un driver garantit

### 1. Il ne panique jamais sur une entrée venue du serveur

Un driver traduit ; il ne suppose pas. Tout ce qui arrive du réseau est une
donnée non fiable : un type inconnu, un `NULL` là où le schéma dit `NOT NULL`,
un entier hors bornes, un encodage invalide.

**Panne concrète :** un serveur MySQL configuré avec un type spatial renvoie un
BLOB qu'un `unwrap()` sur le décodage fait paniquer. Le profil `release`
compile avec `panic = "abort"` : rien n'attrape la panique, l'application meurt, et l'utilisateur perd ses
onglets et ses requêtes non sauvegardées.

Interdits dans un chemin atteignable depuis une réponse serveur : `unwrap()`,
`expect()`, `panic!()`, `unreachable!()`, `todo!()`, indexation de tranche par
plage, `as` sur un entier qui peut déborder.

### 2. Il expose l'annulation, et l'annulation coupe vraiment

Toute méthode qui peut durer accepte d'être annulée, et l'annulation atteint la
requête **côté serveur**, pas seulement le futur côté client.

**Panne concrète :** l'utilisateur ferme l'onglet d'une agrégation de 4 minutes.
Le futur est abandonné, mais la requête continue sur le serveur, occupe une
connexion du pool et un verrou. Au dixième onglet fermé, la base refuse les
connexions et l'utilisateur conclut qu'Oxyn a cassé sa production.

Concrètement : PostgreSQL a `pg_cancel_backend`, MySQL a `KILL QUERY`, SQLite a
`sqlite3_interrupt`. Un driver qui ne peut pas annuler côté serveur le **déclare**
dans ses capacités, il ne fait pas semblant.

### 3. Il produit des `RecordBatch` Arrow, en flux

L'interface de résultat est un flux de `arrow::RecordBatch`
([ADR-0002](adr/0002-arrow-result-model.md)). Aucun driver ne construit la
totalité du résultat avant de rendre la main, et **aucun driver ne renvoie une
représentation en lignes** : la conversion appartient au driver, pas à l'appelant.

**Panne concrète :** `SELECT * FROM events` sur une table de 50 millions de
lignes. Un driver qui matérialise fait grimper la RSS jusqu'à l'OOM killer ; sur
macOS le processus est tué sans trace. L'utilisateur n'a rien fait d'anormal :
il a cliqué sur une table dans la barre latérale.

Deux conséquences que les drivers ratent le plus souvent :

* **Un driver ligne-à-ligne (`sqlx`, la plupart des pilotes SQL) doit accumuler
  en lots**, et le lot a une taille bornée en octets, pas en nombre de lignes :
  mille lignes portant chacune un BLOB d'un mégaoctet, c'est un gigaoctet.
* **Une source sans schéma (MongoDB) infère son schéma par échantillonnage**, et
  cette inférence est déclarée comme telle jusqu'à l'interface. Un champ absent
  de l'échantillon mais présent plus loin doit produire une erreur explicite ou
  un élargissement de schéma — jamais une valeur silencieusement perdue.

### 4. Il distingue trois familles d'erreurs, et il les classe

| Famille | Exemples | Ce que fait l'appelant |
|---|---|---|
| Transitoire | coupure réseau, `too many connections`, verrou expiré | peut retenter, avec recul exponentiel |
| Permanente | erreur de syntaxe, table ou colonne absente, droits insuffisants | ne retente **jamais**, affiche |
| Ambiguë | expiration côté client pendant une écriture | ne retente **jamais**, signale l'incertitude |

**Panne concrète :** un `INSERT` expire côté client alors que le serveur l'a
appliqué. Classé « transitoire » et rejoué, il crée un doublon dans les données
de l'utilisateur, sans aucun message d'erreur nulle part. C'est le cas qui coûte
le plus cher et le plus tentant à traiter par une simple boucle de retry :
l'ambiguïté ne se retente pas.

**Une colonne inconnue est un refus permanent, même prononcé avant l'envoi.**
Une colonne d'aperçu que la relation ne déclare pas — en projection comme en
tri, voir [§6](#6-il-échappe-tout-identifiant-quil-compose) — se rend en
`OxynError::Query`, comme le serveur l'aurait classée : retenter ne fera pas
apparaître la colonne. `OxynError::CatalogUnavailable`, transitoire, dit un catalogue qui
peut revenir — introspection en cours, cache vide. Confondre les deux ferait
proposer « réessayer » à l'utilisateur pour une demande qui échouera toujours
de la même façon.

### 5. Il déclare ses capacités par session, et ne simule rien

Un driver et chaque `Session` exposent un `Capabilities`
([ADR-0003](adr/0003-driver-capabilities.md)) : transactions, annulation côté
serveur, curseurs nommés, requêtes préparées, introspection des index, langages
de requête acceptés.

**Les capacités s'évaluent par session, pas par driver.** La version du serveur,
ses extensions et les droits du compte connecté changent ce qui est disponible :
le même driver PostgreSQL parle à une base 12 sans `MERGE` et à une base 17 qui
l'a, à une base avec `pg_stat_statements` et à une autre sans.

**Panne concrète :** un driver qui émule les transactions par un simple
enchaînement de requêtes laisse l'utilisateur croire qu'un `ROLLBACK` a annulé
son écriture. Ne pas savoir faire est une réponse acceptable ; laisser croire ne
l'est pas.

Corollaire pour un driver non-SQL : une requête porte un `QueryLanguage`
explicite. Le SQL est un cas parmi d'autres, pas le défaut auquel les autres se
ramènent.

### 6. Il échappe tout identifiant qu'il compose

Le SQL que **l'utilisateur écrit** part tel quel : c'est un outil professionnel,
et le SQL arbitraire est la fonctionnalité. Le SQL qu'**Oxyn compose**
— introspection, aperçu de table, tri par colonne, filtre de la barre latérale,
suggestion IA — ne concatène jamais un nom reçu : il passe par la fonction de
citation d'identifiant du driver, et les valeurs sont liées.

**La projection d'un aperçu suit la même règle.** `PreviewShape::columns`
nomme les seules colonnes à lire : le driver prend la liste par
`PreviewShape::projection`, qui la déduplique et la borne à
`MAX_PROJECTED_COLUMNS` noms, vérifie chaque nom contre la description de la
relation, puis le cite comme la relation elle-même. Un nom que la relation ne
déclare pas est refusé par `OxynError::Query`, erreur permanente
([§4](#4-il-distingue-trois-familles-derreurs-et-il-les-classe)), avant
d'atteindre le serveur, comme une colonne de tri inconnue ; une liste vide est refusée par
`OxynError::Config` et ne vaut jamais `SELECT *`, qui lirait justement ce que
personne n'a approuvé. Une projection ignorée n'est pas une dégradation
acceptable : c'est elle qui borne un échantillon aux colonnes cochées
([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)), et un driver qui
ne sait pas la composer refuse l'aperçu.

**Une exception, et une seule** : le prédicat d'aperçu. L'utilisateur y écrit un
fragment de `WHERE` que le driver insère dans un `SELECT` composé par Oxyn —
donc du texte libre dans du SQL composé. C'est délibéré et argumenté dans
[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) : ce champ **est** du SQL
que l'utilisateur écrit, et le relevé Figma `190:1618` le montre comme tel.
Quatre barrières le bornent — reclassification du texte avant toute décision,
session serveur en lecture seule, borne de lignes, et parenthésage
`WHERE (…\n)` qui transforme un commentaire non terminé en erreur de syntaxe
plutôt qu'en `LIMIT` avalé. Cette exception est nommée ici parce que
[I-10](../CLAUDE.md#i-10) renvoie à ce paragraphe : sans elle, une relecture du
code des aperçus conclurait à une violation d'invariant.

**Panne concrète :** une table nommée `"users"; DROP TABLE audit; --` existe
légalement dans PostgreSQL. Un aperçu construit par concaténation exécute la
suppression au simple clic sur cette table dans l'arborescence. La distinction
entre « SQL de l'utilisateur » et « SQL d'Oxyn » n'est pas un détail de style :
c'est la ligne qui sépare un outil d'une arme.

### 7. Il traite les fuseaux et les types temporels comme des données, pas comme du texte

Aucune conversion implicite vers le fuseau local à la lecture. Un `timestamptz`
se transporte en UTC et se rend dans le fuseau que déclare le schéma Arrow de la
colonne ; un `timestamp` sans fuseau se transporte **sans** en inventer un.
**Aucun driver ne convertit pour l'affichage** : il n'existe pas de préférence
de fuseau d'affichage — le fuseau qu'Oxyn annonce pour un résultat est déduit
du schéma (`oxyn_data::timestamp_display`), ce n'est pas un réglage. Si une telle
préférence venait à exister, elle ne toucherait que le rendu (`oxyn-data`,
`cell.rs`), jamais le driver.

**Panne concrète :** Oxyn affiche une valeur convertie dans le fuseau du poste,
l'utilisateur la recopie dans un `UPDATE`, et décale la donnée de deux heures en
base. La corruption est invisible et permanente.

## Ce qu'un driver n'a pas le droit de faire

| Interdit | Pourquoi |
|---|---|
| Dépendre d'`oxyn-exec`, `oxyn-store`, `oxyn-desktop`, `oxyn-ai` ou d'un autre driver | inverse le sens des dépendances. `oxyn-core` **est** au contraire la dépendance attendue : c'est le vocabulaire commun — `ExecRequest`, `OxynError`, `PreviewShape` —, et les deux drivers livrés en dépendent ([ARCHITECTURE](ARCHITECTURE.md#le-sens-des-dépendances)) |
| Exister en double pour deux produits parlant le même protocole | [ADR-0003](adr/0003-driver-capabilities.md) : Redshift ≡ PostgreSQL, MariaDB ≡ MySQL. La différence est une capacité, pas une crate |
| Écrire dans un fichier, ouvrir une fenêtre, lire une variable d'environnement | un driver reçoit sa configuration, il ne va pas la chercher |
| Journaliser une valeur de paramètre ou un identifiant de connexion | [I-03](../CLAUDE.md#i-03) |
| Retenter tout seul | la politique de reprise appartient à l'appelant, qui seul sait si l'opération est rejouable |
| Exécuter une écriture parce que l'appel « avait l'air » d'en être une | [I-02](../CLAUDE.md#i-02) |
| Modifier l'état de session du serveur sans le déclarer | un `SET search_path` invisible change le sens des requêtes suivantes de l'utilisateur |

## Ce qu'un nouveau driver doit fournir pour être accepté

La procédure est dans [`/driver`](../.claude/commands/driver.md) et la revue dans
[la liste de contrôle](../.claude/checklists/revue-driver.md). En résumé : la
déclaration de capacités, la table de correspondance des types **dans les deux
sens** avec les cas de perte documentés, la classification d'erreurs, un test
d'annulation qui prouve l'arrêt côté serveur, et un test de flux sur un volume
qui ne tiendrait pas en mémoire.
