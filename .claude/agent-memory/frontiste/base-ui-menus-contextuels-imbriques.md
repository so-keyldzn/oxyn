---
name: base-ui-menus-contextuels-imbriques
description: Base UI ContextMenu.Trigger arrête la propagation de contextmenu — des menus imbriqués (bloc de code dans une réponse, puce dans une question) s'ouvrent au plus profond, sans code en plus
metadata:
  type: reference
---

`ContextMenu.Trigger` de Base UI (1.8) appelle `stopEvent` (preventDefault + stopPropagation) dans son `onContextMenu` : un trigger imbriqué dans un autre gagne, le parent ne s'ouvre pas. Un `onContextMenu` passé sur l'élément de `render` est fusionné et s'exécute quand même (on y pose l'ancre).

**Why:** évite d'écrire une détection « qui est sous le pointeur » à la main pour des surfaces imbriquées.

**How to apply:** envelopper chaque surface dans son propre `<ContextMenu>` ; pas de `stopPropagation` manuel. Piège voisin : ajouter une story à un composant exempté dans `script/verifier-stories` fait échouer le contrôle (« exemption périmée ») — rendre le composant par la story du parent déjà nommée dans l'exemption, ou retirer l'exemption.
