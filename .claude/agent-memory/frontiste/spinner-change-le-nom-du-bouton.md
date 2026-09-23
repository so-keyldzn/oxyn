---
name: spinner-change-le-nom-du-bouton
description: Un Spinner dans un Button ajoute « Loading » au nom accessible : getByRole({ name: "Declare" }) échoue dans l'état en cours
metadata:
  type: feedback
---

Le `Spinner` de `components/ui` porte `role="status"` et `aria-label="Loading"`.
Posé dans un `Button` (`<Spinner data-icon="inline-start" />`), il entre dans le
nom accessible : le bouton s'appelle « Loading Declare ». Une story d'état « en
cours » qui cherche `getByRole("button", { name: "Declare" })` rend
`TestingLibraryElementError`, sans dire pourquoi.

**Why:** perdu un aller-retour de stories sur `provider-form.stories.tsx`
(2026-09-23).

**How to apply:** dans une story où le bouton peut porter un spinner, chercher
par `name: /Declare$/`. Voir aussi [[pieges-de-la-porte-front]].
