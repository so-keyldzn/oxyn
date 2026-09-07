# Vérification des aperçus PostgreSQL — 2026-09-07

La correction couvre les captures de `pg_database`, `pg_attrdef`,
`pg_aggregate` et l'affichage des colonnes `timestamptz`.

## Changements

- Préparation asynchrone et annulable de l'aperçu : lecture des types de
  colonnes, puis projection citée. Les types internes, les types sans sortie
  binaire et les alias d'OID sont convertis explicitement en texte côté serveur.
  La résolution suit les domaines et les éléments de tableaux.
- Conservation des types natifs des autres colonnes et des limites de lecture.
  La variante Redshift conserve sa composition précédente.
- Les types binaires inconnus restent des octets Arrow avec leur nom de type.
  Ils ne sont plus interprétés comme du texte ni tronqués lors du décodage.
- Activation du support des fuseaux nommés d'Arrow pour la grille et les exports.

## Contrôles exécutés

Le dépôt contenait déjà une fusion non résolue au début du travail. Une copie
temporaire de `HEAD`, complétée par les fichiers de cette correction, a permis
de vérifier celle-ci sans résoudre ni remplacer les changements préexistants.
Les résultats ci-dessous ne valident donc pas l'intégration complète de la fusion.

| Contrôle | Résultat |
|---|---|
| `cargo fmt --all`, puis contrôle du format dans la copie | Succès |
| `cargo test -q -p oxyn-driver-postgres -p oxyn-data -p oxyn-exec -p oxyn-driver-sqlite --lib` | 323 réussis, 13 tests PostgreSQL ignorés par défaut |
| Tests ignorés de `oxyn-driver-postgres`, avec serveur temporaire et exécution séquentielle | 13 réussis, aucun ignoré |
| Régression des aperçus après ajout du domaine ACL et du contrôle des octets/OID | Succès |
| Clippy ciblé, `--no-deps --all-targets -- -D warnings` | Succès sur les quatre crates ci-dessus |
| `cargo doc --no-deps` des quatre crates et d'`oxyn-driver`, avec `RUSTDOCFLAGS=-D warnings` | Succès |
| `make qualite` dans le dépôt principal | Échec au formatage : conflits préexistants dans `oxyn-app` et `oxyn-ui` |

Les tests de hooks ont réussi leurs 42 cas et le socle a passé, avec son
avertissement préexistant sur le motif `paths:` de la règle de tests.
Clippy sans `--no-deps` a rencontré trois avertissements préexistants
`wrong_self_convention` dans `oxyn-catalog/src/model.rs`.

Les tests PostgreSQL ont utilisé un cluster jetable créé pour cette vérification,
sans accès à la connexion montrée sur les captures. Ils couvrent notamment
l'annulation vérifiée côté serveur, les trois catalogues, les noms de colonnes
contenant un guillemet, les valeurs nulles, les ACL imbriquées dans un domaine
et les noms de fonctions. Le test de flux de deux millions de lignes a aussi
été exécuté avec surveillance externe du processus : pic RSS échantillonné de
13 408 Kio, sous une borne d'arrêt de 128 Mio. Cette mesure ponctuelle ne vaut
pas validation de tous les budgets de performance du produit.

## Relecture et limites

Relecture locale des invariants et de la frontière driver : aucun nouveau
blocage identifié. Politique avant préparation, valeurs de métadonnées liées,
identifiants cités, annulation propagée, résultat en flux, aucun rejeu SQL.
Les colonnes explicitement converties pour l'aperçu sont annoncées comme texte.
Le SQL libre reste inchangé et peut encore rencontrer les limites de types de
SQLx ; ce correctif ne remplace pas son protocole de décodage.

Contradiction documentaire préexistante signalée : `DRIVER-CONTRACT.md` interdit
la dépendance des drivers à `oxyn-core`, alors que leurs manifestes et la liste
de revue driver la prévoient. Cette décision de découpage n'est pas modifiée ici.

L'application graphique n'a pas été reconstruite ni revalidée visuellement.
La porte `make qualite` complète reste à franchir après résolution de la fusion.
