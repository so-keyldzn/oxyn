# ADR-0008 — Chaîne d'outils Rust épinglée dans le dépôt

**Statut :** proposé · **Date :** 2026-09-05

## Contexte

Fait vérifié le 2026-09-05 ([RESEARCH-NOTES](../RESEARCH-NOTES.md#chaîne-doutils-rust)) :

- Rust stable est en `1.98.1` ;
- la machine de développement du mainteneur est en `1.89.0`, soit **neuf
  versions mineures de retard** ;
- `gpui 0.2.2` est en **édition 2024**, qui exige au minimum Rust `1.85` ;
- `gpui` ne déclare **aucun MSRV** (`rust_version` absent de ses métadonnées),
  donc rien ne garantit qu'une future version restera compatible avec une
  toolchain ancienne.

`1.89.0` compile l'édition 2024. Mais sans épinglage, la machine de
développement et l'intégration continue utilisent deux compilateurs différents.
Le mode de panne est silencieux dans le sens qui coûte le plus cher : le code
compile en local, et `clippy` en CI signale des diagnostics ajoutés entre 1.89
et 1.98 que le développeur n'a jamais vus. À l'inverse, du code utilisant une
API stabilisée après 1.89 passe en CI et ne compile pas chez lui.

## Décision

Un fichier `rust-toolchain.toml` à la racine épingle une version **exacte** de
la chaîne d'outils, avec les composants `rustfmt` et `clippy`.

La version épinglée est la même partout : poste de développement, intégration
continue, publication. Une montée de version est un changement délibéré, avec
son propre commit, et met à jour
[RESEARCH-NOTES](../RESEARCH-NOTES.md#chaîne-doutils-rust) dans le même commit.

La valeur initiale est à fixer au premier commit de code. Le choix se fait entre
deux options, et il se documente ici :

- `1.98.1`, la stable du jour : impose au mainteneur de mettre à jour sa
  machine ;
- `1.89.0`, la toolchain existante : fige le projet sur un compilateur d'un an,
  sans bénéfice.

La recommandation est `1.98.1` — un projet neuf n'a aucune raison de naître avec
un an de dette d'outillage.

## Conséquences

* **+** Un seul jeu de diagnostics `clippy` : la porte de qualité veut dire la même
  chose sur le poste et en CI.
* **+** L'édition et le MSRV cessent d'être implicites.
* **+** `rustup` installe la bonne version au premier `cargo` lancé dans le dépôt :
  aucune procédure à documenter.
* **−** Un contributeur hors ligne, ou derrière un miroir restreint, doit disposer de
  la version épinglée.
* **−** L'épinglage se maintient : un fichier oublié deux ans est une dette qui
  grossit seule.

**Coût de sortie :** nul, supprimer le fichier suffit. **Reconsidérer si** le projet
accueille des contributeurs dont la distribution impose une toolchain système :
l'épinglage exact devrait alors devenir un minimum.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Ne rien épingler | c'est l'état actuel, et il produit exactement la divergence silencieuse décrite ci-dessus |
| Épingler `stable` sans version | ne fige rien : « stable » désigne un compilateur différent chaque semaine |
| Déclarer seulement un `rust-version` dans `Cargo.toml` | exprime un minimum, n'impose pas la version utilisée ; les deux sont complémentaires, pas substituables |
