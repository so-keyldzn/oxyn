# ADR-0011 — Un workbench dense comme structure commune du workspace

**Statut :** proposé · **Date :** 2026-09-07

## Contexte

La maquette Oxyn présente deux organisations : 37 écrans métier des pages
Figma 04 à 06 emploient des en-têtes de page de 88 px, tandis que les trois
parcours de la page 22 placent les objets et consoles dans un workbench dense.
Les planches d'états de la page 07 reprennent la première organisation.
La demande produit retient une partie bases de données proche d'un IDE et
exige la cohérence entre écrans.

`crates/oxyn-app/src/workspace/` dispose déjà de la navigation entre SQL et
objets. Il conserve toutefois un titre et un sous-titre dans `content.rs` :
l'existence du code ne signifie pas que toute la cible Figma est implémentée.

## Décision

La structure B, le workbench dense de la page Figma 22, est la cible commune.
[UX-SPEC](../UX-SPEC.md#structure-commune-du-workspace) définit sa composition
et la portée des autres pages. Les pages 04 à 07 restent des spécifications de
contenu et de comportement ; leurs anciens en-têtes ne sont pas à reproduire.
Les invariants de sécurité et de traitement des données restent inchangés.

## Conséquences

- **+** Tables, consoles et inspection d'objets partagent le même contexte de
  navigation et conservent davantage de place pour les données.
- **+** Les scénarios existants restent exploitables sans réécrire leurs
  contrats de confidentialité, d'exécution et d'erreur.
- **−** Les anciennes planches ne constituent plus des captures complètes de
  la fenêtre à implémenter ; leur portée doit rester explicitement indiquée.
- **−** L'alignement du code et les variantes de largeur demandent une
  adaptation du workspace, distincte de cet arbitrage de maquette.

**Coût de sortie :** remplacer la structure dans les vues de `oxyn-app` et
`oxyn-ui`, puis reprendre les parcours Figma et leur navigation. Le domaine et
les drivers sont hors de cette structure grâce à leur séparation existante.
Le coût dépend du nombre de vues intégrées ; il dépasse un changement local
de mise en page.

**Reconsidérer si** les tests d'usage montrent que les utilisateurs perdent le
contexte de connexion ou d'objet lors des passages entre consoles, données et
assistants, ou si la variante compacte empêche un parcours essentiel.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Une application à pages avec grand en-tête systématique | Consomme de l'espace utile et ne correspond pas à la direction IDE retenue |
| Deux structures selon la famille de fonctionnalités | Rend ambigus le contexte actif et les règles de navigation entre les écrans |
| Refaire immédiatement toutes les planches métier | Coût inutile pour trancher la structure ; leur contenu reste valide une fois leur portée explicite |
