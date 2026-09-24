# ADR-0036 — L'assistant complète lui-même le catalogue, par le bus et sous des bornes

**Statut :** proposé · **Date :** 2026-09-24

**Précise :** [ADR-0006](0006-ai-privacy-tiers.md), sur le point suivant : ce que
`Metadata` laisse sortir n'est plus « ce que le cache contient », mais « ce que
la question retient », lu du serveur s'il le faut.

**Précise :** [ADR-0030 § 4 bis](0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-la-structure-de-la-base--un-outil-pour-toutes-les-destinations),
sur le point suivant : `Command::DescribeCatalog` reste une lecture du cache qui
ne contacte aucun serveur, mais l'appel `describe_schema` est désormais précédé
des lectures de métadonnées qui manquent, chacune sa propre commande.

## Contexte

Le catalogue se charge **à la demande**, et c'est voulu : une base de 50 000
relations ne s'introspecte pas à la connexion
([ARCHITECTURE](../ARCHITECTURE.md#6-le-catalogue)). À l'ouverture, l'exécuteur ne
lit au plus que le serveur et son premier palier ; la liste des tables d'un
schéma est lue quand l'utilisateur **déplie** ce schéma
(`CatalogRefreshScope::Relations`), les champs d'une table quand il l'**ouvre**.

L'IA lit le même cache, et rien d'autre : c'est ce qui rend le point de passage
unique vérifiable ([I-04](../../CLAUDE.md#i-04)). Conséquence constatée par
l'utilisateur le 2026-09-24 : sur une base SQLite de onze tables jamais dépliée,
l'agent reçoit « Describing 0 of 0 known relations », propose
`SELECT * FROM your_table`, et `describe_schema` rend le même vide. Le diagramme
`erd` et la liste des mentions `@` disent « not found » pour une table non
dépliée. `refresh_catalog` ne relit que la racine : il ne répare rien.

Ce que l'IA sait d'une base dépend donc de ce que l'utilisateur a cliqué, pour
toutes les destinations — fournisseur, agent externe — et toutes les bases.

L'ordre de grandeur qui fixe les bornes : un PostgreSQL distant à 50 ms d'aller-
retour décrit une table en trois requêtes (description, index, clés
étrangères), soit environ 150 ms ; vingt-quatre tables, 3,6 s. Ce chiffre est
une hypothèse de dimensionnement, pas une mesure.

## Décision

**Avant de construire le contexte d'une question, et quand `describe_schema` est
appelé, Oxyn lit lui-même ce qui manque au cache — sans modèle.**

1. **Ce qui est lu.** Le serveur s'il n'a jamais été lu ; la liste des relations
   de chaque schéma jamais listé ; puis les champs, index et clés étrangères des
   relations que la porte **retiendra** — mentions `@` d'abord, puis la
   recherche orientée par la question. Une question qui suit une session déjà
   informée ne lit que ses mentions. `refresh_catalog` relit le serveur puis
   **reliste** chaque schéma, frais ou non ; il ne décrit rien. Le premier `@`
   du panneau sur une connexion (`ai_list_mentionable`) lit le serveur et
   liste les schémas jamais listés, sans rien décrire : la liste des mentions
   ne propose que ce que le cache nomme, et attendre la première question
   pour la remplir revenait à demander à l'utilisateur de déplier l'arbre.
2. **La sélection est celle de la porte.** `oxyn_ai::context::wanted_relations`
   est la fonction que `ContextBuilder::build` appelle pour choisir ; l'hôte
   l'appelle pour charger. Une seule sélection : deux divergeraient.
   `CatalogCache::listing_scopes` (`oxyn-catalog`) dit quels paliers nomment les
   relations — le catalogue de la session seulement, sans les schémas système.
3. **Par le bus.** Chaque lecture est une `Command::RefreshCatalogScope` (ou
   `RefreshCatalog` pour le serveur), soumise par `ExecutorSink` : le
   `PolicyGate` décide, le journal inscrit ([I-01](../../CLAUDE.md#i-01),
   [ADR-0004](0004-command-bus.md)). Ce sont les commandes d'un dépliage de
   l'arbre : lectures de métadonnées, `StatementIntent::Read`, permises partout,
   `production` comprise. Une demande d'approbation, si une politique future en
   levait une, est retirée et le palier reste non lu.
4. **L'acteur est celui qui est à l'origine.** Pour une question et pour le
   premier `@`, `Actor::Human` : poser la question, taper `@`, sont des gestes
   de l'utilisateur, et ce
   qu'elle fait lire est décidé par Oxyn à partir de ses mots et de ses
   mentions — aucun modèle n'a rien demandé. Pour `describe_schema` et
   `refresh_catalog`, `Actor::Agent` de l'agent, par son puits lié
   (`ExecutorSink::for_agent`) : c'est l'appel d'outil du modèle qui cause la
   lecture ([I-07](../../CLAUDE.md#i-07)).
5. **Des bornes, écrites en constantes argumentées** dans
   `crates/oxyn-desktop/src/backend/ai/catalog_fill.rs` : 32 schémas listés,
   24 relations décrites — `ContextPolicy::max_relations`, tenu égal par un
   test —, 5 secondes au total. Les lectures sont **séquentielles** :
   l'exécuteur lit le catalogue d'une connexion sur sa session réservée, sous
   un verrou ; les paralléliser les ferait attendre ce verrou.
6. **Un dépassement n'échoue pas.** Le contexte part avec ce qui est chargé.
   L'encadré dit ce qui manque, compté depuis le cache par la porte elle-même,
   quelle qu'en soit la raison : « N relations not loaded yet », « N schemas not
   listed yet », « the catalog of this connection has not been read yet ». Des
   comptes sans nom, identiques sous tout niveau.
7. **Rien n'est relu.** Un palier `Fetched` ne se relit pas ; `Invalidated`
   (après un DDL émis par Oxyn) se relit une fois. Une lecture en échec n'est
   pas retentée dans la même complétion.
8. **Annulable et hors du thread d'interface.** Le jeton de la question est le
   parent de celui de chaque lecture ; le délai annule la lecture en cours et
   l'attend deux secondes pour qu'elle finisse proprement
   ([I-05](../../CLAUDE.md#i-05)).
9. **Le panneau montre l'étape** : `AiEvent::CatalogReading` avant la première
   lecture, `AiEvent::CatalogRead` et ses comptes après
   ([UX-SPEC](../UX-SPEC.md#le-panneau-montre-ce-qui-se-passe-y-compris-quand-rien-narrive)).

## Conséquences

* **+** L'IA voit la base, pas les clics : une question sur une connexion jamais
  dépliée reçoit ses tables et leurs colonnes, pour toute destination.
* **+** Les autres lecteurs du cache en profitent sans code : l'arbre par
  `CatalogUpdated`, les mentions `@`, le diagramme `erd`.
* **+** Un schéma partiel se dit : le modèle n'invente plus la table qu'il ne
  voit pas, et l'utilisateur lit ce qui manque dans le panneau.
* **−** **L'IA déclenche des lectures sur le serveur que personne n'a cliquées**,
  y compris sur une base de production : jusqu'à une soixantaine de requêtes de
  catalogue par question sur PostgreSQL. C'est ce qui rend la décision coûteuse
  à défaire du point de vue de qui audite le serveur.
* **−** Le journal d'audit grossit d'une entrée par palier lu.
* **−** La première question sur une base inconnue attend jusqu'à cinq
  secondes avant de partir.
* **−** `describe_schema` contacte désormais le serveur ; sa description le dit
  au modèle.
* **−** Une lecture refusée par le serveur (une table sans droit) est retentée à
  chaque question qui la retient — une fois par question, sous les mêmes bornes.
* **−** Les schémas système ne sont jamais listés pour l'IA : une question sur
  `pg_catalog` suppose que l'utilisateur l'a déplié.

**Coût de sortie :** faible dans le code — le module `catalog_fill`, trois
appels et deux variantes d'`AiEvent` ; l'annonce de ce qui manque et
`wanted_relations` restent utiles sans lui. Le coût réel est ailleurs : revenir
en arrière rend à l'IA sa dépendance aux clics, que les utilisateurs auront
cessé de compenser.

**Reconsidérer si** une connexion doit interdire toute lecture serveur faite pour
l'IA (un serveur audité où chaque requête de catalogue compte) — il faudrait
alors un marquage de connexion lu par le `PolicyGate` ; ou si le délai de cinq
secondes est atteint couramment sur des bases réelles, mesure à l'appui ; ou si
un chargement de fond du catalogue apparaît, qui rendrait cette complétion
redondante.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Introspecter tout le catalogue à la connexion | Coûte à tous, IA ou non — l'IA absente ne doit rien coûter ([ADR-0006](0006-ai-privacy-tiers.md)) ; une base de 10 000 tables dépasse le budget du cache (50 000 objets) |
| Laisser le modèle appeler `refresh_catalog` | Dépend du modèle, consomme ses tours, et un agent externe ne sait pas qu'il le faut ; c'est la panne constatée |
| `oxyn-ai` interroge le driver | Contourne le bus et la porte ([I-01](../../CLAUDE.md#i-01), [ADR-0004](0004-command-bus.md)) |
| `Actor::Agent` pour les lectures d'une question | L'identité d'un agent externe naît à son lancement, **après** la composition de son invite ; et le journal attribuerait à un agent des lectures qu'aucun modèle n'a demandées |
| Lectures en parallèle | L'exécuteur les sérialise sur la session réservée au catalogue : aucun gain, et des attentes empilées sur un verrou |
| Retenir les lectures en échec d'une question à l'autre | Un état de plus, sans borne naturelle ni règle d'oubli ; l'échec coûte une requête par question, borné par le délai |
