---
name: gpui-debug-bounds-et-clics
description: Piège GPUI 0.2.2 — debug_bounds rend des bornes hors fenêtre pour un élément débordant, et simulate_click y échoue en silence ; il faut aussi cx.notify() pour obtenir une trame
metadata:
  type: feedback
---

`cx.debug_bounds("id")` de GPUI 0.2.2 rend les bornes du **dernier rendu**, sans
vérifier qu'elles tombent dans la fenêtre. Deux conséquences qui coûtent du temps :

**1. Un `Some(bounds)` ne prouve pas qu'on peut cliquer.** Une rangée `flex` qui
déborde horizontalement place ses derniers enfants au-delà du bord droit :
`debug_bounds` les rend quand même, et `cx.simulate_click(bounds.center(), …)`
n'atteint alors aucun gestionnaire — sans erreur, sans avertissement. Le
symptôme est une assertion d'effet qui échoue alors que l'assertion de présence
passe.

**Why:** rencontré en raccordant le sélecteur de contexte de la barre de console :
le bouton d'annulation ajouté après un bloc de 260 px poussait hors de l'écran à
1600 px de large. Le test trouvait le contrôle et le clic ne faisait rien.

**How to apply:** quand un clic simulé reste sans effet alors que `debug_bounds`
répond, soupçonner le débordement avant la logique. Le bon réflexe de conception
est de ne pas empiler d'action supplémentaire dans une rangée déjà pleine : les
affordances d'état (annuler, recharger) vont sur la ligne de message en dessous,
où elles côtoient la phrase qui les explique.

**2. Modifier l'état d'une entité dans un test ne repeint pas.** Après
`entity.update(cx, |view, cx| { … })` sans `cx.notify()`, `run_until_parked()`
ne produit aucune trame et `debug_bounds` renvoie l'état précédent. Il faut
`cx.notify()` explicitement dans la fermeture, puis `run_until_parked()`.
