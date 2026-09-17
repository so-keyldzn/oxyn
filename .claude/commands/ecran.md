---
description: Ajouter ou modifier un écran de l'interface Tauri
argument-hint: "<l'écran ou le composant, ex. panneau d'historique>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch, Skill
---

Objet : ajouter ou modifier **$ARGUMENTS** dans l'interface Tauri.

## Avant d'écrire

1. **Lire [.claude/rules/front.md](../rules/front.md) avec `Read`**, même si le
   fichier à créer n'existe pas encore : une règle `paths:` ne se charge qu'à la
   lecture d'un fichier correspondant, et le premier fichier d'un domaine s'écrit
   sinon sans elle.
2. [UX-SPEC](../../docs/UX-SPEC.md) — les comportements et les états font autorité.
3. [ARCHITECTURE § 2 bis](../../docs/ARCHITECTURE.md#2-bis-linterface-tauri) — le
   pont IPC et ce qui ne le traverse pas.
4. [FIGMA-HANDOFF](../../docs/FIGMA-HANDOFF.md) — la planche, si elle existe.

## Outils plutôt que mémoire

| Pour | Utiliser |
|---|---|
| ajouter ou composer un composant shadcn sur Base UI | le skill `shadcn`, puis `pnpm exec shadcn add` |
| routes, chargement, état de requête | les skills `tanstack-router-best-practices`, `tanstack-query` |
| commande, canal, capacités de la webview | le skill `tauri-v2`, puis la page officielle — [I-12](../../CLAUDE.md#i-12) |
| lire une planche | le MCP Figma (`get_design_context`, `get_screenshot`) |

## L'ordre qui évite la réécriture

**1. La `Command` d'abord.** Si aucune n'exprime l'action, c'est elle qui manque :
[`/commande`](commande.md). Un écran qui trouve un raccourci vers le store ou un
driver crée le second chemin qu'[I-01](../../CLAUDE.md#i-01) interdit.

**2. La commande Tauri, `async`, qui émet cette `Command`.** Puis le miroir
TypeScript de ce qui traverse, **dans le même commit**. Une commande Tauri
élargit ce qu'un script dans la webview peut faire : [`/securite`](securite.md).

**3. Le composant, données et rappels par props, et une story par état** —
initial, en cours, peuplé, **vide**, erreur. Le vide est celui qu'on oublie, et
c'est le premier que voit un nouvel utilisateur.

**4. La feature qui relie** le composant à `src/lib/ipc`. C'est la seule couche
qui parle au backend ; c'est pourquoi les stories n'en ont jamais besoin.

L'ordre inverse — l'écran d'abord, le backend « branché ensuite » — produit un
composant dont les props épousent un mock, et un miroir IPC écrit pour lui.

## Les pièges propres à ce geste

Les constructions interdites vivent dans [front.md](../rules/front.md) ; ce qui
suit est ce qui se rate **dans l'enchaînement**.

**Le miroir mis à jour d'un seul côté.** Rust renomme `rows` en `cells`, le
TypeScript lit toujours `rows` : la grille affiche « vide » sur un résultat
peuplé. Ni `tsc` ni `cargo` ne le voient.

**L'état d'erreur qui paraphrase.** Le public lit les messages de PostgreSQL : le
message du serveur, code compris, s'affiche tel qu'`IpcError` le porte.

**L'identifiant de connexion dans une story ou un titre.** Une story est publiée
avec Storybook ; [I-03](../../CLAUDE.md#i-03) ne s'arrête pas aux journaux.

**L'écran écrit pour PostgreSQL.** Il suppose un schéma et du SQL, et n'existe pas
pour un driver qui n'en a pas : l'interface se conditionne aux capacités dès
maintenant ([ADR-0003](../../docs/adr/0003-driver-capabilities.md)).

## Vérifier

```bash
make front          # format, lint, types, stories (axe compris), build
make desktop-dev    # la vraie fenêtre, sur un workspace temporaire
make qualite
```

Puis [revue-ui](../checklists/revue-ui.md), et `relecteur-securite` si une
commande Tauri a été ajoutée.
