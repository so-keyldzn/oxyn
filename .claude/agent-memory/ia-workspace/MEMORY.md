- [Piège rustdoc : liens explicites redondants](piege-rustdoc-liens-explicites-redondants.md) — seul l'étage doc de `make qualite` le voit ; `cargo test` et `clippy` se taisent
- [Piège hook : `derive(Debug)` refusé sur un nom de type](piege-hook-derive-debug-nom-de-type.md) — le critère est le *nom* ; `TokenDetails` est un faux positif classique
- [Piège cargo : oxyn-desktop sans cible lib](piege-cargo-oxyn-desktop-sans-lib.md) — tests ciblés avec `--bins`, pas `--lib`
- [Piège vitest : port 63315 occupé](piege-vitest-port-occupe.md) — échec front passager quand un autre vitest tourne ; vérifier et relancer
- [Piège Edit : `\uXXXX` décodé](piege-edit-sequence-u-decodee.md) — une séquence ` ` écrite via Edit devient le vrai caractère ; la construire par `format!`
- [Piège reqwest : délai de connexion](piege-reqwest-delai-de-connexion.md) — `is_connect()` et `is_timeout()` tous deux vrais ; tester la connexion d'abord
- [Piège Base UI : `initialFocus` à l'ouverture seulement](piege-base-ui-initialfocus-a-l-ouverture.md) — un corps remonté sous un dialogue ouvert perd le focus
- [Piège : cargo-nextest absent](piege-nextest-absent.md) — la porte retombe sur `cargo test` ; cibler avec `-p`, pas `nextest run`
- [Piège trait : méthode par défaut non relayée](piege-methode-par-defaut-non-relayee.md) — une enveloppe hérite du défaut au lieu de déléguer ; le compilateur se tait
