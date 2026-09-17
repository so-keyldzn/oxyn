---
name: outil-cluster-postgres-jetable
description: Monter, utiliser et arrêter un cluster PostgreSQL jetable pour les tests `#[ignore]` du driver, et les deux tests du dépôt qui y échouent sans être des régressions
metadata:
  type: reference
---

Les tests `#[ignore]` de `oxyn-driver-postgres` demandent `OXYN_PG_TEST_URL`.
**Ne pas compter sur un cluster existant** : celui de `/tmp` a été vidé entre deux
sessions (constaté le 2026-09-16). En recréer un dans le scratchpad de session.

Protocole vérifié le 2026-09-16 (PostgreSQL 17.11, Postgres.app) :

```sh
export PATH=/Applications/Postgres.app/Contents/Versions/latest/bin:$PATH
initdb -D "$SP/pg/data" -U oxyn_test --auth=trust -E UTF8 --no-locale
pg_ctl -D "$SP/pg/data" -l "$SP/pg/server.log" -o "-p 55439 -h 127.0.0.1 -k ''" start -w
env OXYN_PG_TEST_URL="postgres://oxyn_test@127.0.0.1:55439/postgres" \
  cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
pg_ctl -D "$SP/pg/data" stop -m fast -w   # dans tous les cas
```

`-k ''` désactive la socket Unix : le chemin du scratchpad dépasse la longueur
maximale d'un chemin de socket. Rôle `oxyn_test` en `trust`, aucun mot de passe.

**Tests rouges qui ne sont pas des régressions** (au 2026-09-16) :
`la_lecture_seule_est_imposee_par_le_serveur` et
`un_contexte_declare_ne_desarme_pas_la_lecture_seule` cherchent « lecture
seule » alors que le message est passé en anglais. Autrefois aussi
`previews_handle_system_types_and_preserve_native_columns` (littéral
`'=r/postgres'::aclitem`, rôle absent) ; il passe sur un cluster neuf.

Voir [[piege-bassin-postgres-et-cache-sqlx]] et [[piege-annulation-fenetre-deterministe]].
