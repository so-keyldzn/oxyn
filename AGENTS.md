# Oxyn — consignes pour Codex

Ces consignes s'appliquent uniquement à ce dépôt. Ne pas modifier la
configuration globale de Codex pour les installer.

## Source commune

Avant de travailler, lire [CLAUDE.md](CLAUDE.md) : présentation du produit,
langue, documents d'autorité, invariants I-01 à I-13 et organisation du code.
Ces consignes métier s'appliquent aussi à Codex. Les adaptations d'outillage
ci-dessous remplacent les indications propres à Claude Code.

Code, identifiants, commentaires et erreurs en anglais ; documentation, ADR,
commits et échanges en français. Les documents de `docs/` font autorité sur
le domaine ; signaler toute contradiction avec le code.

## Début de tâche

Lire `git status --short`, puis les manifestes et le plan d'implémentation
pertinents pour établir l'état réel. Préserver les changements préexistants.
Ne pas supposer que le dépôt est vide à partir d'une ancienne note.
Le hook Claude `SessionStart` n'est pas activé par cette adaptation.
Les versions effectives se lisent dans `rust-toolchain.toml`, `Cargo.toml`
et `Cargo.lock` ; leur justification dans `docs/RESEARCH-NOTES.md`.

## Règles à lire avant de modifier ou créer un fichier

Le frontmatter `paths:` des règles Claude ne déclenche aucun chargement dans
cette adaptation Codex. Lire explicitement toutes les règles applicables,
y compris pour un fichier neuf. Résoudre les liens depuis le fichier qui
les contient ; les chemins de commandes shell partent de la racine du dépôt.

| Fichiers concernés | Règle commune |
|---|---|
| Tout fichier Rust | [rust](.claude/rules/rust.md) |
| `drivers/oxyn-driver-*/**`, `crates/oxyn-driver/**`, `crates/oxyn-driver-*/**` | [drivers](.claude/rules/drivers.md) |
| `apps/desktop/**`, `crates/oxyn-desktop/**` | [front](.claude/rules/front.md) |
| `crates/oxyn-ai/**` | [ia](.claude/rules/ia.md) |
| Tout répertoire `tests/` ou `benches/` | [tests](.claude/rules/tests.md) |
| Markdown, y compris les compétences de projet | [documentation](.claude/rules/documentation.md) |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `Makefile`, `deny.toml` | [manifestes](.claude/rules/manifestes.md) |

## Procédures et compétences

Les dix procédures communes de [.claude/commands/](.claude/commands/) sont
accessibles par les compétences locales décrites dans
[.agents/README.md](.agents/README.md), par exemple `$oxyn-driver`,
`$oxyn-commande` et `$oxyn-relire`.
Pour un driver, une commande du bus ou un écran, lire la procédure spécialisée
avant de coder, même si la demande n'invoque pas explicitement la compétence.

Dans les procédures partagées :

- `/nom` signifie lire et suivre `.claude/commands/nom.md`, ou utiliser
  `$oxyn-nom` ; ce n'est pas une commande slash Codex à exécuter.
- `$ARGUMENTS` désigne la demande et le périmètre donnés par l'utilisateur.
- Les blocs marqués `!` sont des commandes à exécuter explicitement si utiles,
  avec les outils disponibles et les permissions de la session.
- `allowed-tools`, `tools`, `model`, `memory`, `permissions.allow` et les
  événements de hooks sont des métadonnées Claude, pas une configuration Codex.
- Les profils de [.claude/agents/](.claude/agents/) servent de guides de
  spécialité. Lire ceux qui concernent la tâche ; effectuer leurs vérifications
  localement. Déléguer seulement si l'utilisateur ou les instructions de la
  session le demandent et si les outils sont disponibles. Ne pas prétendre
  avoir lancé un agent ou activé sa mémoire. Une relecture produit des constats,
  sans modifier les sources relues ; les corrections sont une étape distincte.

## Permissions et contrôles

Cette adaptation n'installe aucun hook Codex et ne transpose pas les permissions
de `.claude/settings.json`. Les hooks Claude ne bloquent donc pas les outils
Codex. `make socle` teste ces hooks ; il ne les installe pas et ne constitue pas
un audit automatique de tout le code Rust.

Respecter les invariants lors des écritures et de la relecture. En particulier,
ne pas lire ni exposer les secrets (`.env`, `.env.*`, clés privées, certificats
privés, `secrets/`, `.ssh/`, identifiants Cargo ou AWS). Ne pas contourner les
contrôles avec `--no-verify`, un push forcé ou un script téléchargé puis exécuté.
Les permissions effectives sont celles de la session ; une procédure du dépôt
ne vaut pas autorisation de publication ni d'accès à une base réelle.

## Fin de tâche

Après modification Rust, formater avec `cargo fmt --all`, puis exécuter
`make qualite`. Pour toute modification, suivre la
[liste de fin de tâche](.claude/checklists/fin-de-tache.md), en adaptant les
relectures comme indiqué ci-dessus. Ne pas créer de commit sans demande.
Si un commit est demandé : `type(portee): sujet`, sujet français en minuscule,
sans point final, première ligne de 72 caractères maximum.

Rapporter les contrôles réellement exécutés et leurs résultats. Si une commande
échoue ou ne peut pas tourner, donner la cause et la limite de validation ;
ne pas annoncer une porte de qualité franchie sans succès effectif.
