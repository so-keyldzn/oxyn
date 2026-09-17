# ADR-0023 — Un fournisseur se déclare par machine, se reclasse à chaque ouverture, et signe ce qu'il propose

**Statut :** proposé · **Date :** 2026-09-10

**Précise :** [ADR-0006](0006-ai-privacy-tiers.md), sur deux points que celui-ci
laissait ouverts : où vit la configuration d'un fournisseur, et ce qui reste
d'une proposition d'agent une fois la conversation refermée.

## Contexte

[ADR-0006](0006-ai-privacy-tiers.md) tranche le niveau de confidentialité :
il appartient à la **connexion**. Il pose aussi qu'« aucun fournisseur n'est
requis : sans configuration, le workspace IA est absent de l'UI ». Ces deux
phrases décrivent un état que le code ne sait pas atteindre.

L'état réel du dépôt, vérifié ce jour : `oxyn-ai` (4 480 lignes) et `oxyn-llm`
(5 988 lignes) sont complets et testés — point de passage unique du contexte,
traduction appel d'outil → `Command`, filtrage des échecs par niveau, huit
familles de fournisseurs. **Aucune crate ne les déclare en dépendance.**
`crates/oxyn-exec/src/sink.rs` expose l'`ExecutorSink` sur lequel le trait
`CommandSink` doit se brancher, et le dit explicitement : la traduction revient
à `oxyn-app`, qui dépend des deux. Elle n'y est pas écrite.

Il manque donc, entre un backend d'agents utilisable et une fonction visible,
trois choses qui ne sont pas du câblage : **où la déclaration d'un point d'accès
est rangée**, **quand son classement local/distant est calculé**, et **ce qu'il
reste, six mois plus tard, d'un texte qu'un agent a proposé**.

Trois contraintes bornent les réponses.

**Un secret ne va pas en base** ([I-03](../../CLAUDE.md#i-03)). La table
`connections` porte déjà la forme retenue : `params` en JSON non secret,
`secret_ref` vers le trousseau du système.

**La résolution DNS est bloquante** (`oxyn_llm::reach::resolve_reach`, dont la
documentation le dit) et [I-05](../../CLAUDE.md#i-05) l'interdit sur le fil
d'interface. Elle est aussi périssable :
[AI-PROVIDERS](../AI-PROVIDERS.md#local-et-distant-ne-se-distinguent-pas-par-lapi)
exige que le classement « se re-vérifie à chaque changement de configuration »,
parce qu'un point d'accès compatible OpenAI en écoute sur `127.0.0.1` peut être
un mandataire qui réémet vers le nuage.

**Le journal d'audit ne répond pas à la question posée.** Il porte l'`Actor` de
chaque commande, donc qui a **exécuté**. Le texte d'une requête, lui, vit dans
`documents` — et un `SELECT` proposé par un agent, collé dans une console,
sauvegardé sous un nom, relu l'an prochain, y est indiscernable de ce que
l'utilisateur a écrit lui-même.

## Décision

### Un fournisseur se déclare par machine, pas par workspace

Le store porte une table `ai_providers` (migration 7), **sans `workspace_id`** :

| Colonne | Sens |
|---|---|
| `id` | l'identité de cette déclaration |
| `kind` | la famille : `anthropic`, `openai`, `gemini`, `openai_compatible`… |
| `label` | le nom que l'utilisateur lui donne |
| `base_url` | le point d'accès, **débarrassé de ses identifiants** avant écriture |
| `model` | le modèle par défaut de cette déclaration |
| `secret_ref` | référence au trousseau, `NULL` pour un point d'accès sans clé |
| `created_at`, `updated_at` | |

L'absence de `workspace_id` est la décision, pas un oubli : un Ollama qui écoute
sur la machine sert tous les workspaces, et le dupliquer par workspace créerait
autant d'endroits où sa configuration peut diverger. **Ce qui reste par
connexion, c'est le niveau** — et c'est exactement la séparation qu'ADR-0006
protège : un fournisseur commun ne fait pas un niveau commun.

Aucune clé n'est écrite dans SQLite. `oxyn_llm::ApiKey` ne dérive pas `Debug`
et ne s'obtient que du trousseau, à l'ouverture d'un runtime.

### Le classement local/distant n'est jamais persisté

`Reach` **n'a pas de colonne**. Il est recalculé sur le pool bloquant à chaque
enregistrement et à chaque ouverture de runtime. Une valeur en base serait une
réponse DNS d'hier appliquée à un envoi d'aujourd'hui, et c'est précisément le
piège du mandataire que la documentation demande d'éviter.

Conséquence visible et voulue : l'écran de configuration montre le classement
**avec l'instant de sa mesure**, et `Reach::Unresolved` s'y affiche comme tel
plutôt que d'être arrondi à « local ». Un point d'accès non résolu compte comme
distant partout où une décision se prend (`Reach::leaves_machine`).

### L'entrée IA existe si et seulement si un fournisseur est déclaré

La condition est la liste rendue par le bus, jamais un drapeau de compilation ni
une préférence. Zéro fournisseur : pas de bouton, pas de panneau, aucune mention
— Oxyn reste un client complet. Le premier fournisseur enregistré fait
apparaître l'entrée sans redémarrage.

**Et il le fait sans nouveau canal.** `SaveAiProvider` est une commande locale :
son issue revient déjà à l'appelant par le `oneshot` de `Backend::dispatch`, et
la vue qui l'a émise en avertit la vue parente par un événement GPUI — le même
mécanisme que le formulaire de connexion emploie aujourd'hui. Le bus
d'exécution n'y gagne **aucune variante** : ce qui a changé n'est pas l'état
d'une base, c'est une configuration locale dont la vue qui l'écrit connaît le
sort. Y publier un événement ferait exactement ce qu'[ADR-0022](0022-rafraichissement-automatique.md)
refuse — un second endroit où penser à publier.

Sur une connexion en `PrivacyTier::Local` avec un unique fournisseur distant,
l'entrée est **montrée et désactivée**, avec la raison — ADR-0006 exige que
`Local` reste utilisable, et une fonctionnalité qui ne marche qu'en `Metadata`
« le dit dans l'interface plutôt que d'échouer sans explication ».

### Ce qu'un agent écrit porte sa provenance

`documents` gagne une colonne `provenance`, JSON borné à 512 octets, `NULL` par
défaut. `NULL` veut dire « écrit par l'utilisateur » — c'est le cas de toutes
les lignes existantes, et c'est vrai.

Sinon, quatre champs : l'agent, la session d'agent, la famille de fournisseur,
le modèle, et l'instant. **Ni l'invite, ni la réponse du modèle, ni l'URL, ni la
clé** : la provenance dit *d'où vient ce texte*, elle n'archive pas la
conversation. Un document dont l'utilisateur réécrit le contenu garde sa
provenance : elle date l'origine, pas la dernière frappe.

La provenance ne remplace pas le journal d'audit et ne s'en déduit pas. Le
journal dit qui a **lancé** une exécution ; la provenance dit qui a **écrit** un
texte. Un agent peut proposer un `SELECT` que personne n'exécute, et un humain
peut exécuter cent fois ce qu'un agent a écrit une fois.

### Rien ne change au chemin d'exécution

`oxyn-app` implémente `CommandSink` par une traduction sans logique de
`DispatchReport` vers `DispatchOutcome`, telle que `sink.rs` la décrit, et
n'ajoute aucun appel de driver. Une proposition d'agent est une `Command`
portant `Actor::Agent` qui traverse le `PolicyGate`
([I-01](../../CLAUDE.md#i-01), [I-07](../../CLAUDE.md#i-07)). Aucune sortie de
modèle n'est exécutée sans cette traversée, `EXPLAIN` compris.

## Conséquences

- **+** L'absence de fournisseur reste un état de premier ordre, testable : la
  liste est vide, l'entrée n'existe pas.
- **+** Un mandataire installé sur la boucle locale ne peut pas se faire passer
  pour un fournisseur local durablement : le classement meurt avec le processus.
- **+** La provenance survit à la conversation, qui, elle, ne survit pas à la
  fermeture de la fenêtre.
- **+** Aucune seconde API d'exécution à auditer : le puits d'agent n'appelle
  que ce que l'interface appelle.
- **−** Une résolution DNS à chaque ouverture de runtime, donc une latence au
  premier message d'une conversation — bornée par le délai de résolution du
  système, que nous ne contrôlons pas.
- **−** Un fournisseur commun à tous les workspaces signifie qu'un utilisateur
  ne peut pas isoler un workspace « client » d'un fournisseur configuré pour un
  autre usage. C'est le niveau de la connexion qui doit porter cette
  séparation ; si l'usage montre que ça ne suffit pas, c'est cet ADR qu'il faut
  rouvrir.
- **−** Une migration de plus, et une colonne dont l'ancien binaire ignore
  l'existence : un Oxyn plus ancien rouvrant le store lira les documents sans
  leur provenance, et la réécrira à `NULL` s'il les sauvegarde. La provenance
  est donc une trace, pas une garantie d'intégrité — la prétendre inviolable
  serait mentir.
- **−** La provenance n'est pas propagée par copier-coller entre deux
  documents : le presse-papiers du système ne porte pas de métadonnée, et lui en
  ajouter une serait un canal de plus à auditer sous I-03.

**Coût de sortie :** une table sans dépendant, une colonne nullable, et un
adaptateur de trait dans `oxyn-app`. `oxyn-ai` et `oxyn-llm` ne connaissent ni
l'un ni l'autre : retirer entièrement l'IA de l'interface reviendrait à retirer
la dépendance et l'écran, sans toucher au reste.

**Reconsidérer si** un utilisateur a besoin de fournisseurs distincts par
workspace — auquel cas la table gagne une portée, pas une copie —, ou si la
résolution à chaque ouverture se révèle coûteuse à la mesure, auquel cas un
cache **avec durée de validité explicite** remplacerait le recalcul, jamais une
colonne persistée.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Ranger les fournisseurs par workspace, comme les connexions | Duplique la même configuration ; la séparation qui compte est celle du niveau, et elle reste par connexion |
| Persister `reach` à côté de `base_url` | Applique une réponse DNS périmée à un envoi actuel : exactement le piège du mandataire local que la documentation demande d'éviter |
| Classer sur la présence de `localhost` dans l'URL, sans résoudre | Un nom peut résoudre ailleurs, et `127.0.0.1` peut être un mandataire ; le classement porte sur l'hôte réel |
| Un fichier de configuration hors du store | Un second format à porter, et une seconde place où le `secret_ref` peut être oublié |
| Une variable d'environnement comme seule source (`ApiKey::from_env`) | Utile en test, invisible à l'utilisateur, et non révocable depuis l'interface |
| Déduire la provenance du journal d'audit | Le journal dit qui a exécuté, pas qui a écrit ; un texte proposé et jamais exécuté n'y figure pas du tout |
| Archiver la conversation à côté du document | Fait entrer des invites et des réponses de modèle dans le fichier de workspace, que I-03 compte parmi les six canaux |
| Masquer l'entrée IA sur une connexion `Local` sans fournisseur local | ADR-0006 veut que `Local` reste utilisable et que le refus s'explique ; une entrée qui disparaît sans raison se lit comme un bug |
