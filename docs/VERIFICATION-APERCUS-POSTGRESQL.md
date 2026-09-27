# Verification of PostgreSQL previews — 2026-09-07

The fix covers the screenshots of `pg_database`, `pg_attrdef`,
`pg_aggregate` and the display of `timestamptz` columns.

## Changes

- Asynchronous and cancellable preparation of the preview: reading the column
  types, then a quoted projection. Internal types, types without binary output
  and OID aliases are explicitly converted to text on the server side.
  Resolution follows domains and array elements.
- The native types of the other columns and the read limits are kept.
  The Redshift variant keeps its previous composition.
- Unknown binary types stay Arrow bytes with their type name.
  They are no longer interpreted as text nor truncated during decoding.
- Support for Arrow named time zones is enabled for the grid and exports.

## Checks run

The repository already contained an unresolved merge when the work started. A
temporary copy of `HEAD`, completed with the files of this fix, made it possible
to verify it without resolving or replacing the pre-existing changes.
The results below therefore do not validate the complete integration of the merge.

| Check | Result |
|---|---|
| `cargo fmt --all`, then format check in the copy | Success |
| `cargo test -q -p oxyn-driver-postgres -p oxyn-data -p oxyn-exec -p oxyn-driver-sqlite --lib` | 323 passed, 13 PostgreSQL tests ignored by default |
| Ignored tests of `oxyn-driver-postgres`, with a temporary server and sequential execution | 13 passed, none ignored |
| Preview regression after adding the ACL domain and the bytes/OID check | Success |
| Targeted Clippy, `--no-deps --all-targets -- -D warnings` | Success on the four crates above |
| `cargo doc --no-deps` of the four crates and of `oxyn-driver`, with `RUSTDOCFLAGS=-D warnings` | Success |
| `make qualite` in the main repository | Failure at formatting: pre-existing conflicts in `oxyn-app` and `oxyn-ui` |

The hook tests passed their 42 cases and the foundation passed, with its
pre-existing warning on the `paths:` pattern of the tests rule.
Clippy without `--no-deps` hit three pre-existing
`wrong_self_convention` warnings in `oxyn-catalog/src/model.rs`.

The PostgreSQL tests used a throwaway cluster created for this verification,
without access to the connection shown in the screenshots. They cover in
particular cancellation verified on the server side, the three catalogs, column
names containing a quote, null values, ACLs nested in a domain
and function names. The two-million-row streaming test was also
run with external monitoring of the process: sampled RSS peak of
13,408 KiB, under a stop bound of 128 MiB. This one-off measurement does not
amount to validating all the product's performance budgets.

## Review and limits

Local review of the invariants and of the driver boundary: no new
blocker identified. Policy before preparation, bound metadata values,
quoted identifiers, propagated cancellation, streamed result, no SQL replay.
The columns explicitly converted for the preview are announced as text.
Free SQL is unchanged and can still hit SQLx's type limits;
this fix does not replace its decoding protocol.

Pre-existing documentation contradiction reported: `DRIVER-CONTRACT.md` forbids
drivers from depending on `oxyn-core`, whereas their manifests and the driver
review list provide for it. This split decision is not changed here.

The graphical application was neither rebuilt nor visually revalidated.
The complete `make qualite` gate remains to be passed after the merge is resolved.
