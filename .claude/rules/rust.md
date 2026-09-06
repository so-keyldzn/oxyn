---
paths:
  - "**/*.rs"
---

# Rust — conventions

Les interdits de fond sont dans [CLAUDE.md](../../CLAUDE.md#invariants). Cette
règle porte ce qui ne vaut que pour du code Rust.

## Erreurs

| Où | Quoi | Pourquoi |
|---|---|---|
| Crate de bibliothèque | `thiserror`, une énumération par frontière | l'appelant doit pouvoir distinguer les cas sans lire une chaîne |
| `oxyn-app`, tests, bancs | `anyhow` | personne ne rattrape par variante en haut de la pile |
| Jamais | `Box<dyn Error>` dans une API publique | efface l'information au moment précis où elle sert |

Une erreur de driver porte sa **classe** — transitoire, permanente, ambiguë
([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)).
La classe est une donnée, pas une déduction faite par l'appelant à partir du
message : un message change, un appelant qui l'analysait casse en silence.

## Constructions à utiliser / jamais

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| `?`, `let … else`, `match` | `unwrap()`, `expect()` hors tests | [I-09](../../CLAUDE.md#i-09) |
| `u32::try_from(n)?` | `n as u32` | `as` tronque en silence ; un `id` de 5 milliards devient 705 032 704 |
| `slice.get(i)` | `slice[i]` sur un index venu d'une donnée | panique sur entrée serveur |
| `#[non_exhaustive]` sur les énumérations publiques | énumération publique fermée | ajouter une variante devient une rupture majeure |
| `impl Trait` en argument | générique inutile | moins de monomorphisation, signatures lisibles |
| `Cow<'_, str>` quand la copie est rare | `String` systématique | mais ne pas contaminer cinq signatures pour un appel par ouverture de fenêtre |

`expect()` est toléré dans un test, un banc, ou une initialisation dont l'échec
est un bug de programmation — jamais sur un chemin atteignable depuis le réseau.
Quand il est utilisé, son message dit **l'invariant supposé**, pas « échec ».

## Async

- une fonction `async` publique ne bloque jamais : tout appel bloquant part sur
  le pool bloquant ([ARCHITECTURE](../../docs/ARCHITECTURE.md#le-modèle-de-threads)) ;
- toute opération qui peut durer accepte l'annulation **et la propage jusqu'au
  serveur** — un futur abandonné ne libère pas une requête distante
  ([I-13](../../CLAUDE.md#i-13) et le contrat de driver) ;
- pas de `tokio::spawn` détaché dont personne ne tient la poignée : une tâche
  qu'on ne peut pas annuler ni attendre survit à la fermeture de l'onglet ;
- attention à l'annulation au milieu d'un `select!` : le futur abandonné peut
  l'être **après** avoir consommé des octets du flux, laissant le décodeur
  désynchronisé. Un point de reprise se conçoit, il ne s'improvise pas.

## Allocations

Pas de règle mécanique. Une seule qui tient :

**On ne remplace pas du code clair par du code rapide sans la mesure qui montre
que ça valait la peine** ([PERFORMANCE](../../docs/PERFORMANCE.md#la-règle-qui-empêche-loptimisation-gratuite)).

En revanche, sur un chemin **par ligne ou par valeur** — la conversion vers
`RecordBatch` en est un —, une allocation par élément est un défaut de
conception dès l'écriture, pas une optimisation à faire plus tard.

## API publiques

- tout élément public porte un `///` qui dit ce qui n'est pas dans la signature :
  les préconditions, ce qui panique, ce qui alloue, ce qui bloque ;
- `#[must_use]` sur ce dont ignorer le résultat est un bug ;
- pas de trait public à une seule implémentation qui n'est pas une frontière
  ([CLAUDE.md](../../CLAUDE.md#organisation-du-code)) ;
- un trait destiné à traverser la frontière WASM respecte les contraintes de
  [PLUGIN-CONTRACT](../../docs/PLUGIN-CONTRACT.md#ce-que-ce-contrat-impose-aux-traits-daujourdhui)
  **dès aujourd'hui** : les corriger en phase 4 coûtera une refonte.

## `unsafe`

Politique dans [SECURITY](../../docs/SECURITY.md#politique-unsafe). Le point qui
se rate : un `// SAFETY:` qui paraphrase le code ne vaut rien. Il dit **pourquoi**
la condition est vraie ici et **qui** la maintiendra vraie.

## Tests

Les conventions vivent dans [tests.md](tests.md) : ce qui se teste, les entrées
hostiles, les trois niveaux de test d'interface, les bancs d'essai.

Le renvoi est nécessaire parce que le `paths:` de cette règle-là ne couvre que
`**/tests/**` et `**/benches/**` : un `#[cfg(test)] mod tests` écrit au bas d'un
fichier source ne la déclenche pas. C'est pourtant la forme majoritaire ici.

## Vérifier

```bash
make qualite
```
