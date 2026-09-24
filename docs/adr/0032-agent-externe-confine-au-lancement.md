# ADR-0032 — Un agent externe connu est confiné au lancement, et n'a d'outils que ceux d'Oxyn

**Statut :** accepté · **Date :** 2026-09-23

**Précise :** [ADR-0026](0026-agents-externes-acp.md), sur un point :
l'ADR supposait que refuser les demandes d'autorisation d'un agent suffisait à
l'empêcher d'agir sur la machine. C'est faux, et la mesure le montre.
Le reste de l'ADR-0026 reste en vigueur, ainsi que
[ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md).

## Contexte

L'ADR-0026 range tout accès d'un agent externe à la machine (fichiers,
commandes, réseau) dans `oxyn_ai::external::permission_for`, avec un refus par
défaut. Or ce refus ne s'applique qu'à ce que l'agent **demande**, et un agent
ne demande que ce que son mode l'oblige à demander.

Mesure du 2026-09-23 sur les adaptateurs épinglés, avec un client qui refuse
tout, comme Oxyn, et une consigne demandant de lancer `touch` sur un fichier
témoin ([RESEARCH-NOTES](../RESEARCH-NOTES.md#confinement-des-adaptateurs-acp--mesure-du-2026-09-23)) :

* **Claude Agent**, dans son mode initial `auto` (lu dans les réglages de
  l'utilisateur), exécute la commande **sans aucune demande**. Le fichier est
  créé ;
* **Codex**, dans son mode initial `agent`, fait de même. En `read-only`, il
  exécute encore sans demander tout ce que son bac à sable permet : lecture
  partout, écriture dans `/tmp` ;
* le sélecteur de modes d'Oxyn proposait en outre « Bypass permissions » et
  « Full access ».

Le contenu d'une base est une entrée hostile ([ia.md](../../.claude/rules/ia.md)).
Un nom de table rédigé comme une consigne suffisait donc à faire lancer une
commande sur la machine de l'utilisateur, derrière une fenêtre de base de
données. C'est précisément ce que l'ADR-0026 voulait empêcher.

Deux faits aggravent le problème :

* en mode strict, un agent demande aussi l'autorisation avant d'appeler les
  outils **d'Oxyn**. Codex la présente sous le genre `execute`, sans nom
  d'outil ; `permission_for` la refuse. Les agents externes ne pouvaient donc
  pas lire la base, ce qui est le défaut signalé par l'utilisateur ;
* Codex charge les serveurs MCP, les plugins et les hooks du `~/.codex` de
  l'utilisateur. Sur la machine de mesure, cela comprend un serveur exposant
  `execute_sql` sur une base distante, ainsi que des plugins de pilotage du
  navigateur et de l'ordinateur.

## Décision

**Un agent dont Oxyn connaît l'adaptateur est confiné au lancement, par les
interrupteurs que cet adaptateur documente. Il est ensuite maintenu dans son
mode le plus strict jusqu'à la fin de la session.** Le type
`oxyn_ai::external::confine::Confinement` porte ce confinement.

**« Connu » veut dire : la version épinglée et mesurée, avec exactement les
arguments qu'Oxyn propose** (`presets::pinned_preset_of`). Une autre version
peut ignorer les interrupteurs. Un argument de plus peut les surcharger. Tout
le reste est traité comme un agent inconnu, avertissement compris. Monter la
version d'un préréglage exige donc de refaire la mesure.

**Un agent connu ne démarre pas à moitié confiné.** Si le confinement ne peut
pas être appliqué tel que mesuré, l'agent n'est pas lancé
(`ExternalError::Unconfinable`). Trois cas le déclenchent : une configuration
Codex présente mais illisible en entier (taille, droits, syntaxe, fichier non
ordinaire), plus de 256 entrées à couper, ou un serveur de l'utilisateur déjà
nommé `oxyn`.

### Claude Agent : les outils d'Oxyn, et aucun autre

Les options du SDK passent par `_meta.claudeCode.options` de `session/new` :

* `tools: []` retire tous les outils intégrés (shell, fichiers, web) ;
* `strictMcpConfig: true` ne garde que le serveur MCP déclaré par Oxyn ;
* `settingSources: []` ne charge ni les règles d'autorisation, ni les hooks,
  ni les serveurs MCP de l'utilisateur ;
* `allowDangerouslySkipPermissions: false` retire le mode `bypassPermissions`
  du catalogue ;
* `allowedTools: ["mcp__oxyn"]` préautorise les outils d'Oxyn **côté agent**.
  Leur contrôle est le `PolicyGate`, avec `Actor::Agent`
  ([ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md)). Une demande
  d'autorisation qu'Oxyn devrait refuser ne protégerait rien de plus : elle
  rendrait seulement la base illisible.

Le mode est ensuite fixé à `default` (« Manual »).

### Codex : ni shell ni web, `read-only`, outils personnels coupés par nom

Deux variables d'environnement, documentées par l'adaptateur, portent le
confinement :

* **`INITIAL_AGENT_MODE=read-only`** ;
* **`CODEX_CONFIG`**, qui contient :
  * `features.shell_tool`, `features.unified_exec`, `features.hooks` et
    `features.apps` à `false`, et `web_search = "disabled"` ;
  * le serveur d'Oxyn, **déclaré dans cette configuration** plutôt que par
    ACP, avec `default_tools_approval_mode = "approve"`. Le jeton passe par
    `OXYN_TOOL_TOKEN`, et le rapport de sortie du processus l'expurge comme
    toute variable transmise. La mesure montre que ce réglage ne s'applique pas
    à un serveur déclaré par ACP ;
  * chaque serveur MCP et chaque plugin du `config.toml` de l'utilisateur, avec
    `enabled = false`. Seuls **les noms** sont lus, et le fichier est borné à
    1 Mio. Aucune valeur n'est retenue, et certaines portent des jetons
    ([I-03](../../CLAUDE.md#i-03)). Le fichier lu est celui que **le
    processus de Codex** chargera : le `CODEX_HOME` déclaré pour l'agent, sinon
    le `.codex` du `HOME` que reçoit l'agent. Jamais le `CODEX_HOME` d'Oxyn,
    que l'enfant ne reçoit pas.

Le mode est ensuite refixé à `read-only`.

### Le mode tient toute la session

* Le mode est fixé **après** `session/new` et **avant** la première question.
  Si l'agent refuse de le prendre, il ne lit aucune question, et la session
  échoue avec `ExternalError::Unconfined`.
* `AgentSettings::modes_locked` retire les modes, ainsi que les options de
  catégorie `mode`, de tout ce qui est affiché ou accepté. Le sélecteur
  disparaît, et un changement de mode est refusé par `check` avant tout envoi.
* `SwitchMode`, que `permission_for` accorde à un agent non confiné, est refusé
  à un agent confiné.
* Si l'agent annonce un autre mode que celui fixé, `ModeWatch` le retient.
  Le tour en cours est annulé aussitôt (`session/cancel`) et se termine par
  `Unconfined` ; toute question suivante est refusée. On ne revient pas en
  arrière : un agent qui a changé de mode une fois n'est plus présumé stable.
* Deux écarts faibles sont assumés. Une annonce de mode arrivée entre la
  réponse à `session/set_mode` et l'armement n'est pas vue ; l'agent n'a alors
  encore reçu aucune question. Une option de mode que l'adaptateur ne classe pas
  dans la catégorie `mode` reste proposée ; aucun des deux adaptateurs mesurés
  n'en déclare.

### Un agent inconnu reste utilisable, et l'écran le dit

Oxyn ne connaît pas les interrupteurs d'un agent déclaré à la main. Il le lance
donc comme avant, mais le formulaire de déclaration et le panneau disent
qu'**Oxyn ne peut pas l'empêcher d'agir seul sur la machine**. C'est
l'utilisateur qui a désigné ce programme, et la déclaration exige déjà une
double confirmation (ADR-0026).

## Conséquences

* **+** Pour Claude Agent, le confinement est structurel : l'agent n'a aucun
  outil de la machine, donc rien à demander. Il ne dépend plus d'un refus qui
  doit être sollicité.
* **+** Les agents externes lisent enfin la base par les outils d'Oxyn. Cela
  était l'objet même de l'ADR-0030, et les refus de `permission_for` le
  rendaient impossible.
* **+** Le choix d'un mode dangereux n'est plus proposé à l'écran.
* **−** **Codex reste confiné par liste noire.** Un plugin actif par défaut et
  absent du `config.toml` de l'utilisateur échappe au confinement. La
  référence de Codex ne documente aucun moyen de couper tous les plugins, ni
  d'ignorer ce fichier. Codex garde aussi l'édition de fichiers : elle demande
  l'autorisation, et Oxyn la refuse.
* **−** Oxyn lit un fichier de configuration d'un autre programme. Il n'en garde
  que des noms, mais il dépend désormais de son format.
* **−** Le confinement repose sur des options **propres à chaque adaptateur**
  (`_meta.claudeCode`, `CODEX_CONFIG`), hors du protocole ACP. Une montée de
  version d'un adaptateur peut les ignorer sans rien casser de visible. Chaque
  montée doit donc refaire la mesure de RESEARCH-NOTES.
* **−** L'utilisateur perd dans Oxyn ses réglages personnels de Claude Code :
  modèle par défaut, instructions, serveurs MCP. C'est voulu, mais visible.
* **−** Un agent déclaré à la main n'est pas confiné. Seul un avertissement
  l'accompagne.

**Coût de sortie :** faible. Tout tient dans `confine.rs`, `ModeWatch` et le
champ `modes_locked` ; retirer le confinement revient à renvoyer `None`. Ce qui
coûterait cher, c'est de le retirer **sans** retirer aussi les agents externes.

**Reconsidérer si** ACP normalise une manière, pour un client, de restreindre
les outils d'un agent ou d'en fixer le mode, ce qui dispenserait des options
propres à chaque adaptateur. Également si Codex documente un interrupteur qui
coupe en bloc les serveurs MCP et plugins de l'utilisateur : la liste noire
deviendrait inutile.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder le seul refus de `permission_for` | Mesuré : l'agent n'a rien demandé et la commande a été exécutée. Un refus qu'on ne sollicite pas ne protège rien |
| Accorder les demandes d'autorisation dont le titre désigne un outil d'Oxyn | Chez Codex, la demande ne porte ni titre ni nom d'outil, seulement un identifiant `exec-…`. Et un titre est un texte produit par l'agent : accorder sur son contenu, c'est accorder sur ce que dit l'agent |
| Lancer Codex avec un `CODEX_HOME` privé | Son jeton de connexion vit dans ce répertoire. Le recopier ferait détenir à Oxyn un secret de l'utilisateur, ce que l'ADR-0026 refuse. Un lien symbolique serait remplacé par un fichier à la première rotation du jeton |
| Suspendre Codex tant qu'il n'est pas confinable en entier | Écarté par l'utilisateur au profit de la liste noire, dont l'écart est écrit ici |
| Refuser tout agent qu'Oxyn ne sait pas confiner | Écarté par l'utilisateur : c'est lui qui désigne le programme, et un avertissement explicite dit ce qu'Oxyn ne peut pas garantir |
