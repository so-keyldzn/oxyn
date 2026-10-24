<!-- oxyn-translation source="docs/adr/0065-agents-for-non-sql-query-languages.md" sha256="a5d0fdde2583" -->

> Traduction française de [docs/adr/0065-agents-for-non-sql-query-languages.md](../../../../docs/adr/0065-agents-for-non-sql-query-languages.md). **La version anglaise fait foi.**

# ADR-0065 — Un agent déclare les langages de requête qu'il écrit, et l'assistant s'ouvre sur tout langage qu'un agent écrit

**Statut :** acceptée · **Date :** 2026-10-23

**Précise :** [ADR-0049](0049-agents-declared-as-markdown-files.md), sur
quatre points : la cible d'un prompt est un langage de requête et pas
seulement un dialecte SQL (§ 10), un fichier d'agent gagne un champ
`languages` (§ 2), la liste fermée des variables gagne `{{language}}` (§ 3),
et une connexion dont le langage n'est pas SQL lit un fragment de langage là
où une connexion SQL lit celui de son dialecte (§ 8, § 9).
[ADR-0055](0055-sql-agent-is-the-default-of-a-new-conversation.md), sur un
point : l'agent que lance une nouvelle conversation sur une connexion où
l'agent SQL n'est pas proposé. Tout le reste des deux ADR reste en vigueur.

## Contexte

Trois drivers sont écrits en parallèle pour des moteurs qui ne parlent pas
SQL — Redis (`QueryLanguage::RedisCommand`), MongoDB (`MongoQuery`) et
Elasticsearch (`SearchDsl`). Leurs matrices de parité ont des lignes IA : la
barre est que l'assistant marche sur chacun comme sur PostgreSQL.
L'[ADR-0064](0064-a-session-classifies-non-sql-requests.md) a donné à leurs
sessions un moyen de classer une requête, et `oxyn_query::console_language`
le langage que parle une console ; l'assistant n'avait ni l'un ni l'autre.
Sur `main` avant ce changement :

* `AgentSpec::applies_to` est un `Vec<SqlDialect>` et `PromptTarget::dialect`
  un simple `SqlDialect` (`crates/oxyn-ai/src/spec.rs`,
  `crates/oxyn-ai/src/agent_file/target.rs`) : un agent ne peut pas dire qu'il
  écrit autre chose que du SQL, et un prompt ne peut pas être rendu pour autre
  chose ;
* `ai_ask` et `ai_start_agent` refusent une session sans
  `Capabilities::SQL` (`crates/oxyn-desktop/src/backend/ai/conversation.rs`,
  `conversation/startup.rs`), et le panneau ferme l'entrée de la même façon
  (`apps/desktop/src/features/assistant/availability.ts`) ;
* six endroits de `conversation.rs` construisent la portée des outils et le
  contexte avec `QueryLanguage::Sql(dialect_for(driver))`, quoi que parle la
  session ;
* les sept fragments de destinataire disent à tout modèle d'étiqueter ses
  instructions `sql`.

En l'état, chaque développeur de driver aurait modifié `oxyn-ai` et
`oxyn-desktop` de son côté : trois réécritures des mêmes fichiers, trois
façons de dire à un modèle quel langage écrire. Cet ADR est le changement
commun ; chaque driver livre ensuite un fichier d'agent et rien d'autre.

## Décision

### 1. La cible d'un prompt est un langage

`PromptTarget` porte `language: QueryLanguage` à la place de
`dialect: SqlDialect` ; `PromptTarget::dialect()` vaut
`language.sql_dialect()`. Le langage est celui que parle la console de la
connexion, `oxyn_query::console_language(driver, capabilities)` :

* pour une question ou le démarrage d'un agent externe, d'après les capacités
  que déclare la **session** (`Backend::session_language`) ; une session qui
  ne déclare aucun langage unique refuse l'assistant, comme elle refuse une
  console ;
* pour le sélecteur d'agent, qui ne nomme aucune session, d'après les
  capacités que le **driver** déclare au mieux (`Driver::capabilities`). Une
  session les restreint, jamais vers un autre langage, et une question
  revérifie auprès de la session.

Cette valeur unique est le langage du prompt, du contexte
(`ContextBuilder::with_language`), de la `ToolScope` dans laquelle chaque
appel d'outil est traduit, et donc de toute requête qu'un agent envoie.

### 2. `languages` dans un fichier d'agent

`AgentSpec` gagne `languages: Vec<QueryLanguage>`, lu et écrit sous les noms
de `QueryLanguage::as_str` : `sql`, `cypher`, `gremlin`, `mongo`, `redis`,
`search-dsl`, `cql`, `partiql`, `influxql`, `flux` (`agent_file::LANGUAGES`).

* **Absent vaut `[sql]`.** Tout fichier d'agent écrit avant cet ADR, et les
  deux agents livrés, gardent leur sens ; une déclaration qui ne le fixe pas
  ne le sérialise pas non plus.
* Une liste explicitement **vide** est refusée (un agent proposé sur aucune
  connexion), tout comme un `applies_to` non vide sans `sql` parmi
  `languages` : les dialectes n'existent qu'en SQL. Un nom inconnu est refusé
  avec sa ligne, comme un dialecte inconnu.
* `offered_for` ajoute une condition : le langage de la cible est listé. Sur
  une cible SQL, `applies_to` restreint toujours par dialecte ; sur toute
  autre cible il n'est pas lu. L'agent SQL n'est donc jamais proposé sur une
  session Redis, et un agent Redis jamais sur une session SQL.

Comme `applies_to` et `recipients`, `languages` restreint où un agent est
proposé et n'accorde rien : les outils, le `PolicyGate` et le niveau sont les
mêmes quel que soit le langage.

### 3. `{{language}}`, et `{{dialect}}` hors SQL

La liste fermée des variables devient `dialect`, `driver`, `environment`,
`language`, `recipient`. `{{language}}` est le `QueryLanguage::as_str` de la
cible : une valeur qu'Oxyn tient, issue d'un ensemble fermé, jamais un contenu
de la base, une réponse du serveur ou un secret — la règle de
l'ADR-0049 § 3. Hors SQL, `{{dialect}}` reçoit le nom du langage plutôt que
de rester vide.

### 4. La composition hors SQL

```text
rendered = role body + "\n\n" + language fragment + "\n\n" + recipient fragment
```

Sur une cible SQL, le fragment de langage **est** le fragment du dialecte,
choisi comme avant. Sur toute autre cible, c'est
`prompts/languages/<language>.md` quand il est écrit (`LANGUAGE_FRAGMENTS`),
et `prompts/languages/generic.md` sinon. `generic.md` ne porte aucune
connaissance de produit : il dit que le langage n'est pas SQL, qu'Oxyn ne
l'analyse pas, qu'une requête ne part tout de suite que si le driver prouve
qu'elle ne fait que lire et compte comme une écriture sinon — retenue pour
approbation, refusée sur `production` —, que chaque requête est isolée, et
que les noms s'écrivent comme le contexte les épelle.

Les fragments de destinataire étiquettent une instruction proposée avec
`{{language}}` au lieu du littéral `sql`. Sur une cible SQL la variable rend
`sql` : **tout prompt rendu pour une cible SQL est, à l'octet près, le prompt
rendu avant cet ADR** — vérifié sur les 308 combinaisons des deux agents
livrés × 11 dialectes × 7 destinataires × 2 environnements au moment de
l'implémentation.

### 5. Ce qu'un agent envoie est classé selon son langage, et passe la porte

Rien de nouveau n'est ajouté sur ce chemin ; il est énoncé parce que c'est lui
qui rend le reste sûr ([I-07](../../../../CLAUDE.md#i-07)). `execute_query` se
traduit en `Command::Execute` dans le langage de la portée, portant
`Actor::Agent`. `oxyn-exec` la reclasse avant le `PolicyGate` : `oxyn-query`
pour SQL, le `classify_intent` de la session pour tout autre langage
(ADR-0064), `Unknown` sinon, qui compte comme mutant. Sur `production`,
l'écriture d'un agent — commande inconnue comprise — est un **refus**
([I-02](../../../../CLAUDE.md#i-02)) ; une lecture prouvée part avec
`limits.read_only` ; ailleurs une écriture attend l'approbation de
l'utilisateur.

### 6. Où l'assistant s'ouvre, et quel agent lance une nouvelle conversation

L'assistant est proposé sur une connexion dès qu'au moins un agent valide —
livré ou de l'utilisateur — est proposé pour son langage. Une session SQL a
toujours l'agent SQL livré ; une session d'un autre langage en a un quand un
driver livre son agent, ou quand l'utilisateur en écrit un.

Une nouvelle conversation qui ne nomme aucun agent lance
`AgentCatalog::default_for` :

* l'agent SQL quand il est proposé pour la cible — toute connexion SQL,
  exactement comme le décide l'ADR-0055 ;
* sinon le premier agent valide que le sélecteur liste pour la cible : les
  agents livrés d'abord, puis ceux de l'utilisateur, chacun par nom. Un
  fichier de l'utilisateur ne déloge donc jamais l'agent qu'Oxyn livre pour
  un langage ;
* aucun : la question est refusée, en nommant le langage qu'aucun agent
  n'écrit.

Le panneau sélectionne le même agent dans un fil vierge (`defaultAgentId`). La
liste qu'il détient est l'union sur toutes les destinations déclarées, alors que
`default_for` choisit pour l'unique destinataire où va la question : le panneau
choisit donc parmi les agents écrits pour une destination que le palier de la
connexion laisse utilisable, celle où il enverrait en premier faisant foi. Un
agent offert seulement là où le palier refuse est écarté, un autre agent étant
disponible.

### 7. Le contexte garde son point de passage unique

Le schéma, les échantillons approuvés, les mentions `@` et les blocs `erd`
d'une connexion qui ne parle pas SQL passent par `ContextBuilder`, sous le
niveau de la connexion, comme pour SQL ([I-04](../../../../CLAUDE.md#i-04),
[ADR-0006](0006-ai-privacy-tiers.md)). `ContextBuilder` énonce déjà le
langage de requête dans son en-tête et cite les noms selon lui ; les genres
d'objets du catalogue sont ceux que déclare le driver — clés et motifs de
clés, collections, index. Aucun chemin n'est ajouté.

### 8. Ce qu'un driver ajoute

Un driver qui veut l'assistant sur son langage livre
`crates/oxyn-ai/agents/<name>.md` avec `languages: [<language>]` et l'ajoute
à `AGENTS` dans `crates/oxyn-ai/src/agent_file/files.rs` — une ligne. Un
fragment de langage partagé par plusieurs agents est facultatif, et tient en
une ligne dans `LANGUAGE_FRAGMENTS`. Aucun changement du desktop ni du front
n'est nécessaire.

## Conséquences

* **+** Un seul mécanisme pour tous les langages : Redis, MongoDB et
  Elasticsearch obtiennent l'assistant en écrivant chacun un fichier, sans
  code partagé à fusionner trois fois.
* **+** SQL n'est pas touché : mêmes agents proposés, même défaut, mêmes
  prompts à l'octet, mêmes fichiers d'agent.
* **+** Le langage qu'écrit un agent, celui dans lequel ses requêtes
  s'exécutent et celui selon lequel l'exécuteur les classe sont une seule
  valeur lue dans la session ; ils ne peuvent pas diverger.
* **+** Un utilisateur peut apporter l'assistant à un langage pour lequel
  Oxyn ne livre aucun agent, sous la même porte.
* **−** Hors SQL, un modèle ne reçoit d'Oxyn que `generic.md` tant que le
  fichier d'agent du driver n'en dit pas plus : ce que le produit accepte est
  à écrire par l'auteur de l'agent, et un agent maigre donne des réponses
  maigres.
* **−** La garde de production d'une requête d'agent dans ces langages repose
  sur le `classify_intent` du driver (ADR-0064) : un `Read` revendiqué à tort
  laisse un agent y exécuter une écriture.
* **−** Le sélecteur liste selon les capacités déclarées par le driver, une
  question selon celles de la session. Un driver qui déclarerait un langage
  que sa session abandonne ensuite montrerait un agent que la question
  refuse ; aucun des drivers livrés ou prévus ne le fait.
* **−** Hors SQL, le défaut dépend de l'ordre du catalogue : deux agents
  livrés pour le même langage se départagent par nom.
* **−** Une conversation dont l'agent enregistré a disparu se replie toujours
  sur l'agent SQL (ADR-0049 § 6) ; sur une session d'un autre langage, ce
  repli est refusé plutôt que remplacé par le défaut du langage.
* **−** « Open in console » côté front ne reconnaît encore qu'un bloc étiqueté
  `sql` ou sans étiquette ; un bloc étiqueté `redis` n'offre que « Copy »
  tant que le panneau ne connaît pas le langage de la session.

**Coût de sortie :** modéré. Revenir en arrière, c'est retirer `languages`
d'`AgentSpec` — tout fichier d'agent qui l'a fixé, ceux des drivers compris,
ne se lit plus —, `{{language}}` des fragments de destinataire et de la liste
des variables, `generic.md`, et la branche de langage du rendu ;
`PromptTarget` revient à un dialecte. Ce qui le borne : `languages` absent
vaut `[sql]`, donc les fichiers d'agent SQL ne portent jamais le champ, et les
prompts SQL n'en ont jamais dépendu.

**À reconsidérer si** un langage a besoin de notes par version qu'un seul
fragment ne peut tenir (ce qui plaiderait pour un axe langage × version, comme
les dialectes pour SQL), si deux agents livrés écrivent un jour le même
langage (le défaut devrait alors être nommé, pas ordonné), ou si une session
peut parler deux langages en plus de SQL à la fois, ce que
`console_language` refuse aujourd'hui.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Chaque driver modifie `oxyn-ai` et `oxyn-desktop` pour son propre langage | Trois réécritures concurrentes des mêmes fichiers et trois façons de nommer un langage à un modèle ; la deuxième et la troisième entreraient en conflit avec la première |
| Surcharger `applies_to` de noms de langages (`applies_to: [redis]`) | Mélange deux axes dans une liste : `[postgres, redis]` voudrait dire « le SQL de PostgreSQL, ou tout Redis », et « tous les dialectes » (vide) voudrait aussi dire « tous les langages » |
| Un `languages` vide veut dire tous les langages, comme un `applies_to` vide veut dire tous les dialectes | Un agent écrit pour SQL avec une liste vide serait proposé sur Redis et lui écrirait du SQL ; pour les langages, le défaut utile est SQL, pas « tout » |
| Garder `PromptTarget::dialect` et ajouter un langage facultatif à côté | Deux champs qui peuvent diverger ; le dialecte est une propriété du langage SQL, et `QueryLanguage::Sql(dialect)` le dit déjà |
| Un fragment `prompts/dialects/redis.md` | Les dialectes sont ceux de SQL ; un fragment Redis sous ce nom serait lu comme un dialecte SQL par le prochain lecteur et par `dialect_named` |
| Aucun fragment de langage hors SQL | Le modèle ne serait pas averti, dans les mots d'Oxyn, qu'Oxyn n'analyse pas le langage et traite une commande inconnue comme une écriture — la seule règle qui compte sur production |
| Garder l'étiquette littérale `sql` dans les fragments de destinataire | Un agent Redis à qui son rôle dit d'écrire des commandes Redis et le fragment de destinataire de les étiqueter `sql` reçoit deux consignes contradictoires dans un même prompt |
| L'assistant s'ouvre sur un langage d'après une liste dans le front (`ASSISTED_LANGUAGES`) | Une liste dans le code est périmée le jour où un driver livre son agent, et ne voit pas l'agent d'un utilisateur ; le catalogue sait déjà ce qui est proposé |
| L'agent SQL reste le défaut partout, et une session non SQL demande à l'utilisateur de choisir | Toute nouvelle conversation sur Redis commencerait par un refus ; la raison de l'ADR-0055 — un défaut que l'utilisateur n'a pas choisi ne doit pas changer avec la liste — tient en préférant les agents livrés |
| Livrer ici les agents Redis, MongoDB et Elasticsearch | La connaissance du produit appartient à qui connaît chaque driver, et se relit avec lui ; ce changement livre le mécanisme et une fixture de test seulement |
