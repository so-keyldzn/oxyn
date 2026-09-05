---
paths:
  - "crates/oxyn-ai/**"
---

# Workspace IA — conventions

Fait autorité : [AI-PROVIDERS](../../docs/AI-PROVIDERS.md) et
[ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md). Cette règle porte ce qui se
rate à l'écriture.

## Le point de passage est unique

Une seule fonction fait entrer du contexte dans une invite, et c'est elle qui
applique le niveau de la connexion ([I-04](../../CLAUDE.md#i-04)). Toute autre
voie est un défaut.

C'est ce qui rend l'invariant vérifiable : on relit un point de passage, pas
chaque appel de chaque agent. Un raccourci « juste pour le schéma, c'est du
`Metadata` de toute façon » détruit cette propriété — plus personne ne peut
répondre à « qu'est-ce qui est sorti ».

## `oxyn-ai` ne parle pas à un driver

Il reçoit du contexte déjà collecté et filtré. Un agent qui va chercher lui-même
ce dont il a besoin contourne à la fois le point de passage et le command bus.

## Une proposition est une `Command`

Portant `Actor::Agent`, traversant le `PolicyGate`
([ADR-0004](../../docs/adr/0004-command-bus.md)). Il n'y a pas d'API « outils »
séparée : c'est précisément ce que l'ADR-0004 refuse, parce qu'un second chemin
est toujours le moins audité.

Y compris pour ce qui « ne fait que lire » : `EXPLAIN ANALYZE` exécute réellement
la requête analysée, `DELETE` compris.

## Le contenu de la base n'est pas une consigne

Un nom de table, un commentaire de colonne, une valeur peuvent imiter une
instruction. Ce sont des **données**, à tout niveau. Le garde-fou n'est pas de
détecter l'injection — c'est que la sortie d'un modèle ne peut de toute façon
rien exécuter sans passer par la porte.

## Local et distant

Le classement se fait sur l'hôte réel **après résolution**, jamais sur la
présence de `localhost` dans l'URL : un point d'accès compatible OpenAI sur
`localhost` peut être un proxy vers le nuage. Il se re-vérifie à chaque
changement de configuration.

## Sans fournisseur

Le workspace IA est **absent de l'interface**, et Oxyn reste un client complet
([ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md)). Un chemin qui appelle un
modèle pour produire un résultat attendu comme déterministe — un tri, un
formatage, une complétion de nom de table — est un défaut de conception.

## Ce qui ne sort sous aucun niveau

Identifiants, chaînes de connexion, jetons, contenu du trousseau
([I-03](../../CLAUDE.md#i-03)). Il n'y a pas de dialogue pour ça : le code ne
doit pas offrir le chemin.
