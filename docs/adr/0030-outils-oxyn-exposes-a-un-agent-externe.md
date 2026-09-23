# ADR-0030 — Un agent externe atteint la base par les outils d'Oxyn, servis en MCP, et par rien d'autre

**Statut :** proposé · **Date :** 2026-09-16

**Précise :** [ADR-0026](0026-agents-externes-acp.md), qui déclarait un agent
externe et lui parlait, sans jamais lui donner de quoi lire la base.

## Contexte

[ADR-0026](0026-agents-externes-acp.md) a posé le mode agent : l'utilisateur
lance Claude Code ou Codex depuis Oxyn, l'agent porte sa propre
authentification, aucune clé ne nous est confiée. Ce que cet ADR n'a pas traité,
parce qu'il regardait la frontière du protocole et non celle du produit :
**l'agent ne peut pas interroger la base**.

Le résultat est un agent qui, dans un atelier de bases de données, ne peut rien
dire de la base. Il répond sur le SQL en général, pas sur le schéma ouvert
devant l'utilisateur. Le retour est sans appel : « le chat et les agents ne
discutent pas avec les bases de données ».

L'assistant interne, lui, y arrive : le modèle demande un outil, `oxyn-ai`
traduit l'appel en `Command` portant `Actor::Agent`, le `PolicyGate` décide, le
bus exécute, et le résultat revient au modèle **encadré**. Tout le mécanisme
existe. Ce qui manque n'est pas une capacité : c'est un **transport** entre
l'agent, qui vit dans un autre processus, et ce mécanisme.

Le protocole en offre un, et c'est précisément celui que les agents savent déjà
parler : **MCP**. `agent-client-protocol` 2.1.0 permet de déclarer des serveurs
MCP à l'ouverture d'une session ACP. L'agent y voit des outils comme il en voit
partout ailleurs ; Oxyn y voit son propre registre.

Reste à choisir le transport, et c'est là que la mesure a corrigé l'intuition.
La crate offre une variante `Acp` qui porte le serveur **en mémoire**, sans
processus ni port — évidemment la plus désirable. Elle est inutilisable : elle
dépend d'une capacité que **ni Claude Agent 0.78.0 ni Codex 1.12.0 n'annoncent**
(Codex répond même `"acp": false`), et elle vit derrière une feature instable.
Le relevé est daté dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#exposer-les-outils-doxyn-à-un-agent-externe--vérification-du-2026-09-16).
Les transports réellement disponibles sont `stdio` — obligatoire pour tout agent
— et `http`, que les deux acceptent.

**Ce qui rend la décision coûteuse à défaire**, et donc ADR : servir des outils
à un processus tiers crée une **seconde porte d'entrée** vers le bus de
commandes. Une porte d'entrée, une fois ouverte, n'est jamais re-auditée comme
la première — c'est exactement ce qu'[I-01](../../CLAUDE.md#i-01) décrit. La
forme qu'on lui donne aujourd'hui est celle qu'on gardera.

## Décision

### 1. Le serveur MCP n'expose que le `ToolRegistry`, tel quel

Les outils servis à l'agent externe sont **exactement** ceux que l'assistant
interne reçoit du `ToolRegistry` d'`oxyn-ai` — aujourd'hui `execute_query`,
`describe_schema` (§ 4 bis) et `request_sample` (§ 4 ter) —
avec leurs schémas JSON d'arguments, produits par le même code.

**Aucun outil n'est écrit pour l'agent externe.** Un outil qui n'existerait que
là serait, par construction, un chemin que l'assistant interne n'emprunte jamais
et que personne ne relit. Ajouter un outil aux agents externes, c'est l'ajouter
au registre, donc aux deux destinations, donc sous la même relecture.

Le revers : un outil ajouté pour l'assistant interne atteint aussi, d'office,
tout agent externe. La liste annoncée est donc **figée par un test**, qui échoue
le premier. Le mettre à jour appelle une relecture de sécurité du pont, pas une
retouche de la liste attendue.

### 2. Chaque appel devient une `Command` portant `Actor::Agent`

Un appel d'outil MCP suit le chemin de l'assistant interne, sans variante :
`ToolRegistry::translate` produit la `Command`, le sink la soumet avec
`Actor::Agent`, le `PolicyGate` décide ([I-01](../../CLAUDE.md#i-01),
[I-07](../../CLAUDE.md#i-07)).

Conséquences, qui ne sont pas des ajouts mais des propriétés héritées :

* une écriture sur une connexion `production` est un **refus**, pas une
  confirmation renforcée ([I-02](../../CLAUDE.md#i-02)) ;
* une écriture ailleurs demande l'**approbation humaine**, affichée avec le SQL
  exact, le nom de la connexion, l'environnement et l'acteur ;
* l'arrêt de la question en cours annule l'appel en cours.

### 2 bis. Un appel répond à la question en cours, et à aucune autre

La session d'un agent survit à une question : elle sert la suivante. Ce qui
rattache un appel — le nœud du panneau où il s'affiche, le bouton « Arrêter »
qui l'atteint, le nombre d'appels permis — ne lui survit **pas**. Chaque question
**ouvre un tour**, et le fermer retire les outils :

* **hors question en cours, rien ne s'exécute.** L'utilisateur ne regarde pas ;
  une confirmation demandée à qui ne regarde pas est cliquée par réflexe ;
* **un plafond d'appels par question**, celui de la boucle interne
  (`max_turns`) : « le même chemin » veut aussi dire les mêmes bornes ;
* **une seule écriture soumise à approbation par question.** La suivante est
  refusée jusqu'à la question d'après : dix demandes d'affilée, c'est ainsi que
  la dixième est approuvée sans être lue.

La première version liait ces trois choses à la question qui avait **lancé**
l'agent. Une première question arrêtée laissait tous les appels suivants partir
déjà annulés ; « Arrêter » sur une question suivante n'atteignait rien ; et entre
deux questions, l'agent pouvait appeler les outils sans limite, hors de la vue de
l'utilisateur. Rien n'échouait.

### 3. Le périmètre vient de l'hôte, jamais de l'agent

Le `ToolScope` — connexion, session, langage — est construit par Oxyn à
l'ouverture de la conversation, à partir de ce que l'utilisateur a lui-même
ouvert. L'agent ne le voit pas et ne peut pas le proposer.

Ce n'est pas une vérification ajoutée au pont : c'est la forme du schéma.
`ExecuteQueryArgs` ne porte **que** `statement`. Il n'existe aucun champ où
écrire une autre connexion, un « read_only: false », ou une intention déclarée.
Un agent qui tenterait d'en ajouter un est refusé par `deny_unknown_fields`.

### 4. Le niveau de confidentialité gouverne par le résultat rendu, pas par une seconde règle

C'est le point sur lequel nous avons d'abord conclu de travers, et la correction
mérite d'être écrite.

L'intuition de départ était : « `execute_query` rend des lignes, donc il faut le
refuser à un agent distant sous `Metadata` ». Elle repose sur une prémisse
fausse. Ce qu'un outil rend au modèle, **des deux côtés**, c'est
`ToolOutcome::render()`, et pour une exécution réussie ce texte est la **forme**
du résultat — `N rows, M batches` — jamais les valeurs. Les valeurs de lignes ne
rejoignent une invite que par le contexte, que `ContextBuilder` gouverne déjà
avec `PrivacyTier::allows_row_values` ([ADR-0006](0006-ai-privacy-tiers.md),
[I-04](../../CLAUDE.md#i-04)).

**Décision :** le pont MCP rend le **même** `ToolOutcome::render()` que la boucle
interne — même résumé, même rapport d'échec expurgé selon le niveau, même
encadrement `untrusted`. Il n'y a donc **pas** de règle de niveau propre aux
agents externes :

| Niveau | Agent externe | `execute_query` | `describe_schema` | `refresh_catalog` | `request_sample` |
|---|---|---|---|---|---|
| `Local` | interdit | — | — | — | — |
| `Metadata` | permis | permis, rend la forme | permis, rend la structure | permis | refusé avant tout écran |
| `Sampled` | permis | permis, rend la forme | permis, rend la structure, aucune valeur | permis | les colonnes que l'utilisateur coche, après son accord (§ 4 ter) |

`request_sample` n'est pas une règle de niveau propre aux agents externes : le
refus hors `Sampled` est `allows_row_values`, appliqué dans le chemin commun des
deux destinations, et les valeurs ne sortent que par `ContextBuilder::build`.

`Local` interdit l'agent externe **avant le lancement du processus** : c'est
ADR-0026, inchangé.

**Le niveau est relu à chaque appel, pas copié au lancement.** Le processus d'un
agent survit à la question qui l'a lancé ; le niveau, lui, est attaché à la
connexion ([I-04](../../CLAUDE.md#i-04)). Chaque `tools/call` relit donc le niveau
enregistré — la même source que `ai_ask` avant une question — et un appel sous
`Local`, ou sur une connexion disparue, est refusé sans rien exécuter. En plus,
les agents d'une connexion sont **relâchés** quand elle est modifiée, supprimée
ou fermée, et quand une question sur elle est refusée : la question suivante
relance l'agent sous ce qui vaut alors.

Écrire une seconde règle aurait été la vraie faute : deux règles de niveau
divergent le jour où l'une des deux change, et divergent en silence.

### 4 bis. La structure de la base : un outil pour toutes les destinations

*Ajouté le 2026-09-23, sur constat.* La première version de cet ADR affirmait
que l'agent externe « lit le schéma ». C'était faux : il ne recevait que la
question, et aucun outil ne lui rendait la structure. Sur une base SQLite, un
agent à qui l'on demandait « les 10 dernières lignes » a lancé
`SELECT name FROM sqlite_master`, reçu `11 rows` — la forme, jamais les valeurs,
comme le veut le § 4 —, et proposé `SELECT * FROM your_table`. La description
d'`execute_query` l'y invitait : elle disait « Reads return rows ».

La consigne qui a tranché : l'accès à la structure est **le même pour toute
destination** — fournisseur intégré, Claude Code, Codex, tout agent à venir —,
et ajouter une destination ne demande aucun code propre au schéma.

**Décision :**

* **Un outil du registre, `describe_schema`**, que la boucle interne et le pont
  MCP exposent tous deux. Il se traduit en une `Command` nouvelle,
  **`DescribeCatalog { connection, focus }`**, qui porte `Actor::Agent` et
  traverse le `PolicyGate` comme toute autre (§ 2). C'est une lecture du cache
  local : elle ne contacte aucun serveur, n'est pas mutante, et le `PolicyGate`
  la permet partout, `production` comprise. `focus` porte les mots de recherche
  de l'agent, bornés à 256 octets — des mots pour classer des noms, jamais du
  texte de requête ([I-10](../../CLAUDE.md#i-10)).
* **L'exécuteur rend le cache, pas un rendu.** Il remet la poignée du catalogue
  (`CatalogHandle`) dans son rapport ; c'est `oxyn-ai` qui la rend, dans
  `ToolOutcome::from_dispatch`, par **`ContextBuilder::build`** — la même
  fonction que le contexte d'une invite, sous le niveau **relu à l'appel** (§ 4),
  avec le même budget et le même encadré `untrusted`. Il n'existe pas de second
  rendu du schéma ([I-04](../../CLAUDE.md#i-04)).
* **Le rendu est le même pour toute base.** Il ne connaît que le modèle commun
  d'`oxyn-catalog` : chemin, sorte d'objet (`table`, `collection`, `index`,
  `key_pattern`, `node_label`…), champs et sous-champs avec leurs types **tels
  que le driver les nomme**, champs inférés, index, clés étrangères. Il dit en
  tête le langage de requête de la connexion. Les noms sont cités comme le
  dialecte SQL les cite, ou, hors SQL, en littéraux JSON. Un driver qui remplit
  le catalogue est couvert sans une ligne de plus dans `oxyn-ai`.
* **Les deux destinations gardent aussi un contexte d'ouverture**, rendu par la
  même fonction : l'assistant interne dans son message système, l'agent externe
  dans l'invite qui ouvre sa session (`AgentPrompt::with_schema`). Trois raisons
  le justifient : l'échantillon approuvé n'entre que par ce contexte, un petit
  modèle local peine à appeler un outil, et la première réponse n'attend pas un
  aller-retour. L'outil sert à ce que ce contexte a laissé hors budget.
  *Corrigé le 2026-09-24 :* cette version disait que l'invite d'un agent externe
  ne porte jamais d'échantillon, faute de pouvoir marquer un échange comme sans
  mémoire. Elle en porte désormais un, épinglé et approuvé, par le même
  `ContextBuilder` ; la mémoire est tenue en relâchant le processus après
  l'échange (§ 4 ter).
* **Ce qui dépend de la destination se dérive, ne se recopie pas.** Le nom du
  serveur MCP est une constante (`mcp::SERVER_NAME`) dont découlent la
  déclaration ACP, la règle de permission de Claude (`mcp__oxyn`, qui couvre
  tous les outils du serveur) et l'approbation de Codex, posée sur le serveur
  entier. Un test vérifie que chaque outil du registre passe les deux
  confinements sans y être nommé.
* la description d'`execute_query` dit désormais ce que l'outil rend : la forme
  du résultat, jamais ses valeurs, et renvoie à `describe_schema`.

La liste figée par le test du § 1 passe à `execute_query` et `describe_schema`.
Ce changement **appelle la relecture de sécurité du pont** que le § 1 exige.

**Limite assumée.** Une réponse de `describe_schema` suit le budget du contexte :
vingt-quatre relations, environ six mille jetons. L'agent parcourt un grand
schéma par mots de recherche, pas par pages. Et le catalogue commun ne connaît ni
les définitions de vues ni les valeurs de champs énumérés : ce qu'il ne sait pas
reste absent.

### 4 ter. Des valeurs approuvées, pour toute destination

*Ajouté le 2026-09-24.* Sous `Sampled`, un agent — interne ou externe — reçoit
les valeurs que l'utilisateur approuve colonne par colonne : épinglées avec la
question, ou demandées par l'outil `request_sample`. La décision, ses bornes et
ses limites sont dans [ADR-0034](0034-echantillon-pour-toute-destination.md) ;
ce qui en regarde le pont :

* l'outil vient du registre et passe par `run_tool_call`, comme les autres ;
  aucune ligne du pont ne lui est propre ;
* la demande passe la même garde qu'une commande (`WriteGate`) : une à la fois,
  jamais pendant qu'une approbation de cet agent attend, seulement pour la
  question ouverte — et une réponse arrivée après la fermeture de la question
  est jetée ;
* l'agent ne peut pas s'approuver : la réponse vient de la commande Tauri
  `ai_answer_sample`, et l'identifiant de la demande ne traverse jamais le pont ;
* un échange qui a porté des valeurs relâche le processus de l'agent ; la
  question suivante en lance un autre.

La liste figée par le test du § 1 passe à `execute_query`, `describe_schema` et
`request_sample`. Ce changement **appelle la relecture de sécurité du pont** que
le § 1 exige.

### 5. Le transport est `http` sur la boucle locale, avec un jeton par conversation

Puisque `Acp` n'existe pas chez les agents réels, il reste `stdio` et `http`.

`stdio` place le serveur MCP dans un sous-processus lancé **par l'agent**, pas
par nous. Ce sous-processus devrait rejoindre l'Oxyn en cours d'exécution — le
seul qui tienne l'exécuteur, le `PolicyGate` et les approbations — par une IPC
de notre invention. Nous écririons donc un second mode binaire et un protocole
de relais pour obtenir la surface que `http` nous donne déjà.

**Décision :** Oxyn sert MCP en **HTTP sur `127.0.0.1`**, et :

* le port est **tiré au hasard**, jamais fixé, jamais écrit ;
* l'écoute est liée à la boucle locale, **jamais** à une autre interface ;
* chaque conversation reçoit un **jeton porteur** tiré à son ouverture, passé à
  l'agent dans les `headers` de `McpServerHttp` ; une requête sans ce jeton
  exact est refusée sans rien révéler ;
* le jeton n'est **ni persisté, ni journalisé, ni affiché**
  ([I-03](../../CLAUDE.md#i-03)) ;
* l'écoute **naît avec la conversation et meurt avec elle**, et les connexions
  déjà acceptées **avec elle** : hors conversation avec un agent externe, Oxyn
  n'écoute rien et ne sert rien. Arrêter seulement l'écoute ne suffit pas — une
  connexion maintenue ouverte (*keep-alive*) continuerait d'atteindre le bus
  après la fin, pour un processus sorti du groupe de l'agent.

Et parce que c'est une surface réseau, quatre règles qui ne sont pas des
options :

* **le jeton est exigé sur *toutes* les requêtes, `initialize` comprise.** Un
  handshake laissé ouvert est un point d'énumération : il dit à qui frappe
  qu'Oxyn écoute, et ce qu'il sert ;
* **la comparaison est à temps constant.** Une comparaison qui s'arrête au
  premier octet différent se mesure, et un jeton se devine octet par octet ;
* **`Origin` et `Host` sont vérifiés**, et tout ce qui n'est pas la boucle
  locale est refusé. Sans cela, une page ouverte dans le navigateur de
  l'utilisateur peut faire résoudre un nom vers `127.0.0.1` et parler à ce port
  — c'est le *DNS rebinding*, et le pare-feu n'y peut rien ;
* **un refus ne dit rien, et c'est voulu.** Jeton absent, jeton faux, origine
  étrangère, `Host` hors boucle locale, mauvais chemin, mauvaise méthode : **la
  même réponse**, même statut, corps vide. Un message qui distingue les causes
  apprend à l'appelant ce qu'il a presque réussi — « jeton invalide » confirme
  que l'origine est acceptée.

  Ce choix rend le débogage moins confortable, et c'est précisément pourquoi il
  est écrit ici : **ne pas l'« améliorer » en précisant les messages.** Qui a
  besoin de comprendre un refus lit les tests du serveur, pas la réponse HTTP.

Deux règles de plus, pour ce qu'un programme local peut faire **sans** le jeton :

* **tout est borné avant l'authentification** : quelques connexions à la fois
  (au-delà, fermées dès l'acceptation), un délai sur les en-têtes — qui borne
  aussi une connexion inactive —, un délai sur le corps. Sans ces bornes,
  n'importe quel programme épuise les descripteurs d'Oxyn, et plus aucune base ne
  se connecte, plus aucun brouillon ne s'enregistre.

  **Limite assumée.** Seize connexions à la fois, fermées dès l'acceptation
  au-delà : un programme local peut priver l'agent de ses outils, mais plus
  Oxyn de ses descripteurs. Nous l'acceptons : l'agent en panne d'outils se voit
  dans le panneau, un Oxyn sans descripteurs perd le travail de l'utilisateur. Les valeurs sont fixées par un test : sans minuteur, `hyper`
  ignore ses délais en silence ;
* **le crate ACP est plafonné à `error` dans les journaux, quoi que dise
  `OXYN_LOG`.** Il journalise chaque message entier en `debug` — le jeton de
  `session/new`, les questions de `session/prompt` —, c'est-à-dire exactement ce
  qu'on demande à un utilisateur d'activer pour un rapport de bogue ; et ses
  lignes `warn` recopient ce que l'agent a envoyé, puisqu'une erreur serde
  « invalid type » cite la chaîne fautive. Le plafond porte sur la **cible**, pas
  sur un niveau : une montée de version qui déplace la même trace reste couverte.
  Le diagnostic n'est pas perdu : là où Oxyn reçoit une erreur de protocole, il
  écrit **sa propre** ligne `warn` — la méthode et le code, jamais le texte de
  l'agent. Le jeton est aussi expurgé de ce que l'agent écrit en retour —
  message d'erreur, `stderr`, avant la coupe de ce dernier.

Nous l'écrivons comme une surface plutôt que comme un détail d'implémentation :
elle appelle une relecture de sécurité à chaque changement du pont.

### 6. L'agent est lancé par Oxyn, pas par la crate

`AcpAgent` sait lancer l'agent, et nous ne l'employons pas. La raison tient en
une ligne de ses sources : `agent_client_protocol/src/acp_agent.rs:263` fait
`std_cmd.envs(&self.config.env)` — un **ajout**. Il n'y a ni `env_clear`, ni
`current_dir`, et `AcpAgentConfig` garde ses trois champs privés. Par ce chemin,
Claude Code démarre avec **tout** l'environnement d'Oxyn : les identifiants
cloud de l'utilisateur, ses URL de bases, ce que son shell a exporté. C'est
[I-03](../../CLAUDE.md#i-03), et un invariant ne se négocie pas contre les
heures que coûte un lancement écrit à la main.

**Décision :** Oxyn construit la commande lui-même, avec

* un environnement en **liste blanche** — ce que l'utilisateur a confirmé, plus
  `PATH`, `HOME`, `USER`, `LANG`, `TMPDIR`, chacune justifiée là où elle est
  écrite. Une liste noire oublie la variable ajoutée six mois plus tard ;
* un **répertoire de travail explicite**, vide, créé pour cet agent et détruit
  avec lui — jamais le workspace d'Oxyn, où vivent les fichiers qui décrivent
  les connexions. Ce répertoire est aussi celui **annoncé à la session ACP**, et
  pas le répertoire temporaire partagé : un agent y charge les consignes et les
  serveurs MCP de son « projet », et tout programme de l'utilisateur peut écrire
  dans `$TMPDIR`. Il est créé en `0o700`, sous un nom aléatoire, **jamais adopté
  s'il existe déjà, et il n'existe aucun repli** : un repli sur `$TMPDIR` lui-même
  a un temps fait effacer tout le répertoire temporaire de l'utilisateur à la fin
  d'une conversation sur un disque plein ;
* un garde qui tue le **groupe** de processus : un agent distribué derrière
  `npx` se ré-attache sinon à pid 1 et survit à la fermeture de la fenêtre,
  nos outils encore ouverts ;
* un code de sortie et une queue de `stderr` en **données typées**, expurgées de
  tout ce qu'Oxyn a passé à l'enfant : le `stderr` d'un processus est un canal
  I-03 comme un autre, et un lanceur qui échoue imprime son environnement.

Ce que la crate faisait gratuitement est donc réécrit ici. C'est le prix de
l'invariant, et il est écrit pour que personne ne « simplifie » en revenant à
`AcpAgentConfig`.

**Et la liste blanche a son propre prix, qu'il faut écrire plutôt que le
découvrir.** Une liste noire oublie ce qu'on ajoutera ; une liste blanche oublie
ce qu'on ignorait. La différence est dans le mode de panne : **une variable
manquante ne casse pas l'agent, elle le dégrade en silence**.

Le cas s'est produit avant même la première livraison. Sans `USER`, Claude Code
se déclare « Not logged in » sur une machine où il est connecté — et
`initialize` **réussit quand même**, avec `authMethods` vide. Le handshake paraît
sain ; le refus tombe au premier prompt. L'utilisateur pose une question et se
voit répondre « connecte-toi ». Rien n'a échoué, quelque chose a simplement
cessé de fonctionner.

Nous gardons la liste blanche, parce que rendre tout l'environnement pour éviter
ce mode de panne rendrait aussi les secrets. Mais la règle qui l'accompagne est
une conséquence de ce prix : **chaque nom porte la mesure qui l'y a mis**, un nom
ne s'ajoute jamais « au cas où », et un nom ne se retire pas sans refaire la
mesure. La mesure elle-même est datée dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md).

### 7. Une absence de réponse vaut refus

Une demande de permission qu'Oxyn ne sait pas satisfaire n'est pas laissée en
suspens : elle est **refusée**. Un client qui ne répond pas laisse l'agent
attendre, et un agent qui attend paraît en panne ; pire, une refonte qui
« corrigerait » l'attente en accordant par défaut inverserait la règle sans que
rien n'échoue.

Le défaut est donc écrit, et testé, des deux côtés : dans le code qui répond, et
dans cet ADR pour que le jour où quelqu'un le trouve gênant, il sache que c'était
voulu.

### 8. Ce que ce choix ne ferme pas, et qu'il faut nommer

Le nombre de lignes est un canal. `SELECT 1 FROM clients WHERE email = '…'`
rend `1 rows` ou `0 rows` : un bit sur une valeur précise, par tour.

Nous l'écrivons plutôt que de le taire, et nous ne le traitons pas ici, pour
trois raisons. Ce canal est **identique pour l'assistant interne** — ce n'est pas
une ouverture du mode agent. Il est **borné** par le plafond d'appels par
question, le même des deux côtés (§ 2 bis), et par le fait que l'utilisateur
voit chaque instruction passer dans le panneau. Et le
fermer demanderait de cacher au modèle si sa requête a rendu quelque chose,
c'est-à-dire de lui retirer le seul retour qui lui permet de corriger une
requête fausse.

Si nous décidons un jour de le fermer, ce sera pour les deux destinations à la
fois, et ce sera un autre ADR.

## Conséquences

* **+** L'agent externe devient utile dans un atelier de bases de données : il
  lit la structure de la base par le même outil que l'assistant interne (§ 4 bis)
  et interroge la base que l'utilisateur a ouverte.
* **+** Aucune surface nouvelle vers le bus : le pont traduit vers le même
  `ToolRegistry` et le même sink. Ce qui est relu une fois vaut pour les deux
  destinations.
* **+** L'utilisateur voit ce que l'agent fait dans le panneau de l'assistant —
  appels, approbations, refus — avec les états déjà en place.
* **−** Une seconde porte d'entrée existe désormais, même si elle débouche sur
  le même couloir. Elle demande sa propre relecture de sécurité à chaque
  changement du pont.
* **−** L'agent apprend la **forme** des résultats, et donc un peu de la base,
  même sous `Metadata`. C'est déjà vrai de l'assistant interne, et c'est le prix
  d'un agent qui peut corriger sa requête.
* **−** Le pont dépend de la façon dont `agent-client-protocol` déclare les
  serveurs MCP. Un changement d'API de la crate se paiera ici.

**Coût de sortie :** faible. Retirer le pont rend les agents externes muets sur
la base, sans toucher à l'assistant interne ni au registre.

**Reconsidérer si** un agent externe obtient un jour le droit de déclencher
autre chose qu'un outil du registre — c'est la ligne d'ADR-0026, et elle vaut
ici mot pour mot. **Reconsidérer aussi** le jour où les adaptateurs annoncent
`mcpCapabilities.acp` : le serveur redeviendrait interne à la session, et la
surface réseau disparaîtrait.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Laisser les agents externes sans outils | C'est l'état d'aujourd'hui, et il rend le mode agent inutile : un agent qui ne peut pas lire la base ne sert à rien dans un atelier de bases de données |
| Refuser `execute_query` sous `Metadata` pour un agent distant | Repose sur une prémisse fausse : l'outil rend la forme du résultat, pas les valeurs. La règle aurait interdit l'usage principal en croyant protéger quelque chose que le résumé n'expose pas |
| Rendre à l'agent un résultat réduit « au compte de lignes et aux types » | C'est déjà ce que rend `ToolOutcome`. Le formuler comme une règle distincte aurait créé la seconde règle de niveau que cet ADR refuse |
| Donner à l'agent un accès direct au driver, ou une connexion à lui | Contourne le bus, le `PolicyGate` et l'approbation humaine. C'est précisément ce qu'[I-01](../../CLAUDE.md#i-01) interdit, et le second chemin ne serait jamais audité comme le premier |
| Écrire des outils MCP spécifiques aux agents externes, plus riches que le registre | Deux jeux d'outils divergent. Celui que personne ne relit est celui que l'agent empruntera |
| Porter le serveur MCP en mémoire, par la variante `Acp` de la session | Le plus désirable, et mesuré inutilisable : ni Claude Agent 0.78.0 ni Codex 1.12.0 n'annoncent la capacité, Codex répond `"acp": false`. À reprendre le jour où ils l'annoncent |
| Servir MCP en `stdio` | Le sous-processus est lancé par l'agent, pas par nous : il devrait rejoindre l'Oxyn vivant par une IPC de notre invention. Un second mode binaire et un protocole de relais, pour la même surface que `http` |
