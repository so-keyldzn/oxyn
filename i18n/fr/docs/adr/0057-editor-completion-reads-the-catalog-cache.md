<!-- oxyn-translation source="docs/adr/0057-editor-completion-reads-the-catalog-cache.md" sha256="351aa936a17a" -->

> Traduction française de [docs/adr/0057-editor-completion-reads-the-catalog-cache.md](../../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md). **La version anglaise fait foi.**

# ADR-0057 — L'éditeur SQL complète depuis le cache du catalogue, lit ce qui manque dans des bornes, et tire son texte d'insertion de Rust

**Statut :** proposé · **Date :** 2026-10-21

**Précise :** [ADR-0036](0036-l-assistant-complete-le-catalogue.md), sur le point
suivant : l'éditeur est un second lecteur du cache du catalogue qui le complète
lui-même. Le mécanisme — le bus, `Actor::Human`, des lectures séquentielles sur la
session réservée au catalogue, rien lu deux fois — est celui de l'ADR-0036 ; la
sélection de ce qu'il faut lire est celle de l'éditeur.

**S'appuie sur :** [ADR-0022](0022-rafraichissement-automatique.md), pour les
signaux par connexion que l'éditeur écoute, et qui gagnent une variante.

## Contexte

L'[issue #275](https://github.com/so-keyldzn/oxyn/issues/275) demande que la
console suggère les tables et les colonnes de la base connectée. Aujourd'hui,
l'éditeur appelle `sql({ dialect, upperCaseKeywords })` sans `schema`
(`apps/desktop/src/components/oxyn/sql-editor.tsx`) : il suggère les mots-clés du
dialecte et rien d'autre, comme le consigne déjà
[IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md) (phase 1). Cinq faits décident
de la conception.

**Le cache est vide là où l'utilisateur en a besoin en premier.** Le catalogue se
charge à la demande ([ARCHITECTURE](../ARCHITECTURE.md#6-le-catalogue)) : les
relations d'un schéma quand on le déplie, les champs d'une table quand on
l'ouvre. Une complétion qui ne lit que le cache ne suggère rien sur une connexion
dont l'arbre n'a jamais été déplié — l'échec que l'[ADR-0036](0036-l-assistant-complete-le-catalogue.md)
a observé pour l'assistant (« 0 of 0 known relations » sur onze tables).
`catalog_tree` renvoie des nœuds sans champs, et `relation_detail` renvoie `None`
tant qu'une relation n'est pas chargée : il n'existe pas de lecture en bloc de ce
que le cache connaît.

**Les lectures du catalogue sont séquentielles et partagées.** L'exécuteur lit le
catalogue d'une connexion sur une session qui lui est réservée, sous un seul
verrou (`crates/oxyn-desktop/src/backend/ai/catalog_fill.rs`) : le dépliage de
l'arbre, la complétion de l'assistant et celle de l'éditeur feraient la queue sur
le même verrou. Le cache lui-même est borné — 1 024 portées et 50 000 objets de
métadonnées (`crates/oxyn-exec/src/catalog/budget.rs`) — et évince sous ce
budget : un nom connu il y a une minute peut avoir disparu.

**La webview ne peut pas savoir quelle connexion a gagné des noms.**
`RefreshSignal::of` (`crates/oxyn-desktop/src/ipc/metadata.rs`) n'émet
`catalogInvalidated { connection }` que pour une DDL terminée. L'événement
`CatalogUpdated` que publie un *chargement* parvient à la webview sous la forme
`catalogUpdated` sur le canal d'exécution, ce qui incrémente `catalogVersion`
(`apps/desktop/src/lib/ipc/events.ts`) : un compteur sans connexion.

**La bibliothèque compose du SQL à partir de noms reçus, et le fait mal.**
`@codemirror/lang-sql` 6.10.0 (lu dans les sources installées le 2026-10-21)
insère un nom nu quand il correspond à `^[a-z_][a-z_\d]*$`, et sinon l'entoure du
premier caractère de `identifierQuotes` du dialecte **sans doubler un guillemet
interne** (`nameCompletion`). Une relation nommée `a"b` est insérée `"a"b"` ; une
relation nommée `order` ou `user`, mots réservés de PostgreSQL 18
([annexe C](https://www.postgresql.org/docs/18/sql-keywords-appendix.html) :
« tokens that are not allowed as column or table names »), est insérée nue. Un nom
est une entrée hostile (`"users"; DROP TABLE audit; --` est légal sur
PostgreSQL). `oxyn-catalog::path::quote_identifier` double le caractère fermant
dans chaque style et constitue l'unique quoting du projet
([DRIVER-CONTRACT §6](../DRIVER-CONTRACT.md#6-il-échappe-tout-identifiant-quil-compose),
[I-10](../../CLAUDE.md#i-10)).

**PostgreSQL résout un nom non qualifié par un état que le front ne connaît
pas.** `search_path` est un état par connexion. Le driver exécute `SET
search_path TO "<schéma>"` avant une exécution quand la console a déclaré un
contexte, `SET search_path TO DEFAULT` sinon, et le remet à zéro avant que la
connexion retourne au pool, pour que l'introspection ne porte jamais le contexte
d'une console (`drivers/oxyn-driver-postgres/src/session.rs`). Une console sans
contexte déclaré est `serverDefault` (`console-panel.tsx`), et rien, ni dans le
front ni dans le driver, ne dit quel est ce défaut. La documentation du serveur
(PostgreSQL 18.6, lue le 2026-10-21) : le défaut est `"$user", public` ; une
entrée `$user` sans schéma de ce nom est ignorée ; `pg_catalog` est toujours
parcouru de fait ; `current_schemas(include_implicit)` renvoie les schémas du
chemin effectif « in their priority order », en omettant ceux qui n'existent pas
ou ne sont pas accessibles ; et un nom qui ne correspond à rien dans le chemin est
une erreur « even if matching table names exist in other schemas ».
[RESEARCH-NOTES](../RESEARCH-NOTES.md#complétion-de-léditeur--vérification-du-2026-10-21)
consigne ces sources.

[PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction) fixe deux chiffres qui
s'appliquent et aucun propre à la complétion : **100 ms** de retour visible après
une frappe, et une frame d'interface sous **8 ms** p99 pendant la saisie.

## Décision

**L'éditeur complète depuis le cache, et Oxyn lit lui-même ce qui manque au
cache, dans des bornes, jamais au rythme des frappes.**

1. **Une lecture en bloc du cache.** Une seule commande IPC en lecture seule,
   `completion_catalog` (nom fixé par la première pull request), de la famille de
   `catalog_tree` : construite dans `crates/oxyn-desktop/src/catalog.rs` à partir
   du cache, sur le pool bloquant, sans appeler de driver. Pour une connexion, elle
   renvoie les espaces de noms et, pour chacun, les relations que le cache détient
   — nature, `system`, `stale`, marques de table virtuelle et fantôme telles que
   `CatalogNode` les porte —, avec les champs de celles qui sont chargées et
   `fieldsLoaded: false` pour les autres, jamais une liste vide qui passerait pour
   « aucune colonne ». Elle renvoie aussi le chemin de résolution (4) et, si une
   borne l'a coupé, `truncated` avec les décomptes (6). L'éditeur la lit au
   montage et quand un signal (5) dit que le cache a changé, garde la réponse en
   mémoire, et la source de complétion lit cette copie : **aucune IPC n'est
   envoyée à une frappe.**
2. **Des chargements bornés, par le bus, pilotés par l'éditeur.** Après un court
   anti-rebond, jamais à une frappe, l'éditeur demande à l'hôte de compléter le
   cache pour ce que l'instruction sous le curseur nomme : les identifiants qu'il
   repère lexicalement après `FROM`, `JOIN`, `INTO`, `UPDATE`, `TABLE` et avant un
   `.`. L'hôte **résout ces noms en Rust** — le dialecte et le chemin de résolution
   décident, pas une seconde règle en TypeScript — et seulement parmi les
   relations que le cache a *listées* : une faute de frappe ou un nom que personne
   n'a listé ne coûte aucune requête et ne peut pas orienter une lecture vers un
   objet arbitraire. Par passe, il lit :
   1. à l'ouverture d'une console, et après invalidation de la racine, le niveau
      serveur, le chemin de résolution et les relations de chaque espace de noms de
      ce chemin jamais listé — au plus `MAX_COMPLETION_LISTINGS` ;
   2. les champs des relations résolues non chargées, ou `Invalidated` — au plus
      `MAX_COMPLETION_DESCRIPTIONS` ;

   le tout dans `COMPLETION_DEADLINE`, en constantes argumentées à côté de
   `catalog_fill` (valeurs fixées par la mesure, 6). Chaque lecture est une
   `Command::RefreshCatalogScope` passant par `ExecutorSink`, les commandes d'un
   dépliage d'arbre : `StatementIntent::Read`, autorisée partout, `production`
   comprise ([I-01](../../CLAUDE.md#i-01)) ; l'acteur est `Actor::Human`, puisque
   taper est un geste de l'utilisateur. Les lectures sont soumises **une à la
   fois**, jamais mises en file à l'avance, si bien qu'un dépliage d'arbre émis
   entre-temps attend derrière une lecture au plus, celle en cours ; une édition
   plus récente annule la passe par son `CancelToken`. Un niveau `Fetched` n'est
   pas relu, un niveau `Invalidated` l'est une fois, et une lecture en échec n'est
   pas retentée dans la passe. Rien ne tourne sur le fil de l'interface
   ([I-05](../../CLAUDE.md#i-05)). **Aucune nouvelle variante de `Command` n'est
   nécessaire pour cette partie** ; la commande IPC qui reçoit les noms est une
   enveloppe sur les commandes existantes.
3. **Le texte d'insertion est composé en Rust, une seule fois.** Chaque nom de
   l'instantané porte sa forme `insert`, produite par une fonction de
   `oxyn-catalog::path` à côté de `quote_identifier` : nu seulement quand le nom
   est un identifiant simple en minuscules **et** n'est pas un mot réservé du
   dialecte, cité avec `quote_identifier` sinon ; un chemin qualifié passe par
   `CatalogPath::qualify_sql`. La table des mots réservés de chaque dialecte est
   une donnée externe : elle est sourcée et datée dans
   [RESEARCH-NOTES](../RESEARCH-NOTES.md) par la pull request qui l'ajoute, et
   **un nom dont on ne sait pas qu'il est sûr est cité**. Le front donne à chaque
   `Completion` un `apply` explicite égal à `insert`, si bien que `nameCompletion`
   ne compose jamais de SQL. Si le modèle de schéma de `lang-sql` ne peut pas le
   porter, le front écrit sa propre source de complétion sur l'instantané — ce que
   décide un spike au début de la deuxième pull request, pas cet ADR. Un test du
   front insère `"users"; DROP TABLE audit; --`, `a"b`, `order` et un nom contenant
   `<img onerror=…>` sur chaque dialecte.
4. **Le chemin de résolution est lu du serveur, jamais supposé.** `CatalogCache`
   détient, pour la connexion, la liste ordonnée des espaces de noms dans lesquels
   un nom non qualifié se résout, ou « inconnu ». Une nouvelle variante de
   `CatalogRefreshScope` le lit avec le premier niveau du serveur, sur la session
   réservée au catalogue — qui est sur le `search_path` par défaut par construction
   (la remise à zéro décrite plus haut). Le driver PostgreSQL répond à partir du
   chemin effectif du serveur (`current_schemas`) ; un fournisseur qui ne sait pas
   répondre **refuse**, et le cache garde « inconnu », ce qui n'est pas une liste
   vide
   ([DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-il-déclare-ses-capacités-par-session-et-ne-simule-rien) ;
   la réponse de chaque driver y est spécifiée par la première pull request). Le
   chemin qu'utilise la complétion est le seul espace de noms déclaré par la
   console quand elle en a déclaré un — `SET search_path TO "<schéma>"` remplace le
   chemin — et le défaut du serveur sinon. Une relation est suggérée **non
   qualifiée** seulement si son espace de noms est dans ce chemin et qu'aucun
   espace de noms antérieur du chemin ne porte le même nom ; toute relation est
   suggérée qualifiée une fois son qualificatif tapé. Les espaces de noms système
   ne sont suggérés qu'après leur qualificatif. Quand le chemin est inconnu, rien
   n'est suggéré non qualifié : des noms qualifiés seulement, et **`public` n'est
   jamais supposé.** Un `SET search_path` tapé dans un script n'est pas suivi.
5. **Un signal de plus.** `RefreshSignal::of` fait correspondre
   `Event::CatalogUpdated` à un nouveau `catalogChanged { connection }` — « le
   contenu du cache a changé : chargement, éviction ou invalidation » —, remis
   comme les autres aux fenêtres qui détiennent la connexion
   ([ADR-0043](0043-multi-fenetre.md)). L'éditeur relit l'instantané sur
   `catalogChanged` et `catalogInvalidated`, de façon coalescée. Le
   `catalogVersion` global n'est pas utilisé, et le cache n'a pas de compteur de
   révision : les signaux disent déjà ce qui a changé. Après une DDL, les noms
   `stale` restent suggérés, signalés, pendant que la prochaine passe de l'éditeur
   les relit : l'exécuteur invalide tout le catalogue après n'importe quelle DDL,
   et vider la complétion après un `CREATE INDEX` serait pire que suggérer un nom
   un instant de plus. Un nom part quand une relecture ne le liste plus, ou quand
   le cache l'évince.
6. **Des bornes mesurées avant d'être écrites.** Cet ADR n'écrit aucun chiffre
   pour `MAX_COMPLETION_LISTINGS`, `MAX_COMPLETION_DESCRIPTIONS`,
   `COMPLETION_DEADLINE` ni pour les plafonds de l'instantané. La première pull
   request les mesure avec [`/benchmark`](../../claude/commands/benchmark.md) sur
   un catalogue au budget du cache (50 000 objets), contre les deux budgets
   existants (100 ms, 8 ms p99), consigne le résultat dans PERFORMANCE et écrit
   les constantes avec leur argument. Un instantané coupé par un plafond le dit
   (`truncated`, avec les décomptes de ce qui a été laissé de côté), et la liste de
   complétion le dit aussi — la formulation va dans UX-SPEC à la deuxième pull
   request. **Une réponse partielle n'est jamais présentée comme complète.**
7. **Ce qu'elle ne promet pas.** Les colonnes sont proposées pour les relations que
   l'instruction nomme selon la règle lexicale ci-dessus, avec leurs alias ; les
   expressions de table communes, les sous-requêtes, `RETURNING`, les fonctions de
   table et le SQL dynamique ne sont pas inférés, et ce que la règle ne sait pas
   résoudre reçoit des relations et des mots-clés, jamais une colonne devinée.
   L'instantané va à la webview et nulle part ailleurs — pas à `oxyn-ai`, qui garde
   sa passerelle unique ([I-04](../../CLAUDE.md#i-04)) —, contient des noms et des
   natures, aucune ligne, et n'est pas persisté ([I-11](../../CLAUDE.md#i-11)).

## Conséquences

* **+** La première frappe sur une connexion jamais dépliée suggère déjà les tables
  du schéma, et `alias.` suggère les colonnes d'une table que personne n'a ouverte.
* **+** Un nom inséré est correct sur chaque dialecte : guillemets doublés, mots
  réservés cités, un nom hostile inerte. Un seul quoting, en Rust, testé une fois.
* **+** PostgreSQL suggère ce qui se résout : les noms non qualifiés du vrai chemin,
  et rien d'inventé quand le chemin est inconnu.
* **+** Aucune IPC à une frappe ; les chargements réutilisent les commandes, la
  politique et le journal du dépliage de l'arbre, et aucune variante de `Command`
  n'est ajoutée pour eux.
* **−** **Oxyn lit le serveur là où personne n'a cliqué**, y compris une base de
  production : borné par passe, mais une console laissée ouverte et éditée produit
  des requêtes de catalogue toute la journée. C'est ce qui rend la décision chère à
  défaire pour qui audite un serveur.
* **−** Le journal d'audit gagne une entrée par niveau lu, comme dans l'ADR-0036.
* **−** L'éditeur, l'arbre et l'assistant partagent une session et un verrou : les
  lectures de l'éditeur peuvent retarder un dépliage d'arbre d'une lecture, et
  remplissent plus vite le cache vers son budget de 50 000 objets, donc évincent
  plus tôt.
* **−** Une table de mots réservés par dialecte à tenir à jour, sourcée et datée :
  un mot réservé par une version ultérieure du serveur et absent de la table est
  inséré nu, et le serveur rejette l'instruction en nommant ce mot — un échec
  visible, pas silencieux.
* **−** Modifie la surface du contrat : une variante de `CatalogRefreshScope` dans
  `oxyn-core`, une méthode optionnelle du fournisseur dans `oxyn-catalog`, une
  réponse de chaque driver, une variante de signal, les sections de DRIVER-CONTRACT
  et d'ARCHITECTURE qui les listent.
* **−** Le chemin par défaut est lu une fois par connexion : un `ALTER ROLE … SET
  search_path` fait entre-temps n'est vu qu'au prochain rafraîchissement de la
  racine.
* **−** Une copie de l'instantané vit dans la mémoire de la webview, bornée par les
  plafonds de (6).
* **−** Si `lang-sql` ne peut pas porter l'`apply` explicite, le front possède une
  source de complétion : plus de code qu'une option de configuration.

**Coût de sortie :** faible côté front — la commande d'instantané, l'adaptateur et
les chargements sont isolés, et les retirer ramène l'éditeur à la complétion de
mots-clés. Le chemin de résolution, le signal `catalogChanged` et la fonction
d'insertion restent utiles sans elle (l'arbre, « Copy as… »). Le vrai coût est
ailleurs : les utilisateurs auront cessé de déplier l'arbre pour obtenir leurs
noms.

**À reconsidérer si** une connexion doit interdire toute lecture serveur que
personne n'a demandée par un clic (un serveur audité) : l'ADR-0036 nomme déjà cette
condition, et un marquage de connexion lu par le `PolicyGate` ferait alors passer
l'éditeur en cache seul — ce que la conception permet, puisque le chargement est
une couche séparée ; si la complétion mesurée dépasse les budgets de 100 ms ou de
8 ms sur un catalogue au budget du cache sans qu'aucun plafond ne la maintienne en
dessous ; ou si `@codemirror/lang-sql` cesse d'être le paquet de langage de
l'éditeur.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Compléter depuis le cache seul | Vide sur une connexion dont l'arbre n'a jamais été déplié — l'échec observé de l'ADR-0036, pour l'éditeur cette fois |
| Introspecter tout le catalogue à la connexion | Coûte à tout le monde, et une base de 10 000 tables dépasse le budget du cache (l'ADR-0036 a écarté la même chose) |
| Interroger le serveur à chaque frappe, ou depuis la webview | Viole [I-01](../../CLAUDE.md#i-01) et [I-05](../../CLAUDE.md#i-05) ainsi que le budget de 100 ms ; une faute de frappe coûterait une requête |
| Laisser `lang-sql` citer ce qu'il insère | Ne double pas un guillemet interne et insère nus les mots réservés (lu dans la 6.10.0) : du SQL composé à partir d'un nom reçu, [I-10](../../CLAUDE.md#i-10) |
| Citer en TypeScript | Une seconde implémentation de DRIVER-CONTRACT §6, qui diverge |
| Citer tous les noms insérés | Sûr, et cela évite la table de mots réservés, mais `FROM "users"` pour chaque table fait que les utilisateurs cessent d'accepter les suggestions |
| Supposer `public` sur PostgreSQL | Le chemin peut différer, et un nom qui n'existe que dans un schéma hors du chemin est une erreur pour le serveur : la complétion suggérerait des noms qui ne se résolvent pas |
| Lire le chemin sur la session de la console | Cette session peut être en cours d'exécution ou dans une transaction, et `search_path` est par connexion ; la session réservée au catalogue est sur le défaut par construction |
| Résoudre les noms de l'instruction en TypeScript | La normalisation de casse et le quoting dépendent du dialecte et du serveur ; une seconde règle à côté de celle de Rust |
| Un compteur de révision dans le cache | Les signaux disent déjà ce qui a changé ([ADR-0022](0022-rafraichissement-automatique.md)) ; un compteur est un état à tenir synchronisé |
| Continuer à utiliser le `catalogVersion` global | Pas de connexion : chaque éditeur relirait à chaque changement de n'importe quelle connexion |
| Vider la complétion sur `stale` | L'exécuteur invalide tout le catalogue après n'importe quelle DDL : la complétion deviendrait vide après chaque `CREATE INDEX` |
