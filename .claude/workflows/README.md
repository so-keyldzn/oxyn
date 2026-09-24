# Workflows

L'enchaînement des gestes pour les travaux qui en demandent plusieurs. Chaque
étape renvoie à la commande ou à l'agent qui la porte : **rien n'est redit ici.**

Un workflow n'est pas une procédure à suivre mécaniquement. C'est l'ordre qui
évite de découvrir trop tard qu'une étape en conditionnait une autre.

## Nouvelle fonctionnalité

1. [`/plan`](../commands/plan.md) — dans quelle phase, quelle crate, quels
   invariants, qu'est-ce qui n'est pas tranché
2. Si quelque chose n'est pas tranché → [`/adr`](../commands/adr.md), **avant**
   de coder
3. [`/commande`](../commands/commande.md) — la commande du bus d'abord
4. [`/implementer`](../commands/implementer.md) ou l'agent du domaine
5. [`/ecran`](../commands/ecran.md) si une interface est concernée
6. [`/relire`](../commands/relire.md)
7. [`.claude/checklists/fin-de-tache.md`](../checklists/fin-de-tache.md)

**L'ordre 3 avant 5 n'est pas négociable** : une vue écrite avant sa commande
appelle un driver « en attendant », et ce second chemin ne disparaît jamais.

### En version orchestrée

[`implementer-senior.js`](implementer-senior.js) enchaîne ces étapes avec
plusieurs agents : cadrage en parallèle (code, skills TanStack/Tauri/shadcn du
dépôt **et** ceux embarqués dans `node_modules`, invariants), plan Opus critiqué
puis révisé, implémentation par les agents du domaine, `make qualite` avec
réparation bornée, relecture par les relecteurs du dépôt, chaque constat soumis
à deux sceptiques de modèles différents. Il **s'arrête sans rien écrire** si le
plan contient une décision non tranchée, et ne commite jamais.

Il se lance en demandant à Claude d'exécuter le workflow `implementer-senior`
avec la tâche en argument.

## Nouveau driver

1. [`/driver`](../commands/driver.md) — la première question est *nouveau
   protocole, ou produit parlant un protocole déjà là ?*
2. Si le protocole existe déjà : **il n'y a pas de crate à créer**, la différence
   se déclare en capacités. Le workflow s'arrête ici.
3. Implémentation par l'agent `driveriste`
4. Les deux tests qui ne se contournent pas : annulation prouvée côté serveur,
   flux sur un volume qui ne tient pas en mémoire
5. [`.claude/checklists/revue-driver.md`](../checklists/revue-driver.md),
   intégralement
6. Agents `relecteur-frontiere` puis `relecteur-invariants`

## Correction d'un défaut

1. **Reproduire d'abord.** Un correctif sans reproduction corrige une hypothèse
2. Écrire le test qui échoue, avant le correctif
3. [`/implementer`](../commands/implementer.md)
4. Le test passe, et `make qualite` aussi
5. **Chercher les jumeaux** : le même défaut existe souvent dans le driver
   voisin, la vue voisine. C'est l'étape la plus rentable et la plus sautée
6. Si le défaut vient d'une divergence code/documentation, corriger **les deux**
   dans le même commit

## Optimisation

1. [`/benchmark`](../commands/benchmark.md) — **mesurer avant**
2. Le chiffre justifie-t-il la complexité ? Si non, le workflow s'arrête, et
   c'est un résultat, pas un échec
3. Optimiser
4. Mesurer après, avec le même protocole
5. Le chiffre avant/après dans le message de commit
6. Si un budget ne peut pas être tenu → [`/adr`](../commands/adr.md), jamais un
   ajustement silencieux du budget

## Conformité shadcn

1. [`/conformite-shadcn --releve`](../commands/conformite-shadcn.md) — relevé mécanique, puis relevé
   par lots en parallèle par l'agent `shadcniste`
2. Les écarts de risque `décision` (registre, jeton, composant de `ui/` à mettre
   à jour) sont tranchés par l'utilisateur **avant** toute correction
3. [`/conformite-shadcn`](../commands/conformite-shadcn.md) — correction par lots disjoints, trois à la
   fois au plus
4. `make qualite`, relevé à nouveau, puis agent `relecteur-invariants`

**Un écran neuf ne passe pas par là** : il s'écrit conforme avec
[`/ecran`](../commands/ecran.md). Cette passe rattrape l'existant.

## Évolution d'architecture

1. Agent `architecte`
2. [`/adr`](../commands/adr.md) — coût de sortie et condition de reconsidération
   inclus
3. Mettre à jour les documents d'autorité **rendus faux** par la décision
4. `docs/README.md` — l'index, dans le même commit
5. Agent `detecteur-divergence` — vérifier qu'aucun document ne dit encore
   l'ancienne chose

## Revue de la documentation

1. Agent `detecteur-divergence`
2. `make socle` — liens morts, règles sans `paths:`, invariants orphelins
3. [`/versions`](../commands/versions.md) si la dernière vérification date de
   plus de 90 jours ; le hook `SessionStart` le signale
4. Agent `documentaliste` pour les corrections

## Publication

Pas encore de workflow : il n'y a rien à publier, et une procédure écrite avant
d'avoir été exécutée une fois est une fiction. À écrire au moment de la première
publication réelle, à partir de ce qui aura été fait.
