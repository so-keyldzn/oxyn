---
name: piege-base-ui-initialfocus-a-l-ouverture
description: Base UI AlertDialog — `initialFocus` n'agit qu'à l'ouverture ; un corps remonté sous un dialogue déjà ouvert perd le focus
metadata:
  type: feedback
---

`initialFocus` d'un `AlertDialogContent` (Base UI) ne s'applique **qu'à l'ouverture**.
Si le dialogue reste ouvert et que son contenu est remonté (changement de `key`,
une demande qui en suit une autre), le focus reste sur un élément retiré du DOM, et
« Cancel » ne le reprend pas. Rien ne l'annonce : les stories d'ouverture restent vertes.

**Why:** constaté le 2026-09-24 sur l'écran d'échantillon : une demande d'agent en
attente derrière l'épingle de l'utilisateur apparaissait sans que Cancel ait le focus.
Une story `play` qui enchaîne deux demandes le montre ; une story à une seule demande, non.

**How to apply:** tout dialogue dont le contenu peut changer sans fermeture redonne
le focus lui-même au montage du corps (`useEffect(() => ref.current?.focus(), [ref])`),
et sa story enchaîne deux contenus avant de vérifier `toHaveFocus`.
