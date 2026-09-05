---
description: Ajouter ou modifier une vue GPUI
argument-hint: "<la vue, ex. grille de résultats>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch
---

Objet : ajouter ou modifier la vue **$ARGUMENTS**.

## Avant d'écrire

1. `docs/UX-SPEC.md` — les comportements font autorité.
2. `docs/PERFORMANCE.md` § budgets d'interaction — 8 ms p99, 100 ms de retour.
3. `.claude/rules/ui-gpui.md`.
4. `docs/adr/0003-driver-capabilities.md` : l'interface est **conditionnelle aux
   capacités**, dès maintenant.

## L'ordre qui évite la réécriture

**1. La commande d'abord.** Si aucune `Command` n'exprime ce que la vue doit
déclencher, c'est la commande qui manque : passer par
[`/commande`](commande.md). Une vue qui appelle un driver crée le second chemin
d'exécution que [I-01](../../CLAUDE.md#i-01) interdit.

**2. Les cinq états, dessinés avant d'être codés.** Initial, en cours, peuplé,
**vide**, erreur. Celui qu'on oublie est toujours le vide, et c'est le premier
que voit un nouvel utilisateur — sur une base neuve, chaque écran est vide.

**3. Les capacités ensuite.** Une surface qui suppose des tables, un schéma ou du
SQL n'existe pas pour un driver qui n'en a pas. Rétrofitter ces conditions dans
une interface écrite pour PostgreSQL est une réécriture, pas une retouche.

## Constructions à utiliser / jamais

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| Les mécanismes asynchrones de GPUI | `block_on`, `blocking_recv` | [I-05](../../CLAUDE.md#i-05) : la fenêtre gèle, l'utilisateur tue le processus |
| Lecture directe des `RecordBatch` | conversion en lignes « pour simplifier » | annule l'accès O(1) et réintroduit l'empreinte mémoire qu'[ADR-0002](../../docs/adr/0002-arrow-result-model.md) élimine |
| Affichage après confirmation du serveur | affichage optimiste sur une écriture | l'utilisateur repart convaincu que sa correction est enregistrée |
| Bouton d'annulation qui atteint le serveur | annulation qui abandonne l'affichage | le bouton ment, et une connexion reste prise |
| Message d'erreur du serveur, code compris | paraphrase rassurante | le public lit les messages de PostgreSQL |

## Les pièges

**Le décodage sur le thread UI.** Un lot Arrow décodé au moment du rendu tient
le thread : la saccade est visible avant même que le profileur ne le soit. Le
budget de 8 ms p99 ne pardonne rien.

**Le défilement qui relance la requête.** Au-delà du budget mémoire, on lit une
page disque. Relancer est doublement faux : le coût est arbitraire, et un
`SELECT` peut ne pas être idempotent.

**La valeur liée affichée dans une infobulle de débogage.**
[I-03](../../CLAUDE.md#i-03) ne s'arrête pas aux journaux.

**Le clavier oublié.** [ADR-0001](../../docs/adr/0001-ui-toolkit.md) identifie
l'accessibilité comme un risque structurel de GPUI, pas comme une finition. Une
vue non atteignable au clavier est un défaut bloquant, parce que la rattraper
après coup sur un toolkit qui ne l'offre pas gratuitement coûte une réécriture.

## Vérifier

```bash
make qualite
```

Puis `.claude/checklists/revue-ui.md`.

Les budgets de trame ne se mesurent **pas** avec `criterion` — un banc
`criterion` sur du GPUI ne mesure rien d'utile. Ils se mesurent avec les
instruments du système
(`docs/PERFORMANCE.md` § ce qui se mesure, et comment).
