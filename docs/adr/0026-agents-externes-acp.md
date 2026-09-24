# ADR-0026 — Un agent externe parle ACP, ne confie aucune clé, et reste hors de portée d'une connexion `Local`

**Statut :** accepté · **Date :** 2026-09-14

**Précise :** [ADR-0023](0023-fournisseurs-declares-et-provenance.md), qui ne
connaissait qu'un seul mode — un fournisseur d'API déclaré avec sa clé.

## Contexte

[ADR-0023](0023-fournisseurs-declares-et-provenance.md) suppose qu'un
fournisseur se déclare avec une **clé**, rangée au trousseau, et qu'Oxyn parle
directement au modèle. C'est le seul mode qu'`oxyn-llm` connaît : sept
fournisseurs compatibles OpenAI, plus Anthropic et Gemini.

Ce mode a un coût dont personne ne discute parce qu'il paraît inévitable :
**Oxyn détient un secret de l'utilisateur**. Tout le travail d'[I-03](../../CLAUDE.md#i-03)
— six canaux à surveiller, pas de `#[derive(Debug)]` sur un porteur de secret,
`ApiKey` qui ne s'affiche jamais — existe pour protéger ce secret. Or la seule
façon certaine de ne pas divulguer une clé est de **ne pas l'avoir**.

Il existe un second mode, et il est déjà éprouvé ailleurs. Les faits, vérifiés
et datés dans [RESEARCH-NOTES](../RESEARCH-NOTES.md#agent-client-protocol--vérification-du-2026-09-14) :

* l'**Agent Client Protocol** est du JSON-RPC sur `stdio` ; un agent local est un
  **processus enfant** de l'application hôte ;
* `agent-client-protocol` **2.1.0** (crates.io, 2026-09-04) est sous
  **Apache-2.0**, déclare `rust-version` 1.88.0 et l'édition 2024 ;
* Zed déclare un agent externe par une commande, des arguments et un
  environnement, et sa documentation est explicite : **aucune clé d'API n'est
  requise** — l'agent porte sa propre authentification, et la facturation comme
  la rétention des données regardent l'utilisateur et le fournisseur de l'agent,
  pas l'éditeur.

Autrement dit : l'utilisateur a déjà Claude Code ou Gemini CLI installé et
authentifié. Lui demander en plus une clé d'API pour Oxyn, c'est lui demander de
payer deux fois et de nous confier un secret de plus.

**Ce qui rend la décision coûteuse à défaire**, et donc ADR : le protocole place
la demande d'autorisation d'outil **du côté du client**. Si Oxyn devient client
ACP, ce point d'entrée devient une frontière de sécurité permanente. Le câbler
au mauvais endroit une fois se paie longtemps.

## Décision

**Oxyn accepte deux modes de fournisseur, et un agent externe n'apporte jamais
de clé.**

### Le mode agent externe

Oxyn est le **Client** ACP. Un agent se déclare comme une commande à lancer —
programme, arguments, environnement — au même endroit que les fournisseurs
d'ADR-0023, dont il est une variante et non un système parallèle. Aucune
référence de secret ne lui est associée : le champ n'existe pas pour lui.

### Deux domaines d'autorisation, et non un seul

**Cette section corrige la première rédaction de cet ADR**, qui affirmait que
`session/request_permission` « est » le point d'entrée du `PolicyGate`. La
lecture du protocole, faite en implémentant, a montré que c'était faux — et la
correction vaut d'être écrite, parce que l'erreur était séduisante.

La demande d'autorisation porte un `tool_call` décrivant un outil **de l'agent**
(`ToolKind::Read`, `Edit`, `Delete`, `Move`, `Execute`, `Fetch`, `Search`,
`Think`, `SwitchMode`, `Other`) : lire un fichier, en éditer un, lancer une
commande. Ce ne sont pas des `Command` d'Oxyn, et il n'existe aucune traduction.
« L'agent veut éditer `/etc/hosts` » ne devient pas une commande de base de
données.

Il y a donc **deux domaines**, et les confondre aurait produit un pont bancal :

| Ce que l'agent demande | Qui décide |
|---|---|
| …**à Oxyn** — exécuter une requête, rafraîchir le catalogue | le `PolicyGate`, par le registre d'outils existant, avec `Actor::Agent` — inchangé, [I-01](../../CLAUDE.md#i-01) et [I-07](../../CLAUDE.md#i-07) tenus comme avant |
| …**à la machine** — fichiers, commandes, réseau | `oxyn_ai::external::permission_for`, **refus par défaut** |

**Oxyn n'est pas un hôte d'agent de code**, et c'est ce qui décide du second
domaine. Rien dans le périmètre d'un atelier de bases de données ne justifie
d'accorder à un sous-processus le droit d'écrire des fichiers ou de lancer des
commandes. Surtout, Oxyn n'a **pas d'écran pour montrer quel** fichier ou
**quelle** commande est en jeu : une confirmation qui ne dit pas ce qu'elle
autorise déplace la responsabilité sans donner de quoi l'exercer, et
[I-02](../../CLAUDE.md#i-02) dit déjà qu'une confirmation finit par être cliquée.

Seuls `Think` et `SwitchMode` sont accordés : ils ne quittent pas l'agent. Tout
le reste est refusé d'office, avec une raison — constante, donc incapable de
recopier un chemin de fichier — renvoyée à l'agent pour qu'il cesse d'insister.
Un genre inconnu du protocole est refusé aussi : accorder ce qu'on ne sait pas
juger, c'est accorder ce que le protocole inventera.

Un éditeur de code accorde ces droits parce que son périmètre les rend sensés et
qu'il sait les montrer. Copier ce choix sans l'un ni l'autre aurait ouvert un
accès au système derrière une fenêtre de base de données.

### Un agent externe vaut `Reach::Unresolved`, toujours

**Oxyn ne peut pas savoir où va le modèle d'un agent externe.** L'agent est un
processus opaque ; il peut parler à un modèle local, à un service distant, ou
changer d'avis entre deux tours. Ce n'est pas une information périmée comme le
`Reach` d'un fournisseur déclaré ([ADR-0023](0023-fournisseurs-declares-et-provenance.md)) :
c'est une information **inconnaissable**.

Le dépôt a déjà une doctrine pour cela, et il n'y a rien à inventer :
`allows_endpoint(PrivacyTier::Local, Reach::Unresolved)` rend `false`, sous le
commentaire « dans le doute, on protège » (`oxyn-ai/src/privacy.rs`). Un agent
externe se range donc dans `Reach::Unresolved`, et la conséquence tombe toute
seule :

| Niveau de la connexion | Agent externe |
|---|---|
| `Local` | **refusé** — la promesse « rien ne sort de la machine » ne peut pas être tenue par un processus dont on ne voit pas la sortie |
| `Metadata` *(défaut)* | permis, et le point de passage unique s'applique comme pour un fournisseur |
| `Sampled` | permis, échantillon approuvé colonne par colonne comme ailleurs |

Le refus se dit à l'écran, avec sa raison, plutôt que de griser une entrée sans
explication — c'est ce qu'impose déjà la barre de connexion pour `Ask AI`.

### Ce qui ne change pas

Le point de passage unique d'[I-04](../../CLAUDE.md#i-04) reste le point de
passage unique : ce qui part vers un agent externe traverse le même filtre que ce
qui part vers un fournisseur. Un agent externe est une **destination**, pas une
dérogation. `oxyn-llm` garde son périmètre — parler à un modèle — et n'apprend
rien d'ACP ; le mode agent vit dans `oxyn-ai`, qui connaît déjà les niveaux.

## Ce qui est déjà écrit

La décision se met en œuvre par tranches, et la première est celle qui porte
l'invariant — pas le transport :

* `oxyn_core::ExternalAgentConfig` déclare un agent : identifiant, nom,
  commande, arguments, environnement. **Il n'a pas de champ de secret**, et
  c'est le sujet. Il n'est **pas** un `AiProviderConfig` : y entrer aurait
  produit une structure dont la moitié des champs ne veut rien dire selon la
  variante ;
* son `Debug` est **manuel** et ne rend que le *nombre* de variables
  d'environnement. Le champ ne doit pas porter de secret, sa documentation le
  dit — mais c'est une consigne à l'utilisateur, pas une garantie, et un
  `tracing::debug!` ajouté plus tard ne doit pas la mettre à l'épreuve
  ([I-03](../../CLAUDE.md#i-03)) ;
* `validate` refuse un nom ou une commande vide, un caractère de contrôle dans
  la commande ou un argument, et borne les deux listes — un fichier d'état écrit
  par un tiers ne doit pas faire allouer à l'ouverture ;
* `oxyn_ai::privacy::agent_reach` et `allows_external_agent` portent la
  conséquence de confidentialité, **dans `oxyn-ai`** : `oxyn-core` ne connaît pas
  `Reach`, et en dépendre inverserait le sens des dépendances. Tenu par
  `un_agent_externe_ne_sert_jamais_une_connexion_locale`.

* `oxyn_ai::external::permission_for` décide du second domaine d'autorisation,
  genre par genre, refus par défaut. Trois tests, dont
  `aucun_acces_au_systeme_nest_accorde` qui énumère les genres plutôt que de les
  traiter en bloc — ajouter une variante au protocole ne doit pas relâcher la
  garantie en silence.

* `oxyn_ai::external::launch_config` traduit une déclaration en configuration de
  lancement, **sans jamais recoller** la commande et ses arguments en une chaîne.

L'exemple du protocole part d'une chaîne unique — `"python my_agent.py"` —
qu'il découpe. Un découpage de ligne de commande est une grammaire, donc une
surface : un chemin contenant une espace, un guillemet ou un point-virgule y
prend un sens qu'on n'a pas voulu. `AcpAgentConfig::new(commande).args(…)`
transmet les parties séparément, et c'est cette voie qui est prise. Tenu par
`la_commande_et_ses_arguments_ne_sont_jamais_recolles`, qui passe
`/opt/mes agents/claude code` et l'argument `; rm -rf /` et vérifie qu'ils
arrivent intacts et distincts.

La validation est **refaite** au lancement, pas seulement à la saisie : une
déclaration peut venir d'un fichier d'état écrit par un tiers ou par une version
future.

La dépendance `agent-client-protocol` est déclarée depuis que ces modules
l'emploient.

* `oxyn_ai::external::option_for` choisit l'option de réponse qui exprime le
  verdict, avec une **asymétrie délibérée** : jamais « autoriser toujours »,
  mais « refuser toujours » dès que l'agent l'offre.

Mémoriser un accord large est une décision que l'utilisateur n'a pas prise et que
rien à l'écran ne lui montrerait. Mémoriser un refus n'en est pas une : ce
qu'Oxyn refuse, il le refusera à chaque fois — c'est une propriété du produit,
pas une humeur —, et laisser l'agent redemander à chaque tour lui ferait perdre
le sien devant un utilisateur qui ne comprendrait pas pourquoi la conversation
tourne en rond. Faute d'option convenable, la réponse est `Cancelled` : prétendre
autoriser en sélectionnant une option de refus, ou l'inverse, serait pire
qu'interrompre. Tenu par trois tests, dont
`aucune_option_convenable_ne_se_remplace_par_son_contraire`.

* `oxyn_ai::external::turn::run_turn` câble le tour complet : vérification du
  niveau, lancement, `InitializeRequest` → `NewSessionRequest` →
  `PromptRequest`, fragments remontés par l'`AgentObserver` **existant**.

Trois choix de ce module méritent d'être écrits, parce qu'ils ne se déduisent pas
du protocole :

* **le niveau se vérifie avant le lancement**, pas avant l'envoi. Démarrer
  l'agent puis refuser de lui parler serait déjà trop tard : le seul fait de le
  lancer peut suffire à lui faire contacter son service ;
* **le répertoire de travail annoncé est un temporaire**, pas celui de
  l'utilisateur. L'agent n'a de toute façon aucune autorisation de toucher au
  disque, mais lui *annoncer* un répertoire de projet lui ferait nommer des
  chemins réels dans ses demandes, donc dans ce qui s'affiche. Ne pas les lui
  donner est plus simple que de les filtrer ensuite ;
* **l'erreur du protocole ne remonte pas telle quelle.** Son message peut
  recopier l'invite ([I-03](../../CLAUDE.md#i-03)) ; ce qui remonte est le fait,
  pas le texte.

Seuls les fragments de texte sont remontés à l'interface. Un `plan` ou un
`tool_call` d'agent externe décrit un travail qu'Oxyn n'a pas autorisé : l'afficher
comme le sien laisserait croire qu'il est en cours. Un refus, lui, se voit — une
conversation qui bute sans rien afficher se lit comme une panne —, et il réutilise
`AiError::ToolNotAllowed` plutôt qu'une variante jumelle, qui aurait donné deux
façons de dire la même chose.

* **migration 9** — la table `external_agents`, et `oxyn_store::ExternalAgents`
  pour la lire et l'écrire. Cinq tests.

Table séparée d'`ai_providers`, et non colonnes ajoutées : un agent n'a ni point
d'accès, ni modèle, ni référence de secret, et les faire cohabiter aurait produit
une table dont la moitié des colonnes ne veut rien dire selon la ligne. Le test
`la_table_na_aucune_colonne_de_secret` lit le **schéma** plutôt que la
documentation — une colonne ajoutée un jour « juste pour un jeton » ne ferait
rougir aucun autre test.

Une ligne devenue illisible est **écartée avec une trace**, pas propagée en
erreur : un `args` corrompu par un éditeur SQLite ne doit pas rendre l'écran de
configuration inutilisable. C'est le parti déjà retenu pour les fournisseurs.

* **le bus** — `ListExternalAgents`, `SaveExternalAgent`, `RemoveExternalAgent`,
  refusées à un `Actor::Agent` par le `PolicyGate`.

Ce refus vaut pour la raison qui protège déjà la déclaration d'un fournisseur,
**en plus fort** : déclarer un agent externe, c'est désigner un **programme à
lancer**. Un agent qui y parviendrait obtiendrait l'exécution de code arbitraire
sur la machine, par le chemin le plus court qui soit. Tenu par
`un_agent_ne_declare_pas_dagent_externe`, dont la déclaration hostile est
`/bin/sh -c "curl … | sh"` — lire la liste, en revanche, reste permis, puisque
cela ne déclare rien.

* **l'écran** — les deux sortes dans une liste, une ligne calculée hors du rendu,
  et la saisie d'un agent validée par le domaine lui-même.

Un choix de saisie mérite d'être écrit, parce qu'il découle du reste : les
arguments se tapent **un par ligne**. Séparer sur l'espace aurait obligé à
inventer des guillemets pour l'argument qui en contient une — donc une grammaire
de ligne de commande, donc exactement la surface que ce lot évite depuis le
lancement. Une ligne par argument n'a aucun cas ambigu : ce que l'utilisateur
tape est ce que le processus reçoit, espaces comprises.

La validation du formulaire **délègue** à `ExternalAgentConfig::validate` plutôt
que de redire ses règles : deux validations divergent, et c'est celle du domaine
qui décide au moment d'écrire. Le formulaire ne fait que la poser plus tôt, pour
que le message arrive pendant la saisie.

* **le panneau** — `Backend::start_agent_turn` rend la **même** forme que
  `start_conversation` : un canal d'événements et un jeton d'annulation.

Le panneau ne sait donc pas laquelle des deux sortes lui parle, et c'est ce qui
lui évite deux affichages pour une seule conversation. `TurnEnd` traduit la
raison d'arrêt du protocole **au bord** d'`oxyn-ai` : `StopReason` ne traverse
pas cette frontière, sans quoi `oxyn-app` dépendrait de la crate du protocole
pour lire une fin de conversation — et le choix de ce protocole cesserait d'être
réversible. `MaxTokens` devient `truncated`, parce qu'une réponse coupée qui ne
le dit pas ressemble à une réponse fausse.

**L'ordre de choix est délibéré : le fournisseur d'abord.** Un agent externe ne
prend le relais que si aucun fournisseur n'est utilisable. Oxyn sait où vont les
données d'un fournisseur ; d'un agent, il ne peut que nommer la destination.
Préférer ce qu'on peut vérifier à ce qu'on ne peut que nommer n'a pas besoin
d'autre justification.

Le lot est **complet** : déclarer, persister, lister, retirer, lancer, converser.
Ce qui reste — l'environnement d'un agent dans le formulaire, un second agent
choisi explicitement plutôt que le premier — est du confort, pas du trajet.

## Conséquences

* **+** **Oxyn ne détient aucun secret dans ce mode.** Le meilleur traitement
  d'un secret est de ne pas l'avoir, et [I-03](../../CLAUDE.md#i-03) n'a alors
  plus rien à protéger de ce côté.
* **+** L'utilisateur réutilise l'abonnement et l'authentification qu'il a déjà,
  au lieu de payer un second accès et de nous confier une clé.
* **+** [I-01](../../CLAUDE.md#i-01) et [I-07](../../CLAUDE.md#i-07) deviennent
  **structurels** : le protocole oblige l'agent à demander, et notre `PolicyGate`
  est ce qui répond.
* **+** La licence (Apache-2.0), l'édition et le MSRV de la crate sont
  compatibles sans toucher à la chaîne d'outils.
* **−** **Une connexion `Local` perd l'accès aux agents externes**, y compris
  quand l'agent tourne réellement contre un modèle local. C'est une restriction
  que l'utilisateur jugera excessive dans ce cas précis, et elle est assumée :
  nous ne savons pas la lever sans lui demander de nous croire sur parole.
* **−** Une dépendance de plus, sur un protocole dont la version 2 est encore un
  **brouillon**. Le numéro de version de la crate ne dit pas celui du protocole,
  ce qui est un piège à documenter à chaque montée.
* **−** **Un second réacteur asynchrone entre dans le processus.** Mesuré le
  2026-09-14 : la crate tire `async-io 2.6.0`, `async-process 2.5.0`,
  `async-signal 0.2.14` et `blocking 1.7.0` en dépendances **normales**.
  `async-io` démarre un fil de réacteur à la première utilisation, et `blocking`
  son propre pool — à côté de Tokio, qui est le modèle de fils documenté par
  [ARCHITECTURE](../ARCHITECTURE.md#le-modèle-de-threads). Ce n'est pas le
  runtime complet qu'on pouvait craindre : `smol` et les exécuteurs globaux ne
  sont **pas** dans le graphe, vérifié par `cargo tree --edges normal`. Mais deux
  réacteurs se cohabitent avec discipline, et [I-05](../../CLAUDE.md#i-05) vaut
  pour les deux.
* **−** Un processus enfant se surveille : mort inattendue, sortie d'erreur à ne
  pas confondre avec du protocole. La crate porte déjà le plus délicat — le
  chemin `ConnectTo` installe un garde qui démolit **le groupe de processus**,
  parce qu'un agent distribué derrière `npx` ou `uvx` survivrait sinon à la
  mort de son lanceur en se ré-attachant à pid 1. C'est précisément le genre de
  détail qu'on écrit mal quand on le réécrit.
* **−** Ce que l'agent fait de nos métadonnées **nous échappe**. Nous ne pouvons
  le dire à l'utilisateur qu'en le nommant, pas en le garantissant.

**Coût de sortie :** faible tant que le mode agent reste une variante de
déclaration et une destination. Il devient élevé si le protocole gagne le droit
de déclencher autre chose que des `Command` — c'est la ligne à ne pas franchir,
et c'est elle qui justifie cet ADR plutôt qu'un simple lot.

**Reconsidérer si** le protocole permet un jour à un client de **vérifier** la
destination réelle du modèle d'un agent : la ligne du tableau sur `Local`
s'ouvrirait, et c'est la seule chose qui l'ouvrirait.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| N'offrir que les fournisseurs d'API | Oblige l'utilisateur à confier une clé et à payer un second accès alors qu'il a déjà un agent installé et authentifié. C'est le coût qu'ADR-0023 ne discutait pas parce qu'il paraissait inévitable |
| Lancer l'agent et **deviner** sa portée (inspection des sockets, des variables d'environnement) | Une heuristique qui se trompe en faveur de l'utilisateur est exactement le mode de panne qu'[ADR-0006](0006-ai-privacy-tiers.md) existe pour empêcher. « Dans le doute, on protège » est déjà la règle du dépôt |
| Permettre les agents externes sur `Local` en **avertissant** l'utilisateur | Un avertissement transforme une garantie en recommandation. `Local` promet « rien ne sort de la machine » : une promesse assortie d'une case à cocher n'est plus une promesse |
| Écrire notre propre protocole de sous-processus | Aucun agent existant ne le parlerait. L'intérêt du mode est précisément de réutiliser ce que l'utilisateur a déjà installé |
| Intégrer ACP dans `oxyn-llm` | `oxyn-llm` ne connaît ni les agents ni les niveaux, et c'est ce périmètre étroit qui le rend relisible. Un agent n'est pas un transport vers un modèle : c'est un pair qui demande des autorisations |
