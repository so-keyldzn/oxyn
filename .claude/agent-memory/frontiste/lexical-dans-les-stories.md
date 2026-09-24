---
name: lexical-dans-les-stories
description: Tester un champ Lexical dans une story — userEvent.type marche, toHaveValue non, et le menu typeahead réécrit son ARIA à chaque frappe
metadata:
  type: feedback
---

Un champ Lexical (`ContentEditable`) dans une story :

* `userEvent.type` et `userEvent.keyboard` de `storybook/test` **fonctionnent**
  (Lexical reçoit bien `beforeinput`) ; inutile de chercher un clavier réel ;
* `toHaveValue` rend `undefined` sur un `contenteditable` : comparer
  `field.textContent` ou `toHaveTextContent` ;
* une composition IME se simule par
  `fireEvent.keyDown(field, { key: "Enter", isComposing: true })` ;
* le menu de `LexicalTypeaheadMenuPlugin` est **portalisé dans `document.body`**,
  pas dans `canvasElement` : `within(document.body).findByRole("listbox", …)`.

Le piège qui coûte le plus : l'ancre du menu est détachée puis rattachée **à
chaque frappe**, et Lexical y remet `aria-label="Typeahead menu"` dans un effet
qui passe après ceux des enfants. Un `setAttribute` dans un effet ne tient donc
qu'une frappe ; il faut un `MutationObserver` sur l'attribut. Pour la classe,
la prop `anchorClassName` suffit. Il laisse aussi `aria-activedescendant` sur
`typeahead-item-0` quand la liste se vide (axe `aria-valid-attr-value`), et une
liste sans option échoue en `aria-required-children`.

La forme qui passe axe en `error` : l'ancre de Lexical est forcée en
`role="presentation"` (sans son `aria-label` ni son `id`), et c'est **le
conteneur qui défile** qui porte `role="listbox"` et `tabIndex={0}`. Un
`tabIndex={-1}` sur un div défilant *dans* la listbox fait échouer
`aria-required-children`, et un `tabIndex={-1}` sur les options ne suffit pas à
`scrollable-region-focusable`. Les lignes d'état (vide, recherche, erreur) sont
des `role="option" aria-disabled`. Le détail sourcé est dans
`docs/RESEARCH-NOTES.md`, section « Interface Tauri et front ».

**Why:** chaque aller-retour `vitest --project storybook` coûte ~15 s, et
l'échec « Unable to find role=listbox » ne dit pas que le nom a changé.

**How to apply:** à tout composant qui monte un `LexicalComposer`, et à toute
future liste typeahead (commandes `/`, par exemple). Voir
[[pieges-de-la-porte-front]].
