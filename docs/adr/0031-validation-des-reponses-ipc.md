# ADR-0031 — Toute réponse du backend est validée à l'entrée du front

**Statut :** accepté · **Date :** 2026-09-16

**Précise :** [ADR-0029](0029-interface-tauri-shadcn.md), qui pose l'interface
Tauri et le pont IPC sans dire ce que le front tient pour acquis de ce qui en
revient.

## Contexte

`apps/desktop/src/lib/ipc/types.ts` porte en tête, depuis son premier commit,
l'avertissement qui décrit exactement le défaut :

> Mirror of `crates/oxyn-desktop/src/ipc.rs`. A field renamed on one side
> without the other fails at runtime only, so both change in the same commit.

Le miroir est tenu à la main, et rien ne le vérifie. `call<T>()` appelle
`invoke<T>()`, dont le paramètre de type est un **cast** : TypeScript écrit
`OpenConnection` sur ce que le backend a sérialisé, sans regarder. Un champ
renommé d'un seul côté produit un `undefined` qui se propage jusqu'à un rendu
vide ou un `TypeError` à trois écrans de sa cause. Le seul contrôle qui existe
aujourd'hui est `isIpcError`, un garde écrit à la main pour le chemin d'erreur,
dans `client.ts`.

Deux propriétés rendent ce silence plus coûteux qu'ailleurs :

1. **Ce qui traverse n'est pas que du Rust que nous contrôlons.** Un nom
   d'objet, une cellule, un message d'erreur de serveur et une réponse de modèle
   remontent par le même pont. `docs/SECURITY.md` les classe déjà en entrées
   hostiles côté rendu ; à l'entrée, ils ne sont pas regardés.
2. **La divergence ne casse pas le build.** Ni `tsc`, ni les stories — qui ne
   parlent jamais au backend — ne peuvent l'attraper. Elle n'apparaît qu'en
   exécution, dans la fenêtre de l'utilisateur.

`zod@4.6.5` était déjà déclaré dans `apps/desktop/package.json` et **importé
nulle part** : zéro occurrence dans les 289 fichiers de `apps/desktop/src`. Une
dépendance sans emploi, et sans relevé daté dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md) ([I-12](../../CLAUDE.md#i-12)).

### Ce que coûte la validation sur le chemin chaud

`read_result_page` est l'appel le plus fréquent du produit : la grille l'émet à
chaque défilement. Ce qu'elle demande est borné par `pageSizeFor` dans
`components/oxyn/result-grid.tsx` — `min(PAGE_SIZE 200, PAGE_CELLS 20 000 /
colonnes)`, soit **20 000 cellules au pire**, jamais les 2 000 lignes que la
commande accepte. Le budget de trame pendant une interaction continue est de
**8 ms p99** ([PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction)).

`Cell` est par ailleurs le **seul type `untagged`** qui traverse la frontière :
validé en union, chacune de ses quatre formes est essayée en séquence. Mesuré le
2026-09-16 sur la machine de dev, `zod@4.6.5`, une page pleine de 20 000
cellules :

| Ce qui valide `rows` | p50 | p99 |
|---|---|---|
| `z.union` des quatre formes | 2,26 ms | 7,72 ms |
| **discriminant positionnel** | **0,83 ms** | **1,16 ms** |
| enveloppe seule, cellules non lues | 0,28 ms | 0,59 ms |

L'union consomme presque toute la trame pour la même garantie qu'un test qui
regarde la valeur au lieu d'essayer des formes. Le discriminant coûte 0,57 ms
p99 de plus que ne rien vérifier du tout — et vérifie tout.

> **Réserve.** Mesure prise sous Node 22 (V8), pas dans la webview, qui est
> WKWebView — JavaScriptCore — sur macOS. L'ordre de grandeur suffit à trancher
> entre les trois lignes ; un chiffre pris sous la vraie webview les
> déplacerait ensemble.
>
> **Une première mesure, écartée**, portait sur des pages de 2 000 lignes — deux
> fois ce que la grille demande réellement — et concluait à 9,36 ms p99, donc à
> une exception nécessaire sur le corps des lignes. Elle est notée ici parce que
> la conclusion qu'elle appelait était fausse, et qu'elle a failli être retenue.

## Décision

**Toute réponse du backend est validée par un schéma `zod` à l'entrée du front.
Sans exception, y compris sur le chemin chaud de la grille.**

1. **`call` prend un schéma, pas un paramètre de type.** La signature devient
   `call<T>(command: string, schema: z.ZodType<T>, args?)`, et le corps parse la
   réponse avant de la rendre. Un `invoke` dont le résultat n'est pas parsé
   n'existe plus. Le point de passage unique d'[I-01](../../CLAUDE.md#i-01)
   devient aussi le point de validation unique.

2. **Les types sont dérivés des schémas.** `types.ts` et chaque
   `lib/ipc/<domaine>.ts` déclarent un schéma, et le type est
   `z.infer<typeof X>`. Une seule déclaration par type : un schéma et un type ne
   peuvent plus diverger. Le miroir avec `ipc.rs` reste manuel — c'est lui que
   la validation rend **bruyant** au lieu de silencieux.

3. **Un échec de validation est une `BackendError` non retryable**, qui nomme le
   champ fautif et la commande. Elle n'est pas rejouée :
   [I-13](../../CLAUDE.md#i-13) vaut ici comme ailleurs, et une réponse mal
   formée le restera au second appel.

4. **`Cell` est validé par un discriminant positionnel, pas par une union.**
   `isCell` regarde la valeur — `null`, chaîne, puis la présence de `text` ou
   d'`unrenderable` — au lieu d'essayer quatre schémas l'un après l'autre. C'est
   le seul endroit où la forme du validateur est dictée par une mesure, et le
   commentaire qui l'accompagne porte les chiffres, pour que personne ne le
   « simplifie » en `z.union` sans savoir ce qu'il paie.

5. **Un message de `Channel` est validé comme une réponse.** `guarded` enveloppe
   les quatre canaux — `ExecutionEvent`, `RefreshSignal`, `ShutdownSignal`,
   `AiUpdate`. Un message illisible est **abandonné avec une trace**, non levé :
   une exception dans `onmessage` serait avalée par les internes de Tauri.

6. **Ce qui gouverne un état terminal se dégrade au lieu de refuser.**
   L'abandon du point 5 se justifie par « personne n'attend ce message ». C'est
   vrai de `ExecutionEvent` — l'état terminal d'une requête vient du
   `CommandOutcome` que résout `execute` — et **faux** de `AiUpdate` : `ai_ask`
   ne résout qu'un `AskStarted`, et la seule sortie de l'état « en cours » est
   un événement `finished` ou `failed`. Un abandon y laisse le tour tourner
   indéfiniment, et le fil refuse alors toute nouvelle question.

   Donc `Ending` et `FailureCategory` retombent sur `unknown` plutôt que
   d'échouer. `unknown` est un repli **du front**, pas une variante que Rust
   émet, et il est classé non rejouable : ce qu'on n'a pas su lire ne se rejoue
   pas ([I-13](../../CLAUDE.md#i-13)).

   La règle générale qu'il faut en retenir : **avant de valider strictement un
   champ, regarder ce que son rejet empêche**. Là où l'échec est muet et bloque
   un état, la validation aggrave la panne au lieu de la nommer.

## Conséquences

* **+** La divergence entre `ipc.rs` et le front devient une erreur nommée, au
  premier appel, au lieu d'un `undefined` propagé. C'est le défaut que le
  commentaire en tête de `types.ts` annonçait sans pouvoir l'empêcher.
* **+** Le schéma est exécutable : il dit ce que le backend promet, et le
  vérifie. Un `type` ne fait que l'affirmer.
* **+** Une dépendance déjà installée cesse d'être du poids mort, et reçoit son
  relevé daté.
* **−** Le miroir reste manuel, et il a maintenant une forme de plus à tenir :
  changer un champ demande de toucher `ipc.rs` **et** le schéma. La validation
  le signale à l'exécution ; elle ne l'évite pas.
* **−** Un validateur de `Cell` dont la forme est dictée par une mesure, donc
  fragile à la relecture : il *ressemble* à une union écrite à la main par
  méconnaissance de `z.union`. Seul son commentaire le défend.
* **−** Trois schémas échappent à la règle générale et se replient au lieu de
  refuser — `Ending`, `FailureCategory`, et les deux enums clos de
  `metadata.ts`. Chacun a sa raison écrite à côté de lui, mais ce sont quatre
  endroits où une divergence de miroir reste **silencieuse**, c'est-à-dire
  exactement ce que cet ADR supprime ailleurs. Le compromis est assumé là où le
  rejet coûte plus cher que la dérive ; il ne s'étend pas par analogie.
* **−** 1,16 ms p99 prélevés sur le budget de trame à chaque page de grille, là
  où il n'y avait rien. C'est 15 % du budget, mesuré sous V8 et non sous la
  webview réelle : la marge est confortable, elle n'est pas infinie.
* **−** Un coût de parse non nul sur chaque autre réponse. Il n'est pas mesuré
  individuellement ; il est dominé par l'aller-retour IPC.

**Coût de sortie :** revenir en arrière, c'est remplacer `z.infer` par les
`interface` qu'il remplace et retirer l'argument `schema` de `call` — mécanique,
et borné par le fait que tout passe par une seule fonction. Ce qui ne se défait
pas à bon compte, ce sont les schémas eux-mêmes, s'ils ont entre-temps accumulé
des contraintes (`.int()`, `.nonnegative()`) qui décrivent le contrat mieux que
`ipc.rs`.

**Reconsidérer si** l'une de ces trois choses arrive : un générateur de types
depuis Rust (`ts-rs`, `specta`) entre au dépôt — il supprimerait le miroir
manuel, et cet ADR n'aurait plus qu'à couvrir la validation ; le parse d'une
réponse ordinaire apparaît dans un profil de trame ; ou `pageSizeFor` cesse de
borner une page à 20 000 cellules, auquel cas le point 4 repose sur une mesure
qui ne décrit plus le pire cas et doit être refaite.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Ne rien valider, garder `invoke<T>` | C'est l'état actuel : un cast que rien ne vérifie, dont le mode de défaillance est l'`undefined` silencieux. Le commentaire en tête de `types.ts` décrit le défaut depuis le début sans pouvoir l'empêcher. |
| Valider `Cell` par `z.union` des quatre formes | 7,72 ms p99 sur une page pleine, contre 8 ms de budget de trame : la garantie est la même que celle du discriminant, le coût est sept fois supérieur. `Cell` étant `untagged`, l'union essaie les formes en séquence là où un test les distingue d'un regard. |
| Valider l'enveloppe et laisser les cellules non lues | Le choix qu'appelait la première mesure, fausse. Une fois le pire cas réel mesuré, il économise 0,57 ms p99 en échange du seul trou de la frontière — sur le chemin par lequel arrivent les données d'un serveur tiers. |
| Garder les `interface` et écrire les schémas à côté, accordés par `z.ZodType<T>` | Diff plus petit, mais deux déclarations à tenir par type. C'est le miroir manuel que cet ADR existe pour réduire, dupliqué une fois de plus à l'intérieur du front. |
| Générer les types TS depuis Rust (`ts-rs`, `specta`) | Supprime la cause plutôt que le symptôme, et reste la bonne réponse à terme. Écarté **ici** : c'est une dépendance et une étape de build côté Rust, donc une décision propre, à prendre pour elle-même et non en sous-produit de l'introduction d'un validateur. Nommé comme condition de reconsidération. |
| Gardes écrites à la main, comme `isIpcError` | Tient pour un type à deux champs, pas pour la trentaine que porte la frontière. Chaque garde est du code non testé qui affirme ce qu'il ne vérifie pas toujours — et il en existe déjà un seul, pour le chemin d'erreur, ce qui montre le rythme auquel ils s'écrivent. |
