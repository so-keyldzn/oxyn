# ADR-0034 — Un échantillon approuvé atteint toute destination par la même porte, et un agent peut en demander un sans jamais l'approuver

**Statut :** proposé · **Date :** 2026-09-24

**Précise :** [ADR-0006](0006-ai-privacy-tiers.md), sur deux points qu'il ne
tranchait pas : **qui** peut déclencher l'échantillon de `Sampled`, et **quelles
destinations** le reçoivent. Le tableau des niveaux est inchangé.

**Complète :** [ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md), dont la
liste d'outils (§ 1), le tableau par niveau (§ 4) et la phrase « l'invite d'un
agent externe ne porte jamais d'échantillon » (§ 4 bis) changent avec cet ADR.

## Contexte

ADR-0006 promet, sous `Sampled`, un « échantillon de lignes approuvé
explicitement, colonne par colonne ». Ce qui était livré le 2026-09-23 en tenait
une seule forme :

* **une seule destination** — le fournisseur intégré. Un agent externe (Claude
  Code, Codex) était refusé, au motif que sa session survit à la question et
  qu'Oxyn « ne peut pas y marquer un échange comme sans mémoire » ;
* **un seul déclencheur** — l'utilisateur, qui épingle une relation depuis le
  catalogue. Aucun outil ne permettait à un agent de demander des valeurs, et
  les outils rendent la forme d'un résultat, jamais ses valeurs
  ([ADR-0030 § 4](0030-outils-oxyn-exposes-a-un-agent-externe.md)).

Conséquence : sur une connexion que l'utilisateur a lui-même passée en
`Sampled`, un agent externe ne pouvait jamais voir une valeur, et aucun agent ne
pouvait dire « j'ai besoin de voir comment `status` est écrit » au moment où il
en a besoin. Le niveau disait « des valeurs peuvent sortir, approuvées » ; le
produit disait « jamais vers un agent, et seulement si vous y pensez avant ».

Trois contraintes encadrent la réponse, et aucune ne se négocie :

* **[I-04](../../CLAUDE.md#i-04)** — une seule fonction fait entrer du contenu
  dans une invite, sous le niveau de la connexion : `ContextBuilder::build`. Une
  seconde voie « pour les agents » serait la voie que personne ne relit ;
* **[I-01](../../CLAUDE.md#i-01), [I-07](../../CLAUDE.md#i-07)** — ce qu'un agent
  déclenche est une `Command` portant `Actor::Agent`, qui traverse le
  `PolicyGate`, lecture comprise ;
* **la mémoire** — un fournisseur n'a que la session qu'Oxyn lui remet ; un agent
  externe est un **processus**, et ce qu'il a lu, il le sait encore à la question
  suivante.

## Décision

### 1. Une seule porte rend les valeurs, pour toute destination

Les valeurs de ligne rejoignent une invite **uniquement** par
`ContextBuilder::with_samples`, puis `build`, qui les écarte sous tout niveau
autre que `Sampled`. Trois appelants, un rendu :

| Destination | Par où |
|---|---|
| Fournisseur, échantillon épinglé | le contexte du message système (inchangé) |
| Agent externe, échantillon épinglé | `AgentPrompt::with_schema(tier, question, cache, language, samples)` — l'invite qui ouvre la session |
| Toute destination, échantillon demandé par l'outil | `ToolOutcome::from_dispatch` sur `DispatchOutcome::Sampled`, rendu par `ContextBuilder` sans relation décrite (`max_relations: 0`) et au plafond de la demande |

Un échantillon que le point de passage écarte — niveau abaissé, budget dépassé —
devient un **refus** rendu au modèle, jamais un « terminé » vide.

### 2. L'outil `request_sample`, dans le registre partagé

`ToolRegistry::builtin` gagne `request_sample { relation, namespace?, columns?,
rows? }`, accordé à `sql_agent()` — donc à la boucle interne **et** au pont MCP,
sans une ligne propre à l'un ou l'autre ([ADR-0030 § 1](0030-outils-oxyn-exposes-a-un-agent-externe.md)).

* **Aucune commande nouvelle.** La traduction produit une `SampleAsk` qui porte
  `Command::PreviewRelation` — la lecture même de l'échantillon épinglé —, bornée
  à `rows` (5 par défaut, **20 au plus**), sans ordre ni prédicat, et les
  colonnes demandées (**64 au plus**, noms de **256 octets au plus**). La
  connexion et la session viennent du `ToolScope` : l'agent ne les nomme pas, et
  `deny_unknown_fields` refuse tout champ inventé.
* **Les noms restent des données** ([I-10](../../CLAUDE.md#i-10)). La relation
  voyage en champ de la commande ; le driver la cite en composant la lecture ;
  les colonnes ne rejoignent aucune instruction — elles désignent ce qu'on
  recopie du résultat. Relation et colonnes sont confrontées au catalogue local
  avant tout écran : un nom inconnu est refusé, jamais deviné.
* **`CommandSink::request_sample`**, méthode du puits existant, dont le défaut
  **refuse**. Une méthode et non un `dispatch` de plus, parce que l'approbation
  colonne par colonne n'est pas une décision du `PolicyGate` — elle choisit ce
  qui sort, pas ce qui s'exécute — et que les colonnes voulues ne sont pas dans
  la commande. Ce n'est pas un second chemin vers l'exécuteur : la lecture
  approuvée repart par **le même** puits, avec `Actor::Agent`, et le
  `PolicyGate` décide encore.

### 3. L'ordre, et rien de lu avant la réponse de l'utilisateur

1. `oxyn-ai` refuse hors `Sampled` **avant** de solliciter le puits, sous le
   niveau de la boucle — relu à l'appel pour le pont MCP (ADR-0030 § 4). Le refus
   dit le niveau et « ne redemande pas ». Aucun écran ne s'ouvre ;
2. le puits (`oxyn-desktop`) **relit le niveau dans le store** — celui d'une
   question interne a pu baisser depuis qu'elle a commencé ;
3. relation et colonnes sont cherchées dans le catalogue ;
4. l'écran d'approbation s'ouvre — **le même** que pour l'épingle, qui nomme
   l'agent demandeur — et l'appel attend, **cinq minutes au plus**, lâché à
   l'arrêt de la question ;
5. refus, expiration ou arrêt : l'agent reçoit « the user declined », et rien
   n'est lu ;
6. approbation : le niveau est relu, l'échange est marqué sans mémoire, la
   lecture part sur le bus, seules les colonnes cochées sont recopiées, le niveau
   est relu une troisième fois, et les lignes reviennent à `oxyn-ai` ;
7. `ToolOutcome::from_dispatch` les rend. **Seulement si ce rendu les garde**, la
   sortie est inscrite dans `ai_egress` et le panneau apprend « envoyé » — si
   l'inscription échoue, rien ne part. Le puits ne l'inscrit pas lui-même : il
   remet à la boucle un reçu (`SampleReceipt`) qu'elle libère une fois le rendu
   connu. Un échantillon écarté pour le budget n'est ni inscrit ni annoncé, et
   la ligne d'outil dit le refus que le modèle a lu.

Deux bornes valent **dans le puits**, donc pour la boucle interne comme pour le
pont MCP, avant tout écran :

* **jamais pendant qu'une approbation de cet agent attend** — lue dans
  l'exécuteur, qui tient les demandes ;
* **un seul écran par échange.** Approuvée, refusée ou expirée, la première
  demande épuise l'échange : toute demande suivante est refusée sans écran, par
  un message constant qui ne dit pas quelle fut la réponse. Sans cette borne,
  rien ne limitait la répétition — la boucle interne n'a pas de plafond d'appels
  par tour, le pont en a huit par question —, et un « non » redemandé finit en
  « oui » par lassitude.

Sur le pont MCP, la demande passe en outre par la même garde qu'une commande :
une à la fois, et seulement pour la question ouverte. Une connexion garde au
plus **quatre** demandes en attente.

### 4. Un agent ne peut pas s'approuver lui-même

La seule réponse à une demande est la commande Tauri `ai_answer_sample` — un
geste de l'utilisateur dans l'écran. L'identifiant de la demande ne part que
vers la webview, jamais vers le modèle ; rien de ce qu'un agent envoie ne porte
d'identifiant ni de décision, et les arguments de l'outil refusent tout champ
qu'ils ne déclarent pas. Une réponse mal formée — colonne non proposée, aucune
colonne — **refuse** la demande plutôt que de la laisser ouverte à un second
essai.

### 5. Un échange qui a porté un échantillon ne laisse aucune mémoire, quelle que soit la destination

* **Fournisseur** : inchangé pour l'épingle ; un échantillon demandé par l'outil
  marque l'échange de la même façon, et sa session n'est pas retenue ;
* **agent externe** : une question qui porte un échantillon épinglé ne continue
  **jamais** une session — elle en ouvre une neuve, qui reçoit structure et
  lignes par `with_schema`. Et tout échange qui a porté un échantillon, épinglé
  ou demandé, **relâche le processus** à sa fin, quelle qu'en soit l'issue. La
  question suivante lance un autre processus et émet `memoryReset` avec la
  raison `sampleNotKept`.

Relâcher retire aussi les demandes d'approbation que cet agent laissait en
attente (`WithdrawOnRelease`) : c'est la règle de tout relâchement, et une
écriture proposée dans la même réponse qu'un échantillon est donc à redemander.

**L'option examinée et écartée** : ouvrir une nouvelle session ACP (`session/new`)
dans le même processus. Elle efface l'historique **du protocole**, pas l'état
**du processus** : ce que l'adaptateur garde en mémoire ou en cache n'est pas
observable, et la garantie dépendrait de l'implémentation de chaque agent. Tuer
le processus est la seule option dont la garantie ne dépend que d'Oxyn.

**Limite assumée**, écrite pour ne pas être promise : ce qu'un agent écrit sur
**son propre disque** — un historique de sessions, un journal — échappe à Oxyn,
relâché ou non. C'est la limite qu'[AI-PROVIDERS](../AI-PROVIDERS.md) nomme déjà
pour ce mode : Oxyn peut nommer ce qui part vers l'agent, il ne peut pas garantir
ce que l'agent en fait. `Local` reste fermé aux agents externes pour cette raison.

## Conséquences

* **+** Toute destination reçoit les valeurs approuvées par **une** fonction :
  la question « qu'est-ce qui est sorti ? » se répond en relisant
  `ContextBuilder::build` et `ai_egress`, pas chaque appelant.
* **+** Un agent qui a besoin de voir une valeur peut le dire au moment où il en
  a besoin, et l'utilisateur tranche dans l'écran qu'il connaît.
* **+** Aucune commande, aucun écran, aucune règle de niveau nouvelle : la
  lecture est `PreviewRelation`, l'écran est celui de l'épingle, le refus hors
  `Sampled` est `allows_row_values`.
* **−** Un écran de consentement peut désormais s'ouvrir **sans que
  l'utilisateur l'ait provoqué** — le cas le plus exposé au clic réflexe. Ce qui
  le borne : rien n'est coché, l'écran nomme qui demande et dit qu'Annuler
  refuse, Annuler a le focus, Entrée n'envoie rien, un seul écran par échange,
  jamais pendant qu'une écriture attend.
* **−** Un échange avec échantillon coûte la session de l'agent : la question
  suivante relance le processus — plusieurs dizaines de secondes au premier
  lancement d'un adaptateur servi par `npx` — et l'agent a perdu le fil.
* **−** Sous `Metadata`, l'outil est annoncé et refuse. Un modèle peut
  l'essayer une fois ; le refus lui dit de ne pas redemander.
* **−** La liste figée d'outils servie aux agents externes change : la
  relecture de sécurité du pont qu'exige ADR-0030 § 1 est **due**.

**Coût de sortie :** faible. Retirer `request_sample` du registre et de
`sql_agent()` supprime le volet « demandé » sans toucher au reste ; retirer le
paramètre `samples` de `with_schema` rend l'agent externe à la structure seule.
Aucun format persisté ne change : `ai_egress` inscrivait déjà un destinataire
par identifiant de déclaration.

**Reconsidérer si** le protocole ACP gagne un moyen **vérifiable** d'oublier une
session (le relâchement deviendrait inutile) ; si un driver déclare un classement
de colonnes secrètes (elles sortiraient de l'offre, comme le prévoit le TODO de
`ai_request_sample`) ; ou si l'usage montre des approbations données sans
lecture — la réponse serait alors de retirer la demande par l'agent, pas
d'alourdir l'écran.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Une `Command::RequestSample` que le `PolicyGate` classe « à approuver » | L'approbation générique approuve une commande entière, sans cocher de colonne, et rend la main à l'agent aussitôt (« awaiting approval ») : il n'y aurait ni sélection colonne par colonne ni valeur à lui rendre après l'accord |
| Rendre les valeurs d'`execute_query` sous `Sampled` | Une requête arbitraire n'est pas un échantillon approuvé colonne par colonne ; ce serait la seconde règle de niveau qu'[ADR-0030 § 4](0030-outils-oxyn-exposes-a-un-agent-externe.md) refuse |
| Pré-cocher les colonnes que l'agent demande | Une case cochée par l'agent est un consentement que l'utilisateur n'a pas donné ; les colonnes demandées **restreignent l'offre**, elles ne cochent rien |
| Garder l'agent externe hors de `Sampled` | C'est l'état qui a motivé cet ADR : un niveau que l'utilisateur a choisi et qu'une destination entière ignore |
| Nouvelle session ACP dans le même processus | Efface l'historique du protocole, pas l'état du processus ; la garantie dépendrait de chaque adaptateur (§ 5) |
| Une API d'outils propre au pont MCP pour les échantillons | Deux jeux d'outils divergent, et c'est celui que personne ne relit que l'agent emprunte ([ADR-0030 § 1](0030-outils-oxyn-exposes-a-un-agent-externe.md)) |
