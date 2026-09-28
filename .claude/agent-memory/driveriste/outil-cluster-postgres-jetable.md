---
name: outil-cluster-postgres-jetable
description: Set up, use and stop a disposable PostgreSQL cluster for the driver's `#[ignore]` tests, and the two repository tests that fail there without being regressions
metadata:
  type: reference
---

The `#[ignore]` tests of `oxyn-driver-postgres` require `OXYN_PG_TEST_URL`.
**Do not count on an existing cluster**: the one in `/tmp` was wiped between two
sessions (observed on 2026-09-16). Recreate one in the session scratchpad.

Protocol checked on 2026-09-16 (PostgreSQL 17.11, Postgres.app):

```sh
export PATH=/Applications/Postgres.app/Contents/Versions/latest/bin:$PATH
initdb -D "$SP/pg/data" -U oxyn_test --auth=trust -E UTF8 --no-locale
pg_ctl -D "$SP/pg/data" -l "$SP/pg/server.log" -o "-p 55439 -h 127.0.0.1 -k ''" start -w
env OXYN_PG_TEST_URL="postgres://oxyn_test@127.0.0.1:55439/postgres" \
  cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
pg_ctl -D "$SP/pg/data" stop -m fast -w   # in every case
```

`-k ''` disables the Unix socket: the scratchpad path exceeds the maximum length
of a socket path. `oxyn_test` role in `trust`, no password.

**Red tests that are not regressions** (as of 2026-09-16):
`read_only_is_enforced_by_the_server` and
`a_declared_context_does_not_disarm_read_only` look for "lecture seule"
while the message has switched to English. Formerly also
`previews_handle_system_types_and_preserve_native_columns` (literal
`'=r/postgres'::aclitem`, missing role); it passes on a fresh cluster.

See [[piege-bassin-postgres-et-cache-sqlx]] and [[piege-annulation-fenetre-deterministe]].
