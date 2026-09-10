---
name: outil-cluster-postgres-jetable
description: Le cluster PostgreSQL jetable des tests `#[ignore]` — comment le démarrer/arrêter, et le test du dépôt qui échoue dessus pour une raison qui n'est pas une régression
metadata:
  type: reference
---

Les tests `#[ignore]` de `oxyn-driver-postgres` demandent `OXYN_PG_TEST_URL`.
Un cluster jetable local existe : son répertoire est écrit dans le fichier
`/tmp/oxyn-constraints-pg-location` (sous-dossier `data`, port dans `port`).
`pg_ctl` vient de `Postgres.app` (`/Applications/Postgres.app/Contents/Versions/latest/bin`).

Protocole vérifié le 2026-09-10 (PostgreSQL 17.11) :

```sh
DIR=$(cat /tmp/oxyn-constraints-pg-location); PORT=$(cat "$DIR/port")
pg_ctl -D "$DIR/data" status   # avant tout start
pg_ctl -D "$DIR/data" -l "$DIR/server.log" -o "-p $PORT -k $DIR -h 127.0.0.1" start -w
env OXYN_PG_TEST_URL="postgres://oxyn_test@127.0.0.1:$PORT/postgres" \
  cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
pg_ctl -D "$DIR/data" stop -m fast -w   # dans tous les cas
```

Le rôle est **`oxyn_test`**, pas `postgres`, et `trust` local.

**Le piège** : `integration::previews_handle_system_types_and_preserve_native_columns`
échoue sur ce cluster avec `role "postgres" does not exist` — sa fixture écrit
un littéral `'=r/postgres'::aclitem`. Ce n'est **pas** une régression, et son
échec laisse derrière lui `oxyn_preview_types` et le domaine `oxyn_preview_acl`
à supprimer à la main (les autres tests nettoient les leurs).
