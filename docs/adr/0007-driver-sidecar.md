# ADR-0007 — Processus sidecar pour les drivers à dépendances natives

**Statut :** proposé · **Date :** 2026-09-05

## Contexte
Oracle (OCI), Couchbase et certains SDK cloud reposent sur des bibliothèques C. Un
segfault ou une fuite mémoire dans une de ces dépendances emporterait tout le workspace
et le travail non sauvegardé de l'utilisateur.

## Décision
Ces drivers tournent dans `oxyn-driverd`, processus séparé exposant les mêmes traits
par-dessus un transport local. Les résultats transitent en **Arrow IPC** (zéro-copie).
Les drivers en Rust pur restent en processus.

## Conséquences
* **+** Un crash ne dégrade qu'une connexion ; le sidecar redémarre à chaud.
* **+** Isolation mémoire, et possibilité de limiter les ressources par processus.
* **+** Le surcoût de frontière est marginal grâce à Arrow IPC.
* **−** Complexité de cycle de vie : supervision, redémarrage, propagation d'annulation.
* **−** Distribution plus lourde (binaire supplémentaire, signature, notarisation macOS).
* Reporté en phase 4 : aucun driver de phase 0-3 n'en a besoin.
