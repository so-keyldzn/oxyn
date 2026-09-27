# Oxyn — instructions for Codex

These instructions apply to this repository only. Do not modify the global
Codex configuration to install them.

## Common source

Before working, read [CLAUDE.md](CLAUDE.md): product overview, language,
authoritative documents, invariants I-01 to I-13 and code organization. These
domain instructions apply to Codex too. The tooling adaptations below replace
the indications specific to Claude Code.

The repository is in English — code, identifiers, comments, errors,
documentation, ADRs, commits; French documents in `docs/` stay authoritative
until translated, and `i18n/fr/` holds French mirrors, English being
authoritative ([ADR-0047](docs/adr/0047-english-as-the-repository-language.md)).
The documents of `docs/` are authoritative on the domain; report any
contradiction with the code.

## Start of task

Read `git status --short`, then the relevant manifests and implementation plan
to establish the real state. Preserve pre-existing changes. Do not assume the
repository is empty based on an old note. The Claude `SessionStart` hook is not
enabled by this adaptation. The effective versions are read in
`rust-toolchain.toml`, `Cargo.toml` and `Cargo.lock`; their justification in
`docs/RESEARCH-NOTES.md`.

## Rules to read before modifying or creating a file

The `paths:` frontmatter of Claude rules triggers no loading in this Codex
adaptation. Explicitly read all applicable rules, including for a new file.
Resolve links from the file that contains them; shell command paths start from
the repository root.

| Files concerned | Common rule |
|---|---|
| Any Rust file | [rust](.claude/rules/rust.md) |
| `drivers/oxyn-driver-*/**`, `crates/oxyn-driver/**`, `crates/oxyn-driver-*/**` | [drivers](.claude/rules/drivers.md) |
| `apps/desktop/**`, `crates/oxyn-desktop/**` | [front](.claude/rules/front.md) |
| `crates/oxyn-ai/**` | [ia](.claude/rules/ia.md) |
| Any `tests/` or `benches/` directory | [tests](.claude/rules/tests.md) |
| Markdown, including project skills | [documentation](.claude/rules/documentation.md) |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `Makefile`, `deny.toml` | [manifestes](.claude/rules/manifestes.md) |

## Procedures and skills

The ten common procedures of [.claude/commands/](.claude/commands/) are
reachable through the local skills described in
[.agents/README.md](.agents/README.md), for example `$oxyn-driver`,
`$oxyn-commande` and `$oxyn-relire`.
For a driver, a bus command or a screen, read the specialized procedure before
coding, even if the request does not explicitly invoke the skill.

In the shared procedures:

- `/name` means read and follow `.claude/commands/name.md`, or use
  `$oxyn-name`; it is not a Codex slash command to run.
- `$ARGUMENTS` stands for the request and the scope given by the user.
- Blocks marked `!` are commands to run explicitly when useful, with the
  available tools and the session's permissions.
- `allowed-tools`, `tools`, `model`, `memory`, `permissions.allow` and hook
  events are Claude metadata, not Codex configuration.
- The profiles in [.claude/agents/](.claude/agents/) serve as specialty
  guides. Read those relevant to the task; perform their checks locally.
  Delegate only if the user or the session instructions ask for it and if the
  tools are available. Do not claim to have launched an agent or enabled its
  memory. A review produces findings, without modifying the reviewed sources;
  corrections are a separate step.

## Permissions and checks

This adaptation installs no Codex hook and does not transpose the permissions
of `.claude/settings.json`. Claude hooks therefore do not block Codex tools.
`make socle` tests these hooks; it does not install them and is not an
automatic audit of all the Rust code.

Respect the invariants when writing and reviewing. In particular, do not read
or expose secrets (`.env`, `.env.*`, private keys, private certificates,
`secrets/`, `.ssh/`, Cargo or AWS credentials). Do not bypass checks with
`--no-verify`, a forced push or a downloaded-then-executed script. The
effective permissions are those of the session; a repository procedure is not
an authorization to publish nor to access a real database.

## End of task

After a Rust change, format with `cargo fmt --all`, then run `make qualite`.
For any change, follow the
[end-of-task checklist](.claude/checklists/fin-de-tache.md), adapting the
reviews as indicated above. Do not create a commit unless asked. If a commit is
asked for: `type(scope): subject`, English subject in lowercase, imperative, no
final period, first line of 72 characters at most.

Report the checks actually run and their results. If a command fails or cannot
run, give the cause and the limit of validation; do not announce a quality gate
passed without actual success.
