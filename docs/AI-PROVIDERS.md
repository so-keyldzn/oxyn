# Frontière IA

> **Autorité** : ce qui traverse la frontière vers un fournisseur d'IA, comment
> le programme s'y prend, et ce qu'il fait des réponses.

Le **quoi** est tranché par [ADR-0006](adr/0006-ai-privacy-tiers.md) : trois
niveaux — `Local`, `Metadata` (défaut), `Sampled` — choisis **par connexion**.
Ce tableau n'est pas recopié ici ; il vit dans l'ADR. Ce document porte ce que
l'ADR ne dit pas : les conséquences dans le code.

Invariants concernés : [I-04](../CLAUDE.md#i-04), [I-07](../CLAUDE.md#i-07),
[I-03](../CLAUDE.md#i-03).

## Pourquoi ce document existe

« Privacy first » et « AI when it adds value » sont deux principes fondateurs
([VISION](VISION.md)) qui, mal appliqués, produisent exactement l'incident que
le produit prétend éviter.

**Panne concrète :** l'utilisateur règle le niveau sur `Sampled` pour sa base de
bac à sable, l'oublie. Trois jours plus tard il ouvre la base client de son
employeur et demande « trouve-moi les doublons ». Si le niveau était global, des
lignes réelles partent chez un fournisseur tiers. Techniquement rien n'a échoué ;
contractuellement, c'est une violation de confidentialité, et elle est
irréversible. C'est la raison pour laquelle le niveau est attaché à la
**connexion** et jamais à l'application, au fournisseur ou à la session.

## Le point de passage est unique

Il existe **une seule** fonction par laquelle du contexte peut rejoindre une
invite, et c'est elle qui applique le niveau. Toute autre voie est un défaut,
pas une optimisation.

Cette contrainte est ce qui rend [I-04](../CLAUDE.md#i-04) vérifiable : on relit
un point de passage, pas chaque appel de chaque agent. Elle se double de la règle
d'architecture qui l'appuie — `oxyn-ai` ne parle jamais à un driver, il reçoit du
contexte déjà collecté
([ARCHITECTURE](ARCHITECTURE.md#le-sens-des-dépendances)).

Ce qui ne sort sous **aucun** niveau, `Sampled` compris : identifiants de
connexion, chaînes de connexion, jetons, contenu du trousseau. Il n'y a pas de
dialogue pour ça — c'est [I-03](../CLAUDE.md#i-03), et le code ne doit pas offrir
le chemin.

## `Metadata` par défaut n'est pas « rien ne sort »

Le défaut de [ADR-0006](adr/0006-ai-privacy-tiers.md) est `Metadata` : le DDL,
les noms, les types, les index, les cardinalités et les plans d'exécution
**sortent** dès qu'un fournisseur distant est configuré et qu'une fonction IA
est utilisée.

C'est un compromis délibéré — sans schéma, un assistant de base de données ne
sert à rien — mais il faut le regarder en face : *un nom de colonne est déjà une
donnée*. Une table `patients` avec une colonne `hiv_status` révèle l'essentiel
sans qu'une seule ligne ne sorte.

Deux conséquences dans le code :

1. **Le niveau effectif est visible en permanence**, pas dans un panneau de
   réglages. Un utilisateur qui ne peut pas dire d'un coup d'œil où part sa
   requête ne donne pas un consentement éclairé.
2. **`Local` doit rester utilisable**, pas être une case qui désactive tout. Une
   fonctionnalité qui ne marche qu'en `Metadata` le dit dans l'interface plutôt
   que d'échouer sans explication ([ADR-0006](adr/0006-ai-privacy-tiers.md), §
   conséquences).

## Local et distant ne se distinguent pas par l'API

Un fournisseur local (Ollama, LM Studio, llama.cpp) et un fournisseur distant
(OpenAI, Anthropic, Gemini, Bedrock, Azure, tout point d'accès compatible OpenAI)
exposent souvent la **même** API. Ils se distinguent par un seul fait : les
données quittent la machine, ou non.

> **Piège à traiter dans le code :** un point d'accès « compatible OpenAI »
> pointé sur `localhost` peut être un proxy qui réémet vers le nuage. Le
> classement local/distant se fait sur l'hôte réel **après résolution**, jamais
> sur la présence de `localhost` dans l'URL, et il se re-vérifie à chaque
> changement de configuration.
>
> **Aucune redirection HTTP n'est suivie ; un `3xx` est une erreur.** Un `307`
> ou un `308` renverrait l'invite, et une clé que la pile HTTP ne sait pas
> retirer (`x-api-key`, `api-key`), vers une origine que personne n'a classée.
> L'erreur demande de pointer l'URL de base sur l'adresse finale, sans recopier
> la destination.

## Un agent externe : la portée n'est pas inconnue, elle est **inconnaissable**

Depuis [ADR-0026](adr/0026-agents-externes-acp.md), une destination peut aussi
être un **agent externe** — un programme déjà installé et authentifié chez
l'utilisateur, lancé en sous-processus et parlant l'Agent Client Protocol. Oxyn
ne détient alors **aucune clé** : l'agent porte sa propre authentification.

Oxyn a un **préréglage** pour deux agents, Claude Code et Codex : un adaptateur
ACP à version épinglée, dont les réglages de confinement ont été mesurés
([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). Tout autre agent —
Gemini CLI, que l'ADR-0026 citait en exemple, compris — se déclare **à la main**,
par sa commande et ses arguments, et n'est **pas confiné**. La déclaration et ce
que l'écran en montre sont décrits dans
[UX-SPEC](UX-SPEC.md#configuration-des-fournisseurs-et-des-agents).

Ce mode déplace la question de ce document, et il faut le dire nettement. Pour un
fournisseur déclaré, la portée se **mesure** : on résout l'hôte, et la réponse
peut être périmée — d'où la re-vérification à chaque ouverture. Pour un agent
externe, il n'y a rien à mesurer. C'est un processus opaque : il peut parler à un
modèle local, à un service distant, ou changer entre deux tours, et **rien dans
le protocole ne permet de le lui demander**.

La conséquence tombe sans mécanique nouvelle, parce que le dépôt traite déjà
`Reach::Unresolved` comme distant :

| Niveau de la connexion | Agent externe |
|---|---|
| `Local` | **refusé** — la promesse « rien ne sort de la machine » ne peut pas être tenue par un processus dont on ne voit pas la sortie |
| `Metadata` *(défaut)* | permis — la structure de la base part avec la première question d'une session d'agent, et sur demande par l'outil `describe_schema` ; les deux rendus par `ContextBuilder` |
| `Sampled` | permis, la structure part comme sous `Metadata`, et l'échantillon **approuvé colonne par colonne** aussi — épinglé par l'utilisateur ou demandé par l'agent avec l'outil `request_sample`. Le processus qui a reçu des valeurs est relâché après l'échange : la question suivante en lance un autre ([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)). Ses outils MCP traversent le `PolicyGate`, relisent le niveau à chaque appel ; `execute_query` ne rend que la forme d'un résultat — jamais ses valeurs |

Le détail de l'échantillon est dans [Échantillon approuvé](#échantillon-approuvé).

Le refus tombe **avant le lancement**, pas avant l'envoi : le seul fait de
démarrer l'agent peut suffire à lui faire contacter son service.

> **Ce qui entre dans l'invite, et par où.** Pour un fournisseur comme pour un
> agent externe, le point de passage est tenu par le type. `run_turn` exige un
> `AgentPrompt` ([ADR-0027](adr/0027-porte-unique-pour-les-deux-destinations.md)),
> qui ne naît que sous le niveau de la connexion : la question seule
> (`from_user`), ou la question précédée de la structure de la base
> (`with_schema`). Cette structure est rendue par `ContextBuilder::build` — le
> même code, le même budget, le même encadré que pour l'assistant interne — et
> elle ne part qu'à l'ouverture d'une session d'agent : une question qui suit une
> réponse de la même session porte la question seule, comme une conversation de
> fournisseur remémorée. `with_schema` reçoit aussi l'échantillon approuvé pour
> **cette** question, qu'il passe à `ContextBuilder::with_samples` : sous tout
> autre niveau que `Sampled`, il est écarté. `from_user`, qui continue une
> session, n'en reçoit jamais — une question avec échantillon ouvre toujours une
> session neuve ([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)).
>
> **La structure au-delà de ce contexte** passe par l'outil `describe_schema`,
> le même pour toute destination — boucle d'outils d'un fournisseur ou pont MCP
> d'un agent externe. Il devient une `Command::DescribeCatalog` qui traverse le
> `PolicyGate`, précédée des lectures de métadonnées qui manquent au cache
> ([Ce que l'IA voit du schéma](#ce-que-lia-voit-du-schéma-et-quand-cest-lu)) ;
> l'exécuteur rend la poignée du catalogue local, et `oxyn-ai` la
> rend par `ContextBuilder::build`, sous le niveau relu à l'appel. Le rendu ne
> connaît que le modèle commun du catalogue : une collection, un motif de clés ou
> un label de graphe s'y décrivent comme une table, dans le langage de requête de
> la connexion ([ADR-0030 § 4 bis](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-la-structure-de-la-base--un-outil-pour-toutes-les-destinations)).

> **Ce qu'Oxyn ne peut pas promettre ici**, et qu'il ne promet donc pas : ce que
> l'agent fait de ce qu'il reçoit. La facturation, la rétention et le traitement
> des données regardent l'utilisateur et le fournisseur de l'agent. Oxyn peut le
> **nommer** ; il ne peut pas le garantir. C'est précisément pourquoi `Local`
> reste fermé à ce mode.

### Ce que l'agent demande, lui, à la machine

Un agent externe demande des autorisations à son client — lire un fichier, en
éditer un, lancer une commande. **Ce ne sont pas des `Command` d'Oxyn**, et il
n'existe aucune traduction : « éditer `/etc/hosts` » ne devient pas une commande
de base de données. Le `PolicyGate` garde son domaine ; celui-ci est traité à
part, et **refusé par défaut**.

Oxyn est un atelier de bases de données : rien dans son périmètre ne justifie
d'accorder à un sous-processus l'accès au système de fichiers ou à un shell.
Surtout, il n'a pas d'écran pour montrer *quel* fichier ni *quelle* commande —
et une confirmation qui ne dit pas ce qu'elle autorise déplace la responsabilité
sans donner de quoi l'exercer. Seuls le raisonnement interne et le changement de
mode sont accordés : ils ne quittent pas l'agent.

**Refuser ne suffit pas.** Un agent ne demande que ce que son mode l'oblige à
demander : mesuré le 2026-09-23, Claude Agent et Codex lançaient une commande
sans rien demander dans leur mode initial. Un agent dont Oxyn connaît
l'adaptateur est donc **confiné au lancement** : ses outils de la machine sont
retirés ou coupés, ses outils personnels désactivés, et il est maintenu dans son
mode le plus strict. Les outils d'Oxyn, eux, sont préautorisés côté agent,
puisque le `PolicyGate` les contrôle ([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)).
Un agent déclaré à la main n'est pas confiné, et l'écran le dit.

Ce que le panneau montre en retour du travail de l'agent — texte, raisonnement,
sorte et état de ses étapes, plan — et ce qu'il n'en montre jamais est décrit
dans [UX-SPEC](UX-SPEC.md#ce-que-le-panneau-montre-dun-agent-externe) ; la
règle précise l'ADR-0026, qui ne remontait que le texte.

## Ce que l'IA voit du schéma, et quand c'est lu

Le contexte se rend depuis le **cache** du catalogue, jamais d'un aller-retour
serveur fait par `oxyn-ai`. Or l'arbre charge ce cache à la demande : la liste
des tables d'un schéma quand l'utilisateur le déplie, les champs d'une table
quand il l'ouvre. Sans complément, l'IA ne saurait que ce qui a été cliqué —
« Describing 0 of 0 known relations » sur une base de onze tables. Oxyn complète
donc le cache **lui-même**, sans modèle, avant de le rendre
([ADR-0036](adr/0036-l-assistant-complete-le-catalogue.md)) :

| Quand | Ce qui est lu | Acteur |
|---|---|---|
| une question ouvre une session — fournisseur, ou agent externe à sa première question | le serveur s'il n'a jamais été lu ; la liste des relations de chaque schéma jamais listé ; puis champs, index et clés étrangères des relations **que la porte retiendra** — mentions `@` d'abord, puis la recherche orientée par la question | `Actor::Human` |
| une question qui suit une session déjà informée | les relations **mentionnées** seulement | `Actor::Human` |
| le premier `@` du panneau sur une connexion (`ai_list_mentionable`) | le serveur s'il n'a jamais été lu, puis la liste des relations de chaque schéma jamais listé — des noms à choisir, **aucune description** | `Actor::Human` |
| l'outil `describe_schema` | comme une ouverture, orienté par ses mots de recherche | `Actor::Agent` |
| l'outil `refresh_catalog` | le serveur, puis la liste des relations de chaque schéma, **même fraîche** | `Actor::Agent` |

Ce qui encadre ces lectures :

1. **Des métadonnées, par le bus.** Chaque lecture est une
   `Command::RefreshCatalogScope` — celle d'un dépliage de l'arbre —, soumise
   par `ExecutorSink` : le `PolicyGate` décide, le journal l'inscrit avec son
   acteur ([I-01](../CLAUDE.md#i-01)). Aucune ligne n'est lue. La politique
   la permet partout, `production` comprise, puisqu'elle n'écrit rien.
2. **La sélection de la porte, et elle seule.** Ce qui est décrit est ce que
   `ContextBuilder::build` retiendra, calculé par la même fonction
   (`oxyn_ai::context::wanted_relations`). Un nom que le cache ne liste pas
   n'est jamais envoyé au serveur : une mention se vérifie contre la liste
   avant d'être lue.
3. **Des bornes, sans échec.** Au plus 32 schémas listés, au plus 24 relations
   décrites — le plafond de la porte —, en 5 secondes au total
   (`crates/oxyn-desktop/src/backend/ai/catalog_fill.rs`). Un dépassement
   n'échoue pas la question : le contexte part avec ce qui est chargé, et
   l'encadré dit ce qui manque — « N relations not loaded yet », « N schemas not
   listed yet », « the catalog of this connection has not been read yet ». Ces
   lignes sont des comptes, sans nom : elles sont les mêmes sous tout niveau.
4. **Rien n'est relu.** Un palier lu et non invalidé depuis ne se relit pas ; un
   DDL émis par Oxyn invalide le catalogue, et la question suivante relit une
   fois. Une lecture en échec n'est pas retentée dans la même complétion.
5. **Annulable.** Arrêter la question annule le jeton de la lecture en cours,
   que le driver honore comme pour un dépliage ; le délai la coupe de la même
   façon, et la lecture est attendue deux secondes pour finir proprement plutôt
   qu'abandonnée au milieu d'un échange.

Le reste du produit en profite sans code de plus : l'arbre se met à jour par
l'événement `CatalogUpdated`, et la liste des mentions `@` comme le diagramme
`erd` lisent le même cache.

## Ce qu'on fait des réponses

**Aucune sortie de modèle n'est exécutée directement** ([I-07](../CLAUDE.md#i-07)).
Une proposition d'un agent est une `Command` comme une autre, portant
`Actor::Agent`, et elle traverse le `PolicyGate`
([ADR-0004](adr/0004-command-bus.md)). C'est la même porte que pour un humain :
il n'y a pas de second chemin d'exécution pour l'IA, et c'est précisément ce qui
fait qu'une injection de consigne cachée dans le contenu d'une base produit une
demande d'approbation visible plutôt qu'une exécution.

Sans exception, y compris pour ce qui « ne fait que lire » : un `SELECT` sur une
vue peut déclencher une fonction, et un `EXPLAIN ANALYZE` **exécute réellement**
la requête qu'il analyse — y compris un `DELETE`.

**Panne concrète :** l'assistant propose « voici la requête pour nettoyer les
doublons » et un mode « exécution automatique » la lance. La condition de
déduplication est fausse d'une jointure. Il n'y a pas de `ROLLBACK` : le `DELETE`
était en autocommit.

Une proposition d'écriture affiche toujours, avant exécution : le SQL exact et
la connexion visée avec son marquage d'environnement. **Ces deux-là sont
garantis.**

Une estimation du nombre de lignes touchées s'y ajoute *quand elle est
disponible* — et à ce jour elle ne l'est jamais : `Preview::estimated_rows` est
toujours `None`, et la revue affiche « Nombre de lignes touchées inconnu. »
L'obtenir demande un `EXPLAIN` avant la revue, dont le coût et la forme diffèrent
par driver ; c'est un lot à part, consigné dans
[IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md).

Ce paragraphe promettait l'estimation sans réserve. C'est le chiffre qui
distingue un `UPDATE` d'une ligne d'un `UPDATE` sans `WHERE` : le promettre sans
le fournir laisse croire à une protection qui n'existe pas, au moment précis où
[I-02](../CLAUDE.md#i-02) compte. Le code, lui, était honnête — il dit
« inconnu ».

## Le contenu de la base n'est pas une consigne

Un nom de table, un commentaire de colonne, une valeur de ligne peuvent contenir
du texte imitant une instruction. Ce sont des **données**, à tout niveau, y
compris quand elles arrivent dans une invite. Voir
[SECURITY](SECURITY.md#surface-dentrée).

## Mentions

Un objet que l'utilisateur nomme d'un `@` dans sa question
([UX-SPEC](UX-SPEC.md#nommer-un-objet-dun-)) est **imposé** au contexte. Il
entre par le point de passage unique, et par aucune autre voie :
`ContextBuilder::with_mentions` (`crates/oxyn-ai/src/context/mentions.rs`).

Ce qu'une mention envoie, et ce qu'elle n'envoie pas :

1. **La structure de l'objet, sous le niveau de la connexion.** Une table, une
   vue, une collection est décrite comme toute relation du contexte — champs,
   types, clés, index, commentaires encadrés —, **en tête**, avant ce que la
   recherche lexicale trouve, et sous le **même** budget. Une colonne
   (`table.colonne`) fait décrire sa relation et désigne la colonne par une
   ligne « mentioned by the user ». **Aucune valeur de ligne** : sous
   `Metadata` comme sous `Sampled`, une mention n'ajoute rien de ce que
   l'échantillon approuvé transporte. Nommer n'est pas envoyer.
2. **Une adresse vérifiée, jamais crue.** La webview envoie une adresse du
   catalogue (et un nom de colonne), pas un libellé. Un objet absent du cache
   local, une colonne que sa description ne liste pas, sont **écartés** : rien
   n'en part, et le compte en est montré (`ignoredMentions`). Au-delà de seize
   mentions, la question est refusée avant de partir.
3. **Ce qui ne tient pas se dit.** Une mention qui dépasse le budget n'est pas
   perdue en silence : son nom figure dans l'encadré, « mentioned by the user
   but not described, over the context budget », et l'en-tête du panneau la
   compte (`omittedMentions`).
4. **Une requête sauvegardée** — de cette connexion, ou d'aucune — part comme
   le texte que l'utilisateur a écrit, encadré et borné à 4 000 caractères. Le
   backend la relit par le bus (`Command::OpenDocument`), sa version
   **enregistrée** seulement ; une requête d'une autre connexion est écartée,
   car ses littéraux ne sont pas ceux de cette base.
5. **Toutes les destinations, par la même porte.** Pour une session qui
   s'ouvre — fournisseur ou agent externe —, les mentions mènent le contexte
   initial. Pour une session **déjà ouverte**, qui connaît le schéma depuis son
   ouverture et dont le message système ne se réécrit pas, les objets mentionnés
   sont rendus par le même `ContextBuilder`, en mode `mentioned_only` — sans
   recherche, puisque le reste est dit —, et **précèdent la question** dans le
   message de l'utilisateur, préambule `untrusted` compris. Le même texte
   (`AgentContext::follow_up`) sert `AgentSession::ask_about` et
   `AgentPrompt::following` : un agent externe déjà lancé n'a pas de second
   chemin.
6. **Ce qui est gardé.** La question enregistrée garde ses mentions — sorte,
   libellé, adresse du catalogue ou identifiant du document — dans une colonne
   JSON lisible sans Oxyn ([I-11](../CLAUDE.md#i-11)), bornée à seize mentions
   et 32 Kio. Des noms, **jamais une valeur** ni le texte d'une requête
   sauvegardée. Ce qui est gardé sert à redessiner les puces, rien d'autre :
   une conversation relue n'en renvoie rien au modèle.

## Échantillon approuvé

Ce que chaque niveau laisse sortir est tranché par
[ADR-0006](adr/0006-ai-privacy-tiers.md) ; qui peut déclencher un échantillon et
quelles destinations le reçoivent, par
[ADR-0034](adr/0034-echantillon-pour-toute-destination.md). Cette section décrit
le seul chemin par lequel des **valeurs de ligne** rejoignent une invite, tel
qu'il est codé dans `oxyn-desktop` (`backend/ai/samples.rs`,
`backend/ai/conversation/sampling.rs`). Il vaut pour un fournisseur intégré
comme pour un agent externe.

1. **L'utilisateur approuve, toujours ; l'agent peut demander, jamais
   approuver.** Deux façons d'ouvrir le même écran d'approbation :
   - **l'utilisateur épingle** un objet depuis le menu du catalogue (« Pin to
     question »). L'action n'existe que sous `Sampled`, quand le panneau a une
     destination utilisable — fournisseur ou agent externe —, sur un objet qui
     porte des lignes ; ailleurs elle est absente, pas grisée. Une question porte
     une seule épingle ; elle tombe dès que la question part, ou dès que le
     niveau ou le destinataire ne la permettent plus. L'offre
     (`ai_request_sample`) porte le nom de la source, les colonnes du catalogue,
     la borne de lignes (5) et le destinataire — **aucune valeur** ;
   - **l'agent demande**, par l'outil `request_sample { relation, namespace?,
     columns?, rows? }` — 5 lignes par défaut, **20 au plus**. Hors `Sampled`,
     l'appel est refusé avant tout écran. Sinon l'écran s'ouvre, nomme l'agent
     qui demande, n'offre que les colonnes qu'il a nommées (toutes s'il n'en
     nomme aucune), et l'appel attend la réponse **cinq minutes au plus**.
     Refus, expiration ou arrêt de la question : l'agent reçoit « the user
     declined », sans aucune valeur. La seule réponse possible est la commande
     Tauri `ai_answer_sample` ; l'identifiant de la demande ne part jamais vers
     le modèle.

   Dans l'écran, rien n'est coché par défaut, rien ne coche tout, et Annuler a
   le focus. Cette garantie vaut contre le **modèle**, pas contre un script de
   la webview : un tel script peut demander une offre et la présenter sans
   montrer l'écran, mais il peut déjà lire des pages de résultat et coller leurs
   valeurs dans la question.
2. **Un jeton du backend, à usage unique.** L'offre émet un jeton aléatoire, lié
   à la connexion, à la conversation, à l'échange qu'il suit, à la source, aux
   colonnes proposées et au destinataire que l'écran nomme : fournisseur,
   modèle et portée réseau. Il expire au bout de dix minutes ; il disparaît
   quand l'utilisateur annule l'écran, quand la conversation est supprimée ou
   la connexion oubliée. Une connexion garde au plus quatre jetons en attente :
   au-delà, le plus ancien est oublié. Le front le reçoit sans l'afficher et le
   renvoie. À la question, le backend vérifie, dans cet ordre :
   - le jeton, retiré dès l'arrivée de la question, **même en cas de refus**,
     y compris quand la question est refusée avant de démarrer ;
   - le niveau, **relu depuis le store** et toujours `Sampled` ;
   - la source identique ;
   - les colonnes cochées incluses dans les colonnes proposées ;
   - le destinataire que l'écran a nommé : le même fournisseur intégré et le
     même modèle, dont l'adresse est **reclassée** et ne porte pas plus loin
     qu'à l'offre — ou le même agent externe, toujours `Unresolved`. Un
     fournisseur et un agent ne s'échangent jamais un jeton, même sous le même
     identifiant.

   Une demande d'agent suit le même ordre, sans jeton : niveau relu dans le
   store, relation et colonnes cherchées dans le catalogue, écran, réponse,
   niveau relu. Une réponse mal formée — colonne non proposée, aucune colonne —
   refuse la demande. Une connexion garde au plus quatre demandes en attente.
   Aucun écran ne s'ouvre pendant qu'une approbation de cet agent attend, et un
   échange n'en ouvre **qu'un** : après une approbation, un refus ou une
   expiration, toute nouvelle demande dans la même réponse est refusée sans
   écran, par un message qui ne dit pas quelle fut la réponse.

   Chaque échec est un refus typé, **avant toute lecture**. La lecture passe
   ensuite par `Command::PreviewRelation` sur le bus — auditée comme une lecture
   de l'utilisateur pour une épingle, comme une commande de l'agent
   (`Actor::Agent`, `PolicyGate`) pour une demande —, avec la borne de lignes
   fixée par le backend quoi que le front ou l'agent envoie. Le driver compose
   la lecture et cite la relation ; les colonnes n'entrent dans aucune
   instruction ([I-10](../CLAUDE.md#i-10)). Le niveau est **relu après la lecture** : s'il n'est plus
   `Sampled`, rien n'est envoyé, et c'est ce niveau relu qui gouverne
   l'invite et le contrôle de l'adresse. Les colonnes non cochées sont
   écartées avant la construction de l'invite, et l'échantillon entre dans le
   contexte par `ContextBuilder`, comme le reste. Juste avant l'envoi, et
   seulement si ce rendu a gardé l'échantillon, la sortie est inscrite dans
   `ai_egress`, reliée à l'aperçu qui a lu les lignes ; si l'inscription
   échoue, rien ne part. Un échantillon écarté pour le budget n'est ni inscrit
   ni annoncé comme envoyé
   ([SECURITY](SECURITY.md#ce-qui-sort-vers-un-destinataire-ia-laisse-une-trace)).
   Aucune valeur n'apparaît dans une trace ni dans un message
   d'erreur.
3. **Une question, pas une mémoire.** L'échange qui a porté l'échantillon n'est
   pas retenu : il part avec un contexte neuf, et la question suivante émet
   `memoryReset` pour la raison `sampleNotKept`, avec une ligne qui le dit. La
   conversation ne garde que « N lignes et K colonnes ». Régénérer ou modifier
   la question ne réutilise pas l'approbation : il faut la redonner. Pour un
   **agent externe**, qui est un processus et se souvient : une question avec
   échantillon épinglé ouvre une session neuve, et tout échange qui a porté des
   valeurs **relâche le processus** à sa fin ; la question suivante en lance un
   autre. Ce que l'agent a pu écrire sur son propre disque échappe à Oxyn
   ([ADR-0034 § 5](adr/0034-echantillon-pour-toute-destination.md#5-un-échange-qui-a-porté-un-échantillon-ne-laisse-aucune-mémoire-quelle-que-soit-la-destination)).
4. **Colonnes secrètes : pas encore de filtre.** Aucun driver ne déclare encore
   de classement de colonnes. Plutôt qu'un filtre sur des noms qui ferait croire
   à une protection, l'offre porte un TODO daté, débloqué par ce classement.
   D'ici là, ce qui est proposé est ce que le catalogue rapporte, et la seule
   barrière est la case que l'utilisateur coche.

## Les messages d'erreur du serveur passent par le niveau

Un message d'erreur de base de données **cite ce qu'il refuse** : PostgreSQL
répond `Key (email)=(dupont@example.com) already exists`, un déclencheur SQLite
concatène la valeur qui l'a déclenché. C'est du contenu de ligne, et il franchit
donc le même filtre que le reste : le critère est celui du niveau — seul
`Sampled` autorise les valeurs de ligne.

Sous `Local` et `Metadata`, ce qui parvient au modèle est ce qui ne peut rien
citer : la **famille** de l'erreur, sa retentabilité — dérivée de la famille, pas
déclarée à côté d'elle —, et l'identifiant d'erreur quand il est marqué comme
tel (`SQLSTATE 23505`, `Error code 1811`). Le message dit explicitement au modèle
de ne pas deviner ce qu'il ne voit pas, et de demander à l'utilisateur de lire
l'erreur complète dans Oxyn — qui, lui, la voit entière.

Le filtre s'applique **à la construction**, pas au rendu : sous un niveau qui
masque, le message n'entre jamais dans la structure. Un `Debug` dérivé ou un
champ de journal ajouté plus tard ne peuvent donc pas le laisser sortir — il
n'est plus là.

Ce que ce choix ne protège pas, et qui est assumé : un serveur hostile peut
écrire `SQLSTATE ABCDE` dans son message et faire ainsi sortir cinq caractères
de son choix par échec. Le canal est borné et connu ; le refuser coûterait le
seul identifiant qu'un professionnel utilise réellement pour chercher une
erreur.

## Persistance du niveau

Le niveau est **écrit avec la connexion** (`connections.privacy_tier`, migration
8) : il lui appartient, donc il lui survit. Sans cette colonne, un réglage pris
sur une base client était perdu à la fermeture, sans message, et la connexion
rouverte repartait au défaut.

Deux replis, et leur asymétrie est la décision :

* **colonne absente** — la ligne vient d'un binaire qui ignorait ce réglage,
  l'utilisateur n'en a donc jamais choisi : le défaut d'[ADR-0006](adr/0006-ai-privacy-tiers.md),
  `Metadata`, s'applique ;
* **valeur illisible** — un réglage existait et son sens s'est perdu. Ce n'est
  pas une absence : le niveau retombe sur le plus contraignant, `Local`. Une
  base dont on ne sait plus ce qu'elle autorisait n'obtient pas le bénéfice du
  doute, exactement comme une connexion sans environnement renseigné vaut
  `production`.

### Régler le niveau

Le niveau se règle **dans le formulaire de connexion**, à la création comme à
la modification, par le champ `AI privacy` : trois choix, `Local`, `Metadata`,
`Sampled`, et sous eux la liste de ce qui sortirait et de ce qui resterait
sur la machine, qui suit la sélection **avant** tout enregistrement. Une
connexion neuve part de `Metadata`, le défaut
d'[ADR-0006](adr/0006-ai-privacy-tiers.md).

Le choix n'est appliqué que par l'enregistrement de la connexion : la
modification passe par `UpdateConnection`, la création par
`CreateConnection`, deux commandes du command bus classées comme du DDL sur la
connexion visée ([I-01](../CLAUDE.md#i-01)). Aucun autre chemin n'écrit
`connections.privacy_tier`. Les garde-fous :

* **sur une connexion `production`**, l'enregistrement exige la confirmation
  qui nomme la connexion ([I-02](../CLAUDE.md#i-02),
  [SECURITY](SECURITY.md#marquage-des-connexions)), comme toute écriture sur
  elle ; pour un `Actor::Agent`, c'est un refus ;
* **abaisser le niveau d'une connexion `production`** — vers `Sampled`, ou de
  `Local` vers `Metadata` — est une décision critique : elle ouvre la sortie de
  la structure ou de lignes d'une base client. Elle est destinée au **dialogue
  natif de l'hôte**, hors de la webview, décidé le 2026-09-24 pour les
  confirmations critiques. L'ADR qui l'écrit n'existe pas encore : d'ici là,
  c'est la confirmation de la webview décrite par
  [SECURITY](SECURITY.md#marquage-des-connexions) qui s'applique, et elle
  seule ;
* **le changement vaut dès la question suivante.** Le niveau est relu par le
  backend à chaque question, jamais pris à la session ; un agent externe
  gardé en vie, lancé sous l'ancien niveau, est relâché à l'enregistrement et
  relancé sous le nouveau ([I-04](../CLAUDE.md#i-04)).

## Absence de fournisseur

Sans fournisseur ni agent externe déclaré, le workspace IA est **absent de
l'interface** et Oxyn reste un client complet
([ADR-0006](adr/0006-ai-privacy-tiers.md) ; le comportement d'écran est dans
[UX-SPEC](UX-SPEC.md#le-workspace-ia-nexiste-que-sil-a-été-configuré)). Un chemin de code
qui appelle un modèle pour produire un résultat que l'utilisateur attend comme
déterministe — un tri, un formatage, une complétion de nom de table — est un
défaut de conception, pas une fonctionnalité.

## Ce qui n'est pas encore tranché

- la présentation, avant envoi, de la liste exacte de ce qui va sortir ;
- l'affichage, pour un agent externe, de ce qu'il a **réellement** demandé et
  qu'Oxyn a refusé : le refus est compté et montré, le détail de la demande ne
  l'est pas, faute d'écran qui puisse le rendre sans recopier des chemins ;
- la compaction de contexte sur les bases à plusieurs milliers de tables, que
  [ADR-0006](adr/0006-ai-privacy-tiers.md) désigne comme un composant à part
  entière.
