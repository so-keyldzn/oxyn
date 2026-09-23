- [Piège rustdoc : liens explicites redondants](piege-rustdoc-liens-explicites-redondants.md) — seul l'étage doc de `make qualite` le voit ; `cargo test` et `clippy` se taisent
- [Piège hook : `derive(Debug)` refusé sur un nom de type](piege-hook-derive-debug-nom-de-type.md) — le critère est le *nom* ; `TokenDetails` est un faux positif classique
- [Piège cargo : oxyn-desktop sans cible lib](piege-cargo-oxyn-desktop-sans-lib.md) — tests ciblés avec `--bins`, pas `--lib`
- [Piège vitest : port 63315 occupé](piege-vitest-port-occupe.md) — échec front passager quand un autre vitest tourne ; vérifier et relancer
- [Piège Edit : `\uXXXX` décodé](piege-edit-sequence-u-decodee.md) — une séquence ` ` écrite via Edit devient le vrai caractère ; la construire par `format!`
- [Piège reqwest : délai de connexion](piege-reqwest-delai-de-connexion.md) — `is_connect()` et `is_timeout()` tous deux vrais ; tester la connexion d'abord
