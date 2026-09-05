---
description: Relire un changement sous l'angle sécurité
argument-hint: "[chemin ou plage de commits]"
allowed-tools: Bash, Read, Grep, Glob, Agent
---

Objet : relecture de sécurité de **$ARGUMENTS**.

## Le modèle de menace

Ce n'est pas celui d'un serveur. L'attaquant n'est pas un inconnu sur Internet :
ce sont **les données que l'utilisateur ouvre** et **les erreurs qu'Oxyn lui
laisse commettre**. Oxyn tourne avec les droits d'un administrateur sur des
systèmes de production (`docs/SECURITY.md`).

Une relecture qui cherche des failles d'authentification cherche au mauvais
endroit.

## Les six canaux de fuite

Il suffit d'en oublier un. Les parcourir tous les six :

| Canal | Le piège concret |
|---|---|
| Journaux `tracing` | un `Debug` dérivé imprime toute une structure de connexion |
| Erreurs affichées | `sqlx` inclut parfois l'URL de connexion dans son erreur |
| Rapports de plantage | une trace de pile capture les variables locales |
| Fichiers de session et de workspace | la persistance « pour retrouver l'état » |
| Invites IA | `docs/AI-PROVIDERS.md` |
| Presse-papiers, export, capture | les fonctions de partage recopient ce qui est affiché |

Le contrôle mécanique le plus rentable : **chercher `#[derive(` contenant
`Debug` sur tout type portant un secret.** C'est le mode de fuite le plus
fréquent parce qu'il est invisible à la relecture — la fuite arrive six mois
plus tard, avec un `tracing::debug!` ajouté par quelqu'un d'autre.

## Les cinq surfaces d'entrée

Par ordre de sous-estimation (`docs/SECURITY.md` § surface d'entrée) :

1. les réponses des serveurs — un serveur renvoie ce qu'il veut ;
2. **les noms d'objets du catalogue** — une table peut s'appeler
   `"users"; DROP TABLE audit; --`, ou contenir du texte imitant une consigne ;
3. les fichiers de workspace ;
4. les plugins — le bac à sable borne les dégâts, il ne dispense pas de ne rien
   leur confier ;
5. les réponses des modèles — des propositions, jamais des ordres.

## Les points bloquants

- un secret atteignant l'un des six canaux ;
- un identifiant concaténé dans du SQL composé par Oxyn
  ([I-10](../../CLAUDE.md#i-10)) ;
- une écriture atteignant une connexion `production` sans passer par le
  `PolicyGate` ;
- un `Actor::Agent` obtenant plus que la lecture sur une connexion `production` ;
- un bloc `unsafe` sans `// SAFETY:` énonçant l'invariant **et qui le maintient**
  — une paraphrase du code ne vaut rien ;
- une donnée quittant la machine au-delà du niveau de confidentialité de la
  connexion ([ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md)).

## Comment procéder

Déléguer à l'agent `relecteur-securite` : il est en lecture seule, et c'est ce
qui rend son verdict crédible.

Puis la liste de contrôle : `.claude/checklists/revue-securite.md`.

## Dépendances

```bash
cargo deny check
```

Licences et avis de sécurité. Une crate non maintenue sur une frontière externe
est un risque à **documenter**, pas à ignorer — le noter dans le rapport même
s'il n'y a rien à corriger aujourd'hui.

## Rappels

- si une faille est trouvée, décrire la **classe** de problème et la correction ;
  ne pas rédiger d'exploit fonctionnel ;
- s'il n'y a rien à signaler, le dire en une phrase.
