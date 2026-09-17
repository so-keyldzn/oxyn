---
name: pieges-de-la-porte-front
description: Trois pièges d'outillage qui font échouer prettier/eslint/vitest sur des stories neuves, et l'ordre de commandes qui les attrape
metadata:
  type: feedback
---

Ordre qui attrape tout sans lancer `make front` (lourd, et il inclut les
fichiers des autres agents) :

```bash
pnpm exec prettier --check 'src/components/oxyn/<motif>*'   # 1
pnpm -s typecheck                                            # 2
pnpm exec eslint <fichier> <fichier> …                       # 3
pnpm exec vitest run --project storybook <stories>           # 4
```

Les trois pièges :

* **`prettier --check` puis `eslint`, jamais l'inverse.** Le reformatage de
  prettier redécoupe les ternaires et les objets, ce qui déplace les lignes que
  eslint venait de signaler.
* **`--check` ne prend pas plusieurs chemins dans une même variable shell** :
  `prettier --check $F` avec `F="a b c"` rend « No files matching the pattern ».
  Passer des motifs entre quotes, un par argument.
* **`@typescript-eslint/no-unnecessary-condition` casse les idiomes de story.**
  `element.textContent ?? ""` est une erreur : dans la config du dépôt,
  `textContent` d'un `HTMLElement` rendu par `getAllByRole` est typé non-nul.
  Écrire `.map((item) => item.textContent)` tout court. Même règle sur
  `TABLE[cle] ?? défaut` quand `TABLE` est un `Record<Union, V>` :
  `noUncheckedIndexedAccess` ne s'applique **pas** à un type mappé à clés
  littérales. Pour garder une lecture défensive d'un mot venu d'un protocole,
  passer par `Object.hasOwn(table, mot) ? table[mot] : repli` — ça garde
  l'exhaustivité à la compilation *et* le repli à l'exécution.
* **Deux règles de nommage et de configuration qui surprennent** :
  `@typescript-eslint/naming-convention` impose un paramètre de type nommé `T`
  ou `T<Quelquechose>` (`<K extends string>` est refusé) ; et
  `react/no-array-index-key` **n'est pas configurée** — un
  `// eslint-disable-next-line` qui la nomme est lui-même une erreur
  (« Definition for rule … was not found »).

Et quatre pièges de test, pas d'outillage mais ils coûtent le même
aller-retour :

* `getByText` sur une phrase qu'une liste répète pour deux entrées rend
  « Found multiple elements ». Passer par `getAllByRole("listitem").map(item =>
  item.textContent)` et comparer le tableau entier ;
* une phrase coupée par un `<strong>` ou un `<span>` au milieu n'est **pas**
  trouvée par `getByText` : comparer le `textContent` d'un ancêtre, ou
  `expect(element).toHaveTextContent(…)` ;
* **un dialogue se cherche dans `document.body`, jamais dans `canvasElement`** :
  il est portalisé hors de la racine de la story, et il **anime son entrée**.
  Le motif du dépôt (`approval-dialog.stories.tsx`) est
  `within(document.body)` + `findAllBy*` + `waitFor(() =>
  expect(x).toBeVisible())`. Un `findBy*` seul résout dès que le nœud existe,
  c'est-à-dire pendant que le dialogue est encore transparent, et l'assertion
  échoue en « Received element is not visible » — mais **passe en isolation**,
  ce qui envoie chercher une interférence entre stories qui n'existe pas ;
* **un contrôle Base UI désactivé n'a pas l'attribut `disabled`.** Un
  `Checkbox` rend `aria-disabled="true"` + `data-disabled` et reste focalisable
  ; `toBeDisabled()` y échoue. Le `Button`, lui, utilise bien `disabled`
  natif — la règle n'est donc pas uniforme, il faut regarder le composant ;
* **axe tourne après le `play`, et voit un popup Base UI encore en fermeture.**
  Un `Select` ou un menu qui s'anime laisse ses gardes de focus
  (`[data-base-ui-focus-guard]`, `aria-hidden` et focalisables) : violation
  `aria-hidden-focus`, et contamination de la story suivante. Finir tout `play`
  qui ouvre un popup par `await waitFor(() =>
  expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull())`
  — motif de `preview-controls.stories.tsx`.

Et deux pièges de composant que les stories révèlent :

* **Envelopper un contrôle dans un `TooltipTrigger` seulement quand il est
  désactivé le remonte** au changement d'état : le focus clavier retombe sur la
  page. Garder l'enveloppe en permanence et basculer `<Tooltip disabled>`.
* **`InputGroup` porte `has-disabled:opacity-50`** : un seul enfant `disabled`
  ternit tout le groupe, champ compris, sous le contraste lisible. Un contrôle
  qui se désactive (pendant une réponse, par exemple) va **hors** du groupe.

**Why:** chaque piège coûte un cycle complet de `vitest --project storybook`
(~10 s de setup Chromium) pour une faute triviale.

**How to apply:** à chaque composant neuf de `src/components/oxyn`. Voir aussi
[[axe-region-defilante-sans-focus]], qui n'apparaît qu'à l'étape 4.
