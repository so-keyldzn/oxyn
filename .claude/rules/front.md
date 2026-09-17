---
paths:
  - "apps/desktop/**"
  - "crates/oxyn-desktop/**"
---

# Interface Tauri — conventions

La décision et ses raisons vivent dans
[ADR-0029](../../docs/adr/0029-interface-tauri-shadcn.md) ; le pont IPC dans
[ARCHITECTURE](../../docs/ARCHITECTURE.md#2-bis-linterface-tauri) ; les
comportements dans [UX-SPEC](../../docs/UX-SPEC.md). Cette règle porte ce qui se
rate en écrivant du code ici.

## Un seul chemin vers le backend

`invoke` n'est appelé que par `call`, dans `src/lib/ipc/client.ts` ; les modules
d'un domaine (`src/lib/ipc/<domaine>.ts`) passent par `call`. Une vue qui l'appelle
elle-même crée le second chemin qu'[I-01](../../CLAUDE.md#i-01) interdit, et c'est
celui qu'une XSS emprunterait. Une fonctionnalité commence par une commande Tauri
dans `crates/oxyn-desktop/src/commands.rs` **qui émet une `Command`** — jamais par
un appel direct au store, au trousseau ou à un driver.

Un domaine vit dans ses modules : `src/backend/<domaine>.rs` (le `impl Backend`),
`src/commands/<domaine>.rs` (les `#[tauri::command]`), `src/ipc/<domaine>.rs` (ce qui
traverse), déclarés par une ligne `mod` et un bloc dans `generate_handler!`. Côté
front, `src/lib/ipc/<domaine>.ts` et `src/features/<domaine>/`.

Ajouter une commande Tauri, c'est élargir ce qu'un script dans la webview peut
faire : [`/securite`](../commands/securite.md) avant de fusionner.

**Une commande Tauri sans `async` tourne sur le thread principal**
([RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#interface-tauri-et-front)). Un corps
qui lit le store, le trousseau ou un lot débordé sur disque y fige la fenêtre
([I-05](../../CLAUDE.md#i-05)) — et rien ne le signale, la lecture étant rapide sur
la machine de dev. `async fn`, ou `#[tauri::command(async)]` ; synchrone seulement
pour un état déjà en mémoire. `code_interdit.py` pose la question à chaque
commande synchrone écrite.

`src/lib/ipc/types.ts` est le miroir de `src/ipc.rs`, et chaque `src/lib/ipc/<domaine>.ts`
celui de `src/ipc/<domaine>.rs`. Un champ renommé d'un seul
côté ne se voit qu'à l'exécution : les deux changent dans le même commit.

## Le miroir est un schéma, pas un type

`call` prend un **schéma** et parse la réponse ; `invoke<T>` ne fait que caster
([ADR-0031](../../docs/adr/0031-validation-des-reponses-ipc.md)). Ce qui en
découle en écrivant :

- un type de la frontière se déclare **une fois**, en schéma, et son type suit :
  `export const X = z.object({…})` puis `export type X = z.infer<typeof X>`.
  Écrire une `interface` à côté d'un schéma recrée le miroir qu'on vient de
  supprimer ;
- un `Option<T>` de Rust est `.nullable()`, **jamais** `.optional()` : aucun
  `skip_serializing_if` n'existe côté `oxyn-desktop`, donc un `None` sort en
  `"champ": null` et la clé est toujours là ;
- le tag d'une union n'est pas toujours `type` — `RunTarget` et `DestinationChoice`
  taguent sur `kind`, `FacetFreshness` sur `state`. Une `z.discriminatedUnion`
  sur le mauvais tag échoue sur **toutes** les réponses, pas sur un cas rare ;
- une variante newtype est **aplatie** par serde : le motif est
  `Inner.extend({ type: z.literal("…") })`, pas un champ imbriqué ;
- un champ que Rust porte en `&'static str` **sans énumération derrière** ne se
  valide pas en `z.enum` : il ferait échouer une réponse valide le jour où une
  valeur s'ajoute. Deux cas, selon ce que le Rust garantit :

  | Le Rust | Le schéma | Pourquoi |
  |---|---|---|
  | produit un ensemble **ouvert** (un identifiant de préréglage, une classe d'erreur) | `z.string()`, le *type* restreint | une valeur inconnue est légitime et doit passer |
  | produit un ensemble **clos par un `match` à bras attrape-tout** (`_ => "unknown"`) | `z.enum([…]).catch("unknown")` | le type reste utilisable par un `switch` ou un `Record`, et l'inconnu dégrade **un badge** au lieu de faire échouer toute la réponse |

  **Un `.catch` suppose une valeur qui dit l'ignorance.** `unknown`, `other` :
  des mots dont le sens est « je n'ai pas su lire ». Là où le type n'en a pas —
  `ProviderKind` n'a que des noms de protocoles —, tout repli est une
  **affirmation fausse**, et le champ se valide strictement. La question n'est
  pas « est-ce gênant d'échouer ? » mais « existe-t-il une valeur honnête ? ».
  Le cas décisif est `AgentProvenance.kind`, qui *signe* ce qu'une conversation
  propose ([ADR-0023](../../docs/adr/0023-fournisseurs-declares-et-provenance.md)) :
  une provenance qui échoue coûte une proposition, une provenance qui ment coûte
  le mécanisme entier. Si la résilience y devient nécessaire, elle s'obtient en
  ajoutant une variante d'ignorance **côté Rust**, comme `Ending::Unknown` —
  jamais en l'inventant dans le front ;
- un `u64` **que le serveur rapporte** (`estimatedRows`, `sizeBytes`) se valide
  sans `.int()` : zod y teste `Number.isSafeInteger`, et au-delà de 2^53 il
  rejetterait une réponse honnête. Les compteurs qu'Oxyn borne lui-même gardent
  `.int()` ;
- un message de `Channel` passe par `guarded` : même frontière, mais un message
  illisible est abandonné avec une trace, parce que personne ne l'attend.

`Cell` est validé par un test positionnel écrit à la main, et non par une
`z.union`. C'est une décision **mesurée** — le commentaire porte les chiffres.
Ne pas la « simplifier ».

## Composants

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| un composant de `src/components/ui` (shadcn, Base UI) | un `div` stylé qui imite un bouton, un menu, un dialogue | clavier, focus et ARIA viennent de Base UI ; les réécrire, c'est les rater |
| `pnpm exec shadcn add <composant>` | écrire un composant shadcn à la main | le skill `shadcn` du dépôt décrit le reste — `render` et non `asChild` sur Base UI |
| jetons sémantiques (`bg-background`, `text-muted-foreground`, `text-env-production`) | couleurs Tailwind brutes, `dark:` manuel | le thème et les contrastes AA sont réglés dans `src/styles.css`, une fois |
| `@hugeicons/react` | `lucide-react` | [UX-SPEC](../../docs/UX-SPEC.md#navigation-du-premier-workspace) impose Hugeicons |
| texte React (`{value}`) | `dangerouslySetInnerHTML` sur une donnée reçue | une cellule, un nom d'objet ou une réponse de modèle sont des entrées hostiles ([SECURITY](../../docs/SECURITY.md#surface-dentrée)) |

`src/components/ui` est **généré** : il n'est ni formaté ni linté par le projet, et
une retouche y est écrasée au prochain `shadcn add --overwrite`. Ce qui est propre à
Oxyn va dans `src/components/oxyn`.

## Chaque composant Oxyn a ses stories, et ses stories sont ses tests

Un composant de `src/components/oxyn` qui dépend d'une opération distante a une
story **par état** : initial, en cours, peuplé, vide, erreur
([UX-SPEC](../../docs/UX-SPEC.md#états-dune-vue)). `make front` les rend dans
Chromium et y passe axe en mode `error` : une violation d'accessibilité fait échouer
la porte.

Les comportements qui protègent l'utilisateur s'écrivent en `play`, pas en
commentaire : `Cancel` focalisé dans une approbation, Entrée qui n'approuve pas, un
identifiant de connexion jamais rendu.

Une story ne parle jamais au backend. Le composant reçoit ses données et ses
rappels par props ; `src/features` fait le lien avec `src/lib/ipc`.

## Le résultat n'est jamais entier dans la webview

La grille demande des pages bornées (`result_page`, 2 000 lignes au plus côté Rust)
pour la seule fenêtre visible. Charger « tout le résultat pour trier en JS »
réintroduit l'OOM qu'[I-06](../../CLAUDE.md#i-06) interdit, avec une webview qui
meurt au lieu d'un processus.

Le formatage d'une cellule est fait en Rust par `oxyn_data::format_cell`. Le front
ne reformate pas une date ni un binaire : l'écran divergerait du fichier exporté.

## Pas de retry

`QueryClient` est configuré `retry: false`. Un appel au backend qui échoue est une
réponse, pas un réseau capricieux ; et une écriture ambiguë ne se rejoue pas
([I-13](../../CLAUDE.md#i-13)). `retryable` vient du backend, jamais d'une analyse du
message.

## CSP

La CSP de production est dans `crates/oxyn-desktop/tauri.conf.json` et reste
stricte. `tauri.dev.json5` la retire **en développement seulement**, parce que Tauri
y injecte des nonces qui bloquent les scripts inline de Vite : la fenêtre reste
blanche, sans erreur. Une page blanche en production n'est donc jamais « la CSP à
relâcher » : c'est un script inline nouveau qu'il faut comprendre.

## Versions

Exactes dans `package.json`, relevées au registre et datées dans
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#interface-tauri-et-front)
([I-12](../../CLAUDE.md#i-12)). `shadcn add` écrit des `^` : les retirer dans le même
commit. Vitest reste en 4 tant que `@storybook/addon-vitest` n'accepte pas la 5.

## Vérifier

```bash
make front          # format, lint, types, tests unitaires et stories, build
make desktop-dev    # la vraie fenêtre, sur un workspace temporaire
```
